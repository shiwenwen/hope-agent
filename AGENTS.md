# Hope Agent

基于 Tauri 2 + React 19 + Rust 的本地 AI 助手桌面应用，内置丰富的 Provider 模板与预设模型，GUI 傻瓜式配置。支持三种运行模式：桌面 GUI（Tauri）、HTTP/WS 守护进程（`hope-agent server`）、ACP stdio（`hope-agent acp`）。

## 开发命令

```bash
npm run tauri dev      # 启动开发模式（前端 + Tauri 热重载）
npm run dev            # 仅前端 Vite 开发服务器
npm run tauri build    # 构建生产包
npx tsc --noEmit       # 前端类型检查
npm run lint           # Lint
node scripts/sync-i18n.mjs --check   # 检查各语言翻译缺失
node scripts/sync-i18n.mjs --apply   # 从翻译文件补齐缺失翻译

# Server 模式（HTTP/WS 守护进程）
hope-agent server start              # 前台启动 HTTP/WS 服务
hope-agent server install            # 注册系统服务（macOS launchd / Linux systemd）
hope-agent server uninstall          # 卸载系统服务
hope-agent server status             # 查看服务运行状态
hope-agent server stop               # 停止服务
```

## 提交前检查（强制）

推送（`git push`）前**必须**本地跑一遍以下四条，任一失败都要先修再推——CI 会对同一组检查投红票，红在 CI 上要等 10 分钟反馈。`npm install` 后会自动启用 [`.husky/pre-push`](.husky/pre-push) 钩子，该钩子就按这个顺序跑这四条；裸跑也可以：

```bash
cargo fmt --all --check                                                    # 对应 CI: rust.yml fmt job
cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings # 对应 CI: rust.yml clippy job
cargo test  -p ha-core -p ha-server --locked                               # 对应 CI: rust.yml test job
npx tsc --noEmit                                                           # 对应 CI: lint.yml 前端类型
npm run lint                                                               # 对应 CI: lint.yml ESLint
```

- **clippy / test 只覆盖 `ha-core` + `ha-server`**（CI 也是如此）—— `src-tauri` 不在本地钩子范围内，tauri-specific 的 lint / test 问题请用 `cargo {clippy,test} --workspace` 主动自查
- **fmt 用 `--all`**，覆盖整个 workspace；钩子用的是 `cargo fmt --all --check`，裸跑时用不用 `--all` 都能对齐 CI，不过 `--all` 更稳
- **应急开关**：
  - `HA_SKIP_PREPUSH=1 git push` — 整段钩子跳过（仅限文档 / 纯 `.md` 改动 / 弱网紧急场合）
  - `HA_SKIP_PREPUSH_TEST=1 git push` — 只跳过 `cargo test`（WIP 分支快速推送，test 让 CI 兜底；fmt/clippy/tsc/eslint 仍然跑）
  - 禁止用 `--no-verify`，因为它会把 GPG 签名等其它钩子也一并绕过

## 项目结构

```
Cargo.toml              Workspace 根（members: crates/ha-core, crates/ha-server, src-tauri）
crates/
  ha-core/              核心业务逻辑（零 Tauri 依赖，纯 Rust 库）
  ha-server/            HTTP/WS 服务器（axum，REST API + WebSocket 流式推送）
src-tauri/              Tauri 桌面 Shell（薄壳，调用 ha-core）
src/                    前端（React + TypeScript）
  components/           chat/ settings/ dashboard/ cron/ common/ ui/ 等
  lib/                  Transport 抽象层：transport.ts + transport-tauri.ts + transport-http.ts
  i18n/locales/         12 种语言翻译文件
skills/                 内置技能（bundled skills，随应用发行；当前 ha-settings / ha-skill-creator / ha-find-skills）
```

