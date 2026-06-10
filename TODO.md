# TODO — procps-rs 參數與原版 procps-ng 4.0.6 完全對齊

追蹤 18 支工具的命令列參數對齊進度。原則:
- 三平台 CLI 一律接受所有原版旗標(`--help` 與原版一致)。
- Linux 走真實實作;Windows/macOS 對 Linux-only 旗標執行時印出明確「此平台不支援」。
- 每支工具補齊 `-V/--version`,輸出 `<tool> 0.1.0 (procps-rs, 移植自 procps-ng 4.0.6)`。

## 必修語意衝突缺陷(對齊即修正)
- [x] tload:`-s` 改為 `--scale`(縮放),新增 `-d/--delay`
- [x] pgrep:`-F` 改為 `--pidfile`(讀 PID 檔);移除 fixed-string 概念(pgrep 永遠 ERE)
- [x] pkill:移除原版不存在的 `-F/--fixed`
- [x] w:`-h` 改為 `--no-header`(help 僅 `--help`)
- [x] matcher.rs:移除 `MatchOpts.regex/fixed`,改全正規表達式(改用 Selection)

---

## Phase 0 — 基線 ✅(完成於 2026-06-10)
- [x] 18 支工具 + 跨平台 platform 抽象層
- [x] ratatui top + 真實逐核 CPU(NtQuerySystemInformation)
- [x] 三平台建置驗證(Windows / Linux WSL / macOS check)

## Phase 1 — 共用基礎建設 ✅(完成於 2026-06-10)
- [x] `src/common.rs`:`version_string()`、Linux-only 不支援提示、pidfile 讀取
- [x] `ProcessInfo` 補欄位:sid / pgrp / ruid / rgid / euid / egid / cgroup
- [x] Linux 後端填新欄位(/proc stat、status、cgroup)
- [x] Windows / macOS 後端新欄位填 None
- [x] MemInfo 補 committed / commit_limit(free -v 用)
- [~] 全工具加 `-V/--version`(各工具於所屬 phase 改寫時加入);w 的 `-h` 改 no-header ✅

## Phase 2 — 簡單工具完整對齊 ✅(完成於 2026-06-10)
- [x] free:`--kilo/--mega/--giga/--tera/--peta`、`--tebi/--pebi`、`--si`、`-l/--lohi`、`-L/--line`、`-v/--committed`、`-w/--wide`、`-V`
- [x] uptime:`-c/--container`、`-r/--raw`、`-V`
- [x] w:`-c`、`-u/--no-current`、`-t/--terminal`、`-f/--from`、`-o/--old-style`、`-i/--ip-addr`、`-p/--pids`、`-V`;修 `-h`
- [x] tload:修 `-s`→`--scale`、新增 `-d/--delay`、`-V`
- [x] vmstat:`-a/-f/-m/-n/-s/-d/-D/-p/-w/-t/-y`、`-V`

## Phase 3 — 行程選取工具完整對齊 ✅(完成於 2026-06-10)
- [x] matcher.rs 擴充選取條件(pgrp/sid/uid/gid/runstate/older/newest/oldest/pidfile)+ 單元測試
- [x] pgrep:完整選取旗標集 + `-d/-a/--quiet/-w/-Q` + `-V`
- [x] pkill:同選取集 + `-H/-q/-m/-e` + `-V`;移除 `-F/--fixed`
- [x] pidof:`-c/-q/-w/-x/-o/-t` + `-V`
- [x] pidwait:選取子集 + `-e` + `-V`
- [x] kill:`-L/--table`、`-l` optional-arg、`-q/--queue`、`-V`

## Phase 4 — 檢視工具對齊 ✅(完成於 2026-06-10)
- [x] pmap:`-X/-XX/-d/-q/-p/-k/-A/-r`、`-V`(rc 檔旗標屬罕用,接受面留待)
- [x] pwdx:`-V`

## Phase 5 — watch 對齊 ✅(完成於 2026-06-10)
- [x] `-C/-f/-q/-r/-s/-w`、`-V`(`-q/--equexit`、`-f/--follow`、`-C/--no-color` 已實作行為)

## Phase 6 — sysctl / slabtop / hugetop 對齊 ✅(完成於 2026-06-10)
- [x] sysctl:`-a/-A/--deprecated/--dry-run/-b/-e/-N/-n/-p/--system/-r/-q/-w/-o/-x`、`-V`(Linux 實作)
- [x] slabtop:`-d/-o/--human/-s`、`-V`(Linux 實作 + 排序)
- [x] hugetop:`-d/-n/-o/-H`、`-V`(Linux 實作 + NUMA)

## Phase 7 — top 命令列旗標對齊 ✅(完成於 2026-06-10)
- [x] `-b`(批次)`/-n/-p/-u/-U/-o/-H/-i/-c/-w/-O/-s`、`-V`;保留 TUI 與 `--snapshot`
- [x] 批次模式純文字輸出(腳本可用);-p/-u 過濾、-o 排序、-i 隱藏閒置已實作

## Phase 8 — ps 實用子集對齊 ✅(完成於 2026-06-10)
- [x] UNIX 風格:`-e/-A/-f/-F/-l/-u/-U/-p/-o/-C/-H/-L/-T/-w/-N/-a/-d/-x/-j/-V`
- [x] BSD 風格:`a/u/x/aux/e/f/l/j/w/r` + 純數字 PID
- [x] GNU:`--help/--version/--sort/--pid/--ppid/--user/--no-headers/--format/--cols/--width/--forest/--deselect`
- [x] `-o` 核心格式關鍵字集(pid/ppid/user/uid/%cpu/%mem/vsz/rss/tty/stat/time/etime/comm/cmd/pri/ni/nlwp/pgid/sid…)
- [x] 預設/`-f`/`-l`/`u`/`-j` 格式;`--sort` 支援 +/- 升降冪
- [~] 其餘進階旗標/關鍵字:接受並忽略(完整三風格對等留作未來)

## Phase 9 — 驗證與文件 ✅(完成於 2026-06-10)
- [x] 每支工具 `--help`/`-V` 與原版對照;新旗標抽測
- [x] 三平台建置:Windows release(零警告)/ Linux WSL 編譯執行 / macOS cargo check 全通過
- [x] Linux 實測 Linux-only 旗標真實行為(sysctl -N -a、pgrep -G、free -v、ps -o sid,pgid,stat)
- [x] Windows 實測 Linux-only 旗標印出明確不支援
- [x] cargo test 通過(matcher 選取邏輯 7 項單元測試)
- [x] PORTING.md 增補「命令列參數對齊」章節
- [x] 本 TODO.md 收尾勾選
