# 自升级（Self-Update）

> 关联源码：[`crates/ha-core/src/updater/`](../../crates/ha-core/src/updater) · [`crates/ha-core/src/tools/app_update.rs`](../../crates/ha-core/src/tools/app_update.rs) · [`crates/ha-core/src/tools/definitions/update_tools.rs`](../../crates/ha-core/src/tools/definitions/update_tools.rs) · [`crates/ha-core/src/platform/`](../../crates/ha-core/src/platform) · [`src-tauri/src/commands/update_bridge.rs`](../../src-tauri/src/commands/update_bridge.rs) · [`skills/ha-self-update/SKILL.md`](../../skills/ha-self-update/SKILL.md)

## 目的

Hope Agent 是单 binary 多形态产品（桌面 GUI / `hope-agent server` 守护进程 / `hope-agent acp`），首装渠道多（DMG / MSI / NSIS / AppImage / Homebrew cask / Scoop / AUR / 自建 apt+dnf repo）。自升级子系统让模型在任意形态下，按对话指令完成「检查 → 确认 → 下载 → 校验 → 替换 → 重启」全流程；不可恢复时通过 `ask_user_question` 让用户在对话里选路径。

## 三档升级路径

`ha_core::updater::recommend_path` 在 [`crates/ha-core/src/updater/mod.rs`](../../crates/ha-core/src/updater/mod.rs) 按运行形态 + install source 路由：