ha-core 按功能域拆分模块，具体用 `ls crates/ha-core/src/` / `Glob` 查看，无需在此维护清单。主要领域：`agent/`（AssistantAgent + Provider + Tool Loop）、`chat_engine/`、`context_compact/`、`memory/`、`skills/`、`tools/`、`channel/`（IM 渠道）、`subagent/`、`team/`、`cron/`、`acp/`、`dashboard/`、`recap/`、`awareness/`、`config/`、`session/`、`project/`、`plan/`、`ask_user/`、`async_jobs/`、`failover/`、`platform/`、`security/`、`logging/`。

## 技术栈

| 层     | 技术                                                                 |
| ------ | -------------------------------------------------------------------- |
| 前端   | React 19 + TypeScript, Vite 8, Tailwind CSS v4, shadcn/ui (Radix UI) |
| 桌面   | Tauri 2                                                              |
| 服务器 | axum (HTTP/WS), clap (CLI)                                          |
| 后端   | Rust, tokio, reqwest（ha-core 库，零 Tauri 依赖）                   |
| 渲染   | Streamdown + Shiki + KaTeX + Mermaid                                 |
| 多语言 | i18next (12 种语言)                                                  |

## 架构约定

### 分层 & 运行模式

- **三 Crate 架构**：`ha-core`（核心业务逻辑，**零 Tauri 依赖**）/ `ha-server`（axum HTTP/WS 薄壳）/ `src-tauri`（Tauri 桌面薄壳）。业务逻辑全进 ha-core，其它两个只做适配
- **三种运行模式**：`hope-agent`（桌面 GUI）/ `hope-agent server`（HTTP/WS 守护进程，含 install/uninstall/status/stop 子命令）/ `hope-agent acp`（stdio ACP 协议）
- **前后端通信**：前端走 Transport 抽象层（`src/lib/transport.ts`），桌面走 Tauri IPC，server 走 HTTP + WS。**新 invoke 必须同时补齐两套适配**
- **EventBus 事件总线**：`ha-core::EventBus` 替代原 Tauri `APP_HANDLE`，让核心逻辑脱离 Tauri 依赖；Tauri shell / axum server 各自订阅并转发到各自前端通道
- **状态管理**：`ha-core::CoreState`（`tokio::sync::Mutex`），Tauri 走 `State<AppState>`，server 走 axum `Extension`；前端保持轻量 React state
- **Guardian 心跳**：桌面 + server 共用 keepalive，保证后台任务（Channel 轮询、Cron 调度等）持续运行
- **系统服务**：`server install` 在 macOS 注册 launchd plist（`~/Library/LaunchAgents/`），Linux 注册 systemd unit（`~/.config/systemd/user/`）
- **API Key 鉴权**：`ha-server/middleware.rs`，支持 `Authorization: Bearer` 头和 `?token=` 查询参数（浏览器 WS 不支持自定义头）。`/api/health` 免鉴权，`api_key=None` 全放行
- **内嵌服务器配置**：桌面内嵌 HTTP 的 bind + api_key 存 `config.json` 的 `server` 字段，修改后需重启

### LLM 主对话

- **Provider**：4 种（Anthropic / OpenAIChat / OpenAIResponses / Codex），集中在 `agent/` 模块
- **Tool Loop**：请求 → 解析 tool_call → 执行 → 回传 → 继续，默认上限 20 轮（`max_tool_rounds=0` 无限）。末轮去 `tools` 字段强制文本收尾。`concurrent_safe` 标只读工具并发、写入工具串行
- **温度三层覆盖**：会话 > Agent > 全局，`AssistantAgent.temperature` 在四 Provider 请求中统一注入（0.0–2.0，`None` = API 默认）
- **Side Query（缓存侧查询）**：`side_query()` 复用主对话 system_prompt + history 前缀命中 prompt cache，Tier 3 摘要 / 记忆提取成本降 ~90%
- **LlmApiAdapter / StreamingChatAdapter trait**：one-shot + streaming 两个抽象（`agent/llm_adapter.rs` + `agent/streaming_adapter.rs`）；各 Provider 实现 trait 避免重复，`streaming_loop.rs` 承担 compaction / cache snapshot / tool dispatch / steer / microcompact / event emit 公共编排。**新"会 spawn tool loop 的 chat 路径"走 `chat_engine::run_chat_engine`，不要绕过自包 `on_delta`**
- **failover executor**：`failover/executor.rs::execute_with_failover` 统一承担 profile 选择 + 错误分类 + 决策。三档 policy：`chat_engine_default` / `side_query_default` / `summarize_default`。**Codex 强制不参与 profile 轮换**
- **降级策略**：ContextOverflow 终止 → RateLimit/Overloaded/Timeout 指数退避重试 2 次 → Auth/Billing/ModelNotFound 跳下一模型
- **Auth Profile 轮换**：`ProviderConfig.auth_profiles` 同 Provider 多 API Key，失败先轮换同 Provider profile 再跳模型；纯内存 cooldown + session 级 sticky。Codex OAuth 不参与
- **连续消息合并**：`push_user_message()` 合并连续 user 消息兼容 Anthropic role 交替

