//! [PLATFORM:WINDOWS] Process enumeration and operations — replaces /proc/[pid]/* and kill(2).
//!
//! How data is obtained:
//! - Listing, ppid, thread count, RSS/VSZ, cumulative CPU time: NtQuerySystemInformation
//!   (SystemProcessInformation). This is a single system-wide snapshot that requires no
//!   per-process handle, so it covers every process on the machine (including SYSTEM-owned
//!   services, "Registry", "Secure System", "Memory Compression", ...) even without admin
//!   rights — unlike CreateToolhelp32Snapshot + OpenProcess, which silently leaves RSS/CPU
//!   zeroed for any process the caller can't open. CreateToolhelp32Snapshot is kept only as
//!   a fallback if the NT call ever fails.
//! - Command line / working directory: NtQueryInformationProcess to get the PEB address, then ReadProcessMemory
//!   to read out RTL_USER_PROCESS_PARAMETERS level by level (CommandLine / CurrentDirectory)
//! - Owner: OpenProcessToken + GetTokenInformation(TokenUser) + LookupAccountSidW
//! - Memory regions (pmap): VirtualQueryEx + K32GetMappedFileNameW
//! - At startup we best-effort enable SeDebugPrivilege (no-op / silently fails unless running
//!   elevated) so that, when possible, OpenProcess can also reach protected/SYSTEM processes
//!   for owner/exe/cmdline — the same trick Task Manager/Process Explorer use.

use std::ffi::c_void;
use std::io;
use std::mem;
use std::path::PathBuf;
use std::sync::Once;
use std::time::{Duration, SystemTime};

use windows_sys::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessInformation,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, LUID, STATUS_INFO_LENGTH_MISMATCH,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LookupAccountSidW, LookupPrivilegeValueW,
    SE_DEBUG_NAME, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQueryEx,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetMappedFileNameW, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ, TerminateProcess,
    WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::SYSTEM_PROCESS_INFORMATION;

use super::{filetime_to_ms, filetime_to_systemtime, wide_to_string};
use crate::platform::types::{MemRegion, ProcessInfo, Signal};

/// RAII wrapper: automatically CloseHandle on leaving scope.
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: only closes the valid handle we own
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn open(pid: u32, access: u32) -> Option<Handle> {
    // SAFETY: OpenProcess returns NULL on failure, which is checked
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() { None } else { Some(Handle(h)) }
}

/// Best-effort: enable SeDebugPrivilege in our own token so OpenProcess can reach
/// protected/SYSTEM-owned processes too (mirrors what Task Manager/Process Explorer do
/// when "Run as administrator"). Silently does nothing when not elevated — our token
/// simply won't have the privilege to enable, and AdjustTokenPrivileges reports that
/// via GetLastError rather than a hard failure we need to react to.
fn enable_debug_privilege() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: all out-parameters are properly sized and initialized before use
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) == 0 {
                return;
            }
            let token = Handle(token);
            let mut luid: LUID = mem::zeroed();
            if LookupPrivilegeValueW(std::ptr::null(), SE_DEBUG_NAME, &mut luid) == 0 {
                return;
            }
            let mut priv_: TOKEN_PRIVILEGES = mem::zeroed();
            priv_.PrivilegeCount = 1;
            priv_.Privileges[0].Luid = luid;
            priv_.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            AdjustTokenPrivileges(token.0, 0, &priv_, 0, std::ptr::null_mut(), std::ptr::null_mut());
        }
    });
}

/// A single process entry read from the system-wide NT snapshot (or, as fallback, Toolhelp).
struct RawProc {
    pid: u32,
    ppid: u32,
    name: String,
    threads: u32,
    rss_bytes: u64,
    vsz_bytes: u64,
    utime_ms: u64,
    stime_ms: u64,
    start_time: Option<SystemTime>,
}

/// Convert a raw 100ns-tick count (as found in SYSTEM_PROCESS_INFORMATION) into a FILETIME
/// so it can go through the existing filetime_to_ms / filetime_to_systemtime helpers.
fn raw_100ns_to_filetime(t: i64) -> FILETIME {
    let t = t as u64;
    FILETIME { dwLowDateTime: (t & 0xFFFF_FFFF) as u32, dwHighDateTime: (t >> 32) as u32 }
}

