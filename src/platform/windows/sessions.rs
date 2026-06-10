//! [PLATFORM:WINDOWS] Logon sessions — replaces utmp, using the Remote Desktop Services API.

use std::io;
use std::ptr;

use windows_sys::Win32::System::RemoteDesktop::{
    WTSActive, WTSEnumerateSessionsW, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
    WTSClientAddress, WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW,
};

use crate::platform::types::SessionInfo;

pub fn sessions() -> io::Result<Vec<SessionInfo>> {
    let mut out = Vec::new();
    // SAFETY: pair Enumerate/Free and Query/Free per the WTS API conventions
    unsafe {
        let mut info_ptr: *mut WTS_SESSION_INFOW = ptr::null_mut();
        let mut count: u32 = 0;
        if WTSEnumerateSessionsW(
            WTS_CURRENT_SERVER_HANDLE,
            0,
            1,
            &mut info_ptr,
            &mut count,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sessions = std::slice::from_raw_parts(info_ptr, count as usize);
        for s in sessions {
            // only list active (logged-in) sessions
            if s.State != WTSActive {
                continue;
            }
            let user = query_string(s.SessionId, WTSUserName).unwrap_or_default();
            if user.is_empty() {
                continue;
            }
            let line = wide_to_string_ptr(s.pWinStationName);
            let host = query_string(s.SessionId, WTSClientAddress).filter(|h| !h.is_empty());
            out.push(SessionInfo {
                user,
                line,
                host,
                login_time: None, // [PORT:WINDOWS] login time requires WTSINFO (WTSQuerySessionInformation), omitted
                idle: None,
            });
        }
        WTSFreeMemory(info_ptr as *mut _);
    }
    Ok(out)
}

unsafe fn wide_to_string_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    // SAFETY: the caller guarantees p is a NUL-terminated wide string or null
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(p, len) };
    String::from_utf16_lossy(slice)
}

unsafe fn query_string(
    session_id: u32,
    info_class: windows_sys::Win32::System::RemoteDesktop::WTS_INFO_CLASS,
) -> Option<String> {
    // SAFETY: pair Query/Free; the buffer and length are written back by the API
    unsafe {
        let mut buf: *mut u16 = ptr::null_mut();
        let mut bytes: u32 = 0;
        if WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session_id,
            info_class,
            &mut buf as *mut _ as *mut _,
            &mut bytes,
        ) == 0
        {
            return None;
        }
        let s = wide_to_string_ptr(buf);
        WTSFreeMemory(buf as *mut _);
        Some(s)
    }
}