### Chat Engine & Streaming

- **聊天流双写 + seq 去重（重载恢复）**：每个 stream delta 同时发 per-call `EventSink`（主路径）和 EventBus `chat:stream_delta`（带 `{sessionId, seq}`，重载保险）。主路径 + Bus 路径共享 `lastSeqRef` cursor 去重，Channel 死时 Bus 路径接管。IM 渠道独立 `ChannelStreamSink` 走 `channel:stream_delta` 不与主 chat 混淆
- **API-Round 消息分组**：tool loop 中 assistant + tool_result 通过 `_oc_round` 元数据分组，压缩切割（Tier 3/4）对齐 round 边界避免拆散 tool_use/tool_result 配对；API 调用前 `prepare_messages_for_api()` 剥离元数据

### 上下文压缩

- **5 层渐进式 + ContextEngine trait**：`context_compact/` 可插拔引擎，默认 `DefaultContextEngine` 行为不变
- **CompactionProvider trait**：Tier 3 摘要策略可插拔，`summarization_model` 配置驱动 `DedicatedModelProvider`
- **Cache-TTL 节流**：`compact.cacheTtlSecs`（默认 300s）Tier 2+ 冷却保护 API prompt cache；usage ≥ 95% 强制覆盖，`0` 禁用
- **反应式微压缩**：tool loop 每轮末尾使用率 ≥ `reactiveTriggerRatio`（默认 0.75）触发 Tier 0 清 `tool_policies=eager` 旧工具结果，cache-safe 不改消息顺序
- **后压缩文件恢复**：Tier 3 摘要后扫描 write/edit/apply_patch 自动注入最近文件当前内容（最多 5 × 16KB），省额外 read

### Memory

- **自动提取**：冷却 ≥ 5 min AND（Token ≥ 8000 OR 消息 ≥ 10），inline 执行共享 side_query cache；检测到 save_memory/update_core_memory 跳过互斥；空闲超时（默认 30 min）兜底 flush
- **Memory Budget**：system_prompt Memory section 按 `Guidelines > Agent memory.md > Global memory.md > SQLite` 优先级消费总字符预算，`effective_memory_budget(agent, global)` 单入口解析。**`recall_memory` / `memory_get` 工具返回完整原文，预算仅约束 system prompt 注入路径**
- **Active Memory 主动召回**：每轮 user turn 前调 `refresh_active_memory_suffix(user_text)`，带 15s TTL 缓存，按 Project → Agent → Global scope 取候选 + bounded side_query 生成 `## Active Memory` suffix。作为独立 cache block 注入不作废静态前缀缓存
- **反省式记忆（COMBINED_EXTRACT_PROMPT）**：`memory_extract` 单次 side_query 同时返回 facts + profile；profile 渲染为独立 `## User Profile` 段（不用 "About You"——"You" 在 LLM system prompt 里默认指 assistant）
- **LLM 语义选择**：候选 > 阈值（默认 8）时 side_query 选最相关 ≤5 条，opt-in `memorySelection.enabled`
- **召回摘要**：命中 ≥ `min_hits`（默认 3）时 bounded side_query 压成 ≤400 字符洞察段落，opt-in
- **Dreaming 离线固化**：idle / cron / manual 三触发 bounded side_query 把候选交 LLM 评估，打分高的 pin，diary markdown 落 `~/.hope-agent/memory/dreams/`；`DREAMING_RUNNING` AtomicBool + RAII 保证并发
- **会话级无痕对话**：`sessions.incognito` 是单一真相源。开启后该 session 后续轮次不注入 Memory / Active Memory / Awareness，不跑 inline / idle / flush-before-compact 自动记忆提取；记忆工具仍保留，但只靠 system prompt 约束模型在用户明确要求“记住 / 回忆 / 查询记忆”时调用

