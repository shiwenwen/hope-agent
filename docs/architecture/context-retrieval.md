# Context Retrieval v2

> 返回 [技术文档索引](../README.md)
>
> 状态：Phase 3.6 已实现。Context Retrieval v2 是 Workspace 面板里的推荐上下文与行动入口：推荐本身只读，不创建新的 durable run；候选行上的聚焦审查 / 聚焦验证按钮会显式调用 Review / Verification owner API。

## 1. 目标

Context Retrieval v2 回答一个用户视角的问题：

> 当前任务下一步最该看哪些上下文？

它把分散在工作台里的信号聚合成一个有优先级的候选列表：

- 当前 Git diff 改动文件。
- 本会话历史读写过的文件与 URL 来源。
- LSP diagnostics。
- Review Engine findings。
- Smart Verification steps。
- 当前 Goal evidence。
- 当前 task 进度。
- 最近 Workflow run / op 状态。
- query 驱动的 file search v2 结果。
- query 驱动的 LSP workspace symbols。

首批优化 coding 场景，但能力本身是通用的 owner-plane 上下文推荐：Goal / Task / Workflow 信号已经证明它不只服务代码文件；后续还可以接入 knowledge source、IDE selection、ACP current file 等非编码或跨工具信号。

## 2. 架构边界

核心实现位于 `ha-core::context_retrieval`，入口是：

```rust
context_retrieval_for_session(db, session_id, ContextRetrievalInput { query, limit })
```

重要边界：

- 推荐查询是只读 owner API：不创建 run、不写 DB、不改变模型状态。
- 候选行动是显式 owner action：`metadata.actions.focusPaths` 只声明可操作目标，GUI 点击后调用 Review / Verification API 创建对应 durable run。
- 不注入 prompt：结果只展示给用户；模型要读取文件仍需显式工具调用。
- session scoped：后端只根据 session 自己的 working dir / project workspace / persisted artifacts 聚合。
- incognito fail-closed：无痕会话返回空 snapshot，并标记 `disabledReason = "incognito"`。
- LSP symbol 是可选增强：没有语言服务或启动失败时只记录 warning，不影响其它候选。

## 3. 候选模型

`ContextCandidate` 统一承载所有来源：

- `kind`: `file | symbol | diagnostic | review_finding | verification_step | goal_evidence | task | workflow_op | url_source`
- `title` / `subtitle`: 用户可扫读标题与补充信息。
- `path` / `line` / `url`: 可定位目标。
- `score`: 后端稳定排序分。
- `reasons`: 为什么推荐。
- `sources`: 贡献来源，如 `git`、`artifacts`、`lsp`、`review`、`verification`、`file_search`。
- `status`: severity / state / action 等短状态。
- `metadata`: 来源特有的结构化补充；可行动候选包含 `actions.focusPaths`、`actions.canReview`、`actions.canVerify`。

文件类候选按 `file:<path>` 去重：Git diff、历史 artifact、file search 命中同一文件时合并 reasons/sources，并保留最高分来源的展示信息。

## 4. 排序策略

排序不是纯字符串匹配，而是“任务信号基础分 + query boost”：

- Review open P0/P1、LSP error、失败验证 step 属于最高优先级。
- 失败 / blocked / awaiting 的 Workflow 信号，以及阻塞型 Goal evidence 会排在普通完成态证据之前。
- in-progress / pending task 高于 completed task。
- Git diff 文件高于普通历史读取文件。
- 最近修改高于最近读取。
- file search v2 和 LSP symbol 只在 query 非空时参与。
- query 不强制过滤既有高危信号，而是给标题、路径、状态、原因匹配项加权。

这保证用户搜索 `parser` 时能看到相关文件/符号，同时不会因为搜索词不匹配而隐藏当前 diff 里的严重诊断或审查阻塞项。

## 5. API

Tauri：

```text
get_context_retrieval(sessionId, query?, limit?)
```

HTTP：

```text
GET /api/sessions/{sid}/context-retrieval?query=<q>&limit=<n>
```

Transport：

```text
get_context_retrieval
```

返回 `ContextRetrievalSnapshot`：

- `sessionId`
- `query`
- `workspaceRoot`
- `candidates`
- `stats`：包含 `gitChanges`、`artifactFiles`、`diagnostics`、`reviewFindings`、`verificationSteps`、`goalEvidence`、`tasks`、`workflowOps`、`fileSearchMatches`、`symbols`、`urlSources`、`warnings`
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
- 带 `actions.focusPaths` 的候选行显示两个紧凑操作按钮：聚焦审查、聚焦验证。
- 聚焦审查调用 `run_code_review({ scope: "local", focusPaths })`，只在匹配文件范围内生成 finding。
- 聚焦验证调用 `run_smart_verification({ scope: "local", focusPaths })`，只基于匹配文件选择最小验证命令。
- URL 来源行用外部打开。
- 自动监听 `lsp:*`、`review:*`、`verification:*`、`workflow:*` 与 `_lagged` 事件刷新。

GUI 不另做文件操作分叉：路径行复用 `useFileActions`，继续遵守本机 / HTTP 的预览、打开、下载矩阵。

聚焦按钮不绕过原控制平面：Review / Verification run 仍写入各自 durable 表、Goal evidence、EventBus 与 Workspace 对应区块；Context Retrieval 只负责把“下一步最该处理哪条上下文”变成一键入口。

## 7. 性能与可靠性

- 默认返回 24 条，最大 50 条，保证 payload 有界。
- 历史 artifacts 只读摘要，不拉取 diff 大内容。
- file search v2 只有 query 非空才运行，继续受 walk cap 约束。
- LSP workspace symbols 只有 query 长度至少 2 时运行，且失败不阻断 snapshot。
- Goal / Task / Workflow 只读 `sessions.db` 摘要，最多取最近少量 run/op/task/evidence，payload 有界。
- Git diff / artifacts 走后台 blocking task，避免卡住 async runtime。
- Context Retrieval 不做持久化，刷新后可以从已有 durable 数据重建。

## 8. 后续增强

- 为 symbols 增加 document symbols fallback，避免 workspace symbol 服务不可用时完全缺席。
- 接入 IDE selection envelope / ACP current file signal，让“当前正在看的文件”成为高权重推荐来源。
- 让 workflow script runtime 暴露 `workflow.review()` / `workflow.verify()` host API，复用同一 focused owner API。
- 引入 eval 指标：context precision、critical context recall、over-read ratio。
