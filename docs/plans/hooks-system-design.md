# Hope Agent Hooks 系统 — 设计与实施方案

> 本设计以 **100% 字段级对齐 Claude Code hooks 协议**为硬目标，让用户可以把社区里现成的 hook 脚本（shell / HTTP 端点）原封不动搬进 Hope Agent。
>
> 参考：`https://code.claude.com/docs/en/hooks`（2026-04-26 复核）

## 修订记录（vs 上一版）

本次根据官方文档最新版（2026-04-26 抓取）做的字段级对齐修正，已直接合并进各章节，集中标注如下，避免实施时按旧版踩坑：

| # | 章节 | 旧版 | 新版（已对齐官方） |
|---|------|------|-------------------|
| R1 | 全局事件总数 | 27 | **28**（新增 `UserPromptExpansion` / `PostToolBatch`，原文档里漏掉了这两个） |
| R2 | Handler 种类 | 4（command / http / prompt / agent） | **5**（新增 **`mcp_tool`** —— 直接调用已配置 MCP 服务器的某个工具） |
| R3 | `PermissionRequest` 输出 | 含 `decision.interrupt` 字段 | **保留 `interrupt`**（官方原文："For deny only: if true, stops Claude"）；同时 `updatedPermissions` 扩展为 6 种 type：`addRules` / `replaceRules` / `removeRules` / `setMode` / `addDirectories` / `removeDirectories`，behavior 增加 `ask` 值 |
| R4 | `PermissionDenied` 输入 | `reason` | **保留 `reason`**（官方文档未发布详细 input schema，仅说明返回 `{retry: true}` 语义）；`if:` 字段作用域增加 `PermissionDenied` |
| R5 | `Notification` 输入 | `message` + `title` | **保留 `message` + 可选 `title`**（官方文档未发布详细 input schema，matcher 仍是 `notification_type`） |
| R6 | `PostToolUseFailure` 输入 | `error` / `is_interrupt` | 增加 **`duration_ms`** |
| R7 | `SessionEnd` source 枚举 | 含 `bypass_permissions_disabled` / `other` | **保留 6 个值**：`clear` / `resume` / `logout` / `prompt_input_exit` / `bypass_permissions_disabled` / `other`（官方 matcher 表完整列出） |
| R8 | `SubagentStart` 输入 | 缺细节 | 官方明确：`agent_type` / `prompt` / `subagent_id`；**子代理语境下 `Stop` hook 自动转 `SubagentStop`** |
| R9 | `WorktreeCreate` 输入 | 仅占位 | 官方明确：`worktree_path` / `branch`；返回值 `hookSpecificOutput.worktreePath`（HTTP）或 stdout 路径（command） |
| R10 | `Elicitation` / `ElicitationResult` 输入 | 仅占位 | 官方提供完整 schema（含 `mcp_server_name` / `elicitation_form.fields[]` / `user_response`）—— 占位字段更新为官方结构 |
| R11 | Plugin hooks.json | 仅占位 | 官方明确顶层可带 `description` 字段；`hooks/hooks.json` 路径固定 |
| R12 | `/hooks` 菜单源标签 | 提到 User/Project/Local/Plugin/Skill | 官方实际是 `User` / `Project` / `Local` / `Plugin` / `Session` / `Built-in`（无 `Skill`，skill 注入算 `Session` 临时）|
| R13 | `permissionDecision: defer` | 一期降级为 ask | 官方明确仅在 `-p` 非交互模式有效，桌面交互模式本就不应触发 → 降级到 ask 是合规行为，不算差异 |

---

## 审核意见处理记录

> 记录来源：2026-04-26 Codex 设计 review。
>
> 状态约定：`待处理` = 已确认但正文尚未完成修正；`已处理` = 正文已经按意见修正并完成一致性检查。

| # | 优先级 | 状态 | 位置 | 审核意见 | 处理要求 |
|---|--------|------|------|----------|----------|
| F1 | P0 | **已处理（2026-04-26）** | §0 修订记录 R3–R7 / §2.1 / §5.1.5 / §5.2.5 / §5.2.6 / §8.4 | 官方协议字段对齐反了：当前官方文档里 `PermissionDenied` 仍是 `reason`，`Notification` 仍是 `message`/可选 `title`，`PermissionRequest` 仍有 `decision.interrupt`，`SessionEnd` 使用 `reason` 且包含 `bypass_permissions_disabled`/`other`。本文把这些字段改成另一套名字，会让官方/社区 hook 脚本零改动复用目标失败。 | **结论：审核成立。** 起因：第一次 WebFetch 的 small model 把字段名"美化"了（`reason→denial_reason`、`message→notification_text`），让我误以为官方更新了 schema。第二次直接抓原文表格才发现真相。已撤回 R3/R4/R5/R7 的字段改名，恢复 `interrupt`、`reason`、`message`/`title`、SessionEnd 6 值。R6（PostToolUseFailure 加 `duration_ms`）保留 —— 那是新增字段不冲突。 |
| F2 | P0 | **已处理（2026-04-26）** | §9.3 / §9.4 | `PreToolUse allow` 会绕过硬安全门：§9.3 写成 `allow -> 跳过 Plan Mode / Approval gate`，但同文后面又说不能绕过 Plan Mode / `denied_tools`。 | **结论：审核成立。** 已重写 §9.3 为四步显式流程（PreToolUse hook → 硬红线 gate → approval gate → dispatch），明确 `allow` **只跳过 user-facing approval prompt**，硬红线（skill `allowed-tools` / Agent capabilities / Plan Mode allowlist / explicit deny rule）一律保留；`updatedInput` patch 之后必须重新过一遍 `enforce_hard_denies`。§9.4 伪代码同步增加 step 2 helper 调用。新增"易错点速查"表防止 reviewer 误读。 |
| F3 | P0 | **已处理（2026-04-26）** | §5.1.3 | `UserPromptSubmit block` 落点太晚：设计要求 block 时 user message 不进历史，但当前 Tauri / HTTP 入口已经在调用 chat engine 前 append 用户消息并可能生成标题。若 hook 放在 `AssistantAgent::chat()`，阻断后 DB / FTS / title 已经污染。 | **结论：审核成立。** 已新增"统一 preflight 层"方案：`agent::preflight::user_prompt_preflight` helper，所有四个入口（桌面 Tauri / HTTP / IM channel / ACP / cron）在调 `chat()` **之前**先调它；只有 `Proceed` 才允许持久化。Block 时 user message **不进 DB / FTS / title**（不靠事后回滚），通过 EventBus 通知前端展示临时拦截气泡。Phase 0.1 PR 包含三入口改造 + helper 骨架（noop 透传），Phase 1.2 真正接入 hook block 时不再改入口。preflight 失败 fail-open，managed policy 可强制 fail-closed。 |
| F4 | P1 | **已处理（2026-04-26）** | §4 / §18 | 配置层级基础设施被低估：本文同时写 `~/.hope-agent/settings.json`、`AppConfig`、project/local/managed 多作用域，但仓库当前只有全局 `~/.hope-agent/config.json` 的 `mutate_config` 路径。 | **结论：审核成立。** 已在 §4 顶部加 callout 明示"多 scope 框架不存在"，并在 §18 拆出独立的 **Phase 0.2 — 多 scope 配置基础设施 PR**，包含 `ConfigScope` 模块、四份配置文件 CRUD、merge、`notify` watch、managed owner 校验、回滚、`mutate_config_scoped` API。Phase 1 hooks 落地在 0.2 合入前**只能**支持 user scope（即写顶层 `~/.hope-agent/config.json` 的 `hooks` 字段），完整四层是 0.2 之后才解锁。Phase 0.2 是通用基建，不限 hooks，未来 permissions / mcp / approval rules 都受益。 |
| F5 | P1 | **已处理（2026-04-26）** | §5.2.4 / §2.1 / 附录 A | `PostToolBatch` 触发范围不兼容：官方语义是每个 tool batch 完成后恰好触发一次，包含整个 batch；本文限定"并发 safe 且数量 >=2"会漏掉单工具 batch、顺序工具和混合 batch。 | **结论：审核成立。** 已重写触发条件：每个 API round（含 1 个 / 多个 / 并发 / 串行 / 混合）的 tool call 全部 settle 后触发**一次**，仅 0 tool call 的纯文本 round 不触发。埋点位置改为 `streaming_loop.rs::append_round_to_history` 之前；payload 加 `round_id`（对齐 AGENTS.md `_oc_round` 元数据）+ 每 call 增加 `async_deferred` 标记。§2.1 / 附录 A 同步更新。 |

---

## 0. Context

当前 Hope Agent 在工具调用、会话生命周期、上下文压缩等关键节点只有内置默认行为，用户/企业无法插入策略：

- 想要在 `Bash` 执行前做命令黑名单（阻断 `rm -rf`）必须改源码；
- 想要在 `Write/Edit` 之后跑 `prettier` / `rustfmt`，没有扩展点；
- 想要在 `UserPromptSubmit` 注入"今天的待办"、脱敏敏感字，没有扩展点；
- 想要在 `SessionStart` 从 env 里加载项目 context，没有扩展点；
- 企业场景下要做审计/合规，目前只能靠事后读日志。

Claude Code 已经把这一套"事件 → 可拔插处理器"做成了成熟协议（**28 个事件 / 5 种处理器类型** / 四层配置作用域 / exit-code + JSON 双通道输出约定）。与其从零设计，不如**完全对齐**该协议：

1. 社区生态可复用：`claude-hooks-*` 类脚本、GitHub 上的 hook 样例、各种 awesome-list 资源，paste 即用；
2. 用户心智零迁移：已经熟悉 Claude Code 的人切到 Hope Agent 不用重学；
3. 文档负担更低：实现语义对齐官方文档，我们只需要写"差异说明"。

本方案是一份完整设计文档，**不写代码**。实施分阶段推进，见 §18。

---

## 1. 目标与非目标

### 1.1 目标（Must）

| # | 目标 | 验收标准 |
|---|------|---------|
| G1 | 字段级对齐官方 28 个 hook 事件 | 输入 payload / 输出 schema / 矩阵器语义逐字段 diff 零漂移 |
| G2 | 5 种 hook 处理器类型全支持 | `command` / `http` / `mcp_tool` / `prompt` / `agent` |
| G3 | 四层配置作用域 | user (`~/.hope-agent/settings.json`) / project (`<project>/.hope-agent/settings.json`) / local (`<project>/.hope-agent/settings.local.json`) / managed policy + skill/agent frontmatter |
| G4 | exit-code + JSON 双通道输出 | `exit 0` 解析 JSON；`exit 2` 阻断并 stderr → Claude；其它非阻断 |
| G5 | Matcher 三种语法完全兼容 | 纯字符串精确 / `A\|B` 列表 / 正则（检测到非 `[A-Za-z0-9_|]` 字符落入 regex 分支） |
| G6 | transcript_path 输出 JSONL（通过镜像） | 官方 hook 脚本用 `jq` 读取 transcript_path 能正常工作 |
| G7 | `CLAUDE_PROJECT_DIR` + `HOPE_PROJECT_DIR` 双注入 | 两个环境变量值一致，官方脚本 paste 即跑 |
| G8 | 热重载 | 修改 `settings.json` 或 GUI 面板保存后，下一次事件已使用新配置，无需重启应用 |
| G9 | GUI + 技能双入口 | `src/components/settings/hooks-panel/` 可视化编辑；`ha-settings` 技能 `category="hooks"` 读写 |
| G10 | 审计日志 | 每次 hook 触发、执行耗时、决策结果、非 0 退出都落 `app_info!` / `app_warn!`，category=`hooks` |

### 1.2 非目标（Won't do in this design）

- **不做**：MCP 服务器本体（官方 `Elicitation` / `ElicitationResult` 事件 payload 保留占位字段，MCP 落地另立 PR）。
- **不做**：`defer` 决策的 headless-mode `--resume` 协程流（当前 `hope-agent` 还没有 `-p` 非交互模式；`defer` 先实现为"等同 ask"的降级语义，加 TODO）。
- **不做**：`WorktreeCreate/Remove`（我们没有 worktree 隔离功能；事件声明但始终不触发，未来如补 worktree 再激活）。
- **不做**：`agent` 类型 hook 的 Tool 访问权限隔离；一期按"和 side_query 一样的能力"落地，不做沙箱。
- **不改**：现有 `approval` / `dangerous mode` / `Plan Mode` 的既有语义。Hook 是"叠加在这些机制之前的新一层"，它们的关系见 §9.3。

### 1.3 和现有系统的关系

```
┌─── UserPromptSubmit hook ───┐
User message ─────▶  chat() ─▶ streaming_loop ─▶ Provider ─▶ tool_call
                                                                  │
                          ┌── PreToolUse hook ◀───────────────────┤
                          ▼                                        │
                  Plan Mode gate ─▶ Approval gate (YOLO bypass) ───┤
                          │                                        │
                          ▼                                        │
                  tool dispatch                                    │
                          │                                        │
                          ▼                                        │
                  tool result                                      │
                          │                                        │
                          ▼                                        │
                  PostToolUse hook ──────────────────────────────▶ loop
```

Hook 层**加在现有 gate 的外侧**——先跑 hook，hook 没拦住才继续走既有的 Plan Mode / Approval / Dangerous 判定。反向亦然：`PostToolUse` 在结果回灌历史**之前**跑。

---

## 2. 协议兼容矩阵（28 事件总览）

下表一行代表一个官方 hook 事件，按"**一期落地 / 二期补全 / 占位不触发**"分组。**Matcher 目标列**告诉你该事件触发时 matcher 和哪个字段比对。**可阻断**列表明 `exit 2` 或 `{"decision": "block"}` 是否真的能拦住流程（对齐官方）。

### 2.1 一期落地（P0，共 13 个）

| # | 事件 | Matcher 目标 | 可阻断 | ha-core 触发位置（文件 : 函数） | 备注 |
|---|------|-------------|-------|-------------------------------|------|
| 1 | `SessionStart` | `source` ∈ {`startup`, `resume`, `clear`, `compact`} | ❌（事件已发生） | `agent::mod::chat` 首条消息前 & `session::resume::load_session` | 支持 `additionalContext` 注入 + `CLAUDE_ENV_FILE` |
| 2 | `SessionEnd` | `source` ∈ {`clear`, `resume`, `logout`, `prompt_input_exit`, `bypass_permissions_disabled`, `other`} | ❌ | `session::db::close_session` / app shutdown / logout / 危险模式回退 | 纯观察 |
| 3 | `UserPromptSubmit` | 无（始终触发） | ✅ | `agent::mod::chat` 收到 user message 之后、push 到 history 之前 | 可 block、可注入 `additionalContext` |
| 4 | `UserPromptExpansion` | 命令名（slash / mcp_prompt 名） | ✅ | `agent::system_prompt` 处理斜杠 / MCP prompt 展开前 | 在 prompt 真正展开成 LLM input 之前可拦；可改写 |
| 5 | `PreToolUse` | `tool_name` | ✅ | `tools::execution::execute_tool_with_context`，visibility 校验后、approval gate **之前** | 决策优先级 `deny > defer > ask > allow`；支持 `updatedInput` 改写工具入参 |
| 6 | `PostToolUse` | `tool_name` | ⚠️（仅注入上下文，不撤销结果） | `tools::execution::execute_tool_with_context` 结果返回后、落历史前 | `{"decision": "block"}` 追加 reason 给 LLM；MCP 工具支持 `updatedMCPToolOutput` |
| 7 | `PostToolUseFailure` | `tool_name` | ❌ | 同上，tool 返回 `Err` 或 panic 被捕获分支 | 纯观察 + `additionalContext` 注入 |
| 8 | `PostToolBatch` | 无（除非该轮 0 tool call） | ✅ | `agent::streaming_loop` 本轮 LLM assistant turn 全部 tool call settle 后、append 历史前 | 每个 API round 触发**一次**（无论并发/串行/混合，含单 tool）；`block` 等价于让 LLM 立刻自查（不撤销结果） |
| 9 | `PermissionRequest` | `tool_name` | ✅ | `tools::approval::check_and_request_approval` 弹窗前 | 可直接做决定，绕过 GUI 弹窗；支持写入 `updatedPermissions` 持久化规则 |
| 10 | `PermissionDenied` | `tool_name` | ❌ | approval 自动模式 classifier 否决时 | `retry: true` 让模型可再试一次 |
| 11 | `Stop` | 无 | ✅ | `agent::streaming_loop::run` 自然结束（`natural_exit=true`），emit_usage 前 | `block` 会让循环多跑一轮；**子代理语境下自动转 `SubagentStop`**（官方约定） |
| 12 | `PreCompact` | `trigger` ∈ {`manual`, `auto`} | ✅ | `context_compact::engine::run_compaction` 入口 | `block` 跳过本次压缩（下次使用率更高时会再触发） |
| 13 | `PostCompact` | 同上 | ❌ | 压缩完成、写入新 history 之后 | 纯观察 |
| 14 | `Notification` | `notification_type` ∈ {`permission_prompt`, `idle_prompt`, `auth_success`, `elicitation_dialog`} | ❌ | `event_bus::emit` 在特定通道（见 §5） | 用于桌面通知桥接 |

> P0 实际事件 = 14 个（含上面 13 个常用 + Notification 桥接）。如果限定"决策语义"事件 13 个，Notification 视作观察通道。

### 2.2 二期补全（P1，共 10 个）

| # | 事件 | Matcher 目标 | 可阻断 | ha-core 触发位置 | 备注 |
|---|------|-------------|-------|----------------|------|
| 15 | `SubagentStart` | agent type | ❌ | `subagent::spawn::spawn_subagent` emit `spawned` 后 | 子会话 prompt 可注入 `additionalContext` |
| 16 | `SubagentStop` | agent type | ✅ | `subagent::spawn` 子任务 terminal state 更新后 | `block` 让父注入再跑一轮；子代理语境下 `Stop` 也走这条 |
| 17 | `StopFailure` | error type | ❌ | `failover::executor` 最终分类错误 | Claude Code 错误类型直接复用 |
| 18 | `TaskCreated` | 无 | ✅ | `subagent::spawn_and_wait` 或未来 TaskCreate 工具 | P1 先复用 `subagent` |
| 19 | `TaskCompleted` | 无 | ✅ | 同上 terminal | 同上 |
| 20 | `TeammateIdle` | 无 | ✅ | `team::runtime` 检测到 teammate 即将 idle | 团队模式才有 |
| 21 | `ConfigChange` | `source` ∈ {`user_settings`, `project_settings`, `policy_settings`}（输入 schema 里 `config_source` 还可能是 `local_settings` / `skills`，matcher 仅前 3 项） | ✅ | `config::persistence::mutate_config` 事务提交前 | hook 可 `block` 回滚 |
| 22 | `CwdChanged` | 无 | ❌ | 新增 `session::cwd::set_cwd` 入口 | 需要先建立 session-level cwd 概念 |
| 23 | `FileChanged` | 字面文件名（`.envrc\|.env` 形式，**非正则**） | ❌ | 新增 `project::file_watcher`（notify crate） | 文件监听是新基础设施 |
| 24 | `InstructionsLoaded` | `load_reason` ∈ {`session_start`, `nested_traversal`, `path_glob_match`, `include`, `compact`} | ❌ | `agent::system_prompt` 组装时记录每次 CLAUDE.md / AGENTS.md 加载 | |

### 2.3 占位不触发（P2 / 未来启用，共 4 个）

| # | 事件 | 状态 | 启用条件 |
|---|------|------|---------|
| 25 | `Elicitation` | 占位 | MCP server 落地（另一个大坑） |
| 26 | `ElicitationResult` | 占位 | 同上 |
| 27 | `WorktreeCreate` | 占位 | worktree 隔离能力落地（当前 `isolation: "worktree"` 仅子代理用） |
| 28 | `WorktreeRemove` | 占位 | 同上 |

### 2.4 协议差异红线

> 任何不能对齐的官方字段都必须明确登记在这里，**不能隐藏差异**。

