# procps-rs 移植說明(Porting Guide)

本專案將 **procps-ng 4.0.6**(C 語言,原始碼在 `procps-v4.0.6/`)移植為 Rust,
並以「跨平台抽象層 + 各平台後端」的結構,讓同一份工具程式可在
**Windows(原生 Win32)/ Linux(/proc)/ macOS(待補)** 上編譯執行。

## 設計原則

1. **工具層與平台層分離。**
   每支工具(`src/bin/*.rs`)只呼叫 `procps::platform` 的抽象函式,完全不含平台條件編譯。
   要新增平台,只需實作後端模組,工具層零改動。

2. **資料語意以 Linux /proc 為基準。**
   抽象型別(`src/platform/types.rs`)的欄位定義對齊 Linux;
   其他平台後端負責把原生資料「翻譯」成這些欄位,拿不到的欄位用 `Option` 或 `0` 表示。

3. **平台專屬程式碼一律標注**,方便日後搜尋與移植:
   | 標注 | 意義 |
   |------|------|
   | `[PLATFORM:WINDOWS]` | Windows 專屬實作(Win32 API) |
   | `[PLATFORM:LINUX]`   | Linux 專屬實作(/proc) |
   | `[PLATFORM:MACOS]`   | macOS 專屬實作 |
   | `[PORT:MACOS]` 等     | 待移植點,註明建議使用的原生 API |

## 架構

```
src/
├── lib.rs              # crate 入口
├── units.rs            # 數值格式化(human_bytes、format_uptime)
├── matcher.rs          # pgrep/pkill/pidof 共用的行程比對
├── platform/
│   ├── mod.rs          # 後端選擇(cfg)+ 對照表
│   ├── types.rs        # 跨平台共用型別(libproc2 對應)
│   ├── windows/        # [PLATFORM:WINDOWS] Win32 後端
│   │   ├── mem.rs      #   GlobalMemoryStatusEx, GetPerformanceInfo
│   │   ├── cpu.rs      #   GetSystemTimes, GetTickCount64
│   │   ├── process.rs  #   Toolhelp32 + OpenProcess + PEB 讀取 + VirtualQueryEx
│   │   └── sessions.rs #   WTSEnumerateSessionsW
│   ├── linux.rs        # [PLATFORM:LINUX] /proc 解析(同 C 版 libproc2 資料來源)
│   └── macos.rs        # [PLATFORM:MACOS] 介面骨架(多數待補)
└── bin/                # 18 支工具,每支一個 binary
```

## 平台 API 對照表

