# Managed Worktree 控制平面

> 返回 [文档索引](../README.md) | 更新时间：2026-07-04

Managed Worktree 是 Hope Agent 的 durable 隔离执行环境。它不是普通 `git worktree` 命令的薄包装，而是一个带持久状态、owner API、GUI 控制、Workflow 绑定、Subagent 隔离和 Hook 扩展点的控制平面。

## 定位

```text
session working dir
  -> managed_worktrees row
  -> ~/.hope-agent/worktrees/<repo-slug>/<worktree-id>/
  -> workflow/subagent execution cwd
  -> restore / archive / handoff
```

目标：

- 长任务、workflow、subagent 可以在隔离目录里修改代码，避免污染主工作区。
- 刷新、重启、恢复时仍能知道 worktree 属于哪个 session / workflow / child session。
- 用户可以在 Workspace 环境面板里创建、恢复、归档和交接 worktree。
- Hook 可以接管 WorktreeCreate，用于自定义 git / 非 git VCS / 企业初始化脚本。

非目标：

- 不替代 Git 分支管理、commit、push、PR。
- 不给模型暴露任意“切换主会话 cwd”的 agent 工具。当前是 owner 平面能力。
- 不在无痕会话里创建 durable worktree。

## 模块

| 层 | 代码 | 责任 |
| --- | --- | --- |
| 核心控制面 | `crates/ha-core/src/worktree.rs` | 表结构、创建、归档、恢复、交接、`.worktreeinclude` 复制、EventBus。 |
| 路径 | `crates/ha-core/src/paths.rs` | `worktrees_dir()` 返回 `~/.hope-agent/worktrees`。 |
| Hooks | `crates/ha-core/src/hooks/*` | `WorktreeCreate` 阻断/替换默认创建；`WorktreeRemove` 观察清理。 |
| Workflow | `crates/ha-core/src/workflow/{types,db,runtime}.rs` | `workflow_runs.worktree_id`，运行时自动 restore 并覆盖 execution cwd。 |
| Goal | `crates/ha-core/src/goal/mod.rs` | 绑定 workflow 后写 `worktree_attached` evidence；生命周期变化刷新 worktree state / path / handoff / dirty snapshot。 |
| Subagent | `crates/ha-core/src/subagent/*` | 用户委派的 subagent 默认尝试创建 managed worktree 并设置 child session cwd。 |
| Tauri | `src-tauri/src/commands/worktree.rs` | 桌面 owner 命令。 |
| HTTP | `crates/ha-server/src/routes/worktree.rs` | Server/Web owner REST API。 |
| GUI | `src/components/chat/workspace/WorkspacePanel.tsx` | 环境面板 managed worktree 列表与 workflow 创建运行位置选择。 |

## 数据模型

`managed_worktrees` 落在 `sessions.db`。

| 字段 | 说明 |
| --- | --- |
| `id` | `wt_*` id。 |
| `session_id` | 拥有者会话。 |
| `child_session_id` | 可选；subagent child session。 |
| `workflow_run_id` | 可选；workflow 反向绑定。`create_workflow_run(worktreeId)` 会在该字段为空时回填 run id。 |
| `purpose` | `manual` / `workflow` / `subagent`。 |
| `state` | `active` / `archived` / `handoff`。 |
| `label` | 展示标签，不作为身份。 |
| `repo_root` | 源仓库根目录。 |
| `source_working_dir` | 创建时的源 cwd。 |
| `path` | managed worktree 绝对路径。 |
| `base_ref` / `base_branch` / `base_sha` | 创建基线。 |
| `git_branch` | worktree 当前分支；默认 detached。 |
| `dirty_snapshot_json` | 归档时的变更快照。 |
| `created_at` / `updated_at` / `archived_at` / `restored_at` / `handed_off_at` | 生命周期时间。 |

## 生命周期

### 创建

1. 校验 session 存在且非 incognito。
2. 解析 session effective working directory 或显式 `sourceWorkingDir`。
3. 要求源目录位于 git worktree 中。
4. 生成 `wt_*` id 和 `~/.hope-agent/worktrees/<repo-slug>/<wt-id>` 路径。
5. 若存在匹配的 `WorktreeCreate` hook，执行 hook；hook 可 block/deny，或返回 `hookSpecificOutput.worktreePath` 接管创建。
6. 无 hook 时执行 `git worktree add --detach <path> <base_sha>`。
7. 复制 `.worktreeinclude` 中 git-ignored 文件，以及 `AGENTS.override.md`。
8. 写 `managed_worktrees` 行并 emit `worktree:created`。

### 恢复

`restore_managed_worktree` 在 path 不存在时用 `base_sha` 重新 `git worktree add --detach` 并重新复制 `.worktreeinclude`。Workflow runtime 发现绑定 worktree 已归档或路径缺失时，会先自动 restore；失败则把 run 标记为 `blocked(worktree_unavailable)`，禁止悄悄回退到父目录执行。

### 归档

