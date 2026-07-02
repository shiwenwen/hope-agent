# Review Engine 控制平面

> 返回 [文档索引](../README.md)
>
> 状态：Phase 3.3 已实现。本文是 `ha-core::review`、`/review`、Review owner API、Goal evidence 与 Workspace 代码审查区块的单一技术事实源。

## 1. 目标

Review Engine 把“请检查我未提交的更改”从一次普通提示词升级为可审计、可恢复、可交互的控制平面能力：

- 读取当前 session 工作目录的 git working-tree diff。
- 把 LSP diagnostics 作为强证据候选。
- 生成 candidate findings。
- 经 verifier 三态收口：`confirmed` / `plausible` / `refuted`。
- 持久化 review run、findings、events。
- 在 Workspace GUI 中展示、刷新、重新审查和标记 finding 状态。
- 将 P0/P1 未解决 review finding 写回 Goal evidence，阻止过早完成。

第一版 intentionally 不依赖外部 LLM 可用性：本地 deterministic reviewer 先保证长任务稳定性和 GUI/Goal/Workflow 的数据闭环。后续 LLM reviewer、parallel reviewer 和 review profiles 会复用同一 run/finding schema。

## 2. 范围

实现范围：

- Core：`crates/ha-core/src/review.rs`。
- DB：`review_runs` / `review_findings` / `review_events` 三表，落在 `sessions.db`。
- Slash：`/review`、`/review status`、`/review resolved|dismissed|false_positive|open <finding>`。
- Owner API：
  - Tauri：`list_review_runs` / `get_review_run` / `run_code_review` / `update_review_finding_status`。
  - HTTP：`GET|POST /api/sessions/{sid}/review-runs`、`GET /api/review-runs/{id}`、`POST /api/review-findings/{id}/status`。
- GUI：Workspace 面板“代码审查”区块。
- Goal：`review_passed` / `review_completed` / `review_finding` evidence。

非目标：

- 不在 Phase 3.3 内实现远程 PR review。
- 不在 Phase 3.3 内自动修改代码；修复仍通过普通 agent/workflow 执行。
- 不把本地 deterministic reviewer 包装成完整安全扫描器。
- 不在无痕会话持久化 review run。

## 3. 数据模型

```text
review_runs
  id              rev_<uuid>
  session_id      owning session
  scope           local
  state           running | completed | failed | cancelled
  base_ref        reserved for future branch/range review; Phase 3.3 rejects non-empty values
  goal_id         open goal at creation time, if any
  summary         reader-facing summary
  stats_json      files/findings/P0-P3/verdict counts
  error
  timestamps

review_findings
  id              revf_<uuid>
  run_id
  session_id
  file_path
  start_line/end_line
  title/body
  category        correctness | security | maintainability | tests | ...
  severity        p0 | p1 | p2 | p3
  verdict         confirmed | plausible | refuted
  status          open | resolved | dismissed | false_positive
  evidence_json   source-specific evidence + verifier/confidence
  timestamps

review_events
  seq             per-run monotonic sequence
  kind            review_started | review_completed | review_failed | finding_created | finding_status_changed
  payload_json    bounded to 64 KiB
```

生命周期跟随 session：session 删除会 cascade 删除 review runs/findings/events。

单次 run 最多持久化 100 条 finding；`stats.findings` 与实际持久化 finding 数一致，`stats.candidateTotal` / `stats.truncatedFindings` 记录被上限裁掉的候选数量。

## 4. Review 流程

```text
run_code_review
  -> create review_run(running)
  -> load_session_git_diff(session)
  -> read cached LSP diagnostics
  -> build changed-line map
  -> generate candidate findings
  -> deterministic verifier
  -> persist findings
  -> complete review_run
  -> link Goal evidence
  -> emit review:* events
```

Diff 来源复用 `session::load_session_git_diff`，因此：

- HTTP client 不能传任意路径。
- review 只能读 session workspace scope。
- 只支持 `scope=local` 的未提交改动审查；非空 `baseRef` / `profiles` 会被拒绝，避免字段被误认为已经生效。
- `focusPaths[]` 是 local scope 的可选收窄条件：后端先读取同一份 session diff / LSP diagnostics，再只保留匹配路径的 changed files 和 diagnostics，stats 中记录 `focused=true` 与 `focusPaths`。它不允许客户端传任意 workspace 外路径。
- 输出 shape 与工作台/右侧 DiffPanel 的文件变更模型一致。

## 5. Candidate 来源