### 工具 & 审批

- **Agent 工具过滤**：`AgentConfig.capabilities.tools` 在 system_prompt / tool_schemas / tool_search / 执行层统一生效。internal 系统工具始终保留，`denied_tools` / skill allowlist / Plan Mode 继续收紧
- **工具结果磁盘持久化**：超过阈值（默认 50KB，`toolResultDiskThreshold`）写 `~/.hope-agent/tool_results/{session_id}/`，上下文仅留 head+tail 预览 + 路径引用
- **审批等待超时**：`approvalTimeoutSecs`（默认 300s，`0` 不限时）；超时动作由 `approvalTimeoutAction` 控制（`deny` / `proceed`）
- **Dangerous Mode / YOLO**：进程级跳过所有工具审批。CLI flag `--dangerously-skip-all-approvals`（进程内不持久化）和 `AppConfig.dangerous_skip_all_approvals`（config.json 持久化）OR 组合。判定入口 `security::dangerous::is_dangerous_skip_active()`，跳过会落 `app_warn!` 审计。**与 Plan Mode 正交**（Plan Mode 限制工具/路径，YOLO 只跳审批）
- **工具调用前说明**：`AppConfig.tool_call_narration_enabled`（默认 `false`）开启后注入 `TOOL_CALL_NARRATION_GUIDANCE`
- **异步 Tool 执行**：`exec` / `web_search` / `image_generate` 标 `async_capable=true`。三道决策：模型显式 `run_in_background` / Agent `async_tool_policy` / 同步预算超时自动后台化。结果落独立 `async_jobs.db` + spool 到 `~/.hope-agent/async_jobs/`，完成后经 `subagent::injection` 注入主对话。`job_status` deferred 工具主动 poll/wait，用 per-job `tokio::sync::Notify` + DB 重读关闭 race；重启回放残留 running → interrupted
- **SSRF 统一策略**：出站 HTTP **必须**走 `security::ssrf::check_url(url, policy, &trusted_hosts)`，redirect 用 `check_host_blocking_sync`。三档 policy：`Strict` / `Default` / `AllowPrivate`，metadata IP 任何 policy 都拒；`trustedHosts` 优先于 policy。**新出站入口严禁自写 IP 校验**。LLM Provider 出站当前不走此检查（`allow_private_network` 仅落字段）
- **会话列表 pending-interaction 指示器**：`SessionMeta.pendingInteractionCount` 合并工具审批 + ask_user 两类。前端按 `!isActive && !channelInfo && pendingInteractionCount > 0` 叠加视觉提示；后端相关路径调 `emit_pending_interactions_changed()` 让前端 300ms 防抖刷新

### Skill 系统