| 字段 / 语义 | 官方 | Hope Agent 实现 | 影响 |
|------------|------|----------------|------|
| `transcript_path` | 指向 JSONL 文件 | 通过 §10 的 JSONL 镜像实现，值 = `~/.hope-agent/sessions/{id}/transcript.jsonl` | 无差异（用户透明） |
| `permission_mode` | `default\|plan\|acceptEdits\|auto\|dontAsk\|bypassPermissions` | 一期仅 `default\|plan\|bypassPermissions`（对应现有 YOLO） | 社区脚本若硬 switch 5 值需兜底 `other` |
| `cwd` | 进程 cwd | session-level cwd（每会话独立，见 §11） | 更精准 |
| `CLAUDE_PROJECT_DIR` | 项目根 | 双注入 `CLAUDE_PROJECT_DIR` + `HOPE_PROJECT_DIR`，值一致 | 无差异 |
| `defer` 决策 | headless 模式阻塞 | 一期降级为 `ask`（手工审批） | 脚本收到 `defer` 会等价 ask，加日志告警 |
| `CLAUDE_ENV_FILE` | SessionStart / CwdChanged / FileChanged 可用 | 一期仅 SessionStart；CwdChanged/FileChanged 二期 | 降级：SessionEnd 前统一 `source` env 一次 |
| `if:` 字段（permission rule syntax） | Bash rule 细到子命令 | 一期仅 tool-name 级 + naive substring；二期补 Bash subcommand 解析 | 脚本写 `Bash(rm *)` 一期走 substring，复杂 pipeline 不拆子命令 |

---

## 3. 整体架构 & 模块划分

### 3.1 模块图

```
crates/ha-core/src/hooks/           <-- 新模块，本期主体
├── mod.rs                          公共类型导出 + HookDispatcher 入口
├── types.rs                        HookEvent enum / HookInput / HookOutput 数据结构
├── config.rs                       HooksConfig + matcher 反序列化 / scope 合并
├── matcher.rs                      字符串 / pipe-list / regex 判别器
├── registry.rs                     按 event → Vec<HookHandler> 的内存索引（热重载）
├── runner/                         handler 执行层
│   ├── mod.rs                      HookHandler trait + 公共调度器（并行 / 去重 / 超时）
│   ├── command.rs                  shell 子进程（复用 tools::exec 的 spawn 模式）
│   ├── http.rs                     reqwest + security::ssrf 校验
│   ├── prompt.rs                   走 side_query，single-turn
│   └── agent.rs                    走 spawn_subagent，多轮 tool loop
├── decision.rs                     多 hook 结果聚合（deny > defer > ask > allow）
├── transcript.rs                   JSONL 镜像写入器（§10）
├── env.rs                          环境变量组装（§11）
├── audit.rs                        审计日志 + 指标上报
└── tests/                          事件/matcher/runner 单测
```

对外只导出：

- `hooks::HookDispatcher`：`async fn dispatch(event: HookEvent, input: HookInput) -> HookOutcome`
- `hooks::HookEvent`：事件枚举（28 个变体，复用官方命名）
- `hooks::HookInput` / `HookOutput`：输入输出 JSON 结构
- `hooks::init(config_source)`：在 `ha-core::init` 里早期调用，完成 registry 预加载 + 文件监听

### 3.2 埋点调用形态

业务代码只需要一行：

```rust
// PreToolUse 示例（伪代码，见 §5.4）
let outcome = hooks::dispatch(HookEvent::PreToolUse, HookInput::PreToolUse {
    common: ctx.common_hook_input(),
    tool_name: call.name.clone(),
    tool_input: call.input.clone(),
    tool_use_id: call.id.clone(),
}).await;
match outcome.decision {
    HookDecision::Allow | HookDecision::Ask => { /* 继续走原有 approval gate */ }
    HookDecision::Deny { reason } => { return ToolResult::denied(reason); }
    HookDecision::Defer => { return ToolResult::deferred(); }  // 一期降级为 Ask
}
if let Some(patched) = outcome.updated_input { call.input = patched; }
for ctx_line in outcome.additional_context { /* inject to next turn */ }
```

**契约**：

- `dispatch` 内部封装 matcher 过滤、并行执行、超时、去重、聚合。调用方只读 `HookOutcome`。
- `HookOutcome::noop()` 是默认值——没配 hook / 全部超时失败时，业务层按"当没发生"继续跑。
- 严禁在业务代码里 match 具体 handler 类型（command / http / …），那是 runner 的事。

### 3.3 数据流：一次 PreToolUse 完整调用链

```
execute_tool_with_context(call)
  │
  ├─▶ hooks::dispatch(PreToolUse, input)
  │     │
  │     ├─▶ registry.matching_handlers(PreToolUse, tool_name)   // matcher 过滤
  │     │     → [h1: command, h2: http, h3: prompt]
  │     │
  │     ├─▶ dedupe_by_identity([h1, h2, h3])                    // 官方：同命令/同 URL 去重
  │     │
  │     ├─▶ join_all([                                          // 并发执行
  │     │       command::run(h1, input, env, timeout=10s),
  │     │       http::run(h2, input, timeout=30s),
  │     │       prompt::run(h3, input, timeout=30s),
  │     │   ])
  │     │
  │     ├─▶ parse_each(stdout, exit_code)                       // §8 协议
  │     │     → [Decision::Allow, Decision::Deny, Decision::Ask]
  │     │
  │     ├─▶ decision::aggregate(...)                            // deny > defer > ask > allow
  │     │     → HookOutcome { decision: Deny, reason, updated_input: None, ... }
  │     │
  │     └─▶ audit::log(event, handlers, outcome, duration)
  │
  ├─▶ if Deny → return denied result to tool loop
  ├─▶ else → 原有 Plan Mode / Approval / YOLO 判定
  └─▶ 执行工具
```

### 3.4 关键数据结构（草案）

```rust
// types.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]  // 对齐官方 SessionStart / PreToolUse 命名
pub enum HookEvent {
    SessionStart, SessionEnd,
    UserPromptSubmit, UserPromptExpansion,                          // ←新增 UserPromptExpansion
    PreToolUse, PostToolUse, PostToolUseFailure, PostToolBatch,     // ←新增 PostToolBatch
    PermissionRequest, PermissionDenied,
    Stop, StopFailure,
    PreCompact, PostCompact,
    Notification,
    SubagentStart, SubagentStop,
    TaskCreated, TaskCompleted, TeammateIdle,
    ConfigChange, CwdChanged, FileChanged,
    InstructionsLoaded,
    Elicitation, ElicitationResult,        // P2 占位（schema 已对齐）
    WorktreeCreate, WorktreeRemove,        // P2 占位（schema 已对齐）
}
// 共 28 个变体

#[derive(Debug, Clone)]
pub struct CommonHookInput {
    pub session_id: String,
    pub transcript_path: PathBuf,
    pub cwd: PathBuf,
    pub permission_mode: PermissionMode,   // 对齐 §2.4 差异
    pub hook_event_name: &'static str,     // HookEvent 的字面值
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum HookInput {
    SessionStart { common: CommonHookInput, source: SessionStartSource, model: String, agent_type: Option<String> },
    UserPromptSubmit { common: CommonHookInput, prompt: String },
    PreToolUse { common: CommonHookInput, tool_name: String, tool_input: serde_json::Value, tool_use_id: String },
    PostToolUse { common: CommonHookInput, tool_name: String, tool_input: Value, tool_response: Value, tool_use_id: String },
    // ... 其余 22 个变体见 §5
}

#[derive(Debug, Default)]
pub struct HookOutcome {
    pub decision: HookDecision,                        // Allow / Deny / Ask / Defer / Block
    pub continue_execution: bool,                      // continue=false 时终止整个循环
    pub stop_reason: Option<String>,                   // 配合 continue=false
    pub system_message: Option<String>,                // 展示给用户
    pub additional_context: Vec<String>,               // 注入下一轮 LLM 上下文
    pub updated_input: Option<serde_json::Value>,      // 仅 PreToolUse / PermissionRequest
    pub updated_mcp_output: Option<serde_json::Value>, // 仅 PostToolUse (MCP)
    pub updated_permissions: Vec<PermissionUpdate>,    // 仅 PermissionRequest
    pub session_title: Option<String>,                 // 仅 UserPromptSubmit
    pub retry: bool,                                    // 仅 PermissionDenied
}
```

### 3.5 组件职责边界

| 组件 | 只负责 | 不负责 |
|------|--------|--------|
| `HookDispatcher` | 入口 + 协调 registry/runner/decision/audit | 业务判定 |
| `HookRegistry` | 保存"事件 → handler 列表"的索引；热重载 | 执行 handler |
| `HookRunner::*` | 把 handler 跑起来，拿到 `RawHookResult` | 业务语义转换 |
| `HookDecision::aggregate` | 按官方优先级合并多个 `RawHookResult` | 落日志 |
| `HookAudit` | 落审计 + 埋点 | 影响决策 |
| `TranscriptMirror` | JSONL 镜像独立后台 writer | 其它 |

### 3.6 为什么放 `ha-core` 而不是 `src-tauri`

1. HTTP 守护进程模式（`hope-agent server`）也需要跑 hook；
2. ACP 模式同样需要；
3. Hook 的核心逻辑是纯逻辑（matcher / 决策聚合），不依赖 Tauri API；
4. 符合 AGENTS.md 分层约定："业务逻辑全进 ha-core"。

唯一跟 Tauri 有交集的是"桌面通知桥"和"GUI 面板"——那是 `src-tauri` / `src/` 的事，Hook 核心只负责 emit event。

---

## 4. 配置 Schema

> **F4 前置依赖（必读）**：本节描述的 **多 scope 配置框架（user / project / local / managed / skill 五层合并）目前在仓库里并不存在**。当前 `crates/ha-core/src/config/` 只支持单一全局 `~/.hope-agent/config.json`，`mutate_config((category, source), |cfg| {...})` 也只写这一处文件；project / local / managed 三层的文件读写、合并、watch、回滚、GUI/skill 写入 contract 全部 **没有**。
>
> 这意味着 hooks 系统**不能**直接按本节方案落地——必须先单独立一个"多 scope 配置基础设施" PR 把这套框架补上（见 §18 Phase 0.2）。在那之前 hooks 实现**只能**支持 user scope（即 `~/.hope-agent/settings.json` 写顶层 `hooks` 字段，与 `AppConfig.hooks` 等价）。
>
> 把多 scope 框架先抽干净有几个好处：
> 1. 它是 hooks 之外的通用能力 —— 后面 permissions、approval rules、mcp 配置都会受益
> 2. hooks 系统第一版只面对单 scope，复杂度大幅降低，可独立验证 dispatcher / runner / matcher 是否对齐官方
> 3. 多 scope 框架本身的 corner case（合并优先级、权限校验、并发写）单独验证，不和 hooks 业务逻辑搅在一起
>
> 本章节其余部分按 **目标态**（多 scope 框架已就位）描述，以保留完整设计意图；阶段性落地以 §18 为准。

### 4.1 作用域（四层 + 两种嵌入）

按加载顺序（后加载优先级更高，但 `managed` 例外，永远最高）：

| 作用域 | 路径 | 典型用途 | 可被 GUI 编辑 | 进入 git |
|--------|------|---------|-------------|---------|
| `userSettings` | `~/.hope-agent/settings.json` | 个人偏好、全局规则 | ✅ | ❌ |
| `projectSettings` | `<project>/.hope-agent/settings.json` | 团队共享的项目规则 | ✅ | ✅ 推荐 |
| `localSettings` | `<project>/.hope-agent/settings.local.json` | 个人在某项目的临时覆盖 | ✅ | ❌ gitignore |
| `managedPolicy` | macOS `/Library/Application Support/HopeAgent/managed-settings.json` / Linux `/etc/hope-agent/managed-settings.json` / Windows `%ProgramData%\HopeAgent\managed-settings.json` | 企业统一策略 | ❌（IT 部署） | ✅（部署流程） |
| `skill/agent frontmatter` | `SKILL.md` / agent definition YAML 头 | 技能/身份激活时附带的 hooks | ⚠️（通过 skill 编辑器） | ✅ |
| `plugin hooks.json`（未来） | `<plugin>/hooks/hooks.json` | 插件包体自带 hook | ❌ | ✅ |

**合并规则**：同一 `HookEvent` 的所有 matcher group **累加**（不是覆盖），但去重按 `(handler_type, handler_identity)` 跑一次——identity = command 按命令字符串，http 按 URL，prompt/agent 按 prompt 文本哈希。`disableAllHooks: true` 会关掉除 managed 之外所有 hook；managed 永远不能被下层关闭。

### 4.2 `hooks` 顶层键（`AppConfig` 扩展）

在 `crates/ha-core/src/config/mod.rs` 的 `AppConfig` 上加一个字段：

```rust
#[serde(default)]
pub hooks: HooksConfig,

#[serde(default)]
pub disable_all_hooks: bool,   // 对齐官方 disableAllHooks
```

`HooksConfig` 结构：

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]  // 事件名用 PascalCase
pub struct HooksConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_start: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub session_end: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_prompt_submit: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_prompt_expansion: Vec<HookMatcherGroup>,    // ←新增
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_tool_use: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_use: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_use_failure: Vec<HookMatcherGroup>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_batch: Vec<HookMatcherGroup>,          // ←新增
    // ... 余下事件同样字段（共 28 个）
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcherGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,      // None == "*"（始终匹配）
    pub hooks: Vec<HookHandlerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookHandlerConfig {
    Command(CommandHookConfig),
    Http(HttpHookConfig),
    McpTool(McpToolHookConfig),       // ←新增第 5 种
    Prompt(PromptHookConfig),
    Agent(AgentHookConfig),
}

// 新增：mcp_tool handler 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolHookConfig {
    pub server: String,                                 // 已配置的 MCP server 名（参考 §McpGlobalSettings.servers）
    pub tool: String,                                   // 该 server 暴露的某个 tool 名（不带 mcp__ 前缀）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,               // 入参模板，支持 ${tool_input.field} / ${prompt} 这类占位符插值
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,                           // 秒，默认沿用 MCP server 自身的 RPC 超时（建议 ≤ 30s）
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHookConfig {
    pub command: String,                                // shell 字符串
    #[serde(default)]
    pub shell: Option<HookShell>,                       // bash | powershell
    #[serde(default)]
    pub timeout: Option<u64>,                           // 秒，默认 600
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_run: Option<bool>,                        // #[serde(rename = "async")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_rewake: Option<bool>,                     // #[serde(rename = "asyncRewake")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "if")]
    pub if_rule: Option<String>,                        // 权限规则语法
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,                             // 仅 skill/agent frontmatter 语境有效
}

// HttpHookConfig, PromptHookConfig, AgentHookConfig 类似
```

**字段命名对齐**：

- 顶层键用 **PascalCase**（`SessionStart`, `PreToolUse`, `UserPromptExpansion`, `PostToolBatch`）—— 官方如此，不按 camelCase 惯例。
- 处理器内字段用 **camelCase**（`asyncRewake`, `statusMessage`, `allowedEnvVars`）—— 官方如此。
- `type` 值用 **小写 snake-like**：`"command" | "http" | "mcp_tool" | "prompt" | "agent"`。
- **`async` 关键字冲突**：Rust 里不能用 `async` 做字段名，所以 `#[serde(rename = "async")]` 映射到 `async_run`。
- **`mcp_tool` 是 5 种 handler 中"零脚本"路线**：直接调用已配置 MCP 服务器上的某个工具，免写 shell / 部署 HTTP；常见用法是把"安全扫描"、"格式化"这种已有 MCP tool 接到 `PostToolUse` 上。`input` 模板里支持的占位符：所有 `tool_input.*` / `tool_response.*` / 通用字段（`session_id` / `cwd` / ...），见 §7.5。

### 4.3 `~/.hope-agent/settings.json` 完整示例

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "\"$CLAUDE_PROJECT_DIR\"/.hope-agent/hooks/block-rm.sh",
            "if": "Bash(rm *)",
            "timeout": 10
          }
        ]
      },
      {
        "matcher": "mcp__.*__write.*",
        "hooks": [
          {
            "type": "http",
            "url": "http://localhost:8080/hooks/pre-tool-use",
            "timeout": 30,
            "headers": { "Authorization": "Bearer $MY_TOKEN" },
            "allowedEnvVars": ["MY_TOKEN"]
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "\"$HOPE_PROJECT_DIR\"/.hope-agent/hooks/fmt.sh",
            "async": true,
            "statusMessage": "Formatting code..."
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          { "type": "command", "command": "/path/to/validate-prompt.sh" }
        ]
      }
    ],
    "SessionStart": [
      {
        "matcher": "startup|resume",
        "hooks": [
          { "type": "command", "command": "~/.hope-agent/hooks/setup-env.sh" }
        ]
      }
    ]
  },
  "disableAllHooks": false
}
```

### 4.4 Skill / Agent Frontmatter 内嵌

对齐官方 frontmatter 写法，新增 `hooks:` 块解析（`ha-core::skills::metadata`）：

```yaml
---
name: secure-bash
description: Bash with extra safety
hooks:
  PreToolUse:
    - matcher: "Bash"
      hooks:
        - type: command
          command: "./scripts/security-check.sh"
          once: true              # 该 skill 激活期间仅跑一次
---
```

**生效范围**：仅在该 skill / agent 激活时注入 registry，切换后自动撤销。`once: true` 仅在 frontmatter 语境生效（设置文件里写会被忽略 + warn）。

### 4.5 `disableAllHooks` 的层级语义

| 层级 | 可写 `disableAllHooks: true` | 效果 |
|------|---------------------------|------|
| managed | ✅ | 彻底关（用户不能打开） |
| user | ✅ | 关 user/project/local/skill 级；managed 仍跑 |
| project | ✅ | 关 project/local/skill；user/managed 仍跑 |
| local | ✅ | 关 local/skill；其余仍跑 |

（对齐官方 hierarchy。`managed` 可额外用 `allowManagedHooksOnly: true` 禁止下层任何 hook。）

### 4.6 读写路径（对齐 AGENTS.md config contract）

- **读**：`ha_core::config::cached_config().hooks` —— 零克隆，热路径可以随便读。
- **写**：`ha_core::config::mutate_config(("hooks", source_label), |cfg| { cfg.hooks.pre_tool_use.push(...) })` —— 自动落盘 + emit `config:changed { category: "hooks" }` + 触发 §3.1 registry 热重载。
- **禁止**：自己 load → 改 → save 的手动三件套。

### 4.7 热重载触发链

```
用户改 settings.json（或 GUI / 技能）
  → config::persistence 检测写入
  → emit EventBus("config:changed", { category: "hooks" })
  → hooks::registry::reload(cached_config().hooks.clone())
  → 下一次 dispatch() 已经是新 registry
```

**不重启进程**。运行中的 hook 执行不打断（已经 spawn 出去的子进程不强杀）。

---

## 5. 事件埋点详细（Per-Event 5.1–5.3）

每个事件条目给你：**埋点位置**（文件:函数，精确到当前主仓代码行级位置）/ **Input JSON schema**（字段级 diff 对齐官方）/ **Output 处理**（允许的字段、阻断语义）/ **备注**（差异、边界）。

### 5.1 会话 & 用户输入事件（共 4 个）

#### 5.1.1 `SessionStart`

**埋点**：
- `crates/ha-core/src/agent/mod.rs::AssistantAgent::chat()` 入口——如果是该会话的**第一条 user message**，且 session 元数据中 `hooks_started=false`，触发 `source="startup"` 或 `source="resume"`（依 session 是否有历史消息判断）。
- `crates/ha-core/src/context_compact/engine::run_compaction` 成功完成后，触发 `source="compact"`。
- `/clear` 或等价操作（见未来 §13.x）清空历史后，触发 `source="clear"`。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "/Users/.../hope-agent/sessions/sess_xxx/transcript.jsonl",
  "cwd": "/Users/.../my-project",
  "permission_mode": "default",
  "hook_event_name": "SessionStart",
  "source": "startup",               // startup|resume|clear|compact
  "model": "claude-opus-4-7",
  "agent_type": "general"            // 可选
}
```