| 路径               | 触发条件                                                 | 实现层                                                                                                                    |
| ------------------ | -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `Tauri`            | `is_desktop() && InstallSource::TauriBundle` 且 bridge 已注册 | `src-tauri/src/commands/update_bridge.rs` 调 [`tauri-plugin-updater`](https://github.com/tauri-apps/plugins-workspace)；未注册时降级 `SelfContained` |
| `PackageManager`   | install source ∈ {brew, scoop, aur, apt, dnf}             | [`package_manager::upgrade`](../../crates/ha-core/src/updater/package_manager.rs) 执行渠道命令；命令模板固定，无 shell 拼接 |
| `SelfContained`    | 装法不可识别 + manifest 提供 bare-binary archive          | [`self_contained::install`](../../crates/ha-core/src/updater/self_contained.rs) 下载 → minisign 校验 → atomic swap → restart |
| `ManualPrompt` (Docker) | `InstallSource::Docker`（`HA_DEPLOYMENT=docker` env，Docker 镜像 `ENV` 烧死） | `app_update` 工具用 Docker 专属 `ask_user_question` 文案引导 `docker pull ghcr.io/.../hope-agent:vX.Y.Z`；**永远**走 manual prompt 而不是 binary swap —— 容器内 binary swap 会被下次 `docker pull` 覆盖 |
| `ManualPrompt` (其它) | 其它三档都不适用                                       | `app_update` 工具调 `ask_user_question` 让用户选 (open releases / force self_contained / abort)                          |

工具层 (`app_update install`) 接受 `prefer_path: "auto" | "package_manager" | "self_contained"` 显式覆盖。失败后用户可通过兜底 prompt 重新指定路径。

## 后台自动检查 + 静默下载（`auto_update`）

配置单一真相源 [`AutoUpdateConfig`](../../crates/ha-core/src/updater/config.rs)（`AppConfig.auto_update`，camelCase）：`checkEnabled` / `checkIntervalHours`（钳到 `[1,168]`）/ `autoDownload` / `notify`，全部默认开。桌面与 headless **共享同一份配置**：

- **headless / server**（`hope-agent server`）：[`updater::auto_check::spawn_auto_update_loop`](../../crates/ha-core/src/updater/auto_check.rs) 在 [`app_init::start_background_tasks`](../../crates/ha-core/src/app_init.rs) 的 **primary-gated** 区块 spawn（仿 dreaming cron loop），`!is_desktop()` 才起（桌面用 JS 链路，避免双检查）。每 `checkIntervalHours` 调 `check_update_full()`；发现新版 emit `app_update:available` + 日志（按版本去重）；`autoDownload && recommended_path==SelfContained` 时调 `self_contained::stage_only` 静默下载 + Minisign 校验到 staging（**不 swap**），emit `app_update:staged`。loop 永不自行替换 binary——install 始终走用户确认的 `app_update install`。
- **桌面**：[`desktopUpdater.ts`](../../src/lib/desktopUpdater.ts) 仍走 `@tauri-apps/plugin-updater`，但读 `auto_update` 配置驱动周期检查；命中后 `autoDownload` 时后台 `update.download()` 预下载（plugin-updater 2.10.1 的 `Update` 支持 download/install 分离）。GUI 入口在「设置 → 关于 → 自动更新」+ 命令 `get_auto_update_config` / `set_auto_update_config`（Tauri + HTTP `GET|PUT /api/config/auto-update`，写时钳 interval）。`ha-settings` 技能侧 `auto_update` 为 **HIGH** 风险（网络 + 重启），写前须二次确认。

**桌面重启选择前置**：发现新版后 UI 提供「更新并重启」（装完自动 `relaunch()`）与「仅更新（稍后重启）」（装完停在「已就绪」态，等用户显式点重启）两选项——**绝不无条件自动重启**，避免打断进行中的对话。`app_update install`（headless）的用户审批契约不变。

## 下载健壮性（重试 + 断点续传）

[`download::download_to`](../../crates/ha-core/src/updater/download.rs) 内置重试 + 断点续传：最多 `MAX_ATTEMPTS=3` 次，指数退避 1s/2s；retry 前读 `dest` 已写字节带 `Range: bytes=<n>-` 续传，服务端 `206` 续写 / `200` truncate 重来 / `416` 删档重来；网络 / IO / 5xx 才重试，SSRF / 4xx / 超 `MAX_DOWNLOAD_BYTES` 直接 bail；完成后比对总字节防短读。`Content-Range` 解析全量大小用于进度与上限校验。

## 冷烟自检 + 自动回滚

[`self_contained::install`](../../crates/ha-core/src/updater/self_contained.rs) 在 `atomic_replace_binary` 之后、`restart_service` 之前跑 `smoke_test`：spawn `current_exe --version`（5s 超时），校验输出含目标版本。失败立即用 `backup::most_recent()` + `atomic_replace_binary` **还原旧 binary** 并 `bail!`（还原也失败则明确提示手动恢复路径）。仅冷烟通过才重启服务。

下载 + 校验 + 解压抽成 `download_and_extract`：已存在且**仍能通过签名校验**的 staging 归档会被复用（静默预下载因此真正省掉 install 时的网络往返）；校验失败的归档立即删除不留作"复用"。

## staging 垃圾回收

[`staging::prune`](../../crates/ha-core/src/updater/staging.rs)：启动时（auto loop init）+ 每次重新 stage 前 + install 成功后调用，清掉非目标版本 / 早于 7 天的 staging 子目录（best-effort，仅 log 不 fail）。backup 树独立，永不被此清理触及。

## 签名信任根（单一 Minisign Pubkey）

[`ha-core/src/updater/keys.rs::MINISIGN_PUBKEY_BASE64`](../../crates/ha-core/src/updater/keys.rs) 与 `src-tauri/tauri.conf.json#plugins.updater.pubkey` 必须**字符串相等**——否则桌面 `tauri-plugin-updater` 和 headless `ha_core::updater::signature::verify_bytes` 会用不同 pubkey，一边静默坏掉。三重防线：

1. 启动期（仅桌面）：`src-tauri/src/setup.rs` 用 `include_str!("../tauri.conf.json")` 拿 pubkey → 调 `keys::assert_pubkey_matches_tauri_conf`，drift 直接 panic 退出。
2. CI / PR：`.github/workflows/lint.yml` 跑 `scripts/verify-updater-pubkey.mjs`。
3. 本地 `.husky/pre-push`：同一脚本拦在 push 前。

私钥（`TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）只存 GitHub Secrets，CI release.yml 调 `pnpm tauri signer sign` 同时签桌面 installer 和 bare binary archive。私钥严禁入仓。

## 发布产物（latest.json 扩展）

[`.github/workflows/release.yml`](../../.github/workflows/release.yml) 单次 release 产出两套产物，最终汇总到同一个 `latest.json`：

- **桌面 installer**：`tauri-action` 输出 DMG / MSI / NSIS / AppImage + 各自 `.sig`，写入 `latest.json#platforms.<plat>.{url, signature}`（tauri 原生格式，不变）。
- **裸 binary archive**：每个 platform build job 末尾跑 `Bundle + sign bare binary` step，把 `target/release/hope-agent[.exe]` 打 `tar.gz` (Unix) / `zip` (Windows) → `pnpm tauri signer sign` 用同一私钥签 → 上传 `hope-agent-{ver}-{plat}.tar.gz` + `.sig` 到 release。
- **manifest 合并**：`patch-manifest` job（`needs: build`）下载所有 `bare-binary-*` artifact + release 上的 `latest.json`，跑 `scripts/patch-latest-json.mjs` 注入 `bare_binary.platforms.<plat>.{url, signature, archive, binary_path}` 后重新上传。

Manifest 结构（[`updater::manifest::Manifest`](../../crates/ha-core/src/updater/manifest.rs)）：

```json
{
  "version": "0.2.1",
  "notes": "...",
  "pub_date": "...",
  "platforms": {
    "darwin-aarch64": { "url": "...", "signature": "..." }
  },
  "bare_binary": {
    "platforms": {
      "linux-x86_64": {
        "url": "...",
        "signature": "...",
        "archive": "tar_gz",
        "binary_path": "hope-agent"
      }
    }
  }
}
```

平台 key 与 tauri-action 一致：`{darwin,linux,windows}-{x86_64,aarch64}`，由 [`manifest::current_platform_key`](../../crates/ha-core/src/updater/manifest.rs) 在运行时返回当前平台串。

## 用户审批契约

`app_update install` / `app_update rollback` **永远**通过 [`tools::ask_user_question::execute`](../../crates/ha-core/src/tools/ask_user_question.rs) 弹结构化 Yes/No 确认。在工具内部实现而不是借 `permission::engine::AskReason::DangerousCommand`，因为：

1. AskReason enum 没有 `SystemUpdate` 变体，挪用现有 EditTool / DangerousCommand 语义不对；
2. 确认对话框需要承载完整升级 plan（current → target / 升级路径 / 服务中断提示 / release notes 摘要），通用审批 dialog 无法承载这些字段；
3. `ask_user_question` 自带 pending 持久化 + replay，用户重启 App 也能续上。

确认收到 Yes 后，工具同步 spawn 一个独立 OS thread 跑 install pipeline（不通过 `async_jobs::spawn_explicit_job`，避免 tool dispatch 二次劫持），主线程立刻返回 `{job_id, status: "started", ...}` 给模型。

## 异步 job 跟踪

`app_update install` 返回的 `job_id` 是 in-memory tracker 的键（`tools::app_update::tracker()` 单例 `Mutex<HashMap>`）。状态包含 `phase` ∈ `starting | running | downloading | verifying | staging | backing | swapping | restarting | done | failed`，`outcome` / `error` 在终态时填充。模型查 `app_update(action="status", job_id=...)`。

进度事件通过 EventBus 推到 UI：

- `app_update:progress` —— 下载字节进度（每 5% 或每 1s 节流，含 `phase` / `percent` / `written` / `total`）+ 阶段切换（`lifecycle` label）。
- `app_update:completed` —— 终态时一次性发送，含 `status` + `outcome` 或 `error`。

**为什么不走 `async_jobs.db`**：install 涉及 binary swap，pipeline 一旦开始就不能被外部 cancel 中断（中途断电留下 staging 半成品，重启后用户重跑即可——不需要持久化进度）。in-memory tracker 简单稳定。

## 跨平台 binary swap

[`crate::platform::atomic_replace_binary`](../../crates/ha-core/src/platform/mod.rs) 暴露统一入口，Unix / Windows 各实现：

- **Unix**：`fs::set_permissions(source, 0o755)` → `fs::rename(source, target)`。Unix `rename(2)` 改 dirent 不动 inode，正在运行的进程继续读旧 inode，新 `exec(2)` 加载新 image。`EXDEV` fallback：sibling tempfile + fsync + rename。
- **Windows**：先 `MoveFileExW(target → target.old, REPLACE_EXISTING)` 把 in-use binary rename 让出位置（Vista+ 允许），再 `MoveFileExW(source → target, REPLACE_EXISTING | WRITE_THROUGH)` 原子发布新 image，最后 `MoveFileExW(target.old, NULL, DELAY_UNTIL_REBOOT)` 调度旧 image 重启时清理。失败时把 aside 还原回 target 防止留下断片。

不允许 `fs::write` 直接覆盖正在运行的 binary——即使 Unix 上能 work，崩溃中途会留下半截文件。

### macOS 桌面 updater 的 EXDEV 守卫

上面的 `atomic_replace_binary` 只覆盖 **headless `SelfContained`** 路径。**桌面 `Tauri`** 路径的 swap 由 `tauri-plugin-updater` 自己做：它用 `tempfile::Builder` 把新 `.app` 解压进默认临时目录，再用 `std::fs::rename` 把旧 bundle 移到备份、把新 bundle 移到安装位置（`updater.rs::install_inner`）。当应用运行在与临时目录不同的卷（外置 / 独立数据卷等）时，**第一步 rename 就返回 `EXDEV`**（"Cross-device link (os error 18)"）导致更新中断——插件把任何非 `PermissionDenied` 的 rename 错误都当致命错误（`EXDEV` **不会**触发 AppleScript / copy 兜底），且 macOS 路径不像 Linux AppImage 路径那样有「多候选目录 + 同 `dev()` 校验」兜底。

防御在启动早期 `src-tauri/src/lib.rs::run()` 顶部调用 [`platform::redirect_updater_tmpdir_if_cross_volume`](../../crates/ha-core/src/platform/mod.rs)：macOS 上若 `.app` 所在卷（比 `dev()`）≠ 默认临时目录卷，则在 `.app` 父目录下建 `.hope-agent-updater-tmp`、**复核该目录 `dev()` 确实落在 bundle 卷**后，用 [`tempfile::env::override_temp_dir`](https://docs.rs/tempfile) 把 **`tempfile` 库的进程内默认临时目录**改到那里，使插件两次 rename 都留在同卷内。返回三态 `UpdaterTmpdir`：`Redirected`（已改）/ `Unchanged`（同卷 / 非 bundle / 非 macOS）/ `CrossVolumeUnfixable`（跨卷但无法在同卷建临时目录——只读挂载如 DMG、或父目录不可写；此时无能为力，`run()` 落一条 warn 面包屑提示用户装到 `/Applications`）。

**为何用 `tempfile` 覆盖而非改 `$TMPDIR` 环境变量**：`override_temp_dir` 只改 `tempfile` 库在**本进程**内的默认目录（插件正是用 `tempfile::Builder` 暂存，故生效），**不动 `$TMPDIR` 环境变量**——因此 `exec` / hooks / MCP 等**子进程**（会继承、甚至显式 whitelist `$TMPDIR`）仍用每用户系统临时目录，不会把临时文件写到应用旁边。`override_temp_dir` set-once + 线程安全，故 `run()` panic-restart 重入无害。**为何 startup 设而非包在某次更新外**：桌面更新两个入口都独立驱动插件 Rust install——GUI「检查更新」菜单走 JS（[`desktopUpdater.ts`](../../src/lib/desktopUpdater.ts)）、`app_update` 工具走 `update_bridge`；只包一个 call site 覆盖不到另一个。普通装在引导卷（= 临时目录同卷）一律 no-op，进程内 temp 改道的代价只由罕见跨卷用户承担。

## Service restart 契约

binary 换好后 [`service_control::restart_service`](../../crates/ha-core/src/updater/service_control.rs) 跑：

- macOS：`launchctl kickstart -k gui/$UID/ai.hopeagent.server`
- Linux：`systemctl --user restart hope-agent.service`
- Windows：`schtasks /End /TN "Hope Agent" && schtasks /Run /TN "Hope Agent"`

成功 ≈ 1-2s 不可用窗口。已注册 service 时由 OS 重启；未注册时返回 best-effort 提示让用户手动重启。

桌面 GUI 进程的"重启"是用户手动操作——`update_bridge.rs` 故意不调 `app.restart()`，避免升级中切断用户正在打的字。前端 updater 安装完成后的按钮路径调用 `@tauri-apps/plugin-process` 的 `relaunch()`；在 Tauri 2.10 / plugin-process 2.3.1 中它映射到 `AppHandle::request_restart()`，Tauri 会先发 `RunEvent::Exit` 给插件，`tauri-plugin-single-instance` 在这个事件里释放 mutex / socket，随后才由 Tauri `process::restart()` 拉起新进程。

## Backup / rollback

升级前 [`backup::store`](../../crates/ha-core/src/updater/backup.rs) 把当前 binary 复制到 `~/.hope-agent/updater/backup/<old_version>/hope-agent[.exe]`。保留最近 **2 个**版本，再多自动 prune。

`app_update rollback` 取 `backup::most_recent`（按 mtime 排序）→ 调 `atomic_replace_binary` 还原 → restart service。同样需要 Yes/No 确认。

## 桌面 ↔ headless 协调（双进程并发）

用户可能桌面 GUI 在跑，同时 `hope-agent server install` 跑 daemon。两者共享同一 binary 文件，升级时需要协调。**当前实现**：

- 桌面端默认通过 `Tauri` 路径走 `tauri-plugin-updater`（独立处理 macOS dmg / Windows installer / Linux AppImage 替换语义）。
- daemon 端独立检查 + 走 `SelfContained` 路径替换 binary 后 service restart。
- 跨进程互斥锁**未接入**——双进程并发升级会有竞态（实际场景罕见，两端通常不会同时触发升级）。需要时再加 advisory file lock。

## 失败路径 → 兜底 `ask_user_question`

工具内部失败处理参考 [`tools/app_update.rs::prompt_manual_install`](../../crates/ha-core/src/tools/app_update.rs) 模板。Skill [`ha-self-update`](../../skills/ha-self-update/SKILL.md) "When things fail" 章节列了每种错误关键字的兜底方案——模型按该决策树触发兜底 prompt 而不是自己 retry。

## 不在 MVP 范围

- 双进程零停机 socket handoff
- **全自动无人值守安装**（不经任何用户审批就 swap + restart）——后台只做检查 + 静默预下载，实际安装/重启仍需用户确认
- Beta / nightly channel 切换（manifest 只有 stable）
- 跨架构迁移（Intel mac → Apple silicon 自动切换 platform_target）
- 升级前事务性快照配置 db（升级不改 user data，rollback 只需 binary）

## 测试矩阵

- 单元：每个 sub-module 内部 `#[cfg(test)] mod tests`（keys / manifest / signature / source_detector / backup / package_manager / app_update）
- 集成：[`tests/updater_e2e.rs`](../../crates/ha-core/tests/updater_e2e.rs) 用 wiremock 测 manifest fetch + binary_swap roundtrip
- 手动端到端：见本文档"三档升级路径"——每个 path × 每个平台至少跑一次 release 验证