- **Bundled Skills**：`skills/` 随应用发行，`discovery.rs::resolve_bundled_skills_dir()` 按"环境变量 → exe 同级/上级 → CARGO_MANIFEST_DIR"优先级定位。优先级 bundled < extra < managed < project
- **`skill` 工具（激活主入口）**：`skill({name, args?})` 取代"模型 read SKILL.md"老路径；inline / fork 两种分发路径复用同一 helper。工具标 `internal + always_load` 保证 deferred tool 场景恒可见
- **斜杠 Skill 内联**：`/skillname args` 内联模式包进 `[SYSTEM: ...]` 消息 + `---` 分隔线；`chat` 命令 `display_text` 让 DB / UI 显示原始斜杠、LLM history 存展开 prompt。**已知 gap**：斜杠内联当前未应用 `allowed-tools` 过滤和 `check_requirements`
- **Skill Fork**：SKILL.md `context: fork` 在 tool 调和斜杠两条路径都生效；`agent:` 指定身份（失败 fallback + warn），`effort:` 透传 `SpawnParams.reasoning_effort`
- **Skill 工具隔离**：SKILL.md `allowed-tools:` 激活时只保留白名单 schema，空列表 = 全部
- **Skill `paths:` 条件激活**：`paths:` 的 Skill 默认不进 catalog，命中路径感知工具时 `bump_skill_version()` 刷下一轮 prompt；kill switch `AppConfig.conditional_skills_enabled`
- **Skill Draft 审核**：`skills::author` 提供 CRUD + Jaccard 0.80 模糊 patch；`security_scan` 拦截 shell pipe / 不可见 Unicode / 凭证特征。SKILL.md `status: active|draft|archived`，面向模型路径跳过非 Active；`auto_review` 默认 promotion=draft 等用户确认
- **Plan 执行层权限强制**：`ToolExecContext.plan_mode_allowed_tools` 执行层白名单检查，与 schema 级过滤 defense-in-depth

### Subagent / Team / 定时任务

- **spawn_and_wait**：`subagent(action="spawn_and_wait")` 前台等待 `foreground_timeout`（默认 30s），超时自动转后台
- **Agent Team 模板**：GUI 预配 + 模型按需发现（`team(action="list_templates" / "create")`）。`TeamTemplateMember.description` 注入子 session 的 `### Your Role Identity` 段；删除模板不影响已实例化运行中 team（`teams.template_id` 悬空保留）
- **定时任务 delivery_targets**：`CronJob.delivery_targets` 把 final assistant text fan-out 到 IM 会话，每 target 10s 超时保护。IM 会话内隐式推断：未显式传时自动取当前 IM 会话；显式 `[]` 关闭发送

### IM Channel

- **架构**：`channel/` 下 12 个插件，状态文件统一落 `~/.hope-agent/channels/`。入站媒体管道：plug 下载 → worker 转 `Attachment` → 归档到 `~/.hope-agent/attachments/{session_id}/` → `agent.chat()` 多模态
- **工具审批交互**：`channel/worker/approval.rs` 监听 EventBus `approval_required`，按 `ChannelCapabilities.supports_buttons` 发原生按钮或文本 fallback；`ChannelAccountConfig.auto_approve_tools=true` 跳审批门控

### Dashboard / Recap / Learning

- **Insights**：`dashboard/insights.rs`——overview delta / cost trend / activity heatmap / hourly / top sessions / model efficiency / health score + orchestrator `query_insights`
- **Learning Tracker**：`session.db.learning_events` 表 + `dashboard::learning` 查询。埋点：`skills::author` CRUD + `tool_recall_memory` 命中；Dashboard Learning Tab 支持 7/14/30/60/90 天窗口
- **/recap 深度复盘**：`recap/` 模块 side_query 提取 facet + 11 个并行 AI 章节。独立 `~/.hope-agent/recap/recap.db` 缓存按 `last_message_ts` 失效；`recap.analysisAgent` 与主对话 Agent 解耦，`facetConcurrency` 限流

### 跨会话 / 全局