| 抽象函式 | Linux 來源 | Windows (Win32) | macOS 建議 API |
|----------|-----------|-----------------|----------------|
| `mem_info` | /proc/meminfo | GlobalMemoryStatusEx + K32GetPerformanceInfo | host_statistics64 + sysctl hw.memsize |
| `cpu_times` | /proc/stat | GetSystemTimes | host_processor_info |
| `per_cpu_times` | /proc/stat cpuN | **NtQuerySystemInformation**(SystemProcessorPerformanceInformation)真實逐核 | host_processor_info per-CPU |
| `uptime` | /proc/uptime | GetTickCount64 | sysctl kern.boottime |
| `loadavg` | /proc/loadavg | **無**(回 None) | getloadavg(3) ✔ 已實作 |
| `list_processes` | /proc/[pid]/* | Toolhelp32 + OpenProcess + PEB | sysctl KERN_PROC + libproc |
| `process_cwd` | /proc/[pid]/cwd | PEB ProcessParameters.CurrentDirectory | proc_pidinfo(PROC_PIDVNODEPATHINFO) |
| `process_maps` | /proc/[pid]/maps | VirtualQueryEx + K32GetMappedFileNameW | mach_vm_region |
| `kill` / `wait` | kill(2) | TerminateProcess / WaitForSingleObject | kill(2) / kqueue |
| `sessions` | utmp | WTSEnumerateSessionsW | utmpx |
| `vm_counters` | /proc/vmstat,/proc/stat | 大多無對應(回 None) | host_statistics64 |

## 工具支援矩陣

| 工具 | Windows | Linux | macOS | 備註 |
|------|:-------:|:-----:|:-----:|------|
| free | ✅ | ✅ | 🟡 | Windows 無 buffers 欄 |
| uptime | ✅ | ✅ | 🟡 | Windows 無 load average |
| w | ✅ | ✅ | 🟡 | Windows 無 idle/JCPU/PCPU |
| tload | ✅ | ✅ | 🟡 | Windows 改用 CPU% 繪圖 |
| vmstat | 🟡 | ✅ | 🟡 | Windows 多數 io/system 欄為 '-' |
| pgrep | ✅ | ✅ | 🟡 | |
| pkill | ✅ | ✅ | 🟡 | Windows 訊號語意受限 |
| pidof | ✅ | ✅ | 🟡 | |
| pidwait | ✅ | ✅ | 🟡 | Windows 用 WaitForSingleObject |
| kill | ✅ | ✅ | 🟡 | Windows 僅 TERM/KILL/INT/HUP/QUIT |
| ps | ✅ | ✅ | 🟡 | Windows STAT 無 R/S/D/Z |
| pmap | ✅ | ✅ | 🟡 | |
| pwdx | ✅ | ✅ | 🟡 | Windows 需權限讀 PEB |
| watch | ✅ | ✅ | ✅ | 不依賴 /proc,三平台完整 |
| top | ✅ | ✅ | 🟡 | **ratatui TUI** + 完整互動按鍵(見下) |
| sysctl | ❌ | ✅ | 🟡 | Linux-only(/proc/sys) |
| slabtop | ❌ | ✅ | ❌ | Linux-only(/proc/slabinfo) |
| hugetop | ❌ | ✅ | ❌ | Linux-only(HugePages) |

✅ 完整 · 🟡 可執行但部分欄位 N/A 或待補 · ❌ 平台無對應概念,執行時印出明確說明

## 重要平台差異

### 訊號(Signal)語意
Windows **沒有 POSIX 訊號**。`platform::Signal` 的映射:
- `Check`(kill -0):僅用 OpenProcess 探測存在與權限,不送訊號
- `Kill`/`Term`/`Int`/`Hup`/`Quit`:一律 `TerminateProcess`(強制終止,**無法被攔截**,等同 SIGKILL)
- `Stop`/`Cont`/`Usr1`/`Usr2` 等:回 `Unsupported` 錯誤

因此 Windows 上 `kill -HUP` **不會**觸發目標程式的重載設定行為,而是直接終止它——使用時需留意。

### 命令列與工作目錄(PEB 讀取)
Windows 沒有 /proc/[pid]/cmdline。本專案以
`NtQueryInformationProcess(ProcessBasicInformation)` 取得目標行程 PEB 位址,
再用 `ReadProcessMemory` 逐層讀出 `RTL_USER_PROCESS_PARAMETERS` 的
`CommandLine` 與 `CurrentDirectory`(偏移為 x64 佈局,見 `process.rs`)。
跨權限等級(如讀取更高權限行程)會因權限不足而失敗,屬預期行為。

### VSZ / RSS 對應
- RSS ← Windows `WorkingSetSize`(實體佔用,語意接近)
- VSZ ← Windows `PagefileUsage`(commit charge,**與 Linux vsize 不完全等價**)

### load average
Windows 無此概念。最接近的 `\System\Processor Queue Length`(PDH)語意不同,
故 `loadavg()` 回 `None`,uptime/w/top 顯示 `n/a`。

### 真實逐核 CPU(top 的逐核量表)
Windows 透過 `NtQuerySystemInformation(SystemProcessorPerformanceInformation, class 8)`
取得每顆邏輯處理器的 IdleTime/KernelTime/UserTime(100ns;KernelTime 已含 IdleTime),
與 Linux 的 /proc/stat cpuN 等價。此 API 半文件化但廣泛使用且穩定;
若呼叫失敗會自動退回「總量÷核數」近似,確保 top 仍可運作。

### top 的 TUI 與互動按鍵
以 [ratatui](https://ratatui.rs) 實作,底層 crossterm 在三平台一致
(Windows 走 ConPTY)。`top --snapshot` 為內建除錯模式:以 TestBackend 渲染單一畫面
並輸出純文字,可在無互動終端機的環境(如 CI、自動化)驗證版面;
`top --snapshot --keys "1m"` 可在渲染前套用一連串按鍵以驗證切換狀態。

互動按鍵(對齊原版 top,盡可能完整實作):

| 鍵 | 功能 | 鍵 | 功能 |
|----|------|----|------|
| q/Esc | 離開 | Enter | 立即刷新 |
| Space | 暫停 | P/M/N/T | 依 CPU/MEM/PID/TIME 排序 |
| R | 反向排序 | `<` `>` | 切換排序欄 |
| ↑↓ PgUp/Dn Home/End | 選取移動 | +/- | 調整刷新頻率 |
| k | 終止選取行程(提示訊號) | r | renice(提示 nice 值) |
| d/s | 設定刷新秒數 | u/U | 過濾使用者 |
| n/# | 限制顯示行數 | L | 搜尋字串(高亮) |
| i | 隱藏閒置 | H | 執行緒模式(本實作以行程為單位) |
| c | 命令列/程式名 | 1 | 逐核/整體 CPU |
| m | 記憶體區開關 | l/t | 摘要列開關 |
| I | Irix/Solaris CPU% 模式 | 0 | 隱藏 0 值 |
| x | 標示排序欄 | y | 標示執行中行程 |
| b | 粗體 | = | 重設全部 |
| ?/h | 說明覆蓋層 | W | 存設定(no-op) |

需要輸入的指令(k/r/d/u/n/L)會在底列顯示提示,Enter 確認、Esc 取消。

[PLATFORM:WINDOWS] `r`(renice)無 POSIX nice,改用 `SetPriorityClass` 把 nice 值
映射到 Windows 優先權類別(HIGH/ABOVE_NORMAL/NORMAL/BELOW_NORMAL/IDLE);
`y`(標示執行中)依行程狀態 `R`,Windows 無此狀態故不作用。

## 命令列參數對齊(與原版 procps-ng 4.0.6)

18 支工具的旗標已對齊原版(詳見 `TODO.md` 的分階段紀錄)。原則:

- **三平台一律接受所有原版旗標**,`--help`/`-V` 輸出與原版一致。
- **Linux-only 概念**(pgrep `-g/-G/-s/-r/--cgroup`、vmstat `-d/-D/-p/-m`、sysctl 全部、
  slabtop、hugetop、kill `-q`、uptime `-c`…)在 Linux 走真實實作;
  Windows/macOS 接受旗標但執行時印出明確的「此平台不支援」(`common::unsupported_note`)。
- **版本字串**統一為 `<tool> <ver> (procps-rs, 移植自 procps-ng 4.0.6)`(`common::version_string`)。

修正過的語意衝突(對齊原版):
- tload `-s` 原為縮放(scale),延遲是 `-d`(原本誤用 `-s` 當延遲)
- pgrep `-F` 為 `--pidfile`(讀 PID 檔);pgrep 永遠是 ERE,無 fixed-string 模式
- w `-h` 為 `--no-header`(help 僅 `--help`)

ps 採三風格(UNIX `-ef` / BSD `aux` / GNU `--sort`)實用子集 + `-o` 核心格式關鍵字;
完整三風格對等(~270 關鍵字)列為未來工作。選取所需的 sid/pgrp/uid/gid/cgroup 欄位
由 `ProcessInfo` 提供(Linux 由 /proc 填入,Windows/macOS 為 None)。

## 如何補完 macOS 後端

`src/platform/macos.rs` 已備妥所有函式骨架(目前多回 `Unsupported`,但 `loadavg`/`kill` 已可用)。
逐一把 `todo(...)` 換成對照表中的原生 API 實作即可,工具層完全不用改。
建議優先順序:`mem_info` → `cpu_times` → `uptime` → `list_processes`。

## 建置

```sh
# Windows(原生 Win32)
cargo build --release

# Linux
cargo build --release            # 在 Linux 或 WSL 中

# macOS(需在 macOS 上,或補完後端後交叉編譯)
cargo build --release

# 跨平台型別檢查(從任一平台)
cargo check --target x86_64-apple-darwin
cargo check --target x86_64-unknown-linux-gnu
```

每支工具是獨立 binary,輸出於 `target/release/`(Windows 為 `.exe`)。