**Output 处理**：
- `exit 0 + stdout 含 JSON`：解析 `hookSpecificOutput.additionalContext` 追加到 system prompt（作为独立 cache block，不破坏前缀缓存）。
- `exit 0 + stdout 纯文本`：整段作为 `additionalContext`。
- `exit 2`：**不阻断**（事件已发生），stderr 打审计日志、通知用户。
- `CLAUDE_ENV_FILE`：一期支持，见 §11.3——hook 可 `echo 'export FOO=bar' >> $CLAUDE_ENV_FILE`，这些 env 在本会话后续 command hook 生效。

**备注**：
- `additionalContext` 拼在系统提示词末尾一个独立 section `## Session Context from Hooks`，上限 10000 字符（对齐官方）。
- `compact` 触发分支需要在 `engine::run_compaction` 里显式调 `HookEvent::SessionStart` with `source="compact"`。

---

#### 5.1.2 `SessionEnd`

**埋点**：
- 应用正常关闭（`src-tauri/src/lib.rs` 的 `on_window_event` 关闭分支 / `ha-server` 收到 SIGTERM）。
- 用户 logout（`ha_core::auth::clear_token`）。
- 会话被 `/clear` / resume 其它会话踢出。
- 一期暂不实现 `prompt_input_exit` / `bypass_permissions_disabled`（这两个需要 interactive shell 上下文，对应 Claude Code CLI 的 REPL 退出事件）。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "permission_mode": "default",
  "hook_event_name": "SessionEnd",
  "source": "clear"                  // clear|resume|logout|prompt_input_exit|bypass_permissions_disabled|other
}
```

**Output 处理**：不可阻断。exit 2 仅展示 stderr 给用户（桌面通知）。无 `hookSpecificOutput` 特殊字段。

**备注**：
- Desktop / ACP / server 三种运行形态都要 emit——EventBus 统一入口可避免重复代码。
- SessionEnd 触发在**数据库最终落盘之后**，因此 hook 里可以安全地 read `transcript_path`。

---

#### 5.1.3 `UserPromptSubmit`

> **F3 修复**：旧版本把埋点写在 `AssistantAgent::chat()` 内部 —— 但 Tauri / HTTP / IM 三个入口在调 `chat()` 之前 **已经** 把 user message 写进了 `session.db.messages`、`messages_fts` 索引、并可能触发 `session::metadata::generate_title`。如果 hook 此时 block，DB / FTS / title 已被污染。本节按"三入口共享 preflight"重写。

**埋点位置（统一 preflight 层）**：

新增 `crates/ha-core/src/agent/preflight.rs::user_prompt_preflight(session_id, raw_prompt, attachments) -> PreflightOutcome`，**所有**用户消息的入口在调 `chat()` **之前** 必须先调这个函数：

| 入口 | 文件 | 调用位置 |
|------|------|---------|
| 桌面 chat | `src-tauri/src/commands/chat.rs::send_message` | 在调 `agent.chat()` 之前；当前流程是先 DB persist → 再 chat()，需要倒过来：先 preflight → 持久化 → chat()；持久化失败时调用方负责回滚（已经在 ha-core 内做） |
| HTTP chat | `crates/ha-server/src/routes/chat.rs::post_chat` | 同上；HTTP 端 stream 启动前必须等 preflight 完成 |
| IM channel | `crates/ha-core/src/channel/worker/streaming.rs` | attachment 下载完成、归档为 `Attachment` 之后，调 `agent.chat()` 之前 |
| ACP | `crates/ha-core/src/acp/...` | ACP `prompt` request 处理入口 |
| 定时任务 / cron 触发 | `crates/ha-core/src/cron/runner.rs` | cron 自动 fire 也算一次 user prompt（system 视角）|

**`preflight` 内部行为**：
1. 触发 `UserPromptSubmit` hook（带 raw prompt、session common input）
2. 若 hook block → **直接返回 `Blocked { reason }`**，调用方 **不写入** DB / FTS / title，只通过 EventBus 给前端发一条 `user_prompt_blocked` 事件展示
3. 若 hook 注入 `additionalContext` / `sessionTitle` → 一并返回，调用方在持久化 user message 后再写 system-reminder 段 / 改 title
4. 若 hook 写了 `updatedInput`（如脱敏 prompt）→ 返回新 prompt，调用方用新 prompt 持久化（**不是**原文）

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "permission_mode": "default",
  "hook_event_name": "UserPromptSubmit",
  "prompt": "用户输入的原始文本"
}
```

**Output 处理**：
- `exit 0 + JSON`：
  - `decision: "block"` + `reason` → **拒绝本次 prompt**，preflight 返回 `Blocked`，user message **不进 DB / 不进 FTS / 不触发 title 生成**。前端展示拦截原因（system 消息样式，不进历史回放）。
  - `hookSpecificOutput.additionalContext` → 调用方在 user message 持久化后，作为独立 `system` 行追加（保证 Anthropic role alternation）。
  - `hookSpecificOutput.sessionTitle` → preflight 完成、user message 持久化后再调 `session::metadata::set_title`（避免 hook block 之后 title 已经写了）。
- `exit 0 + stdout 纯文本`：等价于 `additionalContext`。
- `exit 2`：block，stderr 回显用户。
- 其它非 0：非阻断，stderr 落日志，user message 正常持久化。

**Preflight 调用伪代码**（桌面 / HTTP / IM 共用）：

```rust
// 旧（错）：
//   db.insert_message(user_msg)?;
//   maybe_generate_title(user_msg)?;
//   agent.chat(user_msg).await?;

// 新（对）：
let outcome = agent::preflight::user_prompt_preflight(
    session_id, raw_prompt, attachments
).await?;
match outcome {
    PreflightOutcome::Blocked { reason } => {
        eventbus::emit("user_prompt_blocked", json!({
            "session_id": session_id, "reason": reason
        }));
        return Ok(ChatResponse::Blocked(reason));   // 调用方不再持久化
    }
    PreflightOutcome::Proceed { effective_prompt, additional_context, session_title } => {
        // 此处才允许 DB 写入
        let user_msg = Message::user(effective_prompt);
        db.insert_message(&user_msg)?;
        if let Some(title) = session_title {
            session::metadata::set_title(session_id, &title)?;
        } else {
            maybe_generate_title(&user_msg)?;
        }
        if let Some(ctx) = additional_context {
            db.insert_message(&Message::system_reminder(ctx))?;
        }
        agent.chat(user_msg).await?;
    }
}
```

**前端展示约定**：
- block 时前端显示一个**临时**气泡（icon = ⛔，文案 = `reason`），带"撤销/重写"按钮；点击撤销直接消失，点击重写把原文回填到输入框。
- 该气泡 **不进 session 历史**，刷新页面后消失。

**备注**：
- 附件（图片 / 文件）**不包含**在 `prompt` 字段里——官方只传文本。附件列表通过 `common.attachments[]`（我们扩展字段，不影响官方兼容：官方脚本读不到也不会错）。
- block 决策下，user message **不进入历史 / FTS / title**——三处都靠"preflight 之前不写"达成，**不**通过事后回滚（事后回滚有竞态，前端已收到事件就难撤）。
- preflight 失败时（hook runner 全 timeout / panic）**默认 fail-open**：proceed with raw prompt，记 `app_warn!` —— 否则一个挂掉的 hook 会让用户根本发不出消息。fail-closed 行为留给企业 managed policy 显式开启（`hooks.preflightFailClosed=true`）。

---

#### 5.1.4 `UserPromptExpansion`（新增 / P0）

**埋点**：
- `crates/ha-core/src/agent/system_prompt.rs` 处理 **斜杠命令展开**前——即用户输入 `/skill-name args` 被解析成 skill prompt + 转入 LLM history 之前。
- 未来 MCP prompt 集成后（P2 后续），处理 `mcp_prompt` 展开同样触发。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "permission_mode": "default",
  "hook_event_name": "UserPromptExpansion",
  "expansion_type": "slash_command",         // slash_command | mcp_prompt
  "command_name": "skill-name",              // 不含斜杠
  "command_args": "arg1 arg2",               // 原文 args
  "command_source": "project",               // plugin | project | user
  "prompt": "/skill-name arg1 arg2"          // 原始用户输入
}
```

**Matcher**：`command_name`（如 `meeting-notes`、`/^email-.*/`）。

**Output 处理**：
- `exit 0 + JSON`：
  - `decision: "block"` + `reason` → 拒绝展开，原始斜杠输入退回用户（前端展示拦截原因）。
  - `hookSpecificOutput.additionalContext` → 注入到展开后 prompt 之后作为 system-reminder。
- `exit 2`：block，stderr 回显用户。
- 其它：观察。

**典型用途**：拦截敏感 skill（"`/dump-secrets` 不要执行"）；为某些 skill 自动追加上下文（"`/code-review` 时自动 inline 当前 git diff"）。

**备注**：
- 这个 hook 跑在 `UserPromptSubmit` 之后、prompt 进 LLM history 之前。两者顺序：`UserPromptSubmit hook → 检测到斜杠 → UserPromptExpansion hook → expansion → push history`。
- 命令名校验失败（skill 不存在）走原有 NotFound 路径，不触发本 hook。

---

#### 5.1.5 `Notification`

**埋点**：
- `crates/ha-core/src/tools/approval::check_and_request_approval`：发起审批弹窗前，触发 `notification_type="permission_prompt"`。
- 会话空闲（见 AGENTS.md Memory 章"空闲超时兜底 flush" ~30 min）：触发 `notification_type="idle_prompt"`。
- OAuth 登录成功：`ha_core::auth` 成功分支触发 `notification_type="auth_success"`。
- MCP elicitation 弹窗（P2 占位）：`notification_type="elicitation_dialog"`。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "hook_event_name": "Notification",
  "notification_type": "permission_prompt",     // permission_prompt | idle_prompt | auth_success | elicitation_dialog
  "message": "Claude needs permission to run Bash",
  "title": "Tool approval required"             // 可选
}
```

> 字段说明：官方文档发布时只把 `notification_type` 写到 matcher 表里，并未提供完整 input schema。我们沿用 Claude Code 内部惯用的 `message` + 可选 `title` 字段（与桌面 notification API 同名，对接 hook 脚本最直观）。如未来官方公开新字段名再同步。

**Output 处理**：不可阻断。`hookSpecificOutput.additionalContext` 也会注入（适合"通知了用户，也告诉 Claude 我通知了"的场景）。

**备注**：
- 这个 hook 的典型用途是**桥接桌面通知到 Slack / 手机推送**——命令 hook 可以把 notification 转发出去。
- 我们内置的 `notification.soundEnabled` / `notification.osToastEnabled` 保留，hook 属于**额外**通道（不替代）。

---

### 5.2 工具 & 权限事件（共 7 个）

#### 5.2.1 `PreToolUse`

**埋点**：`crates/ha-core/src/tools/execution.rs::execute_tool_with_context`
- 位置：在 `check_visibility_and_policy` 完成之后、**在 approval gate 之前**。
- 前置条件：hook 能在这个点拿到**未修改**的 `tool_input`，决策能直接变成"跳过 approval" / "改写入参 + 继续" / "拒绝"。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "permission_mode": "default",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": {
    "command": "npm test",
    "description": "Run test suite"
  },
  "tool_use_id": "toolu_01abc"
}
```

`tool_input` 字段**与官方完全对齐**——Bash / Write / Edit / Read / Glob / Grep / WebFetch / WebSearch / Agent / AskUserQuestion 等按官方 schema 序列化（见 WebFetch 抓回的字段清单）。对于 ha-core 独有工具（`exec` / `subagent` / `skill` / `web_fetch` / `memory_*` / ...），用我们自己的 schema，tool_name 保留原名。

**Output 处理**（最复杂，对齐官方精确语义）：

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow|deny|ask|defer",
    "permissionDecisionReason": "...",
    "updatedInput": { /* 完全替换 tool_input */ },
    "additionalContext": "..."
  },
  "continue": true,
  "systemMessage": "...",
  "suppressOutput": false
}
```

| decision | 业务动作 |
|----------|---------|
| `allow` | 跳过 approval gate，直接执行 |
| `deny` | 返回 `ToolResult::error(reason)`，reason 进 LLM 历史；**不**执行 |
| `ask` | 走原有 approval gate（等同于没 hook） |
| `defer` | 一期：降级为 `ask`（见 §1.2 TODO） |

`updatedInput`：**完整替换** `tool_input` 对象；后续 hook / approval / 工具执行都看到新值。适合做脱敏（去掉 `command` 里的 secret）、纠错（自动加 `--` 分隔符）。

**聚合优先级**：`deny > defer > ask > allow`。多个 hook 给 `updatedInput` 时，按官方文档没有严格约定，本方案取"最后一个 allow/ask hook 的 updatedInput"；如有 deny 胜出，`updatedInput` 丢弃。

**备注**：
- Plan Mode 限制 + Dangerous YOLO 独立于 hook：hook `allow` 并不能绕过 Plan Mode allowlist；Plan Mode 拒绝时 PreToolUse 依然会触发（让 hook 能知道并记录），但 PermissionRequest 不会。
- `concurrent_safe` 工具仍按原逻辑并行，hook 也并行；`concurrent_safe=false` 工具串行，hook 同样按该工具调用的发生顺序串行触发。

---

#### 5.2.2 `PostToolUse`

**埋点**：`tools::execution::execute_tool_with_context`
- 位置：tool 返回 `Ok(result)` 之后、**在写入 history / EventBus emit_tool_result 之前**。
- 对 `async_capable` 工具的后台化路径：在主循环看来"同步返回"的那一刻触发；真正 async 完成时通过 `subagent::injection` 注入的路径算后续的 `PostToolUse`（参数里带原 `tool_use_id`）。

**Input**：

```json
{
  "session_id": "sess_xxx",
  "transcript_path": "...",
  "cwd": "...",
  "hook_event_name": "PostToolUse",
  "tool_name": "Bash",
  "tool_input": { /* 和 PreToolUse 一致 */ },
  "tool_response": {
    "stdout": "...", "stderr": "...", "exit_code": 0,
    "duration_ms": 234
  },
  "tool_use_id": "toolu_01abc"
}
```

**Output 处理**：

- `decision: "block"` + `reason`：**不撤销结果**（工具已经跑完了），但把 `reason` 附加到 tool_result 之后作为 system-reminder 给 LLM——用于"linter 检测到问题，让 Claude 看见并自查"场景。
- `hookSpecificOutput.additionalContext`：同上，追加 system-reminder。
- `hookSpecificOutput.updatedMCPToolOutput`（仅 MCP 工具）：**完全替换** tool_response（当前 MCP 未落地，保留字段解析即可）。
- `exit 2` = `{decision: "block", reason: stderr 内容}`。

**备注**：
- 该 hook **必须**在 tool_result 写回 LLM 历史前跑——否则"块化 PostToolUse"达不到"让 Claude 立刻看到 linter 错误"的效果。
- 对 Tool 的"磁盘持久化预览+路径引用"路径（见 AGENTS.md 工具结果磁盘持久化），hook 拿到的是**磁盘完整内容**，而 LLM 看到预览——避免 hook 因为内容被截断而误判。

---

#### 5.2.3 `PostToolUseFailure`

**埋点**：同 `PostToolUse`，但分支在 tool 返回 `Err` 或 `panic` 捕获。

**Input**：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "PostToolUseFailure",
  "tool_name": "Bash",
  "tool_input": { /* ... */ },
  "tool_use_id": "toolu_01abc",
  "error": "Command failed with exit code 1",
  "is_interrupt": false,                  // 用户中断=true
  "duration_ms": 234                      // 失败前耗时
}
```

**Output 处理**：纯观察 + `additionalContext` 注入。不可阻断。

---

#### 5.2.4 `PostToolBatch`（新增 / P0）

> **F5 修复**：旧版本写"仅当并发 ≥2 时触发"是错的。官方语义是 **每个 tool batch（即 LLM 一轮 assistant turn 里发出的所有 tool_use）settle 之后恰好触发一次**，无论这一轮里是 1 个 / 多个、并发 / 串行 / 混合。

**埋点**：`crates/ha-core/src/agent/streaming_loop.rs` —— 在本轮 LLM assistant turn **全部** tool call settle 之后、`append_round_to_history` 把整轮 tool_use + tool_result 落历史**之前**。每个 tool call 自己的 `PostToolUse` / `PostToolUseFailure` 仍然按 §5.2.2 / §5.2.3 在 tool 返回那一刻独立触发；`PostToolBatch` 是**整轮一次**的汇总钩子。

触发与不触发：

| 本轮 tool call 数 / 形态 | PostToolBatch 触发 |
|--------------------------|-------------------|
| 1 个 tool call（串行） | ✅（payload 含 1 条） |
| 2 个并发 concurrent_safe tool | ✅（payload 含 2 条） |
| 3 个串行 + 1 个失败 | ✅（payload 含 4 条，最后一条 `ok=false`） |
| 并发 + 串行混合 | ✅（payload 按发生顺序排，含全部）|
| 0 个 tool call（纯文本回复） | ❌ 不触发 |
| async 后台化路径 | 后台化的那个 call 在 batch 里以"标记 `async_deferred=true`"出现；真正完成后通过下一轮的 batch 重新出现（带原 tool_use_id） |

**Input**：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "PostToolBatch",
  "round_id": "_oc_round=42",                // 对齐 AGENTS.md "API-Round 消息分组" 语义
  "batch_size": 4,
  "tool_calls": [
    {
      "tool_name": "Read",
      "tool_input": { "file_path": "/abs/a.ts" },
      "tool_use_id": "toolu_01",
      "tool_response": "...",                // 或 error
      "duration_ms": 12,
      "ok": true,
      "async_deferred": false
    },
    {
      "tool_name": "Grep",
      "tool_input": { "pattern": "TODO" },
      "tool_use_id": "toolu_02",
      "tool_response": "...",
      "duration_ms": 88,
      "ok": true,
      "async_deferred": false
    }
    // ... 含本轮全部 tool call
  ]
}
```

**Matcher**：无（始终触发）。

**Output 处理**：

- `decision: "block"` + `reason`：**不撤销**任何工具结果（已写入），但把 `reason` 作为 system-reminder 追加给 LLM —— 用于"批量结果统一审计"场景，比如"这一批 Read 你已经读了 3 个文件了，先 summarize 再继续"。
- `hookSpecificOutput.additionalContext`：同上，追加 system-reminder。
- `exit 2` = `{decision: "block", reason: stderr}`。

**典型用途**：
- 在 LLM 一次并发读 6 个文件后强制提示 "进入综合分析阶段，停止继续 read"；
- 防御 LLM 一次性发起过多并发 Bash 副作用；
- 给整轮 tool 输出做集中 redact（写到 transcript 之前再过一遍 PII 过滤）。

**备注**：
- 触发**晚于** 同轮所有 PostToolUse —— 顺序：tool#1 完成 → PostToolUse(tool#1) → tool#2 完成 → PostToolUse(tool#2) → ... → 整轮 settle → **PostToolBatch** → append_round_to_history → 下一轮 LLM。
- 在本节中 "batch" = "API round"，与官方"tool batch"等价。`round_id` 对齐 AGENTS.md `_oc_round` 元数据，hook 作者按 round 聚合时直接用。
- async tool 的后台化路径**不**单独再触发一次 PostToolBatch；后台完成后通过 `subagent::injection` 注入主对话，注入的内容算下一轮的 tool result，会出现在那一轮的 PostToolBatch payload 里（带原 `tool_use_id`，便于 hook 作者关联）。

---

#### 5.2.5 `PermissionRequest`

**埋点**：`tools::approval::check_and_request_approval` **弹窗发起前**（即对接 UI 之前、hook 是"比 UI 先行一步的决策者"）。