- **数据存储**：所有数据在 `~/.hope-agent/`，`paths.rs` 集中管理
- **统一日志**：前后端都进 `logging.rs`（SQLite + 文本双写），API 请求体 `redact_sensitive` 脱敏 + 32KB 截断
- **跨会话行为感知**：每会话 `SessionAwareness` 在 user turn 前按"脏位 / 时间节流 / 语义 hint"决定是否重建"其它会话在做什么" markdown suffix。默认 `structured`（零 LLM 成本），opt-in `llm_digest`（bounded 5s + 5min 节流）。作为独立 cache block 注入不作废静态前缀缓存
- **Awareness 与无痕联动**：session `incognito=true` 时会直接跳过 `refresh_awareness_suffix`，前端 session 级 AwarenessToggle 置灰但不改写 `awareness_config_json`；关闭无痕后恢复原有 awareness 配置
- **会话搜索**：侧边栏 FTS5 `messages_fts` + `<mark>` snippet 高亮 + `SessionTypeFilter` 跨会话类型筛选。结果命中 `load_session_messages_around_cmd` 加载窗口（40/20）+ 滚动定位 + pulse 高亮。**XSS 防御**：HTML escape → 白名单反解 `<mark>`。Find in Chat（`Cmd+F`）复用同一 `SessionDB::search_messages` + session_id 过滤
- **ask_user_question 工具**：1–4 题结构化问答（单选/多选/输入），选项支持 markdown/image/mermaid preview；pending 组持久化到 session SQLite，App 重启 replay 断点续答；IM 渠道按 `supports_buttons` 走按钮或文本
- **延迟工具加载**：opt-in `deferredTools.enabled`，只发核心 ~10 个 schema，其余通过 `tool_search` 元工具按需发现；execution dispatch 不变
- **Persona SoulMd 模式**：`PersonalityConfig.mode: Structured | SoulMd`。SoulMd 跳过结构化 personality section，注入 `~/.hope-agent/agents/{id}/soul.md` + `SOUL_EMBODIMENT_GUIDANCE`，身份行省略 `role_suffix` 避免与 markdown 自述身份双重声明
- **SearXNG Docker 代理**：`web_search.searxng_docker_use_proxy` 控制是否向容器 `settings.yml` 注入 proxies 和环境变量，下次启动生效

### 项目（Project）容器

- **三层 system prompt 注入**：目录清单永远注入 → 小文件（<4KB）8KB 预算内内联 → `project_read_file` 工具按需（强制 `project_extracted_dir` 内）
- **记忆优先级**：Project > Agent > Global，budget 裁剪时项目记忆最先保留；session 属项目时自动记忆提取默认写 Project scope
- **删除级联顺序**：unassign 会话 → 删项目行 + `project_files`（FK）→ `rm -rf projects/{id}/` → 删项目记忆（跨 `memory.db` 单独执行，失败留不可达孤儿）

## 编码规范

### 通用

- **性能和用户体验是最高优先级**
- **核心逻辑必须在 ha-core 实现**：业务逻辑、数据处理、文件 IO、状态管理等一律放 `crates/ha-core/`，`src-tauri/` 仅做 Tauri 命令薄壳，`crates/ha-server/` 仅做 HTTP 路由薄壳，前端只负责展示和交互
- 操作即时反馈（乐观更新、loading 态），动效 60fps（优先 CSS transform/opacity）

### 前端

- 函数式组件 + hooks，不用 class 组件
- UI 组件统一用 `src/components/ui/`（shadcn/ui），不直接用 HTML 原生表单组件
- 样式只用 Tailwind utility class，不写行内 style 和自定义 CSS
- 动效优先复用 shadcn/ui、Radix UI、Tailwind 内置 utility，确认不够用才手写
- 路径别名：`@/` → `src/`
- 布局避免硬编码过小的 max-width（如 `max-w-md`），使用 `max-w-4xl` 以上或弹性伸缩
- **i18n 当次改动涉及的翻译 key 必须 commit 时全 12 语言齐全**：新增或修改的 key 当次就要把所有语言文件补齐（存量缺失的 key 不强制处理）。
- 避免不必要的重渲染（`React.memo`、`useMemo`、`useCallback`）
- **Tooltip 必须使用 `@/components/ui/tooltip`**，禁止用 HTML 原生 `title` 属性。优先使用 `<IconTip label={...}>` 简洁包裹
- **保存按钮统一三态交互**：saving（Loader2 旋转 + disabled）→ saved（绿色 + Check 图标，2 秒恢复）→ failed（红色，2 秒恢复）。使用 `saveStatus: "idle" | "saved" | "failed"` + `saving: boolean` 管理
- **Think / Tool 流式块展示约定**：内容块必须设置合理 `max-height`，超出后内部滚动；流式增量期间需自动滚动至底部，并实时显示耗时（结束后保留最终耗时）

