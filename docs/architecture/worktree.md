# Managed Worktree 控制平面

> 返回 [文档索引](../README.md) | 更新时间：2026-07-12

Managed Worktree 是 Hope Agent 的 durable 隔离执行环境。它不是普通 `git worktree` 命令的薄包装，而是一个带持久状态、owner API、GUI 控制、项目首轮 Bootstrap、Workflow 绑定、Subagent 隔离和 Hook 扩展点的控制平面。Session 内的 Diff、分支、commit、push、Pull Request 和 Local/Worktree 双向安全迁移由独立的 [Session Git 控制平面](git-control.md) 负责。

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

- 不直接实现 Diff、Git 分支管理、commit、push、PR；这些操作由 `git_control` 复用本模块的 Worktree 身份和生命周期记录。
- 不给模型暴露任意“切换主会话 cwd”的 agent 工具。当前是 owner 平面能力。
- 不在无痕会话里创建 durable worktree。

## 模块

| 层 | 代码 | 责任 |
| --- | --- | --- |
| 核心控制面 | `crates/ha-core/src/worktree.rs` | 表结构、创建、归档、恢复、交接、`.worktreeinclude` 复制、EventBus。 |
| 项目首轮 Bootstrap | `crates/ha-core/src/project_bootstrap.rs` | 首次发送前校验、幂等记录、进度事件、取消与启动恢复。 |
| 路径 | `crates/ha-core/src/paths.rs` | `worktrees_dir()` 返回 `~/.hope-agent/worktrees`。 |
| Hooks | `crates/ha-core/src/hooks/*` | `WorktreeCreate` 阻断/替换默认创建；`WorktreeRemove` 观察清理。 |
| Workflow | `crates/ha-core/src/workflow/{types,db,runtime}.rs` | `workflow_runs.worktree_id`，运行时自动 restore 并覆盖 execution cwd。 |
| Goal | `crates/ha-core/src/goal/mod.rs` | 绑定 workflow 后写 `worktree_attached` evidence；生命周期变化刷新 worktree state / path / handoff / dirty snapshot。 |
| Subagent | `crates/ha-core/src/subagent/*` | 用户委派的 subagent 默认尝试创建 managed worktree 并设置 child session cwd。 |
| Tauri | `src-tauri/src/commands/worktree.rs` | 桌面 owner 命令。 |
| HTTP | `crates/ha-server/src/routes/worktree.rs` | Server/Web owner REST API。 |
| Session Git | `crates/ha-core/src/git_control.rs` | Local/Worktree 安全 Handoff、分支 ownership、Session active location；完整契约见 [`git-control.md`](git-control.md)。 |
| GUI | `src/components/chat/workspace/WorkspacePanel.tsx`、`GitControlCard.tsx` | Managed Worktree 列表、项目/Workflow 运行位置选择和 Session Git 入口。 |

## 数据模型

`managed_worktrees` 落在 `sessions.db`。

| 字段 | 说明 |
| --- | --- |
| `id` | `wt_*` id。 |
| `session_id` | 拥有者会话。 |
| `child_session_id` | 可选；subagent child session。 |
| `workflow_run_id` | 可选；workflow 反向绑定。`create_workflow_run(worktreeId)` 会在该字段为空时回填 run id。 |
| `purpose` | `manual` / `workflow` / `subagent`。 |
| `state` | `active` / `archived` / `handoff`；Hook 自定义路径启动清理失败时为 `bootstrap_failed`。 |
| `label` | 展示标签，不作为身份。 |
| `repo_root` | 源仓库根目录。 |
| `source_working_dir` | 创建时的源 cwd。 |
| `path` | managed worktree 绝对路径。 |
| `path_source` | `builtin` / `hook`；清理策略不能靠路径猜测。 |
| `base_ref` / `base_branch` / `base_sha` | 创建基线。 |
| `git_branch` | worktree 当前分支；默认 detached。 |
| `dirty_snapshot_json` | 归档时的变更快照。 |
| `created_at` / `updated_at` / `archived_at` / `restored_at` / `handed_off_at` | 生命周期时间。 |

## 生命周期

### 统一磁盘布局

内建 Managed Worktree 固定放在 Hope Agent 数据目录，不创建在项目相邻目录：