**Input**：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": { /* ... */ },
  "permission_suggestions": [              // 可选，预填的建议规则
    {
      "type": "addRules",
      "rules": [{ "toolName": "Bash", "ruleContent": "Bash(npm test)" }],
      "behavior": "allow",
      "destination": "localSettings"
    }
  ]
}
```

**Output 处理**：

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": {
      "behavior": "allow|deny",
      "updatedInput": { /* 仅 allow 时生效，改写入参 */ },
      "updatedPermissions": [             // 仅 allow 时生效；将被写入对应 scope 的持久化规则；可一次性多条
        { "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": "Bash(npm test)" }], "behavior": "allow", "destination": "localSettings" }
      ],
      "message": "...",                   // 仅 deny 时，告诉 Claude 原因
      "interrupt": false                  // 仅 deny 时；true 时停掉 Claude（=顶层 continue=false 的语义快捷字段）
    }
  }
}
```

| behavior | 动作 |
|----------|-----|
| `allow` | **不弹 UI**，直接放行；可带 `updatedInput`、`updatedPermissions`。**注意官方原文**："a hook returning `allow` does not override a matching deny rule" —— 即 `allow` 会重新过一遍 deny / ask 规则，命中 deny 仍然拒。`updatedInput` 改写后的入参也会重新过 deny / ask 规则。|
| `deny` | **不弹 UI**，直接拒；`message` 回 LLM。如需停整个循环：`interrupt: true`（专属语义）或顶层 `continue: false`。|

**`updatedPermissions[].type` 6 种合法值（对齐官方）**：

| type | 必备字段 | 效果 |
|------|---------|-----|
| `addRules` | `rules` / `behavior` / `destination` | 在指定作用域追加规则 |
| `replaceRules` | `rules` / `behavior` / `destination` | 覆盖该 behavior 下全部规则 |
| `removeRules` | `rules` / `behavior` / `destination` | 删除匹配的规则 |
| `setMode` | `mode` / `destination` | 切 permission mode（`default` / `acceptEdits` / `dontAsk` / `bypassPermissions` / `plan`） |
| `addDirectories` | `directories` / `destination` | 加 working directory（影响 file 类工具 cwd 检查） |
| `removeDirectories` | `directories` / `destination` | 同上反向 |

**`behavior` 合法值**：`allow` / `deny` / `ask`（**含 `ask`**，旧设计漏掉）。

**`destination` 合法值**：`session` / `localSettings` / `projectSettings` / `userSettings`。`session` 是**内存不落盘**，通过 `session::runtime_permissions` 暂存。

`updatedPermissions` 会走 `config::mutate_config` 写入对应作用域。多条规则在同一 mutate_config 闭包里事务性写入。

**备注**：
- PermissionRequest 和 PreToolUse 是**两个**不同的 hook，调用顺序：`PreToolUse → (若放过) Plan Mode → (若放过) Approval gate → PermissionRequest hook → UI 弹窗 → 用户决策`。
- `PreToolUse deny` 会导致 PermissionRequest **不触发**（因为工具根本没到 approval 阶段）。
- `if:` 字段（§6.4）此处生效，可用 `Bash(rm *)` 一类规则把 hook 进一步约束到具体子命令。

---

#### 5.2.6 `PermissionDenied`

**埋点**：自动模式 classifier（Claude Code 的 auto-mode 对应我们未来的 "dontAsk" 分级）或 Plan Mode allowlist 否决时。

**Input**：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "PermissionDenied",
  "tool_name": "Bash",
  "tool_input": { /* ... */ },
  "tool_use_id": "toolu_01abc",
  "reason": "auto-mode classifier blocked: destructive command"
}
```

> 字段说明：官方文档目前没有发布完整 PermissionDenied input schema（只描述了 `{retry: true}` 输出语义）。我们按 Claude Code 既有惯例使用 `reason` 字段（与 PermissionRequest 的 `message`、Stop 的 `reason` 风格一致）。

**Output 处理**：

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PermissionDenied",
    "retry": true                         // 允许 LLM 再次尝试（可能改写命令）
  }
}
```

`retry=true` 时 ha-core 会把"denied"作为 tool_result 回给 LLM，但**不**把工具标记为"永久拒绝"——LLM 下一轮可以换个命令再试。

`exit code` 与 `stderr` **被官方明确忽略**（不可阻断决策本身——用户已经明确拒绝），因此该 hook **必须**通过 `retry` JSON 表达"允许重试"语义。

**`if:` 字段**：本事件也支持 `if:` 过滤（官方 5 个 tool 事件之一）。

---

#### 5.2.7 `Stop`（归在这里因为它和 tool loop 生命周期相关）

**埋点**：`crates/ha-core/src/agent/streaming_loop::run` 自然结束（`natural_exit=true`）、在 `emit_usage` 之前。

**Input**：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "Stop",
  "stop_hook_active": false               // 防止 Stop hook block 后自己又触发 Stop 造成死循环
}
```

`stop_hook_active=true` 表示"这次 Stop 是因为上一次 Stop hook 返回 block 才再跑一轮的"，hook 看到 `true` 应避免再次 block。

**Output 处理**：

- `decision: "block"` + `reason`：**让 tool loop 多跑一轮**，把 `reason` 作为 system-reminder 注入——典型用法："你没调用测试工具，先跑测试"。
- `continue: false` + `stopReason`：**整个循环终止**（比 block 更强），回显给用户。
- 其它：正常结束。

---

### 5.3 压缩 / 子代理 / 配置 / 其它事件（共 17 个）

#### 5.3.1 `PreCompact` / `PostCompact`

**埋点**：`crates/ha-core/src/context_compact/engine::run_compaction`
- PreCompact：`run_compaction` 入口、Tier 选择前。
- PostCompact：`run_compaction` 成功返回前，新 history 已写回但尚未落 EventBus 最终快照时。

**Input**（PreCompact）：

```json
{
  "session_id": "sess_xxx", "transcript_path": "...", "cwd": "...",
  "hook_event_name": "PreCompact",
  "trigger": "auto",                   // auto|manual
  "tier": 3,                            // ha-core 扩展字段：0..=4
  "usage_ratio": 0.92                   // ha-core 扩展字段
}
```

**Output**：
- PreCompact `decision: "block"` → 跳过本次压缩；如 `usage_ratio ≥ 0.95` 官方建议仍强制压缩（我们遵循，block 被忽略 + warn 日志）。
- PostCompact：不可阻断，可 `additionalContext`（追加到下次 turn 的 system-reminder：例如"刚完成压缩，保留了文件路径 X"）。

---

#### 5.3.2 `SubagentStart` / `SubagentStop`

**埋点**：
- `subagent::spawn::spawn_subagent`：`emit_subagent_event("spawned", ...)` 之后 / 子任务真正启动 LLM 前。
- terminal：`spawn_subagent` 内 `update_subagent_status(Completed | Failed | Killed)` 之后。

**Input（SubagentStart）**：

```json
{
  "session_id": "parent_sess",
  "transcript_path": "...",
  "cwd": "...",
  "hook_event_name": "SubagentStart",
  "agent_type": "Explore",
  "prompt": "Find all API endpoints in src/",
  "subagent_id": "child_sess"
}
```

**Input（SubagentStop）**：

```json
{
  "session_id": "parent_sess",
  "transcript_path": "...",
  "cwd": "...",
  "hook_event_name": "SubagentStop",
  "stop_hook_active": false,
  "agent_type": "Explore",
  "subagent_id": "child_sess",
  "stop_reason": "completed",                // completed | error | killed | ...
  "agent_transcript_path": "/Users/.../sessions/child_sess/transcript.jsonl",
  "last_assistant_message": "Final response text..."
}
```

**Output**：
- SubagentStart：`additionalContext` 注入子会话 system prompt（作为独立 section）。
- SubagentStop：`decision: "block"` 让子会话**继续**跑（典型用法："测试没过，再修一轮"）。

**重要约定（官方）**：在子代理的执行上下文里，正常的 `Stop` hook **会自动转发为 `SubagentStop`**——脚本作者不需要在父子两套 hook 里写两份逻辑。我们 dispatcher 实现时统一在 `subagent::spawn` 内拦截：触发条件命中时，event 标签按"是否在子代理上下文"切换。

---

#### 5.3.3 `StopFailure`

**埋点**：`failover::executor::execute_with_failover` 最终分类错误（轮换尝试全部用完）。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "StopFailure",
  "error_type": "rate_limit",            // rate_limit|authentication_failed|billing_error|invalid_request|server_error|max_output_tokens|unknown
  "error_message": "...",
  "provider": "anthropic",
  "model": "claude-opus-4-7"
}
```

**Output**：不可阻断。`additionalContext` 被忽略（此时循环已结束）。典型用途是外呼告警（PagerDuty / Slack）。

---

#### 5.3.4 `TaskCreated` / `TaskCompleted`

一期**复用** subagent 生命周期：`TaskCreate` 工具（官方）→ 我们映射到 `subagent spawn_and_wait`。字段：

```json
{
  "session_id": "...", "hook_event_name": "TaskCreated",
  "task_id": "task_xxx",
  "task_description": "Implement feature X",
  "assigned_to": "Explore"               // agent_type
}
```

**Output**：`decision: "block"` 阻止任务创建 / 完成（延后）。

**备注**：一期暂无专用 TaskManager；如果用户配了这俩 hook，以 SubagentStart/Stop 的同一事件同时触发二份。后期引入 `TaskCreate` 内置工具后再纯化。

---

#### 5.3.5 `TeammateIdle`

**埋点**：`team::runtime` 检测到 teammate 即将 idle 前（轮询间隔里 emit）。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "TeammateIdle",
  "team_id": "...", "teammate_id": "...", "teammate_role": "..."
}
```

**Output**：`decision: "block"` 阻止 idle（推迟进入休眠）。

**备注**：需要现有 team runtime 暴露 idle 检测点；当前 team 模块偏模板化，实际 idle 检测需要先补一层运行时——归到 Phase 2。

---

#### 5.3.6 `ConfigChange`

**埋点**：`config::persistence::mutate_config` 的事务提交**之前**。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "ConfigChange",
  "source": "user_settings",            // user_settings|project_settings|local_settings|policy_settings|skills
  "category": "memory",                 // 我们的 mutate_config 第一参数
  "reason": "user edit via GUI",         // mutate_config 第二参数
  "changed_keys": ["memoryBudget.totalChars"]
}
```

**Output**：`decision: "block"` → `mutate_config` 返回 `Err(Blocked by hook: reason)`，变更回滚。

**备注**：
- 写 hooks 配置**本身**的变更也会触发这个事件——小心配了一个"ConfigChange block 所有 hooks 改动"的 hook 然后把自己锁死。GUI 面板对 `hooks` 类别的改动加一层"强制绕过 ConfigChange hook"开关（安全出口）。

---

#### 5.3.7 `CwdChanged`

**埋点**：**需要新增** `session::cwd::set_cwd(session_id, new_cwd)` 入口——一期以 session 级别维护 cwd（前端可给会话绑定项目目录）。触发点在写入新 cwd 之后。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "CwdChanged",
  "old_cwd": "/Users/x/proj-a",
  "new_cwd": "/Users/x/proj-b"
}
```

**Output**：不可阻断。`CLAUDE_ENV_FILE` 可用（一期不开，P1 再说）。

---

#### 5.3.8 `FileChanged`

**埋点**：**需要新增** `project::file_watcher`（基于 `notify` crate）。订阅的文件名从 hook 配置的 `matcher` 字面提取。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "FileChanged",
  "path": "/Users/x/proj/.envrc",
  "change_type": "modified"             // created|modified|removed
}
```

**Output**：不可阻断。

**备注**：file watcher 本身是新基础设施，成本不小（跨平台、去抖、退避）。Phase 2 单独立一个 tasklet 做。

---

#### 5.3.9 `InstructionsLoaded`

**埋点**：`agent::system_prompt` 组装时，每加载一份 CLAUDE.md / AGENTS.md / `@import` 文件都 emit 一次。

**Input**：

```json
{
  "session_id": "...", "hook_event_name": "InstructionsLoaded",
  "file_path": "/Users/.../proj/CLAUDE.md",
  "memory_type": "Project",             // User|Project|Local|Managed
  "load_reason": "session_start",       // session_start|nested_traversal|path_glob_match|include|compact
  "globs": null,
  "trigger_file_path": null,
  "parent_file_path": null
}
```

**Output**：不可阻断。`additionalContext` 注入到 system prompt 尾部（但必须在组装未完成的阶段，需要同步 hook 支持——见 §7.1 的同步/异步权衡）。

---

#### 5.3.10 `Elicitation` / `ElicitationResult`

P2 占位。事件常量 + payload 结构按官方最新 schema 定义保留，但**永远不触发**（MCP 落地后启用）。settings 里配了不报错。

**Input（Elicitation）**：

```json
{
  "session_id": "...",
  "hook_event_name": "Elicitation",
  "mcp_server_name": "my-server",
  "elicitation_form": {
    "fields": [
      { "name": "api_key", "label": "API Key", "type": "password", "required": true }
    ]
  }
}
```

**Input（ElicitationResult）**：

```json
{
  "session_id": "...",
  "hook_event_name": "ElicitationResult",
  "mcp_server_name": "my-server",
  "user_response": { "api_key": "sk-..." }
}
```

**Output（两者通用）**：

```json
{
  "hookSpecificOutput": {
    "hookEventName": "Elicitation",
    "action": "accept|decline|cancel",
    "content": { "field_name": "value_override" }
  }
}
```

`action=decline` 等价于"用户拒绝填写"；`exit 2` 也走 decline 路径。

**Matcher**：`mcp_server_name`。

---

#### 5.3.11 `WorktreeCreate` / `WorktreeRemove`

P2 占位。等 worktree 隔离能力落地后激活。schema 按官方规范保留：

**Input（WorktreeCreate）**：

```json
{
  "session_id": "...",
  "hook_event_name": "WorktreeCreate",
  "worktree_path": "/abs/path/to/worktree",
  "branch": "feature/foo"
}
```

**Output**：
- Command hook：exit 0 时 stdout 内容必须**只是**新 worktree 的绝对路径；任何**非 0 exit 都失败**整个 worktree 创建（区别于其它事件 exit 1 仅"非阻断错误"）。
- HTTP hook：返回 `{"hookSpecificOutput": {"hookEventName": "WorktreeCreate", "worktreePath": "/abs/path"}}`。

**Input（WorktreeRemove）**：

```json
{
  "session_id": "...",
  "hook_event_name": "WorktreeRemove",
  "worktree_path": "/abs/path"
}
```

**Output**：纯观察。失败仅 debug 日志，不阻断删除流程。

---

### 5.4 埋点函数签名统一约定

**所有埋点共用一个 helper**（避免每个调用点重复拼 `CommonHookInput`）：

```rust
// 伪代码
impl ToolExecContext {
    pub fn common_hook_input(&self, event: &'static str) -> CommonHookInput {
        CommonHookInput {
            session_id: self.session_id.clone(),
            transcript_path: hooks::transcript::path_for(&self.session_id),
            cwd: self.resolve_cwd(),
            permission_mode: self.permission_mode,
            hook_event_name: event,
            agent_id: self.agent_id.clone(),
            agent_type: self.agent_type.clone(),
        }
    }
}
```

每个埋点一行：

```rust
let outcome = hooks::dispatch(
    HookEvent::PreToolUse,
    HookInput::PreToolUse {
        common: ctx.common_hook_input("PreToolUse"),
        tool_name: call.name.clone(),
        tool_input: call.input.clone(),
        tool_use_id: call.id.clone(),
    },
).await;
```

---

## 6. Matcher 引擎

### 6.1 三种语法的判别规则（对齐官方原文）

| Matcher 值 | 判定为 | 语义 |
|-----------|-------|------|
| `None` / `""` / `"*"` | **Wildcard** | 始终匹配 |
| 仅包含 `[A-Za-z0-9_|]` 字符 | **Exact / Pipe-list** | `"Bash"` 精确；`"Edit\|Write"` 拆 `|` 做多精确 OR |
| 包含任何其它字符（`.`, `^`, `*` 带字母组合等） | **Regex** | JavaScript 正则语义（`regex` crate 启用 `unicode` 特性，禁用 `lookaround`） |

**关键**：判别依据是"**出现了哪些字符**"，不是"脚本作者的意图"——这点和官方完全一致，避免猜测。

### 6.2 每事件的匹配目标

见 §2 矩阵表的"Matcher 目标"列。复述关键：

- `PreToolUse / PostToolUse / PostToolUseFailure / PermissionRequest / PermissionDenied` → 匹配 `tool_name` 字符串
- `SessionStart / SessionEnd` → 匹配 `source`
- `Notification` → 匹配 `notification_type`
- `PreCompact / PostCompact` → 匹配 `trigger`（`manual` / `auto`）
- `ConfigChange` → 匹配 `source`（6 个配置源名）
- `StopFailure` → 匹配 `error_type`
- `InstructionsLoaded` → 匹配 `load_reason`
- `SubagentStart / SubagentStop` → 匹配 `agent_type`
- `FileChanged` → 匹配 `path`（基于文件名片段 / 路径）
- 其余无 matcher 的事件：任何 matcher 配置都等价 wildcard，但解析时 warn（用户多配了也不报错）

### 6.3 MCP 工具命名兼容

MCP 工具名**必须**遵循 `mcp__<server>__<tool>`（双下划线分隔）。

典型匹配样例（官方照搬）：

| Matcher | 含义 |
|---------|------|
| `mcp__memory__create_entities` | 精确匹配 memory server 的 create_entities 工具 |
| `mcp__memory__.*` | memory server 全部工具（正则） |
| `mcp__.*__write.*` | 任何 server 的 write 开头工具 |
| `mcp__memory` | **精确**匹配（没有 `.*` → 走 exact），所以匹配不到任何工具（没有叫 `mcp__memory` 的工具） |

最后一行是常见陷阱，文档里显式标出。

### 6.4 `if:` 字段 — 权限规则语法

**作用域**：只在 `PreToolUse / PostToolUse / PostToolUseFailure / PermissionRequest / PermissionDenied` 这 5 个 tool 事件的单个 hook handler 里生效，matcher 组内再细分。**注意 `PermissionDenied` 也含**（旧版漏列，已对齐官方）。

**语法**：单条规则（官方明确无 `&&` / `||`）。格式 `ToolName(arg-pattern)`。

- `Bash(rm *)` / `Bash(rm -rf *)` — 匹配 Bash 命令首 token 开头
- `Edit(*.ts)` — 匹配 Edit 工具的 `file_path` glob
- `Write(src/**)` — glob 匹配路径
- `WebFetch(https://github.com/*)` — 匹配 url

**Bash subcommand 拆分**（对齐官方）：
- 剥离前缀 `VAR=value` 赋值
- 按 `&&` / `||` / `;` / `|` 拆子命令
- 任一子命令命中规则 → 整体命中
- 命令过于复杂无法 parse → **hook 仍然运行**（保守策略）

**一期降级**：只做 tool_name-level exact + naive glob 对第一 token / 第一路径字段；Bash subcommand 真拆分放 Phase 1.5。

### 6.5 数据结构

```rust
#[derive(Debug, Clone)]
pub struct CompiledMatcher {
    original: String,
    kind: MatcherKind,
}

#[derive(Debug, Clone)]
enum MatcherKind {
    Wildcard,
    Exact(Vec<String>),     // pipe list → vec
    Regex(regex::Regex),
}

impl CompiledMatcher {
    pub fn compile(raw: &str) -> Result<Self, MatcherError> {
        if raw.is_empty() || raw == "*" {
            return Ok(Self { original: raw.into(), kind: MatcherKind::Wildcard });
        }
        let only_safe = raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '|');
        if only_safe {
            let parts = raw.split('|').filter(|s| !s.is_empty()).map(String::from).collect();
            return Ok(Self { original: raw.into(), kind: MatcherKind::Exact(parts) });
        }
        let rx = regex::Regex::new(raw).map_err(|e| MatcherError::InvalidRegex { raw: raw.into(), cause: e })?;
        Ok(Self { original: raw.into(), kind: MatcherKind::Regex(rx) })
    }

    pub fn is_match(&self, target: &str) -> bool {
        match &self.kind {
            MatcherKind::Wildcard => true,
            MatcherKind::Exact(list) => list.iter().any(|s| s == target),
            MatcherKind::Regex(rx) => rx.is_match(target),
        }
    }
}
```

