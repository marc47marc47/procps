# procps

[procps-ng 4.0.6](https://gitlab.com/procps-ng/procps) 的 Rust 移植版,
以原生方式跨平台支援 **Windows / Linux / macOS**:

- **Windows**:不依賴 Cygwin/WSL,直接用 **Win32 API** 重寫(Toolhelp32、PEB 讀取、WTS、VirtualQueryEx…)
- **Linux**:解析 `/proc`,與原 C 版 libproc2 相同的資料來源
- **macOS**:抽象層介面已就緒,後端待補(見 [PORTING.md](PORTING.md))

原始 C 程式碼保留在 `procps-v4.0.6/` 供對照。

## 工具一覽(18 支)

`free` `uptime` `w` `tload` `vmstat` `pgrep` `pkill` `pidof` `pidwait`
`kill` `ps` `pmap` `pwdx` `watch` `top` `sysctl` `slabtop` `hugetop`

各工具的平台支援程度見 [PORTING.md 的支援矩陣](PORTING.md#工具支援矩陣)。

## 建置

```sh
cargo build --release
```

產物在 `target/release/`,每支工具是獨立執行檔。

## 設計

工具層(`src/bin/`)只呼叫 `procps::platform` 抽象層,
由編譯期 `cfg` 選擇 Windows / Linux / macOS 後端。
所有平台專屬程式碼以 `[PLATFORM:*]` / `[PORT:*]` 標注,方便擴充與移植。
詳見 [PORTING.md](PORTING.md)。

## 授權

GPL-2.0-or-later(沿用 procps-ng)。
