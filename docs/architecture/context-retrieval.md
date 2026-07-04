# Context Retrieval v2

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 7.3 已实现。Context Retrieval v2 是 Workspace 面板里的推荐上下文与行动入口：推荐本身只读，不创建新的 durable run；候选行上的聚焦审查 / 聚焦验证按钮会显式调用 Review / Verification owner API；IDE / ACP 当前上下文已作为一等信号接入；Domain Context Retrieval 已把通用场景的 document / email thread / calendar event / sheet range / knowledge note / web source / decision / artifact 候选、domain profile、access issue 接入同一 snapshot。

## 1. 目标

Context Retrieval v2 回答一个用户视角的问题：

> 当前任务下一步最该看哪些上下文？

它把分散在工作台里的信号聚合成一个有优先级的候选列表：

- 当前 Git diff 改动文件。
- 本会话历史读写过的文件与 URL 来源。
- LSP diagnostics。
- Review Engine findings。
- Smart Verification steps。
- 当前或最近 Goal evidence。
- 当前 task 进度。
- 最近 Workflow run / op 状态。
- query 驱动的 file search v2 结果。
- query 驱动的 LSP workspace symbols。
- IDE / ACP 当前文件、selection、open tabs、active diagnostic、active symbol。
- Domain workflow / domain evidence 推导出的文档、网页来源、邮件线程、日历事件、表格范围、知识笔记、用户决策和产物。
- Domain access issue：连接器或必需 evidence 缺失时显式展示，不伪造不存在的上下文。

首批优化 coding 场景；Phase 7.3 起它已经是通用 owner-plane 上下文推荐。Coding 的 Git/LSP/review/verification 排序保持原样，domain workflow 只在识别到 domain run、domain evidence、显式 domain 或可推断 Goal 时加权。

## 2. 架构边界

核心实现位于 `ha-core::context_retrieval`，入口是：

```rust
context_retrieval_for_session(db, session_id, ContextRetrievalInput { query, limit, ide_context, domain, template_id, template_version })
```

重要边界：

- 推荐查询是只读 owner API：不创建 run、不写 DB、不改变模型状态。
- 候选行动是显式 owner action：`metadata.actions.focusPaths` 只声明可操作目标，GUI 点击后调用 Review / Verification API 创建对应 durable run。
- 不注入 prompt：结果只展示给用户；模型要读取文件仍需显式工具调用。
- session scoped：后端只根据 session 自己的 working dir / project workspace / persisted artifacts 聚合；Goal evidence 优先取 active goal，若没有 active goal 则回退到最近一个 session goal。
- incognito fail-closed：无痕会话返回空 snapshot，并标记 `disabledReason = "incognito"`。
- LSP symbol 是可选增强：没有语言服务或启动失败时只记录 warning，不影响其它候选。
- IDE context 是可选增强：请求内联 `ideContext` 优先，其次读取 `session_ide_context` 最近快照；没有信号时不影响 server / headless workflow。
- Domain context 是可选增强：显式 `domain/templateId/templateVersion` 优先，其次从 active Goal 绑定的 domain workflow template、`workflow_runs.kind = domain:<domain>`、`domain_evidence_items.domain`、Goal objective / criteria 轻量推断。没有 domain 信号时不影响 coding context。
- 无工作目录仍可召回 Goal / Task / Workflow / Domain evidence / URL 来源；只跳过 Git diff、file search、LSP 等 workspace 相关信号。

## 3. 候选模型

`ContextCandidate` 统一承载所有来源：

- `kind`: `file | symbol | diagnostic | review_finding | verification_step | goal_evidence | task | workflow_op | url_source | ide_context | document | email_thread | calendar_event | sheet_range | knowledge_note | web_source | decision | artifact`
- `title` / `subtitle`: 用户可扫读标题与补充信息。
- `path` / `line` / `url`: 可定位目标。
- `score`: 后端稳定排序分。
- `reasons`: 为什么推荐。
- `sources`: 贡献来源，如 `git`、`artifacts`、`lsp`、`review`、`verification`、`file_search`。
- `status`: severity / state / action 等短状态。
- `metadata`: 来源特有的结构化补充；可行动候选包含 `actions.focusPaths`、`actions.canReview`、`actions.canVerify`。
- Domain 候选的 `metadata.domainActions` 声明可引用、可摘要、可补 evidence、可请求用户确认、可标记冲突、可转 task 等后续 owner action 能力；GUI 已提供“复制引用”的真实轻量动作，其余动作先结构化暴露给后续执行入口。