### 6.6 编译时机

- 配置加载 / 热重载时**一次性**编译所有 matcher，失败的 matcher 记录 warning 但不 panic（回退为 never-match）。
- 执行时 O(N) 扫描所有 matcher group（N ≤ 几十，可接受），命中的 handler 进入执行池。
- 大于 500 条时可上 `BTreeMap<tool_name, Vec<HandlerId>>` 做 exact 索引 + 正则 fallback——目前不需要。

### 6.7 `disableAllHooks` 和 scope 合并之后的 matcher 生效顺序

1. 先按 §4.1 合并四层 + frontmatter 的所有 hooks。
2. 去重按 `(handler_type, identity)`（identity：command.command / http.url / prompt+model 哈希 / agent+model 哈希）。
3. 对保留下来的每一个 handler，逐个 matcher 判定是否匹配当前事件目标。
4. 命中的全部进入执行池（并发）。

**无"第一个匹配就停"**——所有命中都会跑（这是官方协议）。

---

## 7. Hook Runner — 四种 handler 的执行细节

所有 runner 对外暴露的 trait：

```rust
#[async_trait::async_trait]
pub trait HookHandler: Send + Sync {
    fn identity(&self) -> String;      // 去重用
    fn handler_type(&self) -> &'static str; // "command" | "http" | "prompt" | "agent"
    fn default_timeout(&self) -> Duration;

    async fn run(
        &self,
        input: &HookInput,
        env: &HookEnv,
        deadline: Instant,
    ) -> RawHookResult;
}

pub struct RawHookResult {
    pub exit_code: Option<i32>,        // None = handler 没有 exit 概念（http）
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub timed_out: bool,
}
```

### 7.1 同步 vs 异步 vs asyncRewake

| 标志 | 语义 | Runner 返回时机 |
|------|-----|---------------|
| `async: false`（默认） | 阻塞等待 | handler 跑完 |
| `async: true` | spawn 到后台 tokio task | 立刻返回 `RawHookResult::noop()` |
| `asyncRewake: true` | 同上，但后台任务 exit 2 时通过 EventBus 把 stderr 作为 system-reminder 注入下一轮 | 同上 |

`async=true` 的 handler **完全不影响当前决策**——它只是"触发了但不等结果"。

`asyncRewake` 唤醒路径：

```
后台 command 退出 code=2
  → hooks::audit 记录
  → 通过 EventBus emit "hooks:rewake" { session_id, stderr }
  → agent::streaming_loop 订阅该事件
  → 下一轮 user turn 开始时插入 <system-reminder> 段
```

### 7.2 `command` handler

**复用**：`crates/ha-core/src/tools/exec.rs` 的 shell spawn + 登录 shell PATH 解析 + timeout 超时模式。抽出一个公共 helper `exec::spawn_with_timeout(shell, cmd, stdin, env, timeout)`，命令类 hook 和 `exec` 工具共用。

**Shell 解析**：
- 默认：macOS / Linux 用登录 shell（zsh/bash）；Windows 用 PowerShell。
- `shell: "bash"` 强制 bash（`/bin/bash -c`）。
- `shell: "powershell"` 强制 PowerShell 7（`pwsh -Command`）。

**Stdin**：完整 `HookInput` JSON，以 UTF-8 bytes 写入。

**环境变量**：见 §11。

**工作目录**：当前 session 的 `cwd`（§5.3.7），默认为 `project_dir`（如果 session 绑定了项目）或者用户 home。

**超时**：默认 600s（对齐官方 command 默认），可覆盖。超时发送 `SIGTERM`，5s 后 `SIGKILL`（Unix）；Windows 走 `TerminateProcess`。

**输出抓取**：
- stdout 限 1 MiB（超出截断，stderr 记 warn）。
- stderr 同样 1 MiB。
- 非法 UTF-8 用 `String::from_utf8_lossy` 兜底。

**去重 identity**：command 字符串（含前缀展开后的最终字符串）。

### 7.3 `http` handler

**库**：`reqwest`（已在 ha-core 依赖中）。

**SSRF 校验**：**必走** `security::ssrf::check_url(url, policy, trusted_hosts)`。策略：
- 默认：`Default`（允许 loopback，禁 private + metadata）——因为常见用法是 `localhost:8080/hooks/...`。
- 可覆盖：`AppConfig.hooks.http_ssrf_policy`（Strict / Default / AllowPrivate）。
- `trusted_hosts` 共享全局 `ssrf.trusted_hosts`。

**请求**：
- Method: `POST`
- Body: `HookInput` 的 JSON
- `Content-Type: application/json`
- Headers: 用户 `headers` 字段，值里的 `$VAR` / `${VAR}` 只从 `allowedEnvVars` 白名单解析

**超时**：默认 30s。

**响应解析**：

| Response | 解析为 |
|----------|-------|
| 2xx + empty body | 等价 exit 0（无输出） |
| 2xx + plain text | 等价 exit 0 + 文本作为 `additionalContext` |
| 2xx + JSON | 等价 exit 0 + JSON 按 §8 协议解析 |
| 非 2xx | 非阻断错误，记 warn 日志 |
| 连接失败 / 超时 | 非阻断错误 |

**去重 identity**：`method|url|body_hash[:8]`（body 含哈希避免同 URL 不同请求误去重）。

### 7.4 `mcp_tool` handler（新增）

**实现**：直接走 `mcp::api::call_tool(server, tool, input_json)`，复用现有 MCP client。**不**经过 LLM —— hook 只是把"事件输入"塞给某个 MCP tool，把 tool 输出当作决策结果。

**why this exists**：
- 社区已经有大量 MCP server（安全扫描、格式化、git 操作等），用户写 hook 时不必再 wrap 一层 shell。
- 比 `prompt` / `agent` 便宜一个数量级（无 token 成本）。

**配置（重述自 §4.2）**：

```json
{
  "type": "mcp_tool",
  "server": "my_server",
  "tool": "security_scan",
  "input": { "file_path": "${tool_input.file_path}" }
}
```

**`input` 模板插值规则**：

- `${path.to.field}` 从 hook input JSON 取值（path 走 dotted 寻址，如 `${tool_input.command}` / `${tool_response.stdout}` / `${session_id}`）。
- 字符串字段直接替换；非字符串字段（数组 / 对象）也支持，序列化为 JSON 子树嵌入。
- 未命中字段保留原样（`${unknown}` 字面留下，不抛异常），同时记 `app_warn!("hooks", "runner.mcp_tool", "unresolved placeholder ...")`。

**前置检查**：
1. `server` 必须在 `cached_config().mcp_servers` 里**且** `state == Ready`；否则非阻断错误 + warn。
2. `tool` 必须在该 server `cached_tool_defs` 里。
3. tool 调用走与普通 LLM 调用**同一份**审计 + 命名空间（`mcp__<server>__<tool>` 落 `EVT_MCP_TOOL_CALLED`）。

**返回值映射到 hook 协议**：

| MCP tool 返回 | 解析为 |
|--------------|-------|
| 普通成功（`isError=false`） | 等价 exit 0；`tool_response.content[0].text` 若是合法 JSON 按 §8 解析；否则当 `additionalContext` |
| `isError=true` | 等价 exit 2 + stderr = error message → block |
| RPC 失败 / timeout | 非阻断错误 |

**超时**：默认沿用 MCP server RPC 超时（一般 30s），`timeout` 字段可单独覆盖；超过则 abort 调用并记 timeout warn。

**SSRF**：MCP server transport 自身已有 SSRF 防御（见 AGENTS.md MCP 章节）；此处不重复检查。

**去重 identity**：`mcp_tool|server|tool|input_hash[:12]`。

---

### 7.5 `prompt` handler

**实现**：走 `agent::side_query`（共享 system prompt + history 前缀、命中 prompt cache，单轮）。

**模型选择**：`model` 字段，fallback 到 `AppConfig.fallback_models.fast`（一般是 Haiku）。一期 prompt 类型 hook **不能用 tool**——强制 `tools=[]`。

**Prompt 格式**：

```
<hook_context>
  {官方 hook 输入 JSON 全文}
</hook_context>

<user_prompt>
  {用户配置的 prompt 模板，替换 $ARGUMENTS 为 hook JSON}
</user_prompt>

返回 JSON：
  - decision: "allow" | "deny" | "ask" | "defer" | "block"
  - reason: string
  - additionalContext?: string
```

**超时**：默认 30s。

**输出解析**：LLM 返回文本用正则剥离 JSON code fence，再按 §8 解析。失败 → 非阻断 warn。

**去重 identity**：`prompt_text_hash[:16]|model`。

### 7.6 `agent` handler

**实现**：走 `subagent::spawn_and_wait`，`foreground_timeout` 对齐 hook `timeout`（默认 60s）。

**能力**：默认给一份只读工具集（`Read / Glob / Grep / WebFetch`），不给 `Write / Edit / Bash`——hook 是"决策者"，不应副作用。可通过 `AgentHookConfig.allowed_tools` 显式放开。

**Prompt 格式**：和 prompt handler 类似，但放 `$ARGUMENTS` 到 system prompt（让子会话的多轮 tool loop 一开始就知道任务）。

**返回**：子 agent 的 `last_assistant_message` 必须 **整段 JSON**（约定）。

**安全**：agent hook 本质是"用 LLM 判决策"——token 成本高，默认 `async: true` 不阻塞主循环的决策。一期 `agent` 类型不做阻塞决策（decision 永远是 `Allow`），只支持注入 `additionalContext`——避免 LLM 误判阻断关键工具。激进场景让用户用 `command` + 本地规则。

### 7.7 去重机制

同一 event / matcher group 内部已经按配置顺序去重；**跨 scope 合并时**也做一次去重（官方规则）：

```
fn dedupe(handlers: Vec<Box<dyn HookHandler>>) -> Vec<Box<dyn HookHandler>> {
    let mut seen: HashSet<(&'static str, String)> = HashSet::new();
    handlers.into_iter().filter(|h| seen.insert((h.handler_type(), h.identity()))).collect()
}
```

### 7.8 并发执行 & 超时

```rust
let tasks: Vec<_> = handlers.iter().map(|h| {
    let input = input.clone();
    let env = env.clone();
    let deadline = Instant::now() + h.default_timeout();
    tokio::spawn(async move { h.run(&input, &env, deadline).await })
}).collect();

let results = futures::future::join_all(tasks).await;
```

**总超时**：所有 handler 的 max timeout + 5s 作为整体熔断。如果整体熔断触发，未完成 handler 标记 `timed_out=true`，其输出按 `exit 1` 处理（非阻断）。

### 7.9 输出抓取上限

- 单 handler stdout/stderr 各 1 MiB（runner 层截断）。
- JSON 解析入 `additionalContext` 的部分限 **10 000 字符**（对齐官方，超限存到 `~/.hope-agent/hooks/overflow/{timestamp}.json` 并在 additionalContext 里留路径提示）。

### 7.10 执行日志

每个 handler 跑完都落：

```
app_info!("hooks", "runner.command",
    "event={} matcher={:?} cmd={:?} exit={} dur={}ms",
    event, matcher, cmd, exit_code, dur_ms);
```

`category="hooks"` 始终用。`source` 细分：`runner.command` / `runner.http` / `runner.mcp_tool` / `runner.prompt` / `runner.agent` / `dispatch` / `config` / `matcher`。

---

## 8. 输入输出协议

### 8.1 通用输入字段（全部事件共有）

| 字段 | 类型 | 备注 |
|------|------|------|
| `session_id` | string | UUID，对应 `session.db` 主键 |
| `transcript_path` | string | 绝对路径，JSONL 文件（§10 镜像） |
| `cwd` | string | 当前会话 cwd（绝对路径） |
| `permission_mode` | string | `default` / `plan` / `bypassPermissions` / `other`（§2.4 降级） |
| `hook_event_name` | string | 事件字面值，如 `"PreToolUse"` |
| `agent_id` | string\|null | 子代理语境下有值 |
| `agent_type` | string\|null | 同上 |

**字段名一律 snake_case**（对齐官方）。事件特有字段见 §5。

### 8.2 通用输出字段（全部事件共享）

| 字段 | 类型 | 语义 |
|------|------|------|
| `continue` | bool | 默认 `true`；`false` 会终止整个循环（所有事件都生效） |
| `stopReason` | string | 配合 `continue=false` 展示给用户 |
| `suppressOutput` | bool | 默认 `false`；`true` 时 stdout 不进审计日志（仍走 runner 日志） |
| `systemMessage` | string | 展示给用户的警告条（通常是桌面通知） |
| `hookSpecificOutput` | object | 见 §5 每事件 |

**字段名一律 camelCase**（对齐官方 body 字段）。

### 8.3 Exit Code 语义（对齐官方表）

| Exit Code | 含义 | JSON 解析 | 阻断 |
|-----------|------|-----------|------|
| `0` | 成功 | 若 stdout 是合法 JSON object，按本协议解析；否则整段当 `additionalContext`（SessionStart / UserPromptSubmit） | 按 JSON 字段决定 |
| `2` | 阻断错误 | **不**解析 JSON，stderr 作为 `block reason` 回灌给 Claude | ✅（仅部分事件可阻断，见 §2 矩阵） |
| 其它（含 `1`） | 非阻断错误 | 不解析 JSON，stderr 第一行进审计日志 | ❌ |

**陷阱提醒**（对齐官方）：Unix 传统 exit 1 = 失败，但在 hook 协议里 `1` 是"非阻断错误"——要阻断**必须**写 `exit 2`。这点要在 GUI 面板 + 技能文档里双重提醒。

**WorktreeCreate 特例**（P2 占位）：任何非 0 exit 都失败；stdout 的非空路径作为 worktree 目录。

### 8.4 Per-event 输出字段速查

下表列出每事件 `hookSpecificOutput` 允许的字段。**未列出的字段一律忽略 + warn**，避免 hook 作者拼错字段名静默失效。

| 事件 | 允许字段（`hookSpecificOutput` 内 / 顶层） |
|------|-------------------------------|
| `SessionStart` | `additionalContext` |
| `SessionEnd` | —（无） |
| `UserPromptSubmit` | `additionalContext`, `sessionTitle`；顶层 `decision: "block"` + `reason` |
| `UserPromptExpansion` | `additionalContext`；顶层 `decision: "block"` + `reason` |
| `PreToolUse` | `permissionDecision`, `permissionDecisionReason`, `updatedInput`, `additionalContext` |
| `PostToolUse` | `additionalContext`, `updatedMCPToolOutput`；顶层 `decision: "block"` + `reason` |
| `PostToolUseFailure` | `additionalContext` |
| `PostToolBatch` | `additionalContext`；顶层 `decision: "block"` + `reason` |
| `PermissionRequest` | `decision` (嵌套：`behavior`, `updatedInput`, `updatedPermissions`, `message`, `interrupt`)。`interrupt: true` 仅 deny 时生效，等价"停整个 Claude 循环" |
| `PermissionDenied` | `retry` |
| `Notification` | `additionalContext` |
| `Stop` / `SubagentStop` | 顶层 `decision: "block"` + `reason`（注意：这俩事件顶层有 decision，不是 hookSpecificOutput 里） |
| `PreCompact` | `additionalContext`；顶层 `decision: "block"` + `reason` |
| `PostCompact` | `additionalContext` |
| `ConfigChange` | 顶层 `decision: "block"` + `reason` |
| `SubagentStart` | `additionalContext` |
| `Elicitation` / `ElicitationResult` | `action`（accept/decline/cancel）+ `content` 覆盖 |
| `WorktreeCreate` | `worktreePath`（必填，HTTP；command 走 stdout） |
| 其它观察型事件 | — |

### 8.5 输入编码 / 输出解码的统一器

```rust
// command / http 共用一套
impl HookOutput {
    pub fn parse(raw: &RawHookResult, event: HookEvent) -> HookDecisionResult {
        // exit code 优先级：2 > 其它非 0 > 0
        if raw.exit_code == Some(2) {
            return HookDecisionResult::Block { reason: raw.stderr.clone() };
        }
        if matches!(raw.exit_code, Some(n) if n != 0) {
            return HookDecisionResult::NonBlockingError { stderr: raw.stderr.clone() };
        }

        // exit 0 → 尝试 JSON
        match serde_json::from_str::<Value>(&raw.stdout.trim()) {
            Ok(Value::Object(m)) => Self::from_json_object(m, event),
            _ => Self::from_plaintext(&raw.stdout, event),  // SessionStart / UserPromptSubmit 才接受纯文本
        }
    }
}
```

### 8.6 `10 000 字符`注入上限

所有注入 LLM 上下文的 string 字段（`additionalContext`, `systemMessage`, `stopReason`, plaintext-mode stdout）**合并后** ≤ 10 000 字符。超限策略：

1. 写全文到 `~/.hope-agent/hooks/overflow/{event}-{session_id}-{ts}.txt`。
2. 注入的实际文本替换为 `<hook output truncated; full content at {path}>`（约 80 字符）。
3. 审计日志 warn。

### 8.7 编码安全

- JSON 输入字符串写 stdin 时必须 `\n` 结尾（bash `read` 友好）；hook 脚本 `jq` 能直接吃。
- 字符串字段出站必须 UTF-8 safe；用 `crate::truncate_utf8` 截断（AGENTS.md 红线）。
- 输出路径字段必须规范化（`PathBuf::canonicalize` 失败则退回原值 + warn）。

---

## 9. 决策聚合 & 业务回写

### 9.1 聚合优先级（对齐官方）

对 PreToolUse / PermissionRequest 的 `permissionDecision`：

```
deny > defer > ask > allow
```

任一 hook 返回 deny → 全组 deny。否则任一 defer → defer。否则 ask 胜。无 hook 返回决策 → allow（默认）。

对其它事件的 `decision: "block"`：**任一** block 即 block；`block reason` 按出现顺序拼接（`\n\n`）。

对顶层 `continue: false`：**任一** false 即 false；`stopReason` 取第一个非空。

### 9.2 字段合并规则

| 字段 | 合并策略 |
|------|---------|
| `decision` / `permissionDecision` | 按 §9.1 优先级 |
| `reason` / `stopReason` | 第一个非空 |
| `additionalContext` | **顺序拼接**（按 handler 注册顺序），每段用 `---` 分隔 |
| `systemMessage` | **顺序拼接**（换行连接，ToastQueue UI 层展示） |
| `updatedInput` | 取**胜出决策**的那个（allow/ask 里最后一个提供的） |
| `updatedMCPToolOutput` | 取最后一个（多个 hook 改同一个 MCP 输出，用户自担冲突） |
| `updatedPermissions` | 合并去重（按 `(type, rules_hash, destination)`） |
| `sessionTitle` | 第一个非空 |
| `retry` | 任一 true 即 true（PermissionDenied） |
| `suppressOutput` | 任一 true 即 true |

### 9.3 Hook 与现有 gate 的叠加顺序

> **关键约束**（官方原文）："a hook returning `allow` does not override a matching deny rule"。换言之：hook 的 `allow` 不是越权令牌，**只是** "跳过 user-facing 的 ask 弹窗" 的语义。Plan Mode / `denied_tools` / skill `allowed-tools` / dangerous denial 这些 **硬红线** 一律保留，且 `updatedInput` 改写后必须重新过一遍。