### 后端（Rust）

- 新功能放 `crates/ha-core/` 单独模块文件；Tauri 命令在 `src-tauri/src/lib.rs` 注册，HTTP 路由在 `crates/ha-server/src/router.rs` 注册
- 内部用 `anyhow::Result`，命令边界转为 `String`
- 异步命令加 `async`，不要自己 `block_on`
- **禁止使用 `log` crate 宏**（`log::info!` 等），必须使用 `app_info!` / `app_warn!` / `app_error!` / `app_debug!`（定义在 `logging.rs`）。唯一例外：`lib.rs` 的 `run()` 中 AppLogger 初始化之前，以及 `main.rs` 的 panic 恢复
- 日志宏用法：`app_info!("category", "source", "message {}", arg)`
- **核心业务路径必须埋点**：Provider 调用、tool 执行、审批决策、failover 切换、compaction 触发、channel 收发、记忆提取/召回、cron 执行、配置变更等关键节点都要打日志，不要静默成功或静默失败（成功走 `app_info!`、异常走 `app_warn!` / `app_error!`）。日志既服务人工排查，也作为 **agent 自主修复**的首要信息源——内容要带最小复现上下文（输入摘要 + 关键决策 + 结果/错误），`category` / `source` 命名保持稳定以便 grep 聚合
- **禁止对字符串使用字节索引切片**（如 `&s[..80]`），必须使用 `crate::truncate_utf8(s, max_bytes)` 安全截断
- **跨平台分支**：优先用 `#[cfg(unix)]` / `#[cfg(windows)]`（macOS + Linux + BSD 共享 Unix 路径），少写 `target_os = "linux"`。新增跨平台原语统一放 `crates/ha-core/src/platform/`（`mod.rs` 是门面，`unix.rs` / `windows.rs` 是实现），不要在业务代码里散落 `#[cfg]` 分支。调用方用 `crate::platform::xxx()` 单一入口

## 安全红线

- **API Key 和 OAuth Token 禁止出现在任何日志中**
- `tauri.conf.json` CSP 当前为 `null`，不要放行外部域名
- OAuth token 在 `~/.hope-agent/credentials/auth.json`，登出时必须 `clear_token()`

## 易错提醒

- 修改 Tauri 命令后须同步更新 `invoke_handler!` 宏注册列表
- 新增 HTTP API 端点后须在 `crates/ha-server/src/router.rs` 注册路由
- 新增核心功能须放 `crates/ha-core/`，禁止在 ha-core 中引入 Tauri 依赖
- Rust 依赖变更后 `cargo check` 先行验证（workspace 级别）
- 前端新增 invoke 调用时须同步实现 Transport 的 Tauri 和 HTTP 两种适配
- 新增/修改接口时同步更新 [`docs/architecture/api-reference.md`](docs/architecture/api-reference.md) 对应功能域表格，该文档是 Tauri ↔ HTTP 对齐的单一真相源

## 设置（Settings）约定

所有用户可操作的配置必须同时具备 **GUI 入口** 和 **`oc-settings` 技能对应能力**，两者零偏差。新增/修改进入 `AppConfig` / `UserConfig` 且用户需要调整的字段时，**同一 PR 内三件事缺一不可**：

1. **GUI 控件**：[`src/components/settings/`](src/components/settings/) 对应面板，shadcn/ui + 三态保存按钮
2. **技能能力**：[`crates/ha-core/src/tools/settings.rs`](crates/ha-core/src/tools/settings.rs) 加读写分支 + 风险分级 + 必要的副作用提示；同步更新 [`core_tools.rs`](crates/ha-core/src/tools/definitions/core_tools.rs) 里 `get_settings` / `update_settings` 的 `category` enum
3. **技能文档**：在 [`skills/oc-settings/SKILL.md`](skills/oc-settings/SKILL.md) 风险等级表里登记

### 风险等级判定

