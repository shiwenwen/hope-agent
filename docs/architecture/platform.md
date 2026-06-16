# 跨平台抽象层（Platform）

> 返回 [文档索引](../README.md) | 关联：[安全子系统](security.md) · [MCP 客户端](mcp.md) · [进程与并发模型](process-model.md)

[`platform/`](../../crates/ha-core/src/platform/) 是 ha-core 内部的 OS 适配层，所有"在 Unix 与 Windows 行为不同"的原语统一收敛于此。门面 [`mod.rs`](../../crates/ha-core/src/platform/mod.rs) 定义跨平台单一签名，[`unix.rs`](../../crates/ha-core/src/platform/unix.rs) / [`windows.rs`](../../crates/ha-core/src/platform/windows.rs) 各自给具体实现；较大的 OS 领域可以放到子模块（例如 [`service.rs`](../../crates/ha-core/src/platform/service.rs)、[`system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs)）。调用方一律走 `crate::platform::xxx()` 或稳定兼容 wrapper，**业务代码零 `#[cfg]` 分支**。

## 硬规则

- **新增跨平台原语统一放 `platform/`**，不要在业务代码里散落 `#[cfg(target_os = "...")]` / `#[cfg(unix)]` / `#[cfg(windows)]` 分支
- **优先 `#[cfg(unix)]` / `#[cfg(windows)]`** 而不是 `target_os = "linux"`——macOS + Linux + 各 BSD 共享 Unix 路径，少写一次 cfg = 少一类回归
- 调用方一律走 `crate::platform::xxx()` 单一入口
- **签名跨平台对齐**：返回值类型、参数顺序保持一致，让调用方完全不感知是哪个 impl 在执行

## 入口清单

| 入口 | Unix 实现 | Windows 实现 |
|---|---|---|
| `terminate_process_tree(pid: u32)` | `libc::kill(-(pid as i32), SIGKILL)` 杀整个进程组（要求 child spawn 时在 `pre_exec` 里 `setpgid(0,0)`） | `taskkill /F /T /PID <pid>` 走 job tree，`CREATE_NO_WINDOW` 不弹控制台 |
| `send_graceful_stop(pid: u32)` | `libc::kill(pid, SIGTERM)` —— 注意是 pid 不是 -pid，**不**杀整个组 | `taskkill /PID <pid>`（无 `/F`，发 WM_CLOSE / CTRL_BREAK），`CREATE_NO_WINDOW` |
| `detect_system_proxy() -> Option<String>` | `OnceLock` 进程级缓存；优先 env vars（`HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` 大小写），macOS 读 `scutil --proxy`，Linux / BSD 再试 GNOME `gsettings` 与 KDE `kreadconfig6` / `kreadconfig5` | 读 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`，`OnceLock` 进程级缓存避免每次构建 client 都重读注册表；解析 `ProxyEnable` + `ProxyServer`，支持 `http=...;https=...` 协议列表（优先 https） |
| `current_location() -> Option<(f64, f64)>` | macOS 走 CoreLocation；其他 Unix 返回 `None`，业务层继续 IP 定位降级 | 返回 `None`，业务层继续 IP 定位降级 |
| `pdfium_library_candidates() -> &'static [&'static str]` | macOS 返回 Homebrew / `/usr/local` dylib 候选；其他 Unix 返回常见 `.so` 候选 | 返回 `pdfium.dll` |
| `system_permissions_*` | macOS 走 TCC / framework 原生检查与授权触发；非 macOS 返回 unsupported / NotApplicable | 返回 unsupported / NotApplicable |
| `service::{install_service, uninstall_service, service_status, stop_server}` | macOS 写 LaunchAgent plist 并通过 `launchctl` 管理；Linux 写 user systemd unit 并通过 `systemctl --user` 管理；`stop_server` 读 `server.pid` 后发 SIGTERM | 通过 Task Scheduler 创建/删除/查询 per-user 登录任务；`stop_server` 读 `server.pid` 后走 `send_graceful_stop` |
| `default_shell_command(cmdline) -> std::process::Command` | `Command::new("sh").arg("-c").arg(cmdline)` | `Command::new("cmd").raw_arg("/C").raw_arg(cmdline)` —— `raw_arg` 跳过 std 自动加引号，保留 `/C` 后续整段命令的原始语义；并 `creation_flags(CREATE_NO_WINDOW)` 防控制台闪窗 |
| `default_shell_command_tokio(cmdline)` | 同上 std 版，返回 `tokio::process::Command` | 同上 std 版，返回 `tokio::process::Command`（同样带 `CREATE_NO_WINDOW`） |
| `hide_console(&mut std::process::Command)` | no-op（Unix 无控制台窗口概念） | `creation_flags(CREATE_NO_WINDOW=0x0800_0000)`——抑制 spawn 控制台子进程时一闪而过的 `cmd`/`conhost` 窗口，管道输出不受影响 |
| `hide_console_tokio(&mut tokio::process::Command)` | no-op | 同上，`tokio::process::Command` 版（异步 spawn 站点） |
| `os_version_string() -> String` | macOS 优先 `sw_vers -productVersion` → `"macOS 14.2.1"` 形态；其他 Unix 走 `sysinfo::System::long_os_version()` 兜底；都失败时 `"unknown"` | `sysinfo::long_os_version()` + `kernel_version()` 拼成 `"Windows 11 (26100)"` 形态；都缺失时 `"Windows (unknown build)"` |
| `write_secure_file(path, bytes) -> io::Result<()>` | `OpenOptions::create_new + mode(0o600) + write_all + sync_all` → `fs::set_permissions(0o600)`（防 umask 干扰）→ `rename(tmp, path)`，原子 + 0600 + fsync | 同样 temp file → `sync_all`；rename 前 `if path.exists() { remove_file }`（Windows rename 目标存在会失败）；NTFS DACL 继承自 `~/.hope-agent/` 目录（用户 profile 下默认仅 owner + SYSTEM/Administrators 可读） |
| `try_acquire_exclusive_lock(path) -> io::Result<Option<File>>` | `flock(LOCK_EX \| LOCK_NB)` 在 `O_CLOEXEC` 打开的文件上加非阻塞独占锁，`fork` 子进程不继承锁 fd；返回 `Ok(None)` 表示已被其他进程持有 | `OpenOptions::share_mode(0)`（`FILE_SHARE_NONE`）走内核独占打开 + `FILE_FLAG_NO_INHERIT_HANDLE`，`Err(io::ErrorKind::PermissionDenied)` 自动映射为 `Ok(None)` 表示锁已被占 |
| `find_chrome_executable() -> Option<PathBuf>` | 返回 `None`（`chromiumoxide` 自己的 `which` 已覆盖 macOS `.app` 与常见 Linux 路径） | 扫 `%ProgramFiles%` / `%ProgramFiles(x86)%` / `%LOCALAPPDATA%` × `Google\Chrome\Application\chrome.exe` / `Microsoft\Edge\Application\msedge.exe` / `Chromium\Application\chrome.exe`；用环境变量而不是硬编码 `C:\Program Files`，覆盖本地化 / ARM / 备用磁盘 / 用户级安装 |
| `detect_dedicated_gpu() -> Option<DetectedGpu>` | 优先 `nvidia-smi --query-gpu=name,memory.total` 拿权威 VRAM；失败时 macOS 直接返回 `None`（统一内存由 RAM 兜底），Linux 解析 `lspci` VGA/3D 行只回名字、VRAM 留空 | 优先 `nvidia-smi`；失败回落 PowerShell `Win32_VideoController`。注意 `AdapterRAM` 是 32 位字段在 ≥4 GiB 卡上会绕回，此时按 4096 MiB 保守下限上报。供 `local_llm` 选模型预算用 |

## 实现细节备忘

### 进程树 kill（Unix 进程组）

`terminate_process_tree` 给的是 `-(pid as i32)`，`kill(2)` 看到负数 pid 时把信号发到对应进程组（PGID）。要让这条路径有效，**spawn 子进程时必须在 `pre_exec` 里调 `setpgid(0, 0)`**——否则子进程默认共享父进程的 PGID，杀负数 pid 等于杀自己。Hope Agent 里 `tools::exec` / `subagent::spawn` / `cron::scheduler` / `acp_control::runtime_stdio` 等所有创建长跑子进程的入口都已就位，新加路径必须沿用同一约定。

`send_graceful_stop` 是单 pid，不带组，专门给"我自己 supervise 的 child，组级停由我额外控制"的场景。

### 安全写文件（atomic + 0600）

两端都遵循同一相位：

1. `create_dir_all(parent)`
2. 同目录写 temp file（名字 `tmp.<pid>.<nanos>`，避免并发写碰撞）
3. `write_all` + `sync_all`（强制 fsync 到 disk）
4. **Unix**：`set_permissions(0o600)` 二次显式收紧——`OpenOptions::mode(0o600)` 的初始位会被 umask 干扰，加这一步等于"无论 umask 多宽都强制 0600"
5. **Windows**：rename 前检查目标存在则先 remove（Windows rename 不像 POSIX 会自动 unlink 目标）
6. `rename(tmp, path)` 原子替换

**Windows ACL 当前依赖继承**：`~/.hope-agent/` 在用户 profile 下，默认 DACL 已经把"普通用户"挡在外面，但**没有显式 strip 继承的 ACE**。注释里明确点出"hardening to an explicit DACL is a future pass"——威胁建模需要时再加，签名不变向后兼容。

### 隐藏控制台窗口（Windows `CREATE_NO_WINDOW`）

Windows 上用 `std::process::Command` / `tokio::process::Command` spawn 一个**控制台子系统**程序（`git` / `docker` / `node` / `cmd` / `hostname` …）时，即使把 stdout/stderr 重定向走，系统仍会为该子进程**短暂闪出一个 `cmd`/`conhost` 控制台窗口**——在每轮对话都会跑 git/hostname 探测的桌面 GUI 上，表现为"发消息时黑窗一闪而过、还抢输入焦点"。`creation_flags(CREATE_NO_WINDOW)` 让子进程不分配控制台窗口，但保留管道，所以捕获输出照常。

约定：**任何在 Windows 上可能创建进程、且在正常使用路径上会跑的 `Command`，都必须经 `hide_console` / `hide_console_tokio`**（或本就带 flag 的 `run_hidden` / `default_shell_command*`）。`hide_console` 对**找不到程序而返回 `Err` 的调用是零成本 no-op**，所以判定标准取**就低不就高**——只要程序名**有可能**在某些 Windows 环境解析出真进程，就加。

**真正无需加**的只有两类：

1. 被 `#[cfg(unix)]` / `#[cfg(target_os="macos")]` 等非 Windows cfg 包住、根本不在 Windows 编译的站点（如 unix 专属的 `sh` / `pgrep` / `scutil` / `sw_vers` / `launchctl`）
2. 程序名是 **macOS/Linux 独有、Windows 上任何常见环境都不会有**的 binary（`scutil` / `osascript` / `gsettings` / `defaults` …，`Command::new` 找不到直接 `Err`）