```
1. PreToolUse hook
   ├─ deny → 直接 ToolResult::error(reason)，跳过下游所有 gate
   ├─ defer → 一期降级 ask
   ├─ allow → 标记 "skip user approval prompt"，**继续走下游硬红线**
   └─ ask → 标记 "force user approval prompt"，继续走下游硬红线

   （若 PreToolUse 给了 updatedInput，先 patch 到 call.input，再进入 step 2）

2. Hard-deny gates（按顺序，任一命中即 ToolResult::error，不再下推）
   ├─ Skill allowed-tools 过滤   → schema 级过滤当前 active skill 的白名单
   ├─ Agent capabilities/denied_tools → AgentConfig.capabilities 黑白名单
   ├─ Plan Mode allowlist        → ToolExecContext.plan_mode_allowed_tools 校验
   └─ Dangerous explicit deny rule → 命中 deny rule 直接拒（hook allow 也救不了）

   命中以上任一 → 触发 PermissionDenied hook（observer 链路），返回 error

3. Approval gate
   ├─ Dangerous YOLO=true → 跳 user UI（仍然触发 PermissionRequest hook 让观察者知道）
   ├─ rule-based allow（含本轮 hook updatedPermissions 写入的）→ 跳 user UI
   ├─ PreToolUse hook 已 allow → 跳 user UI（**但 step 2 硬红线必须先过**）
   ├─ PreToolUse hook 已 ask 或下游需要确认 → 触发 PermissionRequest hook → UI 弹窗
   │     ├─ PermissionRequest hook decision.allow → 跳 UI 直接放行
   │     ├─ PermissionRequest hook decision.deny → 触发 PermissionDenied hook，返回 error
   │     └─ 否则 UI 弹给用户，等 user 决策
   ├─ auto-mode classifier deny → 触发 PermissionDenied hook
   └─ user 选择 → 最终决定

4. Tool dispatch
```

**易错点速查**：

| 误解 | 实际 |
|------|------|
| "PreToolUse allow 等于 root 通行证" | ❌ 只跳 user approval prompt；硬红线一律保留 |
| "updatedInput 改完就执行" | ❌ 改后重新过 step 2 的硬红线 + step 3 的 deny rule（防止 hook 把 `npm test` 改成 `rm -rf` 绕过黑名单） |
| "Plan Mode 在 PreToolUse 之前" | ❌ Plan Mode 在 PreToolUse 之后、approval 之前；PreToolUse hook 仍然能在 Plan Mode 视角观察到所有 tool call |
| "PreToolUse hook 返回 deny 也要走 Plan Mode" | ❌ hook deny 是顶层短路，直接返回 error，不再下推 |
| "PermissionRequest 不弹 UI 就一定执行" | ❌ 即使 hook decision.allow，也要再过 step 2 硬红线（hook 允许 ≠ 无脑放行）|

**实施要求**：
- `tools::execution::execute_tool_with_context` 必须把 step 2 抽成一个 `enforce_hard_denies(call) -> Result<(), DenyReason>` helper，PreToolUse / `updatedInput` patch / PermissionRequest allow 三个分支都调一次。
- 一处 helper、三处调用，避免任何分支遗漏硬红线。

### 9.4 `updatedInput` 回写实现

```rust
// execute_tool_with_context 伪代码（对齐 §9.3 硬红线 contract）
let mut call = original_call;
let pre_outcome = hooks::dispatch(HookEvent::PreToolUse, ...).await;

// step 1: PreToolUse hook 决策
match pre_outcome.decision {
    Decision::Deny { reason } => return ToolResult::denied(reason),  // 顶层短路
    Decision::Ask | Decision::Allow | Decision::Defer => {
        if let Some(patched) = pre_outcome.updated_input {
            call.input = patched;
            app_info!("hooks", "dispatch", "PreToolUse rewrote tool_input for {}", call.name);
        }
        if let Some(ctx) = pre_outcome.merged_context() {
            /* 插入到下轮 system-reminder */
        }
    }
}

// step 2: 硬红线（任何 hook 都救不了；updatedInput patch 后必须再过一遍）
if let Err(reason) = enforce_hard_denies(&ctx, &call).await {
    hooks::dispatch(HookEvent::PermissionDenied, ...).await;  // observer
    return ToolResult::denied(reason);
}

// step 3: approval gate
let skip_user_prompt = matches!(pre_outcome.decision, Decision::Allow)
    || ctx.dangerous_yolo()
    || rule_based_allow(&call);
if !skip_user_prompt {
    let perm_req = hooks::dispatch(HookEvent::PermissionRequest, ...).await;
    match perm_req.permission_decision {
        Some(PermissionDecision::Allow { updated_input, updated_permissions }) => {
            if let Some(patched) = updated_input { call.input = patched; }
            apply_updated_permissions(updated_permissions).await?;
            // 改写过的 input 仍要再过一次硬红线 + deny rule
            if let Err(reason) = enforce_hard_denies(&ctx, &call).await {
                return ToolResult::denied(reason);
            }
        }
        Some(PermissionDecision::Deny { message, interrupt }) => {
            hooks::dispatch(HookEvent::PermissionDenied, ...).await;
            if interrupt { return ToolResult::halt_loop(message); }
            return ToolResult::denied(message);
        }
        None => { /* 弹 UI 给 user 决策 */ }
    }
}

// step 4: dispatch
execute_tool(&call).await
```

`enforce_hard_denies` 这个 helper 是 F2 修复的核心 —— 它把 skill `allowed-tools` 过滤、Agent capabilities 黑白名单、Plan Mode allowlist、explicit deny rule **统一一处实现**，PreToolUse 后跑一次、PermissionRequest allow 后再跑一次，避免任何 hook 改写绕过黑名单。

### 9.5 `updatedPermissions` 持久化

```rust
for update in pre_outcome.updated_permissions {
    match update.destination {
        PermDest::Session => session_state.add_permission(update).await?,
        PermDest::LocalSettings => config::mutate_project_local(|c| apply(c, update))?,
        PermDest::ProjectSettings => config::mutate_project(|c| apply(c, update))?,
        PermDest::UserSettings => config::mutate_user(|c| apply(c, update))?,
    }
}
```

**事务性**：多个 `updatedPermissions` 在同一个 `mutate_config` 闭包里一次性写入，避免中途失败导致部分生效。

### 9.6 `continue: false` 行为

任何事件返回 `continue=false`：
- 当前业务动作（工具执行 / 下一轮 LLM / 等）**立即中止**。
- `stopReason` 显示给用户（桌面通知 + chat UI 系统消息）。
- 会话保持 alive，用户下条消息可重启。
- 不回滚已发生的副作用（如 Bash 已经跑完）。

### 9.7 防死循环

- `Stop` hook block 后再跑一轮 → 下一次 Stop hook 收到 `stop_hook_active: true`。作者应写 "if stop_hook_active then exit 0"。
- 我们在 `streaming_loop` 里再加一层硬上限：连续 `Stop block` ≥ 3 次 → 强制 stop 并日志 warn（官方没有此保护，属于我们增强）。

### 9.8 `suppressOutput`

仅影响审计日志的展示；runner 内部 `app_info!` 始终记录（便于事后排查）。

---

## 10. JSONL Transcript 镜像

### 10.1 为什么要做

官方协议规定 `transcript_path` 是一个**可读的 JSON Lines 文件**，hook 脚本会拿 `jq` 直接 parse。我们的会话主存储是 SQLite（`session.db::messages`），没法让 hook 脚本 jq。三选一中用户已选 **JSONL 镜像**：SQLite 仍是真相源，额外维护一份 JSONL 文件让 hook 能读。

### 10.2 文件路径

```
~/.hope-agent/sessions/{session_id}/transcript.jsonl
```

对接 `ha-core::paths::session_dir(session_id)` 获取目录（如不存在则懒创建）。

### 10.3 行格式

每一行都是一个 JSON object，**对齐 Claude Code 的 transcript 行结构**：

```json
{"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "..."}]}, "timestamp": "2026-04-21T15:32:01.234Z", "uuid": "msg_xxx", "parentUuid": null, "sessionId": "sess_xxx", "cwd": "...", "version": "1"}
```

| type | 含义 |
|------|------|
| `user` | 用户消息 |
| `assistant` | Claude 输出（含 `content: [ {type:"text"}, {type:"tool_use", id, name, input} ]` 混合块） |
| `tool_result` | 工具返回（`content: [{type:"tool_result", tool_use_id, content}]`） |
| `summary` | 压缩后的合并摘要（Tier ≥ 3 produce） |
| `system` | system-reminder / hook 注入 |

**字段**：`type` / `message` / `timestamp` (ISO 8601 UTC) / `uuid` / `parentUuid` / `sessionId` / `cwd` / `version`。`version` 一期固定 `"1"`。

### 10.4 实现方式

**写入时机**：
- `session::db::insert_message` 成功后**同步**追加一行到 JSONL 文件（阻塞 fs append，单 session 单锁）。
- 写失败 warn，不影响 SQLite。
- 不做 rollback（JSONL 仅只读给 hook，允许轻微漂移）。

**组件**：`crates/ha-core/src/hooks/transcript.rs`

```rust
pub struct TranscriptMirror {
    cache: tokio::sync::Mutex<HashMap<SessionId, BufWriter<File>>>,
}

impl TranscriptMirror {
    pub async fn append(&self, sid: &SessionId, line: TranscriptLine) -> io::Result<()> {
        let mut map = self.cache.lock().await;
        let w = map.entry(sid.clone()).or_insert_with(|| {
            let p = paths::session_dir(sid).join("transcript.jsonl");
            BufWriter::new(OpenOptions::new().create(true).append(true).open(p).unwrap())
        });
        serde_json::to_writer(&mut *w, &line)?;
        w.write_all(b"\n")?;
        w.flush()
    }
}
```

**初始化**：app 启动时扫描 `sessions/*/` 目录无 transcript.jsonl 的旧会话，按 SQLite 回放重建（一次性 IO，后续增量）。回放失败的会话跳过 + warn。

**清理**：删除会话时（`session::db::delete_session`）同步 rm transcript 文件。

### 10.5 性能

- 每条消息一次 fs `write` + `flush`（BufWriter 但我们显式 flush 保证 hook 读到）。
- 典型消息 ~2-10 KB，单 session 日消息量 ~1000 条 → 10 MB/day，可接受。
- `flush` 不等于 `fsync`——进程崩溃可能丢尾部几条；SQLite 仍完整，恢复脚本可回放补。

### 10.6 hook 里怎么读

```bash
# Claude Code 官方脚本 paste 即用：
TRANSCRIPT_PATH=$(jq -r '.transcript_path' < /dev/stdin)
tail -n 5 "$TRANSCRIPT_PATH" | jq -s '.'
```

### 10.7 和 ha-core 现有 message 结构的映射表

ha-core 的 `Message { role, content, tool_calls?, tool_call_id? }` → transcript 行规则：

| 输入 | 输出 `type` | `message.content[]` |
|------|-----------|---------------------|
| user message | `"user"` | `[{type: "text", text: ...}]` + 可选 `[{type: "image", source: ...}]` |
| assistant pure text | `"assistant"` | `[{type: "text", text: ...}]` |
| assistant with tool_calls | `"assistant"` | text + `[{type: "tool_use", id, name, input}]` |
| tool result | `"tool_result"` | `[{type: "tool_result", tool_use_id, content, is_error?}]` |
| Tier 3 summary | `"summary"` | `[{type: "text", text: summary_md}]` |
| system-reminder 注入 | `"system"` | `[{type: "text", text: ...}]` |

### 10.8 和 `_oc_round` 元数据的关系

AGENTS.md 提过 "tool loop 中 assistant + tool_result 通过 `_oc_round` 元数据分组"——我们在 transcript.jsonl 里**保留** `_oc_round` 作为 line 顶层字段（hook 作者如需按 round 聚合可用，不影响 `jq` 基本操作）。

---

## 11. 环境变量 & `CLAUDE_ENV_FILE`

### 11.1 注入给 command hook 的环境变量

| 变量 | 值 | 覆盖策略 |
|------|-----|---------|
| `CLAUDE_PROJECT_DIR` | session 绑定项目根 / session cwd | 进程默认环境 overwrite |
| `HOPE_PROJECT_DIR` | 同上（双注入，值一致） | 同上 |
| `HOPE_AGENT_VERSION` | 当前 `hope-agent` 版本（CARGO_PKG_VERSION） | overwrite |
| `HOPE_SESSION_ID` | 当前 session_id | overwrite |
| `HOPE_TRANSCRIPT_PATH` | §10 JSONL 路径 | overwrite |
| `CLAUDE_CODE_REMOTE` | `"true"` 若为 `hope-agent server` 模式；桌面为 `"false"`；对齐官方语义 | overwrite |
| `CLAUDE_ENV_FILE` | 仅 `SessionStart` / `CwdChanged` / `FileChanged` 注入 | 见 §11.3 |
| `PATH` | 用 `tools::exec::get_login_shell_path()` 解析的登录 shell PATH | 覆盖（避免 `npm`/`python` 找不到） |
| 其它 | 继承父进程；用户可在 `headers`（HTTP）/`env`（command，未来字段）追加 | — |

**未实现**：`CLAUDE_PLUGIN_ROOT` / `CLAUDE_PLUGIN_DATA`（plugin 生态 P2）。

### 11.2 注入给 http hook 的 header 插值

`headers.*` 的 value 里 `$VAR` / `${VAR}` 会被替换——**仅**从 `allowedEnvVars` 白名单取值，避免把 `PATH`、`HOME` 等敏感信息泄给外部服务。

```rust
fn interpolate_header(raw: &str, allowed: &[String]) -> String {
    let re = Regex::new(r#"\$\{?([A-Z_][A-Z0-9_]*)\}?"#).unwrap();
    re.replace_all(raw, |c: &Captures| {
        let name = &c[1];
        if allowed.iter().any(|v| v == name) {
            std::env::var(name).unwrap_or_default()
        } else {
            c[0].to_string()
        }
    }).into_owned()
}
```

### 11.3 `CLAUDE_ENV_FILE` 机制

**用途**：让 hook 在特定事件（SessionStart / CwdChanged / FileChanged）里"持久化"一批 env var，后续所有 command hook 都能读到。

**实现**：
1. 事件触发前生成临时文件 `~/.hope-agent/hooks/env/{session_id}-{ts}.sh`。
2. 把路径设为 `CLAUDE_ENV_FILE` env 传给 hook。
3. Hook 可 `echo 'export FOO=bar' >> $CLAUDE_ENV_FILE`。
4. Hook 跑完后，runner 读回该文件、parse `export KEY=VALUE` 行，写入 session-level env map（`SessionContext::persistent_env`）。
5. 后续该 session 内所有 command hook 的执行环境 merge 这份 env map（source shell 语义：`set -a; source $file; set +a`）。

**一期范围**：仅 `SessionStart`。`CwdChanged` / `FileChanged` 自身是 P1 埋点，等那俩落地后统一开通。

**安全**：
- env map 仅在**本 session**内有效，session 结束清空（不跨 session 污染）。
- 不写入磁盘 config（纯内存）。
- 大小限：单个 value 64 KB，总 map 512 KB，超限丢 + warn。

### 11.4 Env 组装的统一 helper

```rust
// hooks/env.rs
pub struct HookEnv { vars: HashMap<String, String> }

impl HookEnv {
    pub fn build_for_command(ctx: &HookCtx) -> Self { /* §11.1 全部 */ }
    pub fn build_for_http(ctx: &HookCtx, allowed: &[String]) -> Self { /* 仅用于插值 */ }
}
```

---

## 12. 安全 & 审计

### 12.1 红线

**绝对禁止**：

1. 把 API Key / OAuth token 写进 hook input JSON（`message.content` 里如有用户 paste 的 token 另算，hook 作者自己 redact）。
2. 把 session 级别的 `~/.hope-agent/credentials/auth.json` 路径注入 hook env。
3. 让 hook 直接改 `AppConfig.providers[*].api_key`（即使 `ConfigChange` hook 返回 allow，写操作也要过 provider secret 专用通道）。
4. 让 hook 绕过 Plan Mode allowlist / `denied_tools`（见 §9.3）。

### 12.2 审计埋点清单

所有 category=`hooks` 的日志：

| source | 时机 |
|--------|------|
| `config` | 配置加载 / 热重载 / 解析错误 |
| `dispatch` | 每次 `dispatch()` 入口 + 决策结果 |
| `matcher` | matcher compile 失败 |
| `runner.command` | 每条 command hook 执行 |
| `runner.http` | 每条 http hook 执行 |
| `runner.prompt` | 每条 prompt hook 执行 |
| `runner.agent` | 每条 agent hook 执行 |
| `decision` | 聚合结果 + 冲突日志（多 hook 冲突时记录每人的决策） |
| `transcript` | 镜像写入失败 |
| `env` | `CLAUDE_ENV_FILE` parse 异常 |
| `security` | SSRF 拒绝 / allowedEnvVars 未授权变量引用 |

**最小样例**：

```rust
app_info!("hooks", "dispatch",
    "event={} session={} handlers={} decision={:?} dur_ms={}",
    event, sid, handler_count, outcome.decision, dur_ms);

app_warn!("hooks", "runner.command",
    "timeout session={} cmd={} elapsed_ms={}",
    sid, redact(cmd), elapsed_ms);
```

### 12.3 脱敏 & PII

`HookInput` 里可能含敏感字段：
- `tool_input.command`（Bash）可能含 token
- `user prompt` 可能含邮箱 / ID 卡号

**策略**：
- 审计日志里所有 `tool_input` / `prompt` 字段长度 ≥ 200 时截断到 200（`crate::truncate_utf8`）。
- 走 `logging::redact_sensitive` 先跑一遍（AGENTS.md 已有）。

### 12.4 SSRF 统一

http hook 的 URL **必走** `security::ssrf::check_url`；重定向走 `check_host_blocking_sync`。详见 §7.3。

### 12.5 Shell 注入防护

- hook **配置本身**是 shell 字符串，用户有责任自己 quote；我们**不**尝试 parse / rewrite。
- `"$CLAUDE_PROJECT_DIR"` 官方推荐用法，我们在 GUI 面板里 placeholder 预填这个写法 + 警示含空格路径必须 quote。
- `stdin` 里的 JSON 经过 serde 标准编码，不存在 injection 风险。
- command hook 的 stdout 解析用 `serde_json`，不 eval。

### 12.6 超时 / 死锁

- 单 handler 超时见 §7.7。
- 主 `dispatch()` 整体超时上限 = 所有 handler timeout 的 **max + 5s**；即使某个 handler 没死也会被 timeout 后台清理，主流程继续。
- **永不 block 主循环超过 `max_timeout + 5s`**——这是对用户最重要的保证。

### 12.7 资源消耗

- command hook 总并发上限 = `AppConfig.hooks.max_parallel_handlers`（默认 16）。超出的 handler 排队。
- http hook 单独走 reqwest pool，`AppConfig.hooks.http_max_concurrent`（默认 32）。
- prompt / agent hook 共享 side_query / spawn_subagent 的速率——hook 层不做额外限流。

### 12.8 `disableAllHooks` 紧急出口

应用启动参数 `--no-hooks` / 环境变量 `HOPE_NO_HOOKS=1` 在"配了自杀 hook"时救命——**完全跳过**所有 hook dispatch（managed 也关）。启动时该状态写审计日志警示。

---

## 13. GUI 面板

### 13.1 布局

新建 `src/components/settings/hooks-panel/`，按现有 `general-panel/` 模式 Tabs 布局：

```
Settings → Hooks
├─ Overview         已配置事件总览 + 触发统计（过去 24h）
├─ By Event         每个事件一个 Section，展开可编辑 matcher group
│   ├─ PreToolUse   matcher "Bash" → [command ...]
│   ├─ PostToolUse  ...
│   └─ ...
├─ Test Runner      选事件 + 事件 payload 样例 → 手工 dispatch，展示每个 handler 的 stdout/stderr/exit/dur
├─ Scope            user vs project vs local 切换，可看 merged 视图 + 每条 hook 的来源 scope 标签
└─ Emergency        disableAllHooks 开关 + 查看 overflow 文件目录
```

**组件样式**：严格 shadcn/ui + Tailwind，禁用原生 form 控件（AGENTS.md 前端规范）。保存按钮走三态 `saving / saved / failed`。

### 13.2 编辑表单

每条 hook 展开后：