```text
~/.hope-agent/worktrees/<repo-slug>/<wt-id>/
```

- `repo-slug` 由 canonical repo root 派生，只用于目录分组，不作为仓库身份。
- `wt-id` 使用 `wt_<uuid>`，路径不包含分支名，避免 rename 和特殊字符影响生命周期。
- `path_source=builtin` 才允许 Hope Agent 在失败清理中对统一目录执行受控删除。
- Hook 返回的自定义路径记录为 `path_source=hook`；清理只执行 Git-aware remove，禁止对任意路径递归删除。

项目首轮未提交改动的临时快照固定放在：

```text
~/.hope-agent/bootstrap/<request-id>/
├── tracked.patch
├── untracked.manifest
└── metadata.json
```

Session Handoff 的临时目录是 `~/.hope-agent/git-operations/<request-id>/`，不与 Bootstrap 混用，详见 [Session Git 控制平面](git-control.md#8-local--managed-worktree-安全-handoff)。

### 创建

1. 校验 session 存在且非 incognito。
2. 解析 session effective working directory 或显式 `sourceWorkingDir`。
3. 要求源目录位于 git worktree 中。
4. 生成 `wt_*` id 和 `~/.hope-agent/worktrees/<repo-slug>/<wt-id>` 路径。
5. 若存在匹配的 `WorktreeCreate` hook，执行 hook；hook 可 block/deny，或返回 `hookSpecificOutput.worktreePath` 接管创建。
6. 无 hook 时执行 `git worktree add --detach <path> <base_sha>`。
7. 复制 `.worktreeinclude` 中 git-ignored 文件，以及 `AGENTS.override.md`。
8. 写 `managed_worktrees` 行并 emit `worktree:created`。

### 项目首轮 Bootstrap

项目草稿首条消息可携带 `ProjectSessionBootstrapInput`。分支选择与运行位置正交：`local` 和 `worktree` 都可选择后端 Git 信息接口返回的 `refs/heads/*` 或 `refs/remotes/*`。`local` 选择当前分支时保留现有未提交改动；选择其他本地分支时仅在工作区干净的情况下执行 `git switch`；选择 remote-tracking 分支时仅在工作区干净的情况下创建本地 tracking branch。不会自动 stash、reset 或丢弃改动。`worktree` 将 ref 解析为固定 SHA 后创建 detached managed worktree，并在进入 Chat Engine 前把临时 session 的 `working_dir` 绑定到该路径。初始化绑定保持 `active`，不标记为后续用户动作 `handoff`。

HTTP `/api/chat` 携带 `projectBootstrap` 时属于 Git 写操作，在创建临时 Session 前必须通过 `filesystem.allow_remote_writes=true` 闸门；默认配置返回 403。桌面 Tauri 不受该 HTTP 远程写闸门影响。

前端草稿状态：

```ts
interface ProjectRuntimeDraft {
  launchMode: "local" | "worktree"
  baseRef: string | null
  baseRefKind: "local" | "remote" | null
  includeLocalChanges: boolean
}
```

- 新项目草稿默认 `local`，Git 项目在两种 launch mode 下都显示分支选择。
- 默认当前本地分支；detached HEAD 时依次回退 `main`、`master`、第一个本地分支、最后第一个远端分支。
- 只有选择当前本地分支时 `includeLocalChanges=true`；选择其它本地/远端分支时强制 false。
- 切换项目保留 composer 文本、普通附件与文件引用，但清空旧项目 KB attach、Git 缓存、分支和 runtime draft。
- Git 信息刷新后 ref 失效时回退默认分支并提示用户，不能静默提交旧 ref。

后端输入：

```ts
interface ProjectSessionBootstrapInput {
  requestId: string
  launchMode: "local" | "worktree"
  baseRef?: string | null
  includeLocalChanges?: boolean
}
```

该字段只允许无 `sessionId` 的项目草稿使用；已有 Session、普通草稿、项目缺失/归档、目录无效、非 Git、非法 ref、tag、任意 SHA 或跨仓库 ref 均 fail closed。老客户端不传时等价于 `launchMode=local`。后端重新解析 `refs/heads/*` / `refs/remotes/*` 并固定为 commit SHA，不信任前端缓存。

`project_bootstrap_runs` 与 `requestId` 提供持久状态、查询和重复请求保护。`requestId` 限定为字母、数字、`-`、`_`，临时目录为 `~/.hope-agent/bootstrap/<request-id>/`。准备阶段通过 `project:bootstrap_progress` 广播 `resolving_git`、`snapshotting`、`creating_worktree`、`copying_changes`、`binding_session`、`ready`；首轮接管后转换为 `chatting` / `completed` 并发 `project:bootstrap_completed`。失败或取消会在模型调用前删除无消息临时 session、清理内建 Worktree 和 Bootstrap 临时目录。应用重启时只由 primary 把遗留运行标记为 `interrupted` 并执行 Git-aware 清理，secondary 打开数据库不得改动运行态。

状态阶段及可见语义：

| 阶段 | 行为 |
| --- | --- |
| `preparing` / `resolving_git` | 校验项目、工作目录、ref 与 repo root，解析固定 SHA。 |
| `snapshotting` | 仅在当前分支匹配时捕获 tracked/untracked 内容并前后复核 HEAD。 |
| `creating_worktree` | 创建临时 Session 和 detached Managed Worktree，暂不发送 `session_created`。 |
| `copying_changes` | 应用 tracked patch、复制 manifest 文件和 `.worktreeinclude`。 |
| `binding_session` / `ready` | 将 Session cwd 绑定 Worktree，准备进入聊天引擎。 |
| `chatting` / `completed` | 首轮只允许启动一次；真正开始时才对 UI materialize Session。 |
| `failed` / `cancelled` / `interrupted` | 不保存首条消息、不调用模型；按 path source 清理并保留诊断状态。 |

同一 `requestId` 正在执行时重复请求附着既有 run；终态重复请求返回既有结果，不重复创建 Worktree 或启动首轮。重试必须生成新 ID。`ready → chatting` 使用条件更新，确保模型首轮最多启动一次。

选择当前本地分支且 HEAD 与已解析 `baseRef` SHA 一致时，可以复制未提交内容：tracked 内容由 `git diff --binary HEAD --` 捕获并以 `git apply --binary` 应用，非忽略 untracked 文件由 NUL 分隔 manifest 复制；staged 状态不保留。所有路径都必须 canonical containment 校验，symlink、HEAD 变化、patch 冲突或部分复制失败会阻止首轮启动。ignored 文件仍仅由 `.worktreeinclude` 控制，`AGENTS.override.md` 延续特殊复制规则。选择其他本地或远端分支时不携带源工作区改动。

失败清理按以下顺序收口：停止复制任务、写 run 终态、解除 Session/Worktree 绑定、Git-aware remove、prune、删除无消息临时 Session、删除 Bootstrap 目录、广播失败事件。内建统一路径可在 Git remove 后做受控目录清理；Hook 路径清理失败时保留现场并标记 `bootstrap_failed`。

首版项目草稿控制面只提供“本地处理 / 新工作树”和起始分支，不包含命名环境、Setup script、环境变量、Actions 或云端运行。

## Session Git 控制面

工作台 Git 卡和 DiffPanel 统一调用 `crates/ha-core/src/git_control.rs`。所有入口只接受 `sessionId`，再通过 `WorkspaceScope::for_session` 解析 effective working directory；客户端不能传 cwd 或 patch。Tauri 的 `commands/git_control.rs` 与 server 的 `routes/git_control.rs` 只是薄适配，HTTP 写操作继续受 `filesystem.allow_remote_writes=false` 默认闸门保护。

Git snapshot 同时返回 checkout root、HEAD/branch/detached、revision、local 与 remote-tracking branches、remotes、worktrees、dirty/status、ahead/behind、最近提交、active location 和 capability。Diff 分 `unstaged` / `staged` / `all`；hunk ID 由后端对 revision、路径和后端重新生成的 patch 内容计算，mutation 时再次匹配，前端不提交任意 patch。stage / unstage / discard 支持 all、file、hunk；binary、rename、submodule、untracked 与 conflict 按能力降为文件级，discard 必须显式确认。

当前分支关联 GitHub PR 时，工作台读取 PR 标题/描述、head/base、变更统计、reviewers、顶层 reviews、merge state、checks 和 review threads，展示检查、评审、未解决评论与合并冲突；单次最多 100 项 checks / 100 个 thread，并返回截断与分通道错误状态。网络读取只由 session 解析出的 checkout、remote 和当前 PR 决定。所有远端文本属于不可信外部数据；“修复”只把经转义和 `<untrusted_external_data>` 包裹的明确任务要求填入当前会话输入框，不自动发送、提交、推送、回复或合并。Attached Worktree 可在已有分支和 PR 上完成这些操作；detached Worktree 仍必须先创建分支。

自动合并是 Session Git 控制面的显式远端写操作，与 Worktree 生命周期无关。用户选择 merge/squash/rebase 并二次确认后，后端重新验证 revision、当前 PR 和冲突状态，再启用远端自动合并；存在冲突时拒绝，不自动改基、fetch 或移动 Session cwd。Local 与 Managed Worktree 使用相同契约和 `git_operation_runs` 幂等记录。

分支、commit、push、PR 和 Handoff 使用 `git_operation_runs` 持久化 `requestId`、阶段、HEAD、结果和错误。事件为 `session:git_progress`、`session:git_changed`、`session:git_completed`。仓库写操作用 `~/.hope-agent/git-locks/<git-common-dir-hash>.lock` 做跨进程短锁；仓库身份和锁基于 `git rev-parse --git-common-dir`，因此 Local 与 linked Worktree 共享同一把锁，而实际 diff/patch 始终在各自 checkout root 执行。Git/gh 子进程禁用终端提示并有超时；不自动 fetch、stash、pull、rebase，也不提供 force push。

安全 Handoff 的快照位于 `~/.hope-agent/git-operations/<request-id>/`，分别保存 staged patch、unstaged patch、untracked manifest/内容和 metadata。目标必须属于同一 Git common dir 且干净；源 checkout 不得有冲突或 untracked symlink。先完整捕获和校验，再移动 branch ownership、应用 staged/index 与 unstaged/worktree 内容，fingerprint 一致后才更新 Session working dir。失败会按 metadata 恢复源并清理目标；启动 reconciler 将遗留运行标为 `interrupted`，不会自动继续 commit、push、PR 或 Handoff。

本节只说明 Worktree 交界面。Snapshot/DTO、Diff/hunk、索引 mutation、分支、commit/push、PR 详情/checks/reviews/comments/自动合并、幂等事件、跨进程锁和逐阶段 Handoff 回滚的单一真相源是 [Session Git 控制平面](git-control.md)。

### 恢复

`restore_managed_worktree` 在 path 不存在时用 `base_sha` 重新 `git worktree add --detach` 并重新复制 `.worktreeinclude`。Workflow runtime 发现绑定 worktree 已归档或路径缺失时，会先自动 restore；失败则把 run 标记为 `blocked(worktree_unavailable)`，禁止悄悄回退到父目录执行。

### 归档

`archive_managed_worktree` 会先记录 dirty snapshot。仅当 worktree clean 且非 handoff 状态时，才 best-effort `git worktree remove` 并 fire `WorktreeRemove`。有本地变更时保留目录，只更新状态和快照。

### 交接

`handoff_managed_worktree` 是生命周期兼容入口：把父 session 的 `working_dir` 切到 worktree path，并标记 `handoff`，同时触发既有 `CwdChanged` hook。它不复制 staged/unstaged/untracked 状态。

工作台的 Local ↔ Worktree 双向迁移必须调用 `git_control::handoff`。该入口要求同仓库、目标干净并验证 staged/unstaged/untracked fingerprint；只有完整复制和校验成功后才更新 Session cwd，失败按持久 metadata 回滚。两类 handoff 不可互换。

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

GUI 上有三层展示：

- Workspace Environment 面板展示当前 session 相关 managed worktrees，可创建、恢复、交接、归档。
- Workflow run overview 展示本 run 绑定的运行位置；优先读取 managed worktree live row，缺失时用 `run_worktree_attached` trace event 兜底显示 path/state/source。
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
- Bootstrap 临时文件只能写入 Hope 数据目录；Hook 自定义路径失败清理只允许 `git worktree remove`，禁止对任意路径递归删除。
- 工作台双向迁移不得调用生命周期兼容 handoff 绕过 Git 状态复制、fingerprint 校验和失败回滚。