> ⚠️ 不要把 `uname` / `date` / `hostname` 当成"Windows 上不存在"——**Git-for-Windows / MSYS2 / Cygwin / scoop coreutils 都带 `uname.exe` / `date.exe` / `hostname.exe`**（`hostname` 更是 System32 自带）。默认原生 PATH 通常解析不到 `uname` / `date`（落到 fallback），但只要应用从 Git Bash 启动或 PATH 上有 `Git\usr\bin`/MSYS2 就会真 spawn 闪窗。这类"可能解析"的站点（[`system_prompt/helpers.rs`](../../crates/ha-core/src/system_prompt/helpers.rs) 的 `hostname` / `uname` / `date`，每轮系统提示构建都跑）一律加 `hide_console`——加了没坏处，不加就是 Windows 上的偶发闪窗。

**例外（有意不加）**：

- [`guardian.rs`](../../crates/ha-core/src/guardian.rs) 重启自身 binary——前台 `hope-agent server start` 时子进程需要继承父控制台输出，加 flag 会吞掉用户期望看到的日志；桌面 GUI 是 `windows_subsystem=windows` 本就无控制台，与闪窗 bug 无关
- [`tools/exec.rs`](../../crates/ha-core/src/tools/exec.rs) 的 PTY 路径走 `portable-pty` 的 ConPTY，伪控制台不弹可见窗口，且 `CommandBuilder` 不暴露 `creation_flags` 钩子