| 字段 | 控件 |
|------|------|
| `type` | Select：command / http / mcp_tool / prompt / agent |
| `matcher` | Input + "测试" 按钮（给它一个 tool_name / source 值验证是否命中） |
| `command` / `url` / `prompt` | 对应输入（command 用 CodeEditor 语法高亮 bash） |
| `server` / `tool` / `input` | mcp_tool 专用：server 下拉（取自 cached_config().mcp_servers）+ tool 下拉（取自该 server 的 cached_tool_defs）+ JSON 模板编辑器（高亮 `${path}` 占位符） |
| `timeout` | NumberInput（秒） |
| `async` / `asyncRewake` | Switch |
| `shell` | Select（bash / powershell） |
| `headers` | KV 列表（仅 http） |
| `allowedEnvVars` | 多选 chip（仅 http） |
| `if` | Input + 语法提示 |
| `once` | Switch（仅 skill/agent frontmatter 语境） |

### 13.3 实时校验

- matcher regex 即时 compile，错了红框 + 原因 tooltip。
- command hook 的 `"$CLAUDE_PROJECT_DIR"` 写法检查（未 quote / 路径含空格警告）。
- http URL 先过 SSRF 策略（本地 loopback 放行、private 警示）。

### 13.4 触发统计

面板顶部：过去 24h 每事件触发次数 + 平均耗时 + 错误率。数据源：`hooks::audit` 日志 + 独立 `~/.hope-agent/hooks/metrics.db`（SQLite，轻量 rolling-window）。

### 13.5 Test Runner

预置事件 payload 模板；用户改完 JSON → 点"Dispatch"→ 看结果。不落库，不影响真实会话。

### 13.6 安全出口（Emergency）

- `disableAllHooks` 一键开关（相当于写 userSettings.disableAllHooks=true，即时生效）。
- "查看 overflow 文件"按钮打开 `~/.hope-agent/hooks/overflow/`。
- "导出所有 hook 配置"生成一个 zip，方便备份 / 分享。

### 13.7 与其它面板协作

- 在 "Approval" 面板顶部加一条提示条："已有 N 条 `PermissionRequest` hook 生效，它们会在弹窗**之前**决定 → 前往 Hooks → …"。
- 在 "Memory" / "Compact" 面板标注 `PreCompact` hook 数量。

---

## 14. `ha-settings` 技能集成

### 14.1 `update_settings` / `get_settings` category 扩展

在 `crates/ha-core/src/tools/settings.rs` 的 `risk_level()` 中新增：

| category | risk_level | 备注 |
|----------|-----------|------|
| `hooks` | **HIGH** | 整棵 hooks 树读写；写操作需用户二次确认 |
| `hooks.pre_tool_use` | **HIGH** | 单事件细粒度 |
| `hooks.post_tool_use` | **HIGH** | 同上，其它事件类推 |
| `hooks.disable_all` | **HIGH** | 一键关闭开关 |

**HIGH** 是因为命令 hook 可任意执行 shell，等同于给 Claude 加了一个"创建 Bash 后门"的工具——严格 gate 住。

### 14.2 工具 schema 示意

```json
{
  "name": "update_settings",
  "input_schema": {
    "category": "hooks",
    "values": {
      "disableAllHooks": false,
      "pre_tool_use": [
        { "matcher": "Bash", "hooks": [...] }
      ]
    }
  }
}
```

`update_settings` 写 hooks 前**强制弹 AskUserQuestion**（一期硬编码），给出：

- 新增 N 条 hook
- 其中 M 条是 command type
- 展开前 3 条的摘要
- 让用户选 "Apply" / "Cancel"

### 14.3 `skills/ha-settings/SKILL.md` 风险表登记

```markdown
| Category             | Risk   | Description                                         |
| -------------------- | ------ | --------------------------------------------------- |
| hooks                | HIGH   | 读写 hooks 配置树；命令 hook 等同 shell 后门       |
| hooks.disable_all    | HIGH   | 一键关掉所有 hooks（含安全审计 hook）              |
| ...                  |        |                                                     |
```

### 14.4 SKILL.md 对 hooks 的**配套文档页**

技能层面还加一份 `skills/ha-settings/references/hooks.md`：给模型看的"如何正确配置 hook"小抄（事件名清单、常见陷阱、`$CLAUDE_PROJECT_DIR` 用法、exit 2 vs 0 的区别）。

---

## 15. Transport / API 命令

### 15.1 Tauri 命令（`src-tauri/src/lib.rs` `invoke_handler!` 注册）

| Command | 参数 | 返回 | 用途 |
|---------|-----|------|-----|
| `hooks_list_all` | 无 | 合并后的 hooks 树 + 每条 hook 的 scope 来源标签 | GUI Overview / By Event |
| `hooks_test_run` | `{ event, matcher_override?, payload }` | `{ handlers: [{id, stdout, stderr, exit, dur_ms, decision}] }` | GUI Test Runner |
| `hooks_metrics_24h` | 无 | 每事件聚合数据 | GUI Overview |
| `hooks_set_scope` | `{ scope, event, matcher_groups }` | `Result<()>` | GUI 编辑保存 |
| `hooks_emergency_disable` | `{ disable: bool }` | `Result<()>` | Emergency 开关 |
| `hooks_overflow_list` | 无 | `[{path, event, ts, size}]` | overflow 文件查看 |
| `hooks_export` | 无 | 一个 zip 的 base64 | 导出备份 |

对应在 `src/lib/transport-tauri.ts` + `src/lib/transport-http.ts` 加 invoke wrapper（Transport 双适配契约）。

### 15.2 HTTP 路由（`crates/ha-server/src/router.rs` 注册）

| Method | Path | 对应 Tauri 命令 |
|--------|------|----------------|
| `GET` | `/api/hooks` | `hooks_list_all` |
| `POST` | `/api/hooks/test` | `hooks_test_run` |
| `GET` | `/api/hooks/metrics` | `hooks_metrics_24h` |
| `PUT` | `/api/hooks/scope/:scope` | `hooks_set_scope` |
| `POST` | `/api/hooks/emergency` | `hooks_emergency_disable` |
| `GET` | `/api/hooks/overflow` | `hooks_overflow_list` |
| `GET` | `/api/hooks/export.zip` | `hooks_export` |

**鉴权**：所有 hooks 路由要求 API key（和其它敏感路由一致）。

### 15.3 COMMAND_MAP 更新

ACP / channel 复用的 COMMAND_MAP 同步新增这些命令，保持三端对齐（AGENTS.md 硬要求）。

### 15.4 `docs/architecture/api-reference.md` 回写

新增 "Hooks" 功能域表格，列出 7 个 Tauri 命令 + 7 条 HTTP 路由。

---

## 16. 日志 & 观测

### 16.1 日志流向

hooks 系统的所有日志都通过 `app_info!` / `app_warn!` / `app_error!` / `app_debug!` 进入 **`logging/mod.rs`** 的 SQLite + 文本双写（AGENTS.md 红线：禁用 `log` crate 宏）。

**全部事件**均在 `category="hooks"` 下，`source` 细分见 §12.2 清单。

### 16.2 关键决策点必埋（AGENTS.md"核心业务路径必须埋点"）

| 节点 | 级别 | 目的 |
|------|-----|------|
| 配置加载 | `app_info!` | 启动时记录"加载到 N 条 hook"—— grep `source=config` 一眼看清用户配置 |
| 配置热重载 | `app_info!` | 排查 GUI 改完没生效 |
| matcher 编译失败 | `app_warn!` | 用户正则写错会静默降级，必须能查到 |
| `dispatch()` 入口 | `app_info!` | 每次触发一行，带 event / session / handler_count |
| `dispatch()` 出口 | `app_info!` | 决策 + 总耗时 |
| 单 handler 超时 | `app_warn!` | 超时是默认 600s，超了基本是 hook 脚本 bug |
| 单 handler 非 0 退出 | `app_warn!`（`exit 2`）/ `app_error!`（其它非 0） | 区分"用户主动 block"和"脚本崩了" |
| SSRF 拒绝 | `app_error!` | 安全事件，source=`security` |
| `additionalContext` 超 10K 字符溢出 | `app_warn!` | 追踪 hook 作者过度注入 |
| `continue=false` 终止 | `app_warn!` | 强决策，用户需要能看见 |
| Stop hook 死循环保护触发（≥3 次） | `app_error!` | 我们的扩展，记下来方便判定 hook 作者故障 |

### 16.3 指标（`~/.hope-agent/hooks/metrics.db`）

轻量 rolling-window SQLite，单表：

```sql
CREATE TABLE hook_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,                 -- unix millis
    event TEXT NOT NULL,
    handler_type TEXT NOT NULL,          -- command|http|prompt|agent
    matcher TEXT,
    session_id TEXT NOT NULL,
    dur_ms INTEGER NOT NULL,
    exit_code INTEGER,
    decision TEXT,                       -- allow|deny|ask|defer|block|noop
    timed_out INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_hi_ts_event ON hook_invocations(ts, event);
```

- 只保留最近 7 天，每日 cron 删旧（复用 `ha-core::cron` 基础设施）。
- GUI Overview 面板从这里聚合。
- `app_debug!` 不写这张表；指标是"概览"，详细排查看 `logging.db`。

### 16.4 Dashboard Insights 对接

`dashboard/insights.rs` 增加 "hooks_health" 区块：
- 过去 24h hook 触发总数
- 超时率 / 错误率
- Top 3 最慢 handler
- Top 3 最常 block 的事件 + reason 抽样

作用：让用户一眼看到"某个 hook 把会话拖慢了"。

### 16.5 Learning Tracker 对接

`session.db::learning_events` 增加 `hook_*` 事件类型（见 AGENTS.md Learning Tracker 节）：
- `hook_deny_tool` — hook 阻断了某工具，模型学到什么命令不能跑
- `hook_injected_context` — hook 注入的 `additionalContext` 被下一轮利用了

一期可先不实现，P1 再补。

---

## 17. 测试方案

### 17.1 单元测试（`crates/ha-core/src/hooks/tests/`）

| 文件 | 覆盖 |
|------|-----|
| `matcher_test.rs` | 三种 matcher 判别 + 边界（空字符 / `*` / `A\|B\|C` / `^Notebook` / `mcp__.*`） |
| `config_test.rs` | 四层作用域合并 + 去重 + `disableAllHooks` 层级 |
| `dispatch_test.rs` | 决策聚合（`deny > defer > ask > allow`）+ `continue=false` 优先级 + `updatedInput` 合并 |
| `output_parser_test.rs` | exit 0 / 2 / 非 0 解析；plain text fallback（仅 SessionStart / UserPromptSubmit）；JSON 非法字段 warn |
| `env_test.rs` | `CLAUDE_ENV_FILE` 持久化往返；`allowedEnvVars` 白名单 |
| `transcript_test.rs` | 消息格式对齐 Claude Code schema；multi-session 并发写不串行错序 |

### 17.2 集成测试（`crates/ha-core/tests/hooks_integration.rs`）

走真子进程 + fixture hook 脚本（`crates/ha-core/tests/fixtures/hooks/*.sh`）：

| 测试 | 场景 |
|------|-----|
| `bash_block_rm.sh` | PreToolUse exit 2 阻断 rm 命令，返回 reason，tool 未执行 |
| `prompt_rewrite.sh` | UserPromptSubmit 注入 additionalContext，下一轮 LLM 收到了 |
| `slash_block.sh` | UserPromptExpansion 拦截某 skill `/dump-secrets`，返回 reason |
| `batch_audit.sh` | PostToolBatch 在并发 ≥3 个 Read 后注入 system-reminder 让 LLM 总结 |
| `format_after_edit.sh` | PostToolUse async hook 在 Edit 后跑 `rustfmt`，不阻塞 |
| `asyncrewake_linter.sh` | PostToolUse asyncRewake exit 2 → 下一轮 user turn system-reminder 含 stderr |
| `permreq_auto_allow.sh` | PermissionRequest hook 直接 allow + updatedPermissions 持久化到 local |
| `permreq_setmode.sh` | PermissionRequest 用 `setMode: acceptEdits` 切运行模式 |
| `permdenied_retry.sh` | PermissionDenied 返回 `retry: true`，LLM 再次尝试改写命令 |
| `mcp_tool_security_scan.json` | PostToolUse mcp_tool 触发 `my_server.security_scan`，结果 isError=true → block |
| `sessionstart_env.sh` | SessionStart hook 写 `CLAUDE_ENV_FILE`，后续 command hook 读到新 env |
| `http_ssrf_reject.rs` | http hook URL 指 169.254.169.254 → SSRF 拒绝 + 非阻断 |
| `stop_loop_guard.sh` | Stop hook 连续 block → 第 3 次强制 stop |
| `config_change_rollback.sh` | ConfigChange hook block → mutate_config 回滚 |
| `subagent_stop_to_subagent.sh` | 子代理上下文里的 `Stop` hook 自动转为 `SubagentStop`（Stop hook 不应触发） |

### 17.3 GUI / E2E 测试

`src/__tests__/`（Vitest / React Testing Library）：

- hooks-panel 渲染 + 编辑保存 + 触发 Tauri 命令
- Test Runner：模拟 payload → 展示 handler 结果
- Emergency 开关：写 userSettings.disableAllHooks=true 后 `hooks_list_all` 返回空

### 17.4 官方脚本兼容性测试

在 `crates/ha-core/tests/fixtures/hooks/claude-code-compat/` 放 3-5 个**原封不动从 Claude Code 官方文档 / 社区复制**的 hook 脚本（bash + JSON schema）：

- 官方 Bash validator 示例（§16 of docs 那段 `jq -r '.tool_input.command'`）
- 社区 pre-commit hook 类样例（Write/Edit 后 `prettier`）
- 社区 notification Slack webhook 样例

这些脚本必须**零改动**在 Hope Agent 里跑通——这是 G1（字段级对齐）的最硬验收。

### 17.5 手工验证清单

PR 合并前，逐项手动试：

- [ ] `~/.hope-agent/settings.json` 加 PreToolUse Bash hook → Chat 里 `ask Bash to run ls` → hook 触发
- [ ] GUI 面板新建 hook → 保存 → 无需重启，下一次事件已生效
- [ ] 超时 hook（`sleep 9999`）→ 不卡死主循环，日志里有 timeout warn
- [ ] `disableAllHooks=true` → 所有 hook 停用
- [ ] `hope-agent server` 模式下 hook 同样生效
- [ ] 删除会话 → transcript.jsonl 同步删除
- [ ] `--no-hooks` 启动 → hook 完全旁路 + 启动日志 warn

---

## 18. 实施阶段拆分

### 18.1 Phase 0.1 — Hooks 模块骨架（1 PR，纯基础设施）

目的：让后续所有 PR 都能 plug into，不引入任何业务语义改动。

- 新建 `crates/ha-core/src/hooks/` 空模块骨架：`mod.rs` / `types.rs` / `config.rs` / `matcher.rs` / `registry.rs` / `runner/mod.rs` / `decision.rs` / `audit.rs`。
- `AppConfig` 加 `hooks: HooksConfig` 字段（空 default），`disable_all_hooks: bool`。**注意：本 PR 只做 user scope（写 `~/.hope-agent/config.json` 顶层 `hooks` 字段），多 scope 合并留 §18.2。**
- `HookEvent` 28 变体常量 + `HookInput` / `HookOutput` / `HookOutcome` 数据结构全定义。
- `HookDispatcher::dispatch` 先做成**永远返回 `HookOutcome::noop()`**的空跳板——业务侧可以埋点但不影响流程。
- `TranscriptMirror` 基础版（写 JSONL 镜像，附 backfill 函数）。
- `agent::preflight::user_prompt_preflight` helper 骨架（F3 修复用）—— 一期内部实现就是 `Proceed { effective_prompt: raw }` 透传，但**三个入口（Tauri / HTTP / IM）都改造完毕走这个 helper**，把 user message 持久化的位置统一到 helper 之后。这样 Phase 1.2 真正接入 hook block 时不必再改三处入口代码。
- 日志 category=`hooks` 注册。
- 单元测试：matcher 引擎 + 配置反序列化。

**通过标准**：`cargo test` 绿、旧行为零变化、`~/.hope-agent/config.json` 多出 `hooks: {}` 字段兼容老配置；三个入口的 user message 持久化已经统一在 preflight 之后。

### 18.2 Phase 0.2 — 多 scope 配置基础设施（独立 PR，hooks 之外通用）

> **F4 修复**：本 PR 解决 hooks（以及未来 permissions / approval rules / mcp 配置）的多作用域基础设施缺失问题。它**不属于** hooks 模块本身，但 hooks Phase 1 的 §4.1 四层作用域行为强依赖它。**此 PR 必须先合**，否则 Phase 1 只能落地 user scope。

工作内容：

- `crates/ha-core/src/config/scope.rs` 新模块：定义 `ConfigScope ∈ {Managed, User, Project, Local}`、`ConfigSource` 元数据（来源 scope + 文件路径 + 是否可写）、`MergedConfig<T>` trait（针对每个支持多 scope 的字段定义如何合并）。
- 文件读写：
  - `~/.hope-agent/settings.json`（user）—— 与现有 `config.json` 区分；`config.json` 仍是 user scope 单一真相源，新增 `settings.json` 用于 hooks / permissions 这类**作用域感知**字段。Phase 0.3（待写）评估是否合并这两份文件。
  - `<project>/.hope-agent/settings.json`（project）—— 在项目根 `~/.hope-agent/projects/{id}/path` 解析后挂载。
  - `<project>/.hope-agent/settings.local.json`（local）—— 同上但 gitignored。
  - `~/.hope-agent/managed-settings.json`（managed，macOS）/ `/etc/hope-agent/managed-settings.json`（Linux）/ `%ProgramData%\HopeAgent\managed-settings.json`（Windows）。
- `mutate_config_scoped(scope, (category, source), |cfg| {...})` API：写入指定 scope 的文件，`mutate_config(...)` 默认 = `mutate_config_scoped(User, ...)`。
- 文件 watch：用 `notify` crate 监听四份文件 + skill frontmatter 变化，触发 `cached_config()` rebuild + emit `config:changed`。
- 权限校验：managed 文件 owner 必须是 root / admin（macOS / Linux），写入失败 fall back 为只读；project / local 路径必须在当前用户可写目录内。
- 回滚：合并失败（如 managed 字段类型冲突）回退到上一可用快照 + `app_error!`。
- 测试：merge 优先级 / disableAllHooks 层级 / 文件不存在容错 / managed 只读保护 / 同时改两份文件竞态。

**为什么不和 Phase 0.1 合一**：
- 多 scope 框架是通用基建，今后 permissions、mcp、approval 也会用，单独立项便于复用。
- 多 scope 的 corner case（managed 优先级、合并冲突、scope 切换通知）单元测试很重，混进 hooks PR 会让 review 体积爆炸。
- 没有它 hooks 也能跑（只是限 user scope）。先发 hooks Phase 1 + user scope，验收完官方脚本兼容套件，再上多 scope。

**通过标准**：四份配置文件 CRUD/merge/watch/rollback 测试全绿；现有 user scope 行为零变化；`mutate_config(...)` 兼容老调用。

### 18.3 Phase 1 — P0 事件落地（2-3 PR）

> **依赖**：Phase 0.1 完成。多 scope 行为依赖 Phase 0.2，但 Phase 1 PR 1.1 / 1.2 / 1.3 / 1.4 本身可以只支持 user scope 工作，等 Phase 0.2 合入后再扩 GUI Scope 切换 / managed 警示。

按风险从低到高：

**PR 1.1 — 观察型事件（不阻断）**
- `SessionStart` / `SessionEnd` / `Notification` 埋点
- `PostToolUse` / `PostToolUseFailure` 埋点（仅 `additionalContext` 注入，不做 MCP override）
- `PostCompact` 埋点
- Runner 只实现 `command` 类型（最常用，其它后续 PR 扩）
- 审计日志 + overflow 文件机制

**PR 1.2 — 阻断型事件**
- `UserPromptSubmit` / `UserPromptExpansion`（均可 block）
- `PreToolUse`（完整决策：allow/deny/ask/defer，updatedInput 回写）
- `PostToolBatch`（block-as-system-reminder）
- `PreCompact`（可 block）
- `Stop`（可 block + 死循环保护；子代理上下文自动转 SubagentStop）
- Runner 增加 `http` 类型（SSRF 校验闭环）