文件类候选按 `file:<path>` 去重：Git diff、历史 artifact、file search 命中同一文件时合并 reasons/sources，并保留最高分来源的展示信息。
Domain 候选按 URL / path / evidence id 去重，不与 coding file 候选互相覆盖，避免把非代码来源伪装成代码文件。

## 4. 排序策略

排序不是纯字符串匹配，而是“任务信号基础分 + query boost”：

- Review open P0/P1、LSP error、失败验证 step 属于最高优先级。
- 失败 / blocked / awaiting 的 Workflow 信号，以及阻塞型 Goal evidence 会排在普通完成态证据之前。
- in-progress / pending task 高于 completed task。
- Git diff 文件高于普通历史读取文件。
- 最近修改高于最近读取。
- file search v2 和 LSP symbol 只在 query 非空时参与。
- query 不强制过滤既有高危信号，而是给标题、路径、状态、原因匹配项加权。
- IDE current file / selection / active diagnostic / active symbol 是高权重任务信号，优先帮助用户回到“现在正在看”的位置。
- open tabs 是中等权重信号，只作为上下文提示，不压过 P0/P1 finding、error diagnostic 或失败验证。
- Domain evidence 按 evidence type、是否 required、confidence、redaction status 和 query boost 排序；`user_decision`、`message_draft_approved`、`data_quality_checked`、`claim_checked` 这类闭环证据高于普通产物。
- Domain ranker 会用 domain workflow 的 required evidence 和 Goal criteria 给相关候选加小幅 boost，但不会隐藏 coding 高危信号。
- 缺少连接器或必需 evidence 时输出 `accessIssues[]`，例如 research 缺少 web source、meeting prep 缺少 calendar event、data analysis 缺少 sheet/data quality evidence。

这保证用户搜索 `parser` 时能看到相关文件/符号，同时不会因为搜索词不匹配而隐藏当前 diff 里的严重诊断或审查阻塞项。

## 5. API

Tauri：

```text
get_context_retrieval(sessionId, query?, limit?, ideContext?, domain?, templateId?, templateVersion?)
```

HTTP：

```text
GET /api/sessions/{sid}/context-retrieval?query=<q>&limit=<n>&domain=<domain>&templateId=<id>&templateVersion=<version>
GET /api/sessions/{sid}/ide-context
PUT /api/sessions/{sid}/ide-context
DELETE /api/sessions/{sid}/ide-context
```

Transport：

```text
get_context_retrieval
```

返回 `ContextRetrievalSnapshot`：

- `sessionId`
- `query`
- `workspaceRoot`
- `domainContext`：包含 `domain`、`templateId`、`templateVersion`、`templateTitle`、`taskType`、required evidence / approval gates / verification policy 与解析来源。
- `candidates`
- `stats`：包含 `gitChanges`、`artifactFiles`、`diagnostics`、`reviewFindings`、`verificationSteps`、`goalEvidence`、`tasks`、`workflowOps`、`ideContextSignals`、`fileSearchMatches`、`symbols`、`urlSources`、`domainCandidates`、`domainEvidence`、`accessIssues`、`warnings`
- `accessIssues`
- `truncated`
- `disabledReason`
- `generatedAt`

## 6. Workspace GUI

Workspace 面板新增“推荐上下文”区块，位置在“环境”之后、“语义诊断”之前。

用户交互：