### 系统代理缓存

`detect_system_proxy` 两端都用 `OnceLock<Option<String>>` 进程级缓存。理由：`provider/proxy.rs` / `docker/proxy.rs` 等 caller 每次构建 reqwest client 都会调一次，winreg / `scutil` / `gsettings` / `kreadconfig` 都不应该在 hot path 上重复探测。

如果用户在运行时改了系统代理，需要重启 Hope Agent 才能生效——这个 trade-off 有意为之，因为系统代理变更属于罕见配置事件，相比每次重读系统配置更划算。

### `os_version_string` 的 macOS 兜底

`sysinfo::long_os_version()` 在 macOS 上历史返回过 `"MacOS"`、`"Mac OS X"`、`"macOS"` 等不同字符串，且时常落后真实小版本号。所以 Unix 实现里 macOS 分支**优先**调 `sw_vers -productVersion` 拿权威小版本，失败才 fallback 到 sysinfo。Linux 直接 sysinfo——发行版差异由 sysinfo 自己处理。

## 调用方采样

| 入口 | 主要 caller |
|---|---|
| `terminate_process_tree` | [`tools/process.rs`](../../crates/ha-core/src/tools/process.rs) 强杀工具子进程 |
| `send_graceful_stop` | [`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs) IM 渠道进程优雅退出；[`acp_control/runtime_stdio.rs`](../../crates/ha-core/src/acp_control/runtime_stdio.rs) ACP runtime 关闭；[`service_install.rs`](../../crates/ha-core/src/service_install.rs) 系统服务卸载 |
| `detect_system_proxy` | [`provider/proxy.rs`](../../crates/ha-core/src/provider/proxy.rs) LLM 出站代理；[`docker/proxy.rs`](../../crates/ha-core/src/docker/proxy.rs) Docker 容器代理注入 |
| `current_location` | [`weather.rs`](../../crates/ha-core/src/weather.rs) 天气自动定位：系统精确定位失败后降级 IP 定位 |
| `pdfium_library_candidates` | [`file_extract.rs`](../../crates/ha-core/src/file_extract.rs) PDF 渲染 fallback 动态库查找 |
| `system_permissions_*` | [`permissions.rs`](../../crates/ha-core/src/permissions.rs) v2 系统权限目录的 OS 原生检查 / 请求入口 |
| `service::{install_service, uninstall_service, service_status, stop_server}` | [`service_install.rs`](../../crates/ha-core/src/service_install.rs) 保持历史 public API，CLI / updater / Tauri 继续从该 wrapper 进入系统服务管理 |
| `default_shell_command_tokio` | [`tools/exec.rs`](../../crates/ha-core/src/tools/exec.rs) 工具 shell 命令执行 |
| `hide_console` / `hide_console_tokio` | 所有在 Windows 会真实建进程的 `Command`：git 探测（[`filesystem/git.rs`](../../crates/ha-core/src/filesystem/git.rs) / [`session/environment.rs`](../../crates/ha-core/src/session/environment.rs) / [`plan/git.rs`](../../crates/ha-core/src/plan/git.rs)）、`hostname`（[`system_prompt/helpers.rs`](../../crates/ha-core/src/system_prompt/helpers.rs)）、docker（[`docker/`](../../crates/ha-core/src/docker/) 经 `docker_command()` 统一）、MCP stdio（[`mcp/transport.rs`](../../crates/ha-core/src/mcp/transport.rs)）、ACP backend（[`acp_control/`](../../crates/ha-core/src/acp_control/)）、IM sidecar（[`channel/process_manager.rs`](../../crates/ha-core/src/channel/process_manager.rs)）、Chrome（[`browser/spawn.rs`](../../crates/ha-core/src/browser/spawn.rs)）、`gh`、ollama / skill 安装 / hooks shell / 自升级冷烟自检 等 |
| `os_version_string` | [`agent/errors.rs`](../../crates/ha-core/src/agent/errors.rs) 错误报告 / 诊断；`self_diagnosis` 日志 |
| `write_secure_file` | [`mcp/credentials.rs`](../../crates/ha-core/src/mcp/credentials.rs) MCP OAuth token 凭据 0600 原子落盘（**当前唯一调用方**）。注意：主 LLM OAuth `oauth.rs::save_token()` 当前直接用 `std::fs::write` 写 `~/.hope-agent/credentials/auth.json`，**未走** `write_secure_file`——见下文「已知缺口」 |
| `try_acquire_exclusive_lock` | `runtime_lock.rs` 全局单实例守门：桌面 / `hope-agent server` / `hope-agent acp` 三种运行模式启动时拿同一把锁，防止启动恢复 / "global only-one" 后台循环跑两份 |
| `find_chrome_executable` | [`browser_state.rs`](../../crates/ha-core/src/browser_state.rs) Browser 工具自动定位 Chrome / Edge |
| `detect_dedicated_gpu` | [`local_llm/`](../../crates/ha-core/src/local_llm/) 本地 LLM 选模型预算：Windows / Linux 优先 dGPU VRAM 的 50%，探测失败回落系统内存的 50% |

## 已知缺口（技术债）

- **主 LLM OAuth token 落盘没走 `write_secure_file`**：[`oauth.rs::save_token`](../../crates/ha-core/src/oauth.rs) 直接 `std::fs::write(path, json)?` 写 `~/.hope-agent/credentials/auth.json`——既不原子（写到一半 crash 留半截 JSON）也不强制 0600（依赖 umask 和父目录继承）。MCP 凭据已经走了 `write_secure_file`，对照之下这条主 LLM 路径应该也切过来。改动范围小（一行替换 + 错误类型 anyhow 转 io），未来一次专门的安全收尾时一起做。
- **Windows 显式 DACL**：`write_secure_file` 在 Windows 仅依赖 NTFS 继承，没有 strip 继承 ACE 也没有显式只授予 owner。同一进程的低权限子进程理论上能读凭据。当前威胁模型可接受（用户机本地 trust），需要"零本地信任"姿态时按 mod.rs 里"future ACL pass"加固。
- **`detect_system_proxy` 运行时不刷新**：进程级缓存意味着用户运行时改系统代理需要重启应用。如果未来加入"代理变更感知"需求，应给所有平台加同一个缓存失效机制，保持入口语义跨平台一致。

## 关键源文件

| 文件 | 职责 |
|---|---|
| [`crates/ha-core/src/platform/mod.rs`](../../crates/ha-core/src/platform/mod.rs) | 门面：系统级 `pub fn` 入口 + 跨平台 doc 注释，编译期按 `#[cfg(unix)]` / `#[cfg(windows)]` route 到对应 impl |
| [`crates/ha-core/src/platform/unix.rs`](../../crates/ha-core/src/platform/unix.rs) | Unix 实现：`libc::kill` / `sh -c` / `OpenOptions::mode(0o600)` / `sw_vers` 兜底 / `chromiumoxide` 走自己的 which |
| [`crates/ha-core/src/platform/windows.rs`](../../crates/ha-core/src/platform/windows.rs) | Windows 实现：`taskkill /F /T` / `cmd /C raw_arg` / NTFS DACL 继承 / winreg 读 Internet Settings + `OnceLock` 缓存 / `%ProgramFiles%` 三路扫 Chrome |
| [`crates/ha-core/src/platform/system_permissions.rs`](../../crates/ha-core/src/platform/system_permissions.rs) | 系统权限 OS 实现：macOS TCC / framework 原生权限检查与 prompt；非 macOS 明确 unsupported |
| [`crates/ha-core/src/platform/service.rs`](../../crates/ha-core/src/platform/service.rs) | 用户级后台服务 OS 实现：macOS LaunchAgent / Linux user systemd / Windows Task Scheduler；[`service_install.rs`](../../crates/ha-core/src/service_install.rs) 只保留兼容 wrapper |