- **LOW**：UI 偏好、显示配额，不影响成本/安全（theme / language / notification / canvas 等）
- **MEDIUM**：行为调整，影响上下文、成本或输出质量（compact / memory_* / web_search / approval 等）
- **HIGH**：安全、网络暴露、全局键位、凭据、需要重启（proxy / embedding / shortcuts / server / skill_env / acp_control 等）——技能在 `update_settings` 前必须向用户二次确认

### 强制留在 GUI 的例外

**Provider 列表与 API Key**、**IM Channel 配置**、**`active_model` / `fallback_models` 的写入**三类不进 `update_settings` 工具（凭据安全 + 运行时稳定性），技能里只读或完全排除。

### 配置读写 contract（强制）

详见 [`docs/architecture/config-system.md`](docs/architecture/config-system.md)。硬规则：

- **读** 走 `ha_core::config::cached_config()`（`Arc<AppConfig>` 快照），禁止重新引入 `Mutex<AppConfig>` 或本地克隆
- **写** 走 `ha_core::config::mutate_config((category, source), |cfg| {...})`，禁止 `load_config()` + `save_config()` 手动克隆-改-存模式——无法防并发 lost-update（历史 image_generate stale bug 的根因）
- 写路径自动 emit `config:changed` 事件并落 autosave 备份，不要手动模拟

旧的 `AppState::config: Mutex<AppConfig>` 字段已于 2026-04-20 删除，PR 里出现 `state.config.lock()` 一律 reject。

## 文档维护

技术文档索引见 [`docs/README.md`](docs/README.md)，分为架构文档（`docs/architecture/`）和调研文档（`docs/research/`）。

代码改动时**必须同步更新文档**：

| 改动类型                  | 需更新                                          |
| ------------------------- | ----------------------------------------------- |
| 新增/删除功能、命令、模块 | `CHANGELOG.md`、`AGENTS.md`                     |
| 技术栈/架构/规范变更      | `AGENTS.md`                                     |
| 已有子系统架构变更        | `docs/architecture/` 对应文档                   |
| 新增架构级能力            | `docs/architecture/` 新建文档 + `docs/README.md` 索引   |
| 新增/删除 Tauri 命令、HTTP 路由、`COMMAND_MAP` 条目 | `docs/architecture/api-reference.md` 对应功能域表格 |
| 功能变化导致 README 过时  | `README.md` + `README.en.md`（同一 PR 双语同步） |
| 新增调研/对比分析         | `docs/research/` 新建调研文档                   |
| 修改 README 任一语言版本  | 同一 PR 内同步另一语言（`README.md` ↔ `README.en.md`） |
| 新增/修改 Release Notes   | 同一 PR 内中英双份（`docs/release-notes/vX.Y.Z.md` ↔ `vX.Y.Z.en.md`） |

- `CHANGELOG.md`：[Keep a Changelog](https://keepachangelog.com/) 格式
- `docs/README.md`：文档索引，新增/删除文档时同步更新
- **架构文档强制**：功能架构发生改动（子系统边界、数据流、持久化格式、跨模块 contract 等）必须回头更新 `docs/architecture/` 下对应文档；**新增架构级能力**（新子系统、新核心模块、新协议层）必须在同一 PR 内新建架构文档并登记到 `docs/README.md`，不能只在 `AGENTS.md` 里一笔带过
- **README 回写强制**：功能变化（新增/删除/重大行为改动）导致 `README.md` 里描述过时、截图失效、功能矩阵漂移时，必须在同一 PR 内更新 `README.md`（并连带 `README.en.md`），不能留"下次顺手改"
- **README 双语同步强制**：项目根目录只有 `README.md`（中文）和 `README.en.md`（英文）两个面向用户的 README。**任何对其一的文案/结构/截图/链接改动，必须在同一次提交内对另一份做等价修改**，不能留待后续"顺手补"，避免中英版本漂移
- **Release Notes 双语同步强制**：`docs/release-notes/` 下每个版本必须同时有 `vX.Y.Z.md`（中文）和 `vX.Y.Z.en.md`（英文），两份顶部互加 `简体中文 · English` 切换链接。任一改动在同一次提交内同步到另一份，与 README 规则一致