`archive_managed_worktree` 会先记录 dirty snapshot。仅当 worktree clean 且非 handoff 状态时，才 best-effort `git worktree remove` 并 fire `WorktreeRemove`。有本地变更时保留目录，只更新状态和快照。

### 交接

`handoff_managed_worktree` 把父 session 的 `working_dir` 切到 worktree path，并标记 `handoff`。这是用户明确 owner 操作，会触发既有 `CwdChanged` hook。

## Workflow 集成

`CreateWorkflowRunInput.worktree_id` 可选。创建时校验：

- worktree 存在；
- 属于同一 session；
- 状态为 `active` 或 `handoff`。
- `workflow_runs.worktree_id` 是执行期真相源；`managed_worktrees.workflow_run_id` 是 GUI / 审计 / 清理用反向索引。创建 run 时若反向索引为空，会回填 run id 并 emit `worktree:updated`；若已绑定其它 run，不覆盖。

runtime 构造 `WorkflowSessionContext` 时，如果 run 绑定 `worktree_id`：

- 读取 managed worktree；
- archived / path missing 时自动 restore；
- 将 `session_context.working_dir` 覆盖为 worktree path；
- 追加 `run_worktree_attached` trace event。

因此 `workflow.fileSearch` / `workflow.read` / `workflow.grep` / `workflow.tool` / `workflow.validate` / `workflow.diff` 都使用绑定 worktree 作为默认 cwd。

## Goal Evidence 集成

绑定 Goal 的 workflow run 如果带 `worktree_id`，创建后会写一条 `goal_links(target_type='worktree', relation='worktree_attached')`。这条 evidence 是执行环境证据，记录：

- `worktreeId`、`runId`、`reverseWorkflowRunId`。
- `state`、`purpose`、`label`、`path`、`pathExists`。
- `repoRoot`、`sourceWorkingDir`、`baseRef`、`baseBranch`、`baseSha`、`gitBranch`。
- `dirtySnapshot`、`archivedAt`、`restoredAt`、`handedOffAt`。

`create_managed_worktree`、`link_managed_worktree_to_workflow_run`、`archive_managed_worktree`、`restore_managed_worktree`、`handoff_managed_worktree` 都会 best-effort 刷新这条 evidence。刷新失败只写 `app_warn`，不让 Worktree 生命周期操作失败。

语义边界：

- `worktree_attached` 是 positive contextual evidence，让 Goal detail、timeline 和模型下一轮 prompt 能看见改动落点与交接状态。
- 它不是 strong completion evidence，不能单独让 Goal completed。
- archived / missing path 不在 Goal evaluator 里一概判 blocker；真正执行时仍由 Workflow runtime 对不可用 worktree fail closed / block。

GUI 上有两层展示：

- Workspace Environment 面板展示当前 session 相关 managed worktrees，可创建、恢复、交接、归档。
- Goal detail 的 Worktrees 区块只展示 `worktree_attached` evidence，服务目标审计：state、path、base、dirty snapshot、handoff / run 关联一眼可见。

## Subagent 集成

`SpawnParams.isolate_worktree` 控制 child session 是否尝试创建 managed worktree。

- 用户可见 `subagent` / `batch_spawn` 工具默认 `true`。
- 内部 plan / team / hook / skill fork 当前保持 `false`，避免内部 helper 默认制造大量 worktree。
- 创建成功后 child session `working_dir` 指向 worktree path，并注入一段额外 system context。
- 创建失败时继承父 session effective working directory 并 `app_warn!`，不直接阻断 subagent。

## GUI 交互

Workspace 环境面板展示最近 managed worktrees：

- 状态：Active / Archived / Handoff。
- 类型：Manual / Workflow / Subagent。
- dirty summary：clean、变更数量、路径已清理或 base ref。
- 操作：创建、恢复、交接、归档。

Workflow 创建面板有“运行位置”选择：

- 当前目录；
- 新隔离工作树；
- 已有 active/handoff managed worktree。

默认仍是当前目录；用户显式选择隔离 worktree 后才创建或绑定。

## Hooks

`WorktreeCreate` 是阻断型事件。匹配后必须返回：

```json
{
  "hookSpecificOutput": {
    "worktreePath": "/absolute/path/to/worktree"
  }
}
```

如果 hook 返回 `block` / `deny`，创建失败。若没有任何 handler 或 name 不匹配，走内建 git 创建。

`WorktreeRemove` 当前是观察事件，在内建 clean remove 成功后 fire，payload 包含 `worktree_path`。

## 红线

- 所有 durable worktree 创建必须经过 `SessionDB::create_managed_worktree`。
- incognito session 禁止创建 managed worktree。
- label 只展示，身份必须使用 `wt_*` id。
- Workflow 绑定 worktree 不参与 `script_hash`。
- Workflow 绑定 worktree 不可用时必须 fail closed/block，不能静默改用父目录。
- Worktree 的 Goal evidence 只能描述执行环境与交接状态，不能替代 validation / review / workflow completion。
- `.worktreeinclude` 只复制 git ignored 文件；跳过 symlink，不覆盖 git 语义。