第一版 candidate 来源是确定性的：

| 来源 | 条件 | severity | verdict |
| --- | --- | --- | --- |
| LSP diagnostic | diagnostic 落在 changed line 上 | error→P1，warning→P2，其余 P3 | error confirmed，其余 plausible |
| Conflict marker | changed line 包含 `<<<<<<<` / `=======` / `>>>>>>>` | P1 | confirmed |
| Possible secret | changed line 疑似 private key / API key / AWS key | P1 | confirmed |
| Debug output | 非 test/spec changed line 新增 `console.log` / `debugger` / `dbg!` / `println!` / `print` | P2 | plausible |
| No test update | source file 变化但 diff 无 test/spec 文件 | P3 | plausible |
| Truncated diff | 文件超出 inline review cap | P3 | plausible |

verifier 不是“再问同一个模型自审”，而是独立纯函数 `verify_candidate()`：

- 明确证据如 conflict marker、secret、P1 LSP error → `confirmed`。
- 需要语境判断的项 → `plausible`。
- 低置信候选可进入 `refuted`，并默认非 open。

## 6. Goal Evidence

Review run 创建时会绑定当前 open goal（或显式 `goalId`）。完成后：

- 无 P0/P1 open finding：写 `review_passed` evidence。
- 有阻塞 finding：写 `review_completed` evidence，并为每条 P0/P1 open finding 写 `review_finding` evidence。
- finding 状态变更时，Goal link metadata 会更新 `status`，并重新汇总 run 级 `review_passed` / `review_completed` evidence。

Goal evaluator 已把 P0/P1 unresolved `review_finding` 视为 blocker；`resolved` / `dismissed` / `false_positive` 不再阻止完成。

## 7. Slash 命令

| 命令 | 行为 |
| --- | --- |
| `/review` | 运行 local review，返回摘要和 open findings |
| `/review run` | 同 `/review` |
| `/review status` | 列出最近 review runs |
| `/review status <id>` | 展示指定 run 的 findings |
| `/review resolved <finding>` | 标记 finding 已修复 |
| `/review dismissed <finding>` | 标记 finding 已忽略 |
| `/review false_positive <finding>` | 标记 finding 为误报 |
| `/review open <finding>` / `/review reopen <finding>` | 重新打开 finding |

slash 输出是普通 Markdown fallback；GUI 使用 owner API 展示结构化卡片。

## 8. GUI

Workspace 面板新增“代码审查”区块，位于“语义诊断”之后、“进度/Workflow”之前。

显示内容：

- 最新 review run 摘要与短 id。
- P0/P1/P2/P3 计数。
- open findings 数与阻塞状态 pill。
- 最多 6 条 open findings：severity、verdict、category、文件位置、body。
- 操作：重新审查、刷新、标记已修复、忽略、误报。
- “推荐上下文”候选行可触发 focused review；生成的 run 仍出现在本区块并写入同一套 events / Goal evidence。

刷新触发：

- 首次打开。
- 当前 turn 从 active 变 idle。
- EventBus `review:created` / `review:updated` / `review:finding_updated` / `_lagged`。
- active review run 存在时低频轮询。

## 9. EventBus

| 事件 | Payload |
| --- | --- |
| `review:created` | `ReviewRun` |
| `review:updated` | `ReviewRun` |
| `review:finding_updated` | `ReviewFinding` |
| `review:event` | `ReviewEvent` |

事件只作为刷新信号；完整快照仍从 owner API 读取，避免事件丢失导致 UI 状态不完整。

## 10. 安全与隐私

- Incognito session 拒绝创建 durable review run。
- HTTP 路径不接受 arbitrary path；只按 session id 解析 workspace。
- Review Engine 只读文件/diff/LSP diagnostics，不执行项目代码。
- finding evidence 中疑似 secret 行会脱敏，不把完整 token 写进 DB。
- review finding 是用户可处理的证据，不自动修改代码、不自动提交。

## 11. 后续增强

- LLM reviewer：在 deterministic candidates 之后追加 model-generated candidates。
- Parallel profiles：correctness / security / concurrency / frontend / accessibility / tests。
- Review verifier v2：独立 reviewer agent 三态确认，带 evidence quote 和反证。
- Workflow host API：`workflow.review()`，在 validate 后自动进入 review。
- Inline comment handoff：PR/代码编辑器侧的可定位评论导出。
- Re-review：finding resolved 后自动对相关 hunk 做 focused review，并复用现有 `focusPaths` 输入。
