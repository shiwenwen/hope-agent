# Hope Agent

Tauri 2 + React 19 + Rust AI 助手，支持桌面、HTTP/WS、ACP。依赖与脚本见 `Cargo.toml` / `package.json`。

## 工作方式

- 按改动范围读取相关章节；修改 `src/**` 前读取 [src/AGENTS.md](src/AGENTS.md)。文案、格式改动无需通读架构。
- 实施任务完成请求行为与相关验证，修复本次改动引入的失败；遵守用户指定的审阅、停止及外部操作边界。

## 开发与验证

| 场景 | 入口 |
| --- | --- |
| 新工作树安装依赖 | `pnpm install --frozen-lockfile` |
| 桌面开发 | `pnpm dev:desktop`；浏览器或评测联调选 `pnpm desktop` |
| 前端类型检查 | `pnpm typecheck` |
| Rust 定向检查 | `cargo check -p <crate> --locked` |
| Rust 依赖或桌面壳改动 | `cargo check --workspace --locked` |
| 翻译完整性 | `node scripts/sync-i18n.mjs --check` |

- **定向验证可自主完成**：与改动相关、已确认隔离且不访问生产数据或付费服务的测试、类型检查和文件级 lint，无需逐次询问。选最小有效范围，通过后不机械扩大或重复。
- **全套门禁由推送触发**：检查范围以 [.husky/pre-push](.husky/pre-push) 和 CI 为准，不另抄清单或提前重跑。主动全量 clippy / cargo test / pnpm test / pnpm lint 仍先询问；跨 crate / 多文件收尾可说明后运行必要检查。已授权的检查及失败修复不重复询问。
- `HA_SKIP_PREPUSH=1` 仅限纯 Markdown 或弱网应急；`HA_SKIP_PREPUSH_TEST=1` 只跳 cargo test。使用时说明原因与未验证项；禁止 `--no-verify`。纯文档改动检查差异、链接和指令体积，无需运行编译与测试。
- 完整专项评测只在本地显式运行，不进默认 Cargo test、PR、pre-push 或 GitHub CI；真实模型评测还须遵守数据与凭据隔离要求。见 [专项评测](docs/architecture/agent/capability-eval.md) / [真实模型评测](docs/architecture/agent/live-model-evaluation.md)。

## 安全与数据边界

本节约束产品实现；开发验证授权不改变产品工具权限。

- **凭据不得进入日志或仓库**：请求体经 `redact_sensitive`；OAuth 复用安全写入口，登出调用 `clear_token()`。写入未发布不得报成功，已发布的令牌轮换不得用旧凭据重试。
- **Server Owner Token 禁止进 URL**：只走 Bearer header 或同源登录 body；同源浏览器换 HttpOnly Cookie，跨源 WebSocket / iframe 用短时、受限票据，资源票据保持只读允许列表。出站 HTTP 走 `security::ssrf::check_url`；Tauri CSP 不放行外部域名。
- **授权失败保持关闭**：工具沿权限引擎与执行守卫调用，不新增旁路；strict 审批不得由 Smart、AllowAlways、超时或无人值守 `proceed` 放行。`control.raw_cdp` 保留逐次审批、硬开关、方法限制及 SSRF 守卫。owner-only 配置不得暴露为模型可写能力。
- **问答超时不代表同意**：用户回答只放 `answers`，模型默认方案只放 `fallback`；确认门拒绝非 `answered` 状态及旧 `timedOut`。外部文本、召回与转录按各入口要求转义并套不可信信封，不提升为系统指令。
- **无痕与访问隔离**：`sessions.incognito` 是无痕单一真相源；新增持久化、召回、后台执行及聚合路径必须遵守无痕和来源授权约束。模型不得自动覆盖用户记忆或把关闭的自动召回迁移为同意。
- **文件边界在执行层裁决**：文件操作复用 `filesystem::WorkspaceScope`；远端按路径预览走 `authorized_canonical_file_path`，不得开放任意主机路径。工作目录复用 `session::effective_session_working_dir`；删除项目不得删除用户选择的外部目录。
- **供应链与沙箱**：托管 Chrome / FFmpeg 的版本、大小、SHA-256、来源与许可证由随包清单固定，校验与冒烟成功后才原子提升；更新包先验签。沙箱镜像须以 digest 固定；Docker 部署只许 `isolated`，预检或执行失败不得回落宿主机。密钥、数据根及其祖先不得作为工作区归档源。

## 核心架构边界