- 默认展示当前 session 的推荐上下文。
- 输入关键词后 debounced 重新召回。
- 手动刷新按钮。
- 文件 / 诊断 / review / symbol 行可复用统一文件操作策略预览当前文件。
- IDE context 行展示“IDE”来源，Context 区块的信号计数包含 `ideContextSignals`。
- 带 `actions.focusPaths` 的候选行显示两个紧凑操作按钮：聚焦审查、聚焦验证。
- 聚焦审查调用 `run_code_review({ scope: "local", focusPaths })`，只在匹配文件范围内生成 finding。
- 聚焦验证调用 `run_smart_verification({ scope: "local", focusPaths })`，只基于匹配文件选择最小验证命令。
- URL 来源行用外部打开。
- Domain profile 以小条显示模板 / 识别来源；access issues 直接列出缺口和下一步动作。
- Domain 候选显示类型图标与动作 chips；可引用来源提供“复制引用”按钮。
- 没有工作目录时区块仍启用，只跳过文件搜索 / Git / LSP，保留 Goal / Task / Workflow / Domain evidence。
- 自动监听 `lsp:*`、`review:*`、`verification:*`、`workflow:*` 与 `_lagged` 事件刷新。

GUI 不另做文件操作分叉：路径行复用 `useFileActions`，继续遵守本机 / HTTP 的预览、打开、下载矩阵。

聚焦按钮不绕过原控制平面：Review / Verification run 仍写入各自 durable 表、Goal evidence、EventBus 与 Workspace 对应区块；Context Retrieval 只负责把“下一步最该处理哪条上下文”变成一键入口。

## 7. 性能与可靠性

- 默认返回 24 条，最大 50 条，保证 payload 有界。
- 历史 artifacts 只读摘要，不拉取 diff 大内容。
- file search v2 只有 query 非空才运行，继续受 walk cap 约束。
- LSP workspace symbols 只有 query 长度至少 2 时运行，且失败不阻断 snapshot。
- IDE context 最多纳入 current file、selection、active diagnostic、active symbol 和去重后的 open tabs；selection 文本会截断后进入候选标题 / metadata。
- Goal / Task / Workflow 只读 `sessions.db` 摘要，最多取最近少量 run/op/task/evidence，payload 有界。
- Domain evidence 只读 `domain_evidence_items`，默认最多 80 条；session artifacts 只读摘要并按 domain 文件类型筛选。
- Git diff / artifacts 走后台 blocking task，避免卡住 async runtime。
- Context Retrieval 不做持久化，刷新后可以从已有 durable 数据重建。

## 8. Session IDE Context

`session_ide_context` 是 session-scoped owner-plane 快照，字段包含：

- `source`：例如 `acp`、`ide`、`desktop`。
- `currentFile`
- `selection { path?, startLine?, endLine?, text? }`
- `openTabs[]`
- `activeDiagnostic { path?, line?, severity?, message? }`
- `activeSymbol { name?, kind?, path?, line? }`

写入入口：

- Tauri：`save_session_ide_context` / `get_session_ide_context` / `clear_session_ide_context`。
- HTTP：`GET|DELETE /api/sessions/{sid}/ide-context`，`PUT /api/sessions/{sid}/ide-context` body 为 `{ "context": SessionIdeContext }`。
- ACP：`newSession` / `loadSession` / `prompt` 请求的 `_meta.ideContext` 或 `_meta.ide_context` 会被 best-effort 持久化。

约束：

- incognito session 拒绝持久化 IDE context。
- 快照只用于推荐、focused review evidence 和 GUI 展示；不会提升为 system 指令。
- ACP 写入失败只记录 warning，不让 prompt 失败。

## 9. 后续增强

- 为 symbols 增加 document symbols fallback，避免 workspace symbol 服务不可用时完全缺席。
- 引入 over-read ratio 与趋势报告，补充现有 context precision / critical context recall。
- 将 `domainActions` 接到真实 owner action：从候选创建 evidence、生成摘要、发起 ask_user、标记冲突、转成 task。
- 接入真实连接器只读候选（Gmail / Calendar / Drive / Sheets）时必须继续走 access issue + 授权边界，不得伪造缺失来源。

已接入 Workflow / Eval：

- `workflow.review()` / `workflow.verify()` 已复用 focused owner API，workflow 内产生的 review finding、verification step 与 Goal evidence 会进入 Context Retrieval 候选。
- Phase 3.8 fixture 已覆盖 workflow-bound review / verification host API 产生的上下文召回。
- Phase 3.10 fixture 已覆盖 profile-specific review 与 IDE context recall。
- Phase 7.3 单测覆盖 domain evidence -> web source 候选、research domain profile 自动识别、缺失 required evidence 以 access issue 暴露。
