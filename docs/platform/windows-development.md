# Windows 开发指南

这份文档针对想要在 Windows 上开发、打包或运行 Hope Agent 的人。如果你只想用预编译的 `.msi` / `.exe`，跳到[安装预编译版本](#安装预编译版本)即可。

## 平台支持矩阵

| 模式 | macOS | Linux | Windows |
| --- | :---: | :---: | :---: |
| 桌面 GUI (`hope-agent`) | ✅ | ✅ | ✅ |
| 守护进程 (`hope-agent server`) | ✅ (launchd) | ✅ (systemd) | ✅ (Task Scheduler) |
| ACP stdio (`hope-agent acp`) | ✅ | ✅ | ✅ |
| 浏览器自动化 | ✅ | ✅ | ✅ (自动探测 Chrome / Edge) |
| 系统代理自动探测 | ✅ (scutil) | ❌ (读 `HTTPS_PROXY` env) | ✅ (注册表) |
| 天气本机定位 | ✅ (CoreLocation) | ❌ (IP 定位) | ❌ (IP 定位) |
| iMessage 渠道 | ✅ | ❌ | ❌ |
| signal-cli 渠道 | ✅ | ✅ | ⚠️ 需显式配 `channels.signal.cli_path` 指向 `signal-cli.bat` |
| WeChat / Telegram / Discord / Slack 等其他渠道 | ✅ | ✅ | ✅ |

## 安装预编译版本

Release 产物由 [`.github/workflows/release.yml`](../../.github/workflows/release.yml) 在 tag push 时自动构建。下载对应版本的 `.msi`（推荐）或 `.exe` (NSIS) 安装即可。

首次启动时，如果系统缺少 WebView2 Runtime（Win10 1809 以前或精简版系统），安装器会自动下载并安装 Bootstrapper。

## 本地开发

### 前置

1. **Visual Studio 2022 Build Tools**（C++ workload + Windows 10/11 SDK）
   - 下载：<https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022>
   - 必选组件：`Desktop development with C++`、`Windows 11 SDK`
2. **Rust (MSVC toolchain)**
   ```powershell
   rustup default stable-msvc
   rustup target add x86_64-pc-windows-msvc
   ```
3. **Node.js 20+**（安装时勾选 "Add to PATH"）
4. **WebView2 Runtime**（Win11 自带；Win10 需[手动装 Evergreen Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)）
5. **NASM**（可选但强烈推荐）——`openssl-src` 编译 vendored OpenSSL 时需要
   ```powershell
   choco install nasm
   # 或从 https://www.nasm.us/ 下载后把安装目录加进 PATH
   ```

### 第一次构建

```powershell
# 克隆并进入目录
git clone https://github.com/shiwenwen/hope-agent.git
cd hope-agent

npm install
cargo check --workspace    # 建议先跑一次，第一次编译 vendored openssl 会慢 2-3 分钟

# 开发模式（前端热重载 + Tauri 窗口）
npm run tauri dev
```

### 常见坑

- **长路径**：Windows 默认 `MAX_PATH=260`。`target/` 目录层级深，偶发长路径错误。开启长路径支持：
  ```powershell
  # 管理员 PowerShell
  git config --global core.longpaths true
  New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force
  ```
- **Windows Defender 实时保护**：会反复扫描 `target/`，本地构建能慢 3-5 倍。加排除目录：
  ```powershell
  # 管理员 PowerShell
  Add-MpPreference -ExclusionPath "$(Resolve-Path .\target)"
  ```
- **`openssl-src` 编译失败 `perl: command not found`**：Strawberry Perl 没装。GitHub Actions `windows-latest` 自带，本地需自己装（`choco install strawberryperl`）。
- **`Cannot find wix toolset`**：`.msi` 打包需要 WiX Toolset。Tauri CLI 会自动下载，但网络受限时可能失败。可手动装 WiX 3.14 + 设 `WIX` 环境变量。

## 运行 Server 模式

```powershell
# 前台运行
hope-agent server start

# 注册为用户级 Task Scheduler 任务（下次登录自动启动，立即 /Run 一次）
hope-agent server install

# 查看状态（解析 schtasks /Query 输出）
hope-agent server status

# 停止当前进程（taskkill 走 server.pid）
hope-agent server stop

# 卸载任务
hope-agent server uninstall
```

设计说明：Windows 上 `install` 注册的是一个 **Task Scheduler 任务**（不是真正的 Windows Service），和 macOS `launchd` + Linux `systemctl --user` 保持行为一致——都是"用户登录后台自启"，不走 SCM dispatcher，不需要管理员权限。如果你需要真正的 Windows Service（开机即启、无需登录），手动用 [nssm](https://nssm.cc/) 包装：

```powershell
nssm install Hope Agent "C:\Program Files\Hope Agent\hope-agent.exe" "server" "--bind" "127.0.0.1:8420"
```

## CI 验证

- PR 自动跑 `.github/workflows/rust.yml`：`ubuntu-latest` + `windows-latest` + `macos-14` 三平台矩阵的 `cargo clippy -D warnings` + `cargo test`
- 打 `v*` tag 触发 [`.github/workflows/release.yml`](../../.github/workflows/release.yml)：三平台 tauri-action 并行构建 → 自动创建 draft release 上传 `.msi` / `.exe` / `.dmg` / `.AppImage` / `.deb`。代码签名证书目前为空（`tauri.conf.json` 的 `bundle.windows.certificateThumbprint` 字段），需要时在 repo secret 里加 `WINDOWS_CERTIFICATE` + `WINDOWS_CERTIFICATE_PASSWORD` 并调 `tauri-action` 的 signing 参数。

## 已知限制

- **iMessage 渠道**：macOS-only，Windows 上面板隐藏该 Channel 类型
- **CoreLocation 定位**：macOS-only；Windows 退化到 IP 定位（仍能拿到城市级经纬度）
- **ConPTY**：PTY 模式 (`exec pty=true`) 需要 Windows 10 1809 以上；旧系统会 `app_warn` 并建议调用方关 PTY 重试
- **Windows Service**：目前 `server install` 注册的是 Task Scheduler 任务而非真正的系统服务——不支持"开机即启、无需登录"场景。用 nssm 绕过
