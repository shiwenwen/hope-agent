# Smart Verification 控制平面

> 返回 [文档索引](../README.md)
>
> 状态：Phase 3.4 已实现。本文是 `ha-core::verification`、Smart Verification owner API、Goal evidence 与 Workspace 验证区块的单一技术事实源。

## 1. 目标

Smart Verification 把“现在应该跑什么检查？”从人工经验升级为 durable 控制平面能力：

- 基于当前 session workspace 的 git diff 选择最小相关检查。
- 读取 `AGENTS.md` / `CLAUDE.md` 中的项目规则提示。
- 自动运行低风险、单点、项目规则允许的命令。
- 把全量 lint/test 等重检查作为 gated suggestion 展示，不偷偷执行。
- 持久化 verification run、steps、events。
- 将验证结果写回 Goal evidence，成为 final audit 的强证据或 blocker。
- 在 Workspace GUI 中展示推荐、运行状态、通过/失败/门控统计和失败输出摘要。

第一版刻意保持 deterministic selector，不依赖外部 LLM。后续 LLM/历史成功率/测试影响分析可以替换 selector，但不改变 run/step/event 与 GUI/Goal 的契约。

## 2. 范围

实现范围：

- Core：`crates/ha-core/src/verification.rs`。
- DB：`verification_runs` / `verification_steps` / `verification_events` 三表，落在 `sessions.db`。
- Owner API：
  - Tauri：`list_verification_runs` / `get_verification_run` / `plan_smart_verification` / `run_smart_verification`。
  - HTTP：`GET /api/sessions/{sid}/verification-runs`、`POST /api/sessions/{sid}/verification-runs/plan`、`POST /api/sessions/{sid}/verification-runs/run`、`GET /api/verification-runs/{id}`。
- GUI：Workspace 面板“验证”区块。
- Goal：`validation_passed` / `validation_failed` / `validation_completed` evidence。

非目标：

- 不在 Phase 3.4 内自动运行项目全量 pre-push suite。
- 不支持用户输入任意 shell 命令；selector 只生成内置白名单形态。
- 不在无痕会话持久化 verification run。
- 不替代 `workflow.validate`；workflow 显式验证仍按 workflow op 写 evidence。

## 3. 数据模型

```text
verification_runs
  id              ver_<uuid>
  session_id      owning session
  scope           local
  state           planned | running | completed | failed | cancelled
  goal_id         open goal at creation time, if any
  summary         reader-facing summary
  stats_json      total/runnable/gated/passed/failed/skipped/commands
  error
  timestamps

verification_steps
  id              vers_<uuid>
  run_id
  session_id
  seq
  command         generated command string
  cwd             repo root or session workspace root
  title/reason
  category        rust | frontend | i18n | sanity | policy
  risk            low | medium | high
  auto_run        low-risk steps only
  state           pending | running | passed | failed | skipped | timed_out
  exit_code
  output_preview  bounded stdout+stderr preview
  duration_ms
  timestamps

verification_events
  seq             per-run monotonic sequence
  kind            verification_created | verification_planned | verification_completed | verification_failed | step_selected | step_started | step_completed
  payload_json    bounded to 64 KiB
```

生命周期跟随 session：session 删除会 cascade 删除 verification runs/steps/events。

## 4. Selector

Selector 输入：

- 当前 session 工作目录。
- `session::load_session_git_diff()` 的 working-tree diff。
- git repo root。
- `AGENTS.md` / `CLAUDE.md` 中的项目规则提示。
- 可选 `focusPaths[]`：在同一 session diff 内收窄 changed files 后再进入 selector。

当前规则：

| 改动面 | 推荐 |
| --- | --- |
| Rust crate source | `cargo check -p <crate> --locked` |
| Rust test file | `cargo check -p <crate> --tests --locked` |
| TypeScript / React / package surface | `pnpm typecheck` |
| i18n locale 或 sync 脚本 | `node scripts/sync-i18n.mjs --check` |
| API / transport surface 或纯文档 | `git diff --check` |
| Cargo workspace manifest | gated `cargo check --workspace --locked` |
| 项目规则提到 pre-push/full suite | gated `pnpm lint && pnpm test` |

`auto_run=true` 只给低风险单点检查。高风险或重检查保存在 step 中并标记 `skipped`，用户可以看到建议，但不会被 Smart Verification 自动执行。

`focusPaths[]` 不改变 `scope=local` 的安全边界：它只过滤已经由 session workspace 解析出的 diff 文件，不能让 HTTP/Tauri 客户端指定任意 cwd 或任意 shell 命令。stats 会记录 `focused=true` 与规范化后的 `focusPaths`。