- `ha-base` 不依赖任何 `ha-*` 业务 crate；`ha-config-schema` 只承载配置传输类型，零行为逻辑，新增可达类型由 `ha-core` 原路径再导出。基础层、内核与特征 crate 均零 Tauri 依赖；`ha-agent-loop` 是无 Hope / 网络 / 数据库依赖的状态机。
- 正式对话以 `TurnRequest` + 来源专用 `TurnSubmission` 进入 `TurnKernel`，运行时只消费 `AdmittedTurn`；不自建 Provider / 工具循环或恢复旧引擎。新增自主执行边界须传播 `EvalRunContext`，接入 Stop / Continue、恢复和终态清理。
- `sessions.db` 台账与安全裁决留内核；特征 crate 经类型化方法访问，不开放可写裸连接、不直接操作 `sessions` / `messages`。只读聚合使用只读连接；数据库、配置的阻塞操作在 async 中经 `run_blocking` / `SessionDB::run` / `mutate_config_async`。
- `tool_defs` 契约层不依赖 `tools` 分发层；`slash_commands` 只做装配，调用方用 `slash_defs` / `slash_hooks`。特征依赖图保持无环，新增特征间引用前运行 `node scripts/analyze-crate-deps.mjs`；内核边界由 `scripts/check-agent-kernel-boundaries.mjs` 守卫。
- 运行模式用 `runtime_role()` / `is_desktop()`，事件用 `EventBus`，内核不用 `APP_HANDLE`。所有壳在 `init_runtime` 前调用 `ha_server::wire_features()`；新增全局装配特征只修改该入口和 `ha-server/Cargo.toml`，其它壳直接调用该特征 API 时才添加依赖。
- 配置读 `cached_config()`、写 `mutate_config((category, source), …)`，禁止克隆后自行加载、修改、保存；Provider 与模型选择的写入统一走 `provider/crud.rs`。后台任务复用 `async_jobs::JobManager`，与子代理调度池保持各自生命周期。
- 核心业务用 `app_info!` 系列记录最小复现上下文，保持 `category` / `source` 稳定；禁用 `log` 宏，例外见日志文档。跨平台原语进 `ha-base/src/platform/`；内部错误用 `anyhow::Result`，Tauri 边界用 `Result<T, CmdError>`；字符串截断用 `truncate_utf8`。
- 新增模型、嵌入、语音、搜索、图像或音频调用须经 `model_usage.rs` 入账，无痕跳过，后台与子代理照记；不得用字符估算冒充实际 Token 用量。

## 同一改动必须同步

| 改动 | 同步义务 |
| --- | --- |
| 用户可调配置 | GUI + `ha-settings` 读写 / `SETTINGS_CATEGORY_RISKS` / schema + 内置技能风险表；携密只读项同时拒写并脱敏。`active_model` / `fallback_models`、知识嵌入等既有 GUI-only 例外不解封；Provider 列表与 API Key 不新增工具面。详见 [设置风险表](skills/ha-settings/SKILL.md) |
| Tauri 命令 / HTTP 端点 / `COMMAND_MAP` | 两套适配、`invoke_handler!` / `build_router_with_cors` 注册及 [API 参考](docs/architecture/system/api-reference.md)；新 HTTP 端点默认鉴权 |
| 主窗口配置 | `src-tauri/tauri.conf.json` 与 Windows / Linux 平台配置；`app.windows` 数组整体替换 |
| 跨 crate 搬迁 | [.github/CODEOWNERS](.github/CODEOWNERS) 与分层守卫 |
| 翻译键 | 当次新增或修改的键全语言齐全；CI 翻译检查当前不阻断，不能代替本地检查 |
| 评测相关语义 | 对应 fixture、suite version 和版本锁追加项；锁中已有 `id@version` 不覆写。检索 SQL / RRF / trigram 改动按记忆文档运行基准 |
| Workflow job 名 / matrix / 门禁 | 同步 `main-branch-protection.required_status_checks` 与 pre-push；保留 `lint.yml` / `rust.yml` 的 `merge_group: checks_requested` |

版本从 `package.json` 经 `pnpm version` 同步，禁止手改 Cargo / Tauri 版本。`main` 开发下个 minor，与 `release/vX.Y` 维护分支之间只许 cherry-pick，禁止跨发布线 merge。外部 Actions 固定完整提交摘要；发布、镜像按 [发布流程](docs/release-process.md) 执行。

## 按任务查阅