/// Read a UNICODE_STRING that points into our own `NtQuerySystemInformation` buffer.
fn image_name(name: &windows_sys::Win32::Foundation::UNICODE_STRING, pid: u32) -> String {
    if name.Buffer.is_null() {
        return match pid {
            0 => "System Idle Process".to_string(),
            4 => "System".to_string(),
            _ => format!("[pid {pid}]"),
        };
    }
    // SAFETY: Buffer/Length come from a kernel-filled UNICODE_STRING inside a buffer we own
    unsafe {
        let slice = std::slice::from_raw_parts(name.Buffer, name.Length as usize / 2);
        String::from_utf16_lossy(slice)
    }
}

/// System-wide process snapshot via NtQuerySystemInformation(SystemProcessInformation).
///
/// Unlike CreateToolhelp32Snapshot, this single call also returns each process's thread
/// count, working-set size, commit charge, and cumulative kernel/user CPU time directly —
/// no per-process OpenProcess needed, so SYSTEM-owned/protected processes are fully
/// populated even when we can't open a handle to them.
fn snapshot_nt() -> io::Result<Vec<RawProc>> {
    let mut buffer_size: usize = 512 * 1024;
    loop {
        let mut buf: Vec<u8> = vec![0u8; buffer_size];
        let mut needed: u32 = 0;
        // SAFETY: buf is sized `buffer_size` bytes and that size is passed through
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buf.as_mut_ptr() as *mut c_void,
                buffer_size as u32,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            buffer_size = if needed == 0 { buffer_size * 2 } else { needed as usize + 64 * 1024 };
            continue;
        }
        if status < 0 {
            return Err(io::Error::other(format!(
                "NtQuerySystemInformation(SystemProcessInformation) failed: {status:#x}"
            )));
        }

        let mut out = Vec::new();
        let mut offset: isize = 0;
        loop {
            // SAFETY: offset stays within `buf`, which is large enough per the successful
            // call above; NextEntryOffset is kernel-provided and walks entry by entry
            unsafe {
                let p = buf.as_ptr().offset(offset) as *const SYSTEM_PROCESS_INFORMATION;
                let pi = &*p;

                let pid = pi.UniqueProcessId as usize as u32;
                // `Reserved2` is InheritedFromUniqueProcessId (a HANDLE-sized PPID) —
                // windows-sys folds this undocumented-ish field into a reserved slot.
                let ppid = pi.Reserved2 as usize as u32;
                // `Reserved1` packs WorkingSetPrivateSize(8) + HardFaultCount(4) +
                // NumberOfThreadsHighWatermark(4) + CycleTime(8) + CreateTime(8) +
                // UserTime(8) + KernelTime(8) = 48 bytes, in that order.
                let create_time = i64::from_ne_bytes(pi.Reserved1[24..32].try_into().unwrap());
                let user_time = i64::from_ne_bytes(pi.Reserved1[32..40].try_into().unwrap());
                let kernel_time = i64::from_ne_bytes(pi.Reserved1[40..48].try_into().unwrap());

                out.push(RawProc {
                    pid,
                    ppid,
                    name: image_name(&pi.ImageName, pid),
                    threads: pi.NumberOfThreads,
                    rss_bytes: pi.WorkingSetSize as u64,
                    vsz_bytes: pi.PagefileUsage as u64,
                    utime_ms: filetime_to_ms(raw_100ns_to_filetime(user_time)),
                    stime_ms: filetime_to_ms(raw_100ns_to_filetime(kernel_time)),
                    start_time: filetime_to_systemtime(raw_100ns_to_filetime(create_time)),
                });

                if pi.NextEntryOffset == 0 {
                    break;
                }
                offset += pi.NextEntryOffset as isize;
            }
        }
        return Ok(out);
    }
}