**PR 1.3 — 权限决策链 + MCP runner**
- `PermissionRequest`（完整 decision + updatedPermissions 6 种 type 持久化）
- `PermissionDenied`（`reason` 字段 + `retry` 语义；`if:` 字段 5 事件全覆盖）
- Plan Mode / YOLO / Approval 与 hook 的叠加顺序（§9.3）端到端跑通
- Runner 增加 `prompt` 类型 + **`mcp_tool` 类型**（前提：MCP 客户端已 ready；否则该 handler 解析为非阻断错误）

**PR 1.4 — GUI 面板 + 技能**
- `hooks-panel` 所有 Tabs（Overview / By Event / Test Runner / Scope / Emergency）
- `ha-settings` 技能 `category="hooks"` 读写 + 风险登记 + 强制二次确认
- Tauri 命令 + HTTP 路由 7 个
- `docs/architecture/api-reference.md` 回写
- `docs/architecture/hooks.md` 新建（§19 文档回写强制）

**通过标准**：§17.4 官方脚本兼容套件 5 个样例全绿。

### 18.4 Phase 2 — P1 事件补全（2 PR）

**PR 2.1 — 子代理 & 失败链**
- `SubagentStart` / `SubagentStop`
- `StopFailure`
- `TaskCreated` / `TaskCompleted`（复用 subagent）
- Runner 增加 `agent` 类型（只读工具集）

**PR 2.2 — 配置 & 环境**
- `ConfigChange`（含对自身 hooks 改动的安全出口）
- `InstructionsLoaded`（system_prompt 组装埋点）
- `CLAUDE_ENV_FILE` 机制
- `TeammateIdle`（需先给 team runtime 补 idle 检测，上游依赖单独立项）

### 18.5 Phase 3 — 基础设施补齐（2 PR）

**PR 3.1 — 文件监听 & cwd**
- `session::cwd` 模块 + `CwdChanged` 事件
- `project::file_watcher`（notify crate）+ `FileChanged` 事件
- `CLAUDE_ENV_FILE` 扩展到这俩事件

**PR 3.2 — Dashboard 集成**
- Insights `hooks_health` 区块
- Learning Tracker 的 `hook_*` 事件类型
- Metrics rolling-window 窗口自动清理

### 18.6 Phase 4（可选 / 未来）

- MCP 落地 → 激活 `Elicitation` / `ElicitationResult`
- Worktree 隔离能力 → 激活 `WorktreeCreate` / `WorktreeRemove`
- Plugin hooks.json 规范
- `defer` decision 的 headless-mode 流（需要先做 `-p` 非交互模式）
- `if:` 字段 Bash subcommand 真拆分

### 18.7 每阶段退出条件

- 所有 Phase 1 PR 必须 §17.1 单元测试 + §17.2 集成测试绿。
- Phase 1.4 结束跑一遍 `cargo fmt --all --check` / `cargo clippy ... -D warnings` / `cargo test -p ha-core -p ha-server` / `pnpm typecheck` / `pnpm lint`（AGENTS.md 提交前检查）。
- CHANGELOG.md 每 PR 更新 "Added / Changed / Fixed"。
- README / README.en 在 Phase 1.4 之后同步提到 "Hooks (Claude Code compatible)" 一行特性。

---

## 19. 兼容性 & 风险

### 19.1 官方脚本兼容性

**强目标**：凡是 Claude Code 官方文档里出现的 hook 样例 / 社区仓库里常见的 hook 脚本，**零改动**在 Hope Agent 下能跑。

**兜底策略**：
- `transcript_path` JSONL 镜像（§10）保证 `jq` 脚本可读。
- `CLAUDE_PROJECT_DIR` 环境变量保留（§11.1）。
- `exit 0 / 2` 语义逐字段对齐（§8.3）。
- JSON 字段命名：input 下划线 / output 驼峰，与官方一致（§8.1 / §8.2）。
- 未识别字段 warn 但不 panic——允许脚本给我们发"多出来的字段"而不 crash。

**已知差异**（§2.4 已登记，此处回顾）：

| 差异点 | 影响 | 缓解 |
|--------|-----|------|
| `permission_mode` 仅 3 值 | 社区脚本 switch 5 值时未覆盖分支走 `other` | 保留 `other` 作为 fallback + doc 明示 |
| `defer` 决策降级为 `ask` | headless 脚本的 defer 逻辑变成了手工审批 | doc 明示 + 日志 warn 每次触发 |
| `if:` Bash subcommand 一期不拆 | 复杂 pipeline 规则会"误放宽" | Phase 4 补；一期用 tool-level `matcher` 兜底 |
| MCP 事件 / Worktree 事件永不触发 | 依赖它们的脚本静默不工作 | doc 明示 + GUI 面板灰色展示"未启用" |

### 19.2 性能风险

| 风险 | 触发 | 缓解 |
|-----|------|------|
| hook 超时拖慢工具链 | 用户配的 command hook `sleep 9999` | 默认 timeout 600s（太长）→ 建议 GUI 默认模板用 30s；整体熔断 `max + 5s` |
| `PreToolUse` 在**每次**工具调用前跑 | tool loop 20 轮 × N 个 hook = 大量 spawn | 去重 + 并发 + async 选项；文档推荐 command hook 写得轻 |
| `PostToolUse` 阻塞历史落地 | async=false 时 fmt 脚本慢 → 下一轮 LLM 等 | 文档推荐 `async=true` + `asyncRewake=true` 的"非阻塞审计"模式 |
| Transcript JSONL 随消息数线性增长 | 超长会话每条都 flush 一次 fs | BufWriter + 每 100 条或 500ms 强制 flush；`fsync` 只在会话关闭时 |
| matcher 正则 ReDoS | 用户写 `(a+)+b` 被外部事件撞 | 启用 `regex` crate DFA；匹配 timeout 100ms，超时记 warn + 视为未命中 |

### 19.3 安全风险

| 风险 | 缓解 |
|-----|------|
| 恶意 hook 脚本盗 API key | hook env 不含 credential 路径；脚本 cwd 限 session cwd 但不隔离 fs → 提示用户仅信任 settings 源 |
| `update_settings` 工具被 LLM 滥用开 shell 后门 | category=`hooks` HIGH + 强制 AskUserQuestion 二次确认 |
| SSRF 进内网 | `security::ssrf::check_url` 必走；`http` hook URL placeholder 提示 |
| hook 在 project-level settings.json 里塞恶意 command，团队成员 pull 到后自动执行 | IDE 打开陌生 project 时**首次**检测到 project hooks 弹警示 dialog：列出前 N 条 command，用户手动 ack 才启用 |
| `managed` 层被普通用户覆盖 | managed 路径要 root / admin 写（macOS `/Library/Application Support/`、Linux `/etc/`）；app 启动时校验 owner |

### 19.4 死锁 / 无限循环

| 场景 | 保护 |
|------|-----|
| `Stop` hook 永远 block | 连续 3 次强制 stop（§9.7） |
| `ConfigChange` hook 拦所有变更，包括自身关闭 | `--no-hooks` 启动参数 + `HOPE_NO_HOOKS=1` env 紧急出口（§12.8）；GUI Emergency 面板 |
| `PreToolUse` hook 内部自己调 `hope-agent` 工具造成递归 | dispatch 加"已在 hook 执行中"标志，嵌套触发直接 noop + warn |
| `UserPromptSubmit` hook 调用 HTTP self-loop | 同上 dispatch recursion guard |

### 19.5 升级 / 降级路径

- **新字段加入**（官方偶尔会加）：`HookInput` 用 `#[serde(flatten)]` + `extra_fields: HashMap<String, Value>` 兜住；未定义字段不丢。
- **老版本回滚**：config schema 向后兼容（新字段都带 `#[serde(default)]`），旧版读不到新字段也不 panic。
- **Claude Code 协议 breaking change**：文档维护 "Compatibility Matrix" 一节，标 `Hope Agent X.Y` 对齐 `Claude Code Z.W`；超前 / 滞后都写清。

### 19.6 用户教育

- `skills/ha-settings/references/hooks.md` 给 LLM 看：何时用何种 hook、常见陷阱、exit 2 vs 0。
- GUI 面板每个字段旁边 `IconTip` 指向"为何这样设计"的 inline doc。
- Test Runner 降低试错成本。
- 发布 blog post：「Hope Agent 也有 Hooks 了」含 3 个最常用场景示例（格式化、阻断危险命令、同步到 Slack）。

### 19.7 回滚策略

如果上线后发现严重问题：

1. **快速**：用户可设 `disableAllHooks=true` 或 `HOPE_NO_HOOKS=1` 关闭，不影响会话其它功能。
2. **稳**：`AppConfig.hooks_kill_switch=true`（一期就加这个配置，默认 `false`）→ `HookDispatcher::dispatch` 直接短路返回 noop。
3. **最差**：回滚到上一个 hope-agent 版本，`hooks: {}` 字段仍在配置文件里但被忽略。

---

## 20. 开放问题 & 后续决策点

以下问题**不卡本方案通过**，但实施过程中需要定点决策。每条标"触发时点"——到时候再拍板就来得及。

### 20.1 `defer` 真实现

**问题**：官方 `defer` 语义是"让当前 headless 进程挂起，等待用户 resume"。Hope Agent 没有 `-p` 非交互模式，一期降级为 `ask`。

**何时需要决策**：当用户反馈"我有 CI 场景跑 hope-agent，想让 hook 真的挂起"。

**可选路径**：
- **路径 A**：给 `hope-agent server` 加一个"defer waiting queue"：`defer` 把会话冻结到 DB，通过 HTTP `/api/sessions/:id/resume` 恢复。
- **路径 B**：保持 `ask` 降级，引导用户直接用 `command` hook 退出 + 外部编排（airflow / GitHub Actions）。
- **默认建议**：路径 B（YAGNI）。

### 20.2 Skill / Agent frontmatter 里 hooks 的冲突语义

**问题**：两个同时激活的 skill 都声明了 `PreToolUse` hook，顺序如何？

**一期方案**：按 skill 加载顺序（alphabetical by skill name），去重后保留所有。

**遗留**：如果用户两个 skill 给出矛盾决策，走 §9.1 优先级（deny > defer > ask > allow）—— 但他们可能困惑"为什么我的 skill allow 却被另一个 skill deny 了"。GUI 里 By Event 视图显示每条 hook 的 source scope（含 skill 名字）能缓解。

**何时需要决策**：社区出现 skill 生态、skill 间冲突投诉多起来时。

### 20.3 `updatedInput` 连续改写的 audit trail

**问题**：PreToolUse 两个 hook 都给 `updatedInput`，后者覆盖前者——前者的改动丢了。用户需要 audit 看"谁改了什么"。

**一期方案**：`hooks_invocations` metrics 表每条 handler 的 updatedInput 存原始 JSON（BLOB 列）。GUI Test Runner 分段展示。

**遗留**：生产线上真触发时看不到这个 diff（GUI 只有事后 metrics），Phase 1 先只落 metrics，Phase 2 再做"实时 diff viewer"。

### 20.4 Hook 触发的**指数退避**

**问题**：同一 hook 反复超时，应该惩罚它吗？

**一期方案**：不做。每次都原样跑，信任用户会自己从日志发现。

**何时考虑**：用户投诉"一个 hook 总是超时，每次 tool call 都卡 30s"。引入 per-handler 失败计数 + 3 连续超时自动 disable 24h + 通知用户。

### 20.5 Plugin hooks.json 生态

**问题**：未来 Hope Agent 可能有 plugin 机制（类似 Claude Code 的 plugin 体系），plugin 能自带 hooks.json。

**一期方案**：配置 schema 保留"plugin scope"为 P2 占位。hook 的来源标签枚举里加 `Plugin { plugin_id }` 变体但不触发。

**何时决策**：plugin 机制立项时。

### 20.6 Managed policy 分发

**问题**：企业 IT 如何推送 `managed-settings.json`？手工复制文件、Jamf / Intune 脚本、还是 hope-agent 自带 pull 机制？

**一期方案**：不做自带。仅校验路径 + owner（root / admin 才能写），并在 GUI Settings 里展示"已加载 managed policy"只读视图。

**何时决策**：第一家企业客户要求集中管理时。

### 20.7 `statusMessage` 的 UI 展示风格

**问题**：官方 `statusMessage` 在 Claude Code CLI 里是顶部状态条。桌面 GUI 有现成的 Toast 组件，但 chat 内也可以以 system-reminder 展示——选哪个？

**候选**：
- **A**：Toast + auto-dismiss 5s（轻量、不进历史）
- **B**：Chat 内系统消息块（进历史、可回顾）
- **C**：两者都做，用户 settings 切换

**初步建议**：A（Toast）+ 常驻 "Hook status" 面板可回看最近 N 条。历史轻量原则。

### 20.8 `CLAUDE_CODE_REMOTE` 语义

**问题**：官方该 env 表示"远程 headless session"。Hope Agent `server` 模式算不算？ACP 算不算？

**一期方案**：
- 桌面（Tauri） → `"false"`
- `hope-agent server` → `"true"`
- `hope-agent acp` → `"true"`

**遗留**：官方脚本里 `if [[ "$CLAUDE_CODE_REMOTE" == "true" ]] then ...` 会把桌面排除，ACP 和 server 等同远程—— 符合直觉。

### 20.9 多会话并发 hook 资源争用

**问题**：用户同时开 5 个会话，每个会话触发 hook → 全局并发上限怎么设？

**一期方案**：§12.7 的上限是**全局**（不分会话）。16 并发 command + 32 并发 http。超出排队。

**何时决策**：用户反馈"我开了 10 个会话，一个在跑慢 hook 把其它都挤住了"—— 届时改成 per-session 限流。

---

## 附录 A：事件 → 埋点位置速查

| 事件 | 代码位置 |
|------|---------|
| SessionStart (startup/resume) | `crates/ha-core/src/agent/mod.rs::AssistantAgent::chat` 首条消息前 |
| SessionStart (compact) | `crates/ha-core/src/context_compact/engine::run_compaction` 成功返回前 |
| SessionStart (clear) | `crates/ha-core/src/session/db.rs` clear 分支 |
| SessionEnd | 同上 close_session / app shutdown / logout |
| UserPromptSubmit | `crates/ha-core/src/agent/mod.rs::AssistantAgent::chat` 收到 user msg 后、push 到历史前 |
| UserPromptExpansion | `crates/ha-core/src/agent/system_prompt.rs` 斜杠命令解析后、push expansion 到历史前 |
| PreToolUse | `crates/ha-core/src/tools/execution.rs::execute_tool_with_context` visibility 后、approval 前 |
| PostToolUse | 同上 tool result 返回后、落历史前 |
| PostToolUseFailure | 同上 `Err` / panic 分支 |
| PostToolBatch | `crates/ha-core/src/agent/streaming_loop.rs` 本轮全部 tool call settle 后、`append_round_to_history` 之前（每 API round 触发一次） |
| PermissionRequest | `crates/ha-core/src/tools/approval::check_and_request_approval` 弹窗前 |
| PermissionDenied | approval auto-mode classifier 否决 / Plan Mode allowlist 否决 |
| Notification | EventBus `approval_required` / 空闲 / auth_success |
| Stop | `crates/ha-core/src/agent/streaming_loop::run` 自然结束、emit_usage 前 |
| StopFailure | `crates/ha-core/src/failover/executor::execute_with_failover` 终态错误 |
| PreCompact | `crates/ha-core/src/context_compact/engine::run_compaction` 入口 |
| PostCompact | 同上成功返回前 |
| SubagentStart | `crates/ha-core/src/subagent/spawn.rs::spawn_subagent` spawned emit 后 |
| SubagentStop | 同上 terminal 更新后 |
| TaskCreated / TaskCompleted | 一期复用 subagent；未来 TaskCreate 工具 |
| TeammateIdle | `crates/ha-core/src/team/runtime.rs` idle 检测（需新增） |
| ConfigChange | `crates/ha-core/src/config/persistence::mutate_config` 事务提交前 |
| CwdChanged | `crates/ha-core/src/session/cwd.rs::set_cwd`（新增） |
| FileChanged | `crates/ha-core/src/project/file_watcher.rs`（新增） |
| InstructionsLoaded | `crates/ha-core/src/agent/system_prompt.rs` 每次加载 memory/instructions 时 |
| Elicitation / ElicitationResult | MCP 落地后激活，占位 |
| WorktreeCreate / WorktreeRemove | worktree 能力落地后激活，占位 |

---

## 附录 B：与其它文档的回写清单

按 AGENTS.md "文档维护"强制要求，本能力落地时**同一 PR 内**需要更新：

| 文档 | 改动 |
|------|------|
| `CHANGELOG.md` | Added: Hooks system (Claude Code compatible) |
| `AGENTS.md` | "架构约定"新增"Hooks"小节（指向本文档）；"易错提醒"加"新增 hook 事件需埋点 + 测试" |
| `docs/README.md` | 索引新增 `architecture/hooks.md` |
| `docs/architecture/hooks.md` | **新建**，本文档精简公开版（用户视角，不讲实现细节） |
| `docs/architecture/api-reference.md` | 新增 "Hooks" 功能域表格（7 Tauri 命令 + 7 HTTP 路由） |
| `docs/architecture/config-system.md` | hooks 字段写入走 `mutate_config` 的例子 |
| `README.md` / `README.en.md` | 特性清单加一行 "Hooks (Claude Code compatible)" |
| `skills/ha-settings/SKILL.md` | 风险表新增 hooks = HIGH |
| `skills/ha-settings/references/hooks.md` | **新建**，模型视角的 hooks 使用指南 |

---

## 附录 C：术语对照表

| 本文用词 | 对应官方用词 | 说明 |
|---------|-------------|------|
| "Hook 事件" | `Hook event` | 28 种（一期 P0 14 / 二期 P1 10 / P2 占位 4）|
| "Handler" / "处理器" | `Hook` | 一个 `{type, command/url/prompt, ...}` 配置项 |
| "Matcher group" | `matcher block` | 一个 `{matcher, hooks: [...]}` 对象 |
| "作用域 / scope" | `source`（在 `source` 字段里出现）/ `settings hierarchy` | user / project / local / managed / skill |
| "JSONL 镜像" | （我们专有） | `transcript_path` 指向的文件 |
| "紧急出口" | （我们扩展） | `--no-hooks` / `HOPE_NO_HOOKS=1` |

---

## 附录 D：设计决策摘要

| # | 决策 | 缘由 |
|---|------|------|
| D1 | 字段级 100% 对齐官方 | 生态复用 > 自主设计（见 Context） |
| D2 | 28 事件一次性声明，分阶段埋点 | 避免后续 `HookEvent` enum breaking change |
| D3 | 四种 handler 类型一次性支持 | command 覆盖 80% 场景，但 http/prompt/agent 是官方承诺的，不做会被社区质疑 |
| D4 | JSONL 镜像方案（非按需导出） | hook 脚本 `jq tail` 友好；IO 成本可接受 |
| D5 | `CLAUDE_PROJECT_DIR` + `HOPE_PROJECT_DIR` 双注入 | 官方脚本 paste 即用 + 未来品牌独立 |
| D6 | Hook 层加在既有 gate 外侧 | 不改既有语义；hook `allow` 不能绕过 Plan Mode / denied_tools 硬红线 |
| D7 | `category="hooks"` 统一日志 | AGENTS.md 要求；grep 友好 |
| D8 | `ha-settings` 技能写 hooks 强制二次确认 | command hook = shell 后门，HIGH 风险不让 LLM 静默开 |
| D9 | 紧急出口 `--no-hooks` / `HOPE_NO_HOOKS` | 防"自杀 hook"锁死 |
| D10 | Phase 1 只做 P0 14 事件 | 降低首轮实现风险；剩下 14 个事件分 2-3 期补齐 |

---

**文档结束**。