单次 run 最多选择 8 条 step，避免 Workspace 面板和后台执行被一次大 diff 拖垮。

## 5. 执行模型

`run_smart_verification` 的语义是“创建 durable run 并启动后台执行”：

```text
run_smart_verification
  -> create verification_run(running)
  -> select steps
  -> persist verification_steps
  -> return running snapshot
  -> tokio background task executes auto_run steps
  -> skipped gated steps
  -> complete/fail run
  -> link Goal evidence
  -> emit verification:* events
```

命令执行：

- 通过 `platform::default_shell_command_tokio()` 统一 shell 行为。
- 注入 `tools::exec::login_shell_env()`，保证桌面 GUI 能找到 `cargo` / `pnpm` / `node`。
- 每条 auto-run step 默认 120s 超时，`git diff --check` 为 30s。
- stdout + stderr 合并后只持久化 32 KiB preview。
- step 失败或超时会使 run 进入 `failed`。

重启恢复：

- `SessionDB` 初始化时把遗留 `running` verification run fail-closed 标记为 interrupted。
- 已完成 step 和事件仍保留，方便用户知道中断前跑到了哪里。

## 6. Goal Evidence

run 创建时绑定当前 open goal（或显式 `goalId`）。终态后：

- 存在 failed / timed_out step：写 `validation_failed`。
- 存在 passed step 且无失败：写 `validation_passed`。
- 没有可运行命令、只有计划/门控建议：写 `validation_completed`。

`validation_passed` 是 strong positive evidence，可帮助 Goal final audit 完成；`validation_failed` 是 blocker，只能被更新的 `validation_passed` 覆盖；`validation_completed` 只记录验证选择已完成，不单独完成 Goal。

## 7. Owner API 与 GUI

Owner API 不接受任意 cwd/path，只按 session id 解析 workspace。HTTP 与 Tauri 对齐；`plan_smart_verification` / `run_smart_verification` 可接受可选 `focusPaths[]` 作为 selector 输入。

| 能力 | Tauri | HTTP |
| --- | --- | --- |
| 列 run | `list_verification_runs` | `GET /api/sessions/{sid}/verification-runs` |
| 详情 | `get_verification_run` | `GET /api/verification-runs/{id}` |
| 只推荐 | `plan_smart_verification` | `POST /api/sessions/{sid}/verification-runs/plan` |
| 推荐并运行 | `run_smart_verification` | `POST /api/sessions/{sid}/verification-runs/run` |

Workspace 面板“验证”区块显示：

- 最新 run 摘要与短 id。
- 可跑 / 通过 / 失败 / 门控统计。
- 推荐验证、运行推荐、刷新。
- 最多 6 条 step：命令、原因、风险、状态、exit code、耗时。
- 失败/超时 step 的输出摘要。
- “推荐上下文”候选行可触发 focused verification；生成的 run 仍出现在本区块，并继续走后台低风险执行、Goal evidence 与 EventBus。

刷新触发：

- 首次打开。
- 当前 turn 从 active 变 idle。
- EventBus `verification:created` / `verification:updated` / `verification:step_updated` / `verification:event` / `_lagged`。
- active verification run 存在时低频轮询。

## 8. EventBus

| 事件 | Payload |
| --- | --- |
| `verification:created` | `VerificationRun` |
| `verification:updated` | `VerificationRun` |
| `verification:step_updated` | `VerificationStep` |
| `verification:event` | `VerificationEvent` |

事件只作为刷新信号；完整快照仍从 owner API 读取。

## 9. 安全与隐私

- Incognito session 拒绝创建 durable verification run。
- HTTP 不能传任意 cwd/path；workspace 由 session scope 决定。
- 命令不是用户输入的 arbitrary shell，而是 selector 生成的白名单模式。
- 高风险/重命令默认 `auto_run=false`。
- 输出只保留 bounded preview，不把超长日志写进上下文或 UI。

## 10. 后续增强

- 基于历史 run 成功率和耗时做排序。
- 根据 changed symbol / test ownership 做更细粒度 test impact。
- GUI 支持用户批准单条 gated step 后运行。
- 与 Review Engine 组合成“修复后 focused review + focused verification”自动闭环，并复用现有 `focusPaths` 输入。

已接入 Workflow：

- `workflow.verify({ focusPaths?, maxCommands? })` 复用同一 Smart Verification selector，在 workflow 内创建 durable verification plan。
- 该 host API 只规划不执行命令；需要实际跑命令时继续使用 `workflow.validate()` 或 owner 面板运行 verification step。
- workflow 绑定 Goal 时，计划完成会写入 `validation_completed` evidence，表示验证路径已生成，不等同于 `validation_passed`。