/// Toolhelp snapshot listing all processes (name / pid / ppid / threads) — fallback used
/// only if the NT-based snapshot above ever fails.
fn snapshot_toolhelp() -> io::Result<Vec<RawProc>> {
    // SAFETY: the snapshot handle is checked, then closed after the loop
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snap = Handle(snap);
        let mut entry: PROCESSENTRY32W = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut out = Vec::new();
        if Process32FirstW(snap.0, &mut entry) != 0 {
            loop {
                out.push(RawProc {
                    pid: entry.th32ProcessID,
                    ppid: entry.th32ParentProcessID,
                    name: wide_to_string(&entry.szExeFile),
                    threads: entry.cntThreads,
                    rss_bytes: 0,
                    vsz_bytes: 0,
                    utime_ms: 0,
                    stime_ms: 0,
                    start_time: None,
                });
                if Process32NextW(snap.0, &mut entry) == 0 {
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// All processes on the system, preferring the complete NT-based snapshot and falling back
/// to Toolhelp32 only if that call is ever unavailable (e.g. blocked by policy).
fn snapshot() -> io::Result<Vec<RawProc>> {
    snapshot_nt().or_else(|_| snapshot_toolhelp())
}

fn process_times(h: HANDLE) -> Option<(u64, u64, Option<std::time::SystemTime>)> {
    // SAFETY: the four FILETIME output parameters are valid
    unsafe {
        let mut creation: FILETIME = mem::zeroed();
        let mut exit: FILETIME = mem::zeroed();
        let mut kernel: FILETIME = mem::zeroed();
        let mut user: FILETIME = mem::zeroed();
        if GetProcessTimes(h, &mut creation, &mut exit, &mut kernel, &mut user) == 0 {
            return None;
        }
        Some((
            filetime_to_ms(user),
            filetime_to_ms(kernel),
            filetime_to_systemtime(creation),
        ))
    }
}

fn process_memory(h: HANDLE) -> Option<(u64, u64)> {
    // SAFETY: PROCESS_MEMORY_COUNTERS is output-only
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = mem::zeroed();
        if K32GetProcessMemoryInfo(h, &mut pmc, mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32) != 0
        {
            // WorkingSetSize ≈ RSS; PagefileUsage ≈ commit charge (used as a VSZ approximation)
            Some((pmc.WorkingSetSize as u64, pmc.PagefileUsage as u64))
        } else {
            None
        }
    }
}

fn process_owner(h: HANDLE) -> Option<String> {
    // SAFETY: the token handle and buffer size are handled per the API conventions
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(h, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let token = Handle(token);
        let mut len = 0u32;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr() as *mut c_void, len, &mut len)
            == 0
        {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid = tu.User.Sid;
        let mut name = [0u16; 256];
        let mut name_len = name.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut sid_type = 0i32;
        if LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &mut name_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut sid_type,
        ) == 0
        {
            return None;
        }
        Some(wide_to_string(&name))
    }
}

#[allow(non_snake_case)]
mod nt {
    //! [PLATFORM:WINDOWS] Undocumented/semi-documented structures and calls needed to read the PEB.
    //! Struct layouts come from ntdll; we read only the offsets we need and pad the rest with placeholder fields.
    use super::*;
    use windows_sys::Wdk::System::Threading::{
        NtQueryInformationProcess, PROCESSINFOCLASS,
    };

    pub const PROCESS_BASIC_INFORMATION_CLASS: PROCESSINFOCLASS = 0; // ProcessBasicInformation

    #[repr(C)]
    pub struct PROCESS_BASIC_INFORMATION {
        pub ExitStatus: i32,
        pub PebBaseAddress: *mut c_void,
        pub AffinityMask: usize,
        pub BasePriority: i32,
        pub UniqueProcessId: usize,
        pub InheritedFromUniqueProcessId: usize,
    }

    /// Read the PEB address from the target process.
    pub fn peb_address(h: HANDLE) -> Option<*mut c_void> {
        // SAFETY: standard use of NtQueryInformationProcess(ProcessBasicInformation)
        unsafe {
            let mut pbi: PROCESS_BASIC_INFORMATION = mem::zeroed();
            let mut ret = 0u32;
            let status = NtQueryInformationProcess(
                h,
                PROCESS_BASIC_INFORMATION_CLASS,
                &mut pbi as *mut _ as *mut c_void,
                mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                &mut ret,
            );
            if status < 0 {
                return None;
            }
            Some(pbi.PebBaseAddress)
        }
    }
}

unsafe fn read_mem<T: Copy>(h: HANDLE, addr: *const c_void) -> Option<T> {
    let mut val: T = unsafe { mem::zeroed() };
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            h,
            addr,
            &mut val as *mut T as *mut c_void,
            mem::size_of::<T>(),
            &mut read,
        )
    };
    if ok != 0 && read == mem::size_of::<T>() {
        Some(val)
    } else {
        None
    }
}