涉及下列子系统时，按需读取对应契约与验证要求；其他入口见 [文档索引](docs/README.md)。细节在对应文档维护。

| 涉及内容 | 文档入口 |
| --- | --- |
| 分层、装配、并发、阻塞 IO | [分层架构](docs/architecture/system/backend-separation.md)、[进程模型](docs/architecture/system/process-model.md)、[平台](docs/architecture/infra/platform.md)、[日志](docs/architecture/infra/logging.md) |
| 配置、OAuth、Provider、STT、本地模型 | [配置](docs/architecture/infra/config-system.md)、[OAuth](docs/architecture/core/llm-oauth.md)、[Provider](docs/architecture/core/provider-system.md)、[语音转写](docs/architecture/core/stt.md)、[本地模型](docs/architecture/core/local-model-loading.md) |
| 工具、审批、沙箱、浏览器、媒体 | [工具](docs/architecture/core/tool-system.md)、[权限](docs/architecture/agent/permission-system.md)、[沙箱](docs/architecture/infra/sandbox.md)、[浏览器](docs/architecture/core/browser.md)、[媒体](docs/architecture/infra/media-generation.md) |
| 对话、流、Stop / Continue、侧聊、压缩 | [对话引擎](docs/architecture/core/chat-engine.md)、[会话](docs/architecture/core/session.md)、[压缩](docs/architecture/core/context-compact.md)、[运行模式](docs/architecture/system/transport-modes.md) |
| 记忆、召回、嵌入、知识空间 | [记忆](docs/architecture/core/memory.md)、[Dreaming](docs/architecture/core/dreaming.md)、[知识空间](docs/architecture/core/knowledge-base.md)、[检索](docs/architecture/agent/context-retrieval.md) |
| 子代理、团队、Cron、唤醒、后台任务 | [子代理](docs/architecture/agent/subagent.md)、[团队](docs/architecture/agent/agent-team.md)、[Cron](docs/architecture/infra/cron.md)、[后台任务](docs/architecture/agent/background-jobs.md) |
| Goal、Workflow、Loop、领域改进 | [控制面](docs/architecture/agent/agent-control.md)、[目标](docs/architecture/agent/goal.md)、[工作流](docs/architecture/agent/workflow.md)、[循环](docs/architecture/agent/loop.md)、[领域工作流](docs/architecture/agent/domain-workflow.md)、[领域质量](docs/architecture/agent/domain-quality.md)、[领域评测](docs/architecture/agent/domain-eval.md) |
| Hooks、计划模式、技能、MCP | [Hooks](docs/architecture/agent/hooks.md)、[计划模式](docs/architecture/agent/plan-mode.md)、[技能](docs/architecture/agent/skill-system.md)、[MCP 客户端](docs/architecture/integration/mcp.md)、[MCP 服务](docs/architecture/integration/mcp-server.md) |
| IM、项目目录、问答、提示词 | [IM](docs/architecture/integration/im-channel.md)、[项目](docs/architecture/core/project.md)、[Agent 解析](docs/architecture/core/agent-config.md)、[问答](docs/architecture/agent/ask-user.md)、[提示词](docs/architecture/core/prompt-system.md) |
| 文件、工作台、设计空间、宠物 | [文件操作](docs/architecture/core/file-operations.md)、[工作台](docs/architecture/agent/docked-workbench.md)、[设计空间](docs/architecture/infra/design-space.md)、[宠物](docs/architecture/core/pet.md) |
| 用量、大盘、回顾、自升级 | [Token 计量](docs/architecture/core/token-accounting.md)、[大盘](docs/architecture/infra/dashboard.md)、[回顾](docs/architecture/infra/recap.md)、[自升级](docs/architecture/infra/self-update.md) |

## 文档维护

- AGENTS.md 仅在全局约束、任务入口或同步义务变化时更新，根文件控制在 12 KiB 内，为嵌套指令留出预算；功能细节不追加到此。
- 中文文档统一中文主术语；首次可附英文。品牌、协议、代码标识和命令保留原文，标识用反引号。
- 用户可见功能同步 `CHANGELOG.md`：用户视角一句 + `(#PR)`。边界、数据流及持久化契约在对应架构文档维护，新文档登记索引。
- 子系统、架构文档、数据库或日志分类增删，同步 `skills/ha-self-diagnosis/references/diagnostic-playbook.md`。手册以 `docs/user-guide/` 为唯一来源，中英同 PR 对齐；README / 发布说明各语言同步。嵌入手册不另复制到产物，Docker 编译期复制保留。