unsafe fn read_wstr(h: HANDLE, addr: *const c_void, byte_len: usize) -> Option<String> {
    if addr.is_null() || byte_len == 0 || byte_len > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u16; byte_len / 2];
    let mut read = 0usize;
    let ok = unsafe {
        ReadProcessMemory(
            h,
            addr,
            buf.as_mut_ptr() as *mut c_void,
            byte_len,
            &mut read,
        )
    };
    if ok != 0 {
        // Some processes report a CommandLine/CurrentDirectory Length that includes the
        // NUL terminator (or padding); a literal embedded '\0' would otherwise reach
        // stdout as a raw NUL byte and make tools like `grep` treat the whole stream as
        // binary. A real Windows command line never legitimately contains one.
        Some(String::from_utf16_lossy(&buf[..read / 2]).replace('\0', ""))
    } else {
        None
    }
}

/// Read PEB → ProcessParameters → (CommandLine, CurrentDirectory).
/// Offsets are for the x64 layout: PEB+0x20 = ProcessParameters;
/// in RTL_USER_PROCESS_PARAMETERS, CurrentDirectory.DosPath (UNICODE_STRING) @0x38,
/// CommandLine (UNICODE_STRING) @0x70. UNICODE_STRING = { u16 Length; u16 Max; u8 pad[4]; u64 Buffer }.
fn read_peb_strings(h: HANDLE) -> (Option<String>, Option<String>) {
    #[cfg(target_pointer_width = "64")]
    const PARAMS_OFFSET: usize = 0x20;
    #[cfg(target_pointer_width = "64")]
    const CMDLINE_OFFSET: usize = 0x70;
    #[cfg(target_pointer_width = "64")]
    const CURDIR_OFFSET: usize = 0x38;

    // 32-bit layout (if compiling for a 32-bit target in the future)
    #[cfg(target_pointer_width = "32")]
    const PARAMS_OFFSET: usize = 0x10;
    #[cfg(target_pointer_width = "32")]
    const CMDLINE_OFFSET: usize = 0x40;
    #[cfg(target_pointer_width = "32")]
    const CURDIR_OFFSET: usize = 0x24;

    // SAFETY: all reads go through ReadProcessMemory and check the return value, returning None on failure
    unsafe {
        let Some(peb) = nt::peb_address(h) else {
            return (None, None);
        };
        let params: *mut c_void =
            match read_mem(h, (peb as usize + PARAMS_OFFSET) as *const c_void) {
                Some(p) => p,
                None => return (None, None),
            };

        // read UNICODE_STRING: Length (u16) and Buffer (pointer)
        let read_ustr = |field_off: usize| -> Option<String> {
            let len: u16 = read_mem(h, (params as usize + field_off) as *const c_void)?;
            let buf_ptr: *mut c_void = read_mem(
                h,
                (params as usize + field_off + mem::size_of::<usize>()) as *const c_void,
            )?;
            read_wstr(h, buf_ptr, len as usize)
        };

        let cmdline = read_ustr(CMDLINE_OFFSET);
        let curdir = read_ustr(CURDIR_OFFSET);
        (cmdline, curdir)
    }
}

/// Roughly split a command-line string into argv (not exactly equivalent to CommandLineToArgvW, but good enough for display).
fn split_cmdline(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in s.chars() {
        match c {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    args.push(mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        args.push(cur);
    }
    args
}

fn build_info(raw: RawProc) -> ProcessInfo {
    let mut info = ProcessInfo {
        pid: raw.pid,
        ppid: raw.ppid,
        name: raw.name,
        threads: raw.threads,
        rss_bytes: raw.rss_bytes,
        vsz_bytes: raw.vsz_bytes,
        utime_ms: raw.utime_ms,
        stime_ms: raw.stime_ms,
        start_time: raw.start_time,
        state: '?', // [PORT:WINDOWS] Windows has no R/S/D/Z process state; only individual threads do
        ..Default::default()
    };

    // Owner/exe/cmdline still need a live handle, which SYSTEM/protected processes may
    // deny even after enable_debug_privilege(); rss/vsz/times above already came from the
    // system-wide NT snapshot, so a denied handle here no longer means "no data at all".
    // First try full access (reading the PEB needs VM_READ + QUERY_INFORMATION), then fall back on failure.
    if let Some(h) = open(info.pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ) {
        fill_from_handle(&mut info, h.0, true);
    } else if let Some(h) = open(info.pid, PROCESS_QUERY_LIMITED_INFORMATION) {
        fill_from_handle(&mut info, h.0, false);
    }
    if info.cmdline.is_empty() {
        // fall back to using the image name as argv[0]
        info.cmdline = vec![
            info.exe
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| info.name.clone()),
        ];
    }
    info
}

fn fill_from_handle(info: &mut ProcessInfo, h: HANDLE, can_read_mem: bool) {
    // Prefer the fresher per-handle reading when available; otherwise keep the values
    // already populated from the NT snapshot in build_info().
    if let Some((u, s, start)) = process_times(h) {
        info.utime_ms = u;
        info.stime_ms = s;
        info.start_time = start;
    }
    if let Some((rss, vsz)) = process_memory(h) {
        info.rss_bytes = rss;
        info.vsz_bytes = vsz;
    }
    if let Some(owner) = process_owner(h) {
        info.user = owner;
    }
    // full image path
    if let Some(path) = full_image_path(h) {
        info.exe = Some(PathBuf::from(&path));
    }
    if can_read_mem {
        let (cmdline, _cwd) = read_peb_strings(h);
        if let Some(cl) = cmdline {
            info.cmdline = split_cmdline(&cl);
        }
    }
}

fn full_image_path(h: HANDLE) -> Option<String> {
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    // SAFETY: the buffer size is passed as an in/out parameter
    unsafe {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        if QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size) != 0 {
            Some(wide_to_string(&buf[..size as usize]))
        } else {
            None
        }
    }
}

pub fn list_processes() -> io::Result<Vec<ProcessInfo>> {
    enable_debug_privilege();
    let snap = snapshot()?;
    Ok(snap.into_iter().map(build_info).collect())
}

pub fn process_info(pid: u32) -> io::Result<ProcessInfo> {
    enable_debug_privilege();
    let snap = snapshot()?;
    snap.into_iter()
        .find(|p| p.pid == pid)
        .map(build_info)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no such pid: {pid}")))
}

/// /proc/[pid]/cwd equivalent: read the PEB CurrentDirectory.
pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
    let h = open(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ).ok_or_else(|| {
        io::Error::new(io::ErrorKind::PermissionDenied, "cannot open process (permission required)")
    })?;
    let (_cmdline, cwd) = read_peb_strings(h.0);
    cwd.map(PathBuf::from)
        .ok_or_else(|| io::Error::other("cannot read working directory"))
}

/// /proc/[pid]/maps equivalent: scan the address space with VirtualQueryEx.
pub fn process_maps(pid: u32) -> io::Result<Vec<MemRegion>> {
    let h = open(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "cannot open process"))?;
    let mut out = Vec::new();
    let mut addr: usize = 0;
    // SAFETY: VirtualQueryEx queries region by region; mbi is output-only
    unsafe {
        loop {
            let mut mbi: MEMORY_BASIC_INFORMATION = mem::zeroed();
            let n = VirtualQueryEx(
                h.0,
                addr as *const c_void,
                &mut mbi,
                mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            );
            if n == 0 {
                break;
            }
            if mbi.State == MEM_COMMIT {
                let perms = protect_to_perms(mbi.Protect);
                let mut mapping = None;
                let mut name = [0u16; 512];
                let len = K32GetMappedFileNameW(
                    h.0,
                    mbi.BaseAddress,
                    name.as_mut_ptr(),
                    name.len() as u32,
                );
                if len > 0 {
                    mapping = Some(wide_to_string(&name[..len as usize]));
                }
                out.push(MemRegion {
                    base: mbi.BaseAddress as usize,
                    size: mbi.RegionSize,
                    perms,
                    mapping,
                });
            }
            let next = (mbi.BaseAddress as usize).wrapping_add(mbi.RegionSize);
            if next <= addr {
                break;
            }
            addr = next;
        }
    }
    Ok(out)
}

fn protect_to_perms(protect: u32) -> String {
    let base = protect & 0xff;
    // windows-sys's PAGE_* are typed constants that would be treated as bindings in a match pattern; use an if chain for explicit comparison instead.
    let (r, w, x) = if base == PAGE_READONLY {
        (true, false, false)
    } else if base == PAGE_READWRITE || base == PAGE_WRITECOPY {
        (true, true, false)
    } else if base == PAGE_EXECUTE {
        (false, false, true)
    } else if base == PAGE_EXECUTE_READ {
        (true, false, true)
    } else if base == PAGE_EXECUTE_READWRITE || base == PAGE_EXECUTE_WRITECOPY {
        (true, true, true)
    } else {
        (false, false, false)
    };
    format!(
        "{}{}{}",
        if r { "r" } else { "-" },
        if w { "w" } else { "-" },
        if x { "x" } else { "-" }
    )
}

/// kill(2) equivalent.
/// [PLATFORM:WINDOWS] Windows has no POSIX signals:
/// - Check (-0): use OpenProcess only to probe existence and access
/// - Kill/Term/Int/Hup/Quit: always TerminateProcess (forced, cannot be intercepted)
/// - Stop/Cont/Usr1/Usr2 etc.: unsupported, return an Unsupported error
pub fn kill(pid: u32, sig: Signal) -> io::Result<()> {
    match sig {
        Signal::Check => {
            if open(pid, PROCESS_QUERY_LIMITED_INFORMATION).is_some() {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        Signal::Kill | Signal::Term | Signal::Int | Signal::Hup | Signal::Quit => {
            let h = open(pid, PROCESS_TERMINATE)
                .ok_or_else(|| io::Error::last_os_error())?;
            // SAFETY: the handle has TERMINATE access
            if unsafe { TerminateProcess(h.0, 1) } != 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "[PLATFORM:WINDOWS] signal {:?} has no equivalent semantics on Windows (only 0/TERM/KILL/INT/HUP/QUIT are supported)",
                sig
            ),
        )),
    }
}

pub fn process_exists(pid: u32) -> bool {
    open(pid, PROCESS_QUERY_LIMITED_INFORMATION).is_some()
}

/// Wait for a process to exit: OpenProcess + WaitForSingleObject(SYNCHRONIZE).
pub fn wait_process_exit(pid: u32, timeout: Option<Duration>) -> io::Result<bool> {
    // SYNCHRONIZE access right (windows-sys places it under Storage::FileSystem; use its value directly to avoid pulling in an extra feature)
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let Some(h) = open(pid, SYNCHRONIZE) else {
        // failure to open usually means it has already exited
        return Ok(true);
    };
    let ms = timeout.map(|t| t.as_millis() as u32).unwrap_or(u32::MAX);
    // SAFETY: the handle has SYNCHRONIZE access
    let r = unsafe { WaitForSingleObject(h.0, ms) };
    if r == WAIT_OBJECT_0 {
        Ok(true)
    } else if r == WAIT_TIMEOUT {
        Ok(false)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// renice equivalent: Windows has no POSIX nice, so set the process priority class (PriorityClass) instead.
/// Maps nice values (-20..19) to Windows priority classes.
pub fn set_nice(pid: u32, nice: i32) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::SetPriorityClass;
    const PROCESS_SET_INFORMATION: u32 = 0x0200;
    // Windows priority class constants
    const HIGH: u32 = 0x0000_0080;
    const ABOVE_NORMAL: u32 = 0x0000_8000;
    const NORMAL: u32 = 0x0000_0020;
    const BELOW_NORMAL: u32 = 0x0000_4000;
    const IDLE: u32 = 0x0000_0040;

    let class = if nice <= -10 {
        HIGH
    } else if nice < 0 {
        ABOVE_NORMAL
    } else if nice == 0 {
        NORMAL
    } else if nice <= 10 {
        BELOW_NORMAL
    } else {
        IDLE
    };

    let h = open(pid, PROCESS_SET_INFORMATION)
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "cannot open process (permission required)"))?;
    // SAFETY: the handle has SET_INFORMATION access
    if unsafe { SetPriorityClass(h.0, class) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
