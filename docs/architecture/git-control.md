# Session Git 控制平面

> 返回 [文档索引](../README.md) | 更新时间：2026-07-12

Session Git 控制平面负责 Hope Agent 工作台中的仓库状态、Diff 审阅、索引操作、分支、提交、推送、GitHub Pull Request，以及 Local 与 Managed Worktree 之间的安全交接。桌面端和 HTTP/server 端都只适配 `ha-core::git_control`，不各自实现 Git 业务逻辑。

本文描述已经落地的运行时契约。Managed Worktree 的创建、归档、恢复、项目首轮 Bootstrap、Workflow 和 Subagent 集成见 [Managed Worktree 控制平面](worktree.md)。项目草稿的项目归属与首发状态见 [项目系统](project.md)。

## 1. 边界与分层

```text
GitControlCard / DiffPanel
        |
        v
Transport (Tauri / HTTP)
        |
        v
ha-core::git_control
        |
        +-- SessionDB / git_operation_runs
        +-- WorkspaceScope::for_session
        +-- git / gh 子进程
        +-- managed_worktrees / project bootstrap
```

| 层 | 代码 | 责任 |
| --- | --- | --- |
| 核心编排 | `crates/ha-core/src/git_control.rs` | 仓库解析、snapshot、diff、mutation、branch、commit、push、PR、Handoff、锁和恢复。 |
| Git 公共读取 | `crates/ha-core/src/filesystem/git.rs`、`session/environment.rs` | 分支、dirty、worktree、同步状态等底层读取，供项目草稿和会话控制面复用。 |
| Worktree 生命周期 | `crates/ha-core/src/worktree.rs` | Managed Worktree 创建、归档、恢复和 owner 记录。 |
| Tauri 适配 | `src-tauri/src/commands/git_control.rs` | `spawn_blocking` 调核心函数并映射桌面命令错误。 |
| HTTP 适配 | `crates/ha-server/src/routes/git_control.rs` | REST DTO、diff scope 解析、remote-write gate。 |
| Transport | `src/lib/transport.ts`、`transport-tauri.ts`、`transport-http.ts` | 公共 TypeScript DTO 与双运行模式调用映射。 |
| 工作台 | `src/components/chat/workspace/GitControlCard.tsx`、`PullRequestPanel.tsx` | 紧凑状态卡、分支/提交/PR/Handoff 对话框，以及独立 PR 详情面板中的 checks、评审、冲突和自动合并。 |
| Diff 审阅 | `src/components/chat/diff-panel/DiffPanel.tsx` | staged/unstaged/all、文件/hunk 操作、Review 评论上下文。 |

核心边界：

- Owner API 只接收 `sessionId`，客户端不能提交任意 cwd、仓库根目录、PR 编号或任意 patch。
- 后端通过 session effective working directory 和 `WorkspaceScope::for_session` 解析 checkout；解析失败即拒绝。
- 读取接口不修改仓库；HTTP 写接口额外受 `filesystem.allow_remote_writes=false` 默认闸门保护。
- 所有会影响 refs、index、working tree 或远端的操作都在后端重新校验 revision、HEAD 和仓库身份。

## 2. 仓库身份、工作目录与锁

一次请求同时维护三个路径概念：

| 概念 | 说明 |
| --- | --- |
| `workspace_root` | Session 实际工作目录，允许是仓库子目录；用于保持会话作用域。 |
| `checkout_root` | `git rev-parse --show-toplevel` 得到的当前 checkout 根；diff、index 和 patch 在这里执行。 |
| `common_dir` | `git rev-parse --git-common-dir` 的 canonical 路径；Local 与 linked Worktree 共享仓库身份。 |

跨进程写锁为：

```text
~/.hope-agent/git-locks/<common-dir-hash>.lock
```

使用 `common_dir` 而不是 checkout 路径计算锁，确保 Local 与同仓库 Worktree 不会并发修改共享 refs。锁只包围必要的 Git 写临界区；Git 自身的 index/ref lock 仍作为第二层保护。

Git 与 `gh` 子进程统一设置 `GIT_TERMINAL_PROMPT=0` 并带超时，避免桌面或 server 因交互式凭据提示永久阻塞。不自动执行 fetch、stash、pull、rebase 或任何 force push。

## 3. Snapshot 与能力

`SessionGitControlSnapshot` 是工作台 Git 卡的单一快照：

```ts
interface SessionGitControlSnapshot {
  root: string
  head: string | null
  branch: string | null
  detached: boolean
  revision: string
  branches: GitBranchInfo[]
  remotes: GitRemoteInfo[]
  worktrees: WorktreeInfo[]
  dirty: GitDirtySummary
  status: WorkspaceGitStatus
  sync: WorkspaceGitSync
  lastCommit: WorkspaceGitCommit | null
  activeLocation: "local" | "worktree"
  managedWorktreeId?: string | null
  capabilities: GitCapabilities
}
```

`revision` 是写操作的乐观并发令牌，覆盖 HEAD、index 和 working tree 状态。前端必须把读取时的 revision 带回；仓库变化后后端返回 stale 错误，前端刷新 snapshot/diff 并要求用户重新确认。

`capabilities` 根据 detached、busy、仓库状态和运行位置给出 switch/create branch、commit、push、PR、Handoff 可用性。它用于界面解释与禁用，不替代执行层校验。

分支读取规则：

- 使用 `git for-each-ref` 读取 `refs/heads/*` 与 `refs/remotes/*`。
- 排除 `origin/HEAD` 等 remote symbolic ref。
- 使用 `git worktree list --porcelain` 标记已被其它 Worktree checkout 的本地分支和路径。
- 不主动 fetch；列表只反映本地已经存在的 refs。

## 4. Diff 与索引操作

### 4.1 Diff scope

`SessionGitDiffSnapshot.scope` 支持：

- `unstaged`：index → working tree，并补充非忽略 untracked 文件。
- `staged`：HEAD → index。
- `all`：HEAD → working tree，用于整体审阅。

单侧文本最大读取 256 KiB，超过后标记 `truncated`。binary、submodule、rename/copy、untracked、conflict 仍返回文件元数据，但只开放安全的文件级操作。

### 4.2 Hunk 身份

后端根据 revision、path、hunk header 和完整 patch 内容生成 `hunkId`。前端 mutation 只回传 `hunkId`，不能上传 patch。执行时后端在锁内重新生成 hunks 并精确匹配；匹配失败视为 stale，不执行旧 patch。

### 4.3 Stage、Unstage 与 Discard

| 操作 | all | file | hunk |
| --- | --- | --- | --- |
| Stage | `git add -A` | 路径限定 `git add` | 后端 patch `git apply --cached` |
| Unstage | `git reset HEAD` | 路径限定 reset | 后端 patch reverse 到 index |
| Discard | restore tracked + 删除 untracked manifest | restore 或删除该 untracked 文件 | 后端 patch reverse 到 worktree |

Discard 必须携带 `confirmDiscard=true`。未跟踪文件 discard 等价于删除；路径先做相对路径和 canonical containment 校验。冲突文件允许用户解决后 stage，但禁止 hunk discard。操作成功后返回刷新后的同 scope snapshot，前端保持当前文件与滚动位置。

## 5. 分支

### 切换分支

- 有 staged、unstaged、untracked 或 conflict 时禁止切换到其它分支；不自动 stash。
- 本地分支必须来自后端 snapshot 的 `fullRef`，已被其它 Worktree checkout 时拒绝。
- remote-tracking ref 只在工作区干净时创建本地 tracking branch。
- 不接受 tag、任意 SHA 或客户端自行拼接的 ref。

### 创建分支

- 允许从当前 HEAD 原地创建分支，并保留现有 staged/unstaged/untracked 内容。
- 分支名先经 `git check-ref-format --branch` 验证。
- detached Managed Worktree 必须先创建分支，才能 commit、push 或创建 PR。
- 创建成功后同步 `managed_worktrees.git_branch`，保持生命周期记录与真实 checkout 一致。

## 6. Commit 与 Push

Commit 输入包含 subject、可选 body、`stageAll` 和 `pushAfter`：

- subject 必须是非空单行；默认只提交 staged 内容。
- `stageAll=true` 时，在同一仓库锁和 revision 校验内先 `git add -A` 再提交。
- 保留仓库 hooks、签名和作者配置，不传 `--no-verify`。
- detached HEAD 禁止 commit。
- commit 已成功但可选 push 失败时，返回成功提交和 `warning`，不把已经产生的 commit 误报为回滚。

Push 规则：

- 有 upstream 时执行普通 `git push`。
- 无 upstream 时只有 `setUpstream=true` 才允许选择 remote 并执行 `git push -u`；默认 remote 为 `origin`。
- upstream 已知且 behind/diverged 时拒绝，不自动同步。
- 不提供 force、force-with-lease、删除远端分支或修改远端 URL 的入口。

## 7. GitHub Pull Request

网络访问是显式、按需的。普通 Git snapshot 不调用网络；只有读取 PR preflight、PR feedback、打开/创建 PR 时调用已安装并认证的 `gh`。

### 7.1 Preflight

`GitPullRequestPreflight` 依次验证：

1. 当前 checkout 已附着本地分支。
2. remote host 是 GitHub 或 GitHub Enterprise。
3. 本机/服务器存在 `gh`。
4. `gh auth status --hostname <host>` 成功。
5. 可以解析 `owner/repository`、默认分支和当前分支关联 PR。

失败返回稳定的 `errorCode`/`errorMessage`，工作台展示不可用原因。已有 PR 时主操作变为打开 PR；没有 PR 时进入创建对话框。

### 7.2 创建 PR

- title 必填，body 可选，draft 默认开启。
- base 优先远端默认分支，再回退 `main`、`master`。
- 分支未推送时只有用户确认 `pushFirst` 才顺序执行 push → `gh pr create`。
- 未提交本地内容不会进入 PR，创建对话框必须明确提示。
- PR 创建使用 `requestId` 幂等保护，不会因重连重复创建。

### 7.3 PR 详情

已有 PR 的主操作打开独立右侧 PR 面板，而不是把用户直接带离工作台或覆盖 composer。该面板接入统一 exclusive right panel 体系，可与工作台、Diff、文件和浏览器面板切换，复用共享宽度、窄屏折叠与 overlay 策略；切换 Session 时关闭，避免展示旧分支内容。详情由后端根据当前 Session checkout、remote 和 branch 解析，客户端不能指定 PR 编号，包含：

- 标题、描述、作者、head/base branch、增删行数和变更文件数；
- requested reviewers、review decision 和每位审阅者的最新顶层 review summary；历史上已被后续评审取代的状态不进入修复队列；
- mergeable / merge state、是否存在冲突；
- checks 明细、未解决 review thread；
- 自动合并状态与合并方式。

PR 标题、描述、评审正文和远端身份均属于外部不可信数据。界面只作纯文本展示；进入修复任务时统一限制长度、转义并放入不可信数据封装。

### 7.4 Checks、Review 与合并冲突

`pull_request_feedback` 聚合两个独立通道：

- `gh pr checks <number> --json ...`：返回 pass/fail/pending/cancel/skipping、workflow、描述、时间和链接。
- GitHub GraphQL `reviewThreads(first: 100)`：每个 thread 只读取第一条根评论，并通过 `totalCount` 计算回复数，返回作者、文件、行号、正文、链接、resolved/outdated 状态；不下载未展示的回复正文。

行为契约：

- 单次最多返回 100 个 checks 和 100 个 review thread；超过后分别标记 `checksTruncated` / `commentsTruncated`。
- checks 与 comments 独立容错；一个通道失败时另一个仍可展示，并分别返回 `checksError` / `commentsError`。
- 工作台摘要只统计失败/运行中/成功 checks，以及未解决且未过期的评论。
- 详情支持手动刷新；当前实现每 30 秒做一次有界轮询，同一 session/HEAD/branch 的手动刷新和轮询共享一个带错误收口的在途请求。切换 session、HEAD 或 branch 会丢弃旧请求结果。
- 完整刷新失败时可以保留上一次成功数据用于参考，但必须显式标记为可能过期，并禁用基于远端新鲜状态的“修复”和自动合并，直到刷新成功。
- 顶层 review summary 与 requested reviewers 来自当前 PR 详情；要求修改的 review 与未解决 thread 可以分别修复或合并成一个“全部修复”任务。
- `mergeable=CONFLICTING` 或 `mergeStateStatus=DIRTY` 时显示独立冲突状态；“修复冲突”只生成限定为当前 head/base 的修复任务，不自动 merge、commit 或 push。
- 评论可作为 DiffPanel 的额外上下文展示，但不改变仓库 diff，也不会自动写代码。

### 7.5 “修复”入口的安全边界

“修复”只把结构化任务填入当前输入框，用户确认发送后才进入正常聊天与工具审批流程。它不会自动：

- 发送消息；
- 运行命令或修改文件；
- commit、push 或创建 PR；
- 回复或 resolve GitHub 评论。

PR 标题、描述、head/base、检查描述、评审与评论正文、作者、路径和 URL 都属于外部不可信数据。进入 prompt 前限制条目数和单项长度、转义 `<`/`&`，并包裹在 `<untrusted_external_data>` 中；这些字段不得出现在可信任务描述中，正文中的指令不得提升为系统指令。

### 7.6 自动合并

自动合并是显式远端写操作，不与“修复 PR”绑定：

- 只允许当前分支已经关联的 open PR，客户端不能提交任意 PR 标识。
- 用户必须先打开二次确认弹窗，并选择 `merge` / `squash` / `rebase`；请求必须携带 `confirmAutoMerge=true`。
- 确认弹窗明确提示：如果仓库保护条件已经满足，启用后 PR 可能立即合并。
- 存在合并冲突时拒绝，不尝试自动改基、拉取或解决冲突。
- 后端在执行前重新验证 revision、当前 PR 和冲突状态，通过 `gh pr merge --auto` 启用；不提供管理员绕过、force 或分支删除入口。
- 操作写入 `git_operation_runs`，同一 `requestId` 不会重复启用；HTTP 端受 `filesystem.allow_remote_writes` 闸门保护。

## 8. Local / Managed Worktree 安全 Handoff

`activeLocation` 描述 Session 当前运行位置，不复用 Managed Worktree 生命周期 `state`。安全 Handoff 不是简单改 `sessions.working_dir`，而是带状态验证和回滚的 Git 操作。

临时目录：

```text
~/.hope-agent/git-operations/<request-id>/
├── staged.patch
├── unstaged.patch
├── untracked.manifest
├── untracked/
└── metadata.json
```

流程：

1. 解析源/目标 checkout，要求共享同一 `common_dir`，目标属于当前 session 或允许的 child session Worktree。
2. 拒绝活跃聊天回合、后台 Job 或 Workflow 正在使用目录；目标必须干净。
3. 拒绝 unresolved conflict 和 untracked symlink。
4. 分别捕获 staged binary patch、unstaged binary patch、非忽略 untracked 文件和内容 hash。
5. 记录源/目标 HEAD、branch、checkout、Worktree ownership 与 fingerprint。
6. 在锁内重新验证源未变化，必要时移动分支 ownership。
7. 把 staged patch 应用到目标 index+worktree，再应用 unstaged patch和 untracked manifest。
8. 校验目标 staged/unstaged/untracked fingerprint 与源一致。
9. 校验通过后更新 Session working directory 和 active location；最后删除临时目录。

失败恢复：

- 源尚未清理时，撤销目标本次 manifest 中的内容，保持源不变。
- 源已经清理时，根据 metadata 恢复源，再清理目标。
- 只删除 manifest 记录的 untracked 文件，保留目标中无关的新文件。
- 若 HEAD 或外部文件在操作中变化，停止破坏性回滚并保留诊断信息，避免覆盖用户并发修改。
- 应用启动时 primary reconciler 将遗留 run 标为 `interrupted` 并尝试恢复原位置；不会自动继续 Handoff、commit、push 或 PR。

旧的 `handoff_managed_worktree` 是 Managed Worktree 生命周期 owner 操作，只负责显式绑定一个 Worktree cwd 并记录 `state=handoff`。工作台 Local/Worktree 双向迁移必须使用 `git_control::handoff`，不能用旧入口绕过改动复制与回滚。

## 9. 幂等、进度与恢复

`git_operation_runs` 落在 `sessions.db`：

```text
id, session_id, operation, status, stage,
before_head, after_head, result_json,
error_code, error_message,
created_at, updated_at, completed_at
```

适用于 branch、commit、push、PR 创建、自动合并和 Handoff：

- `requestId` 全局唯一；同 ID/同 session/同 operation 返回已有终态结果。
- 同 ID 指向不同 session 或 operation 时拒绝。
- 已在运行的长操作不重复执行；客户端可通过 run 查询恢复进度。
- Git 状态改变后发 `session:git_changed`；长操作发 `session:git_progress`；终态发 `session:git_completed`。
- Handoff 的 `ready/running` 类阶段只能单向推进，启动恢复依据持久 stage 和 metadata 决定回滚动作。

## 10. Owner API

| Tauri Command | HTTP | 类型 |
| --- | --- | --- |
| `load_session_git_control_cmd` | `GET /api/sessions/{id}/git` | 只读 snapshot |
| `load_session_git_diff_snapshot_cmd` | `GET /api/sessions/{id}/git/diff?scope=...` | 只读 diff |
| `mutate_session_git_index_cmd` | `POST /api/sessions/{id}/git/index` | 写：stage/unstage/discard |
| `switch_session_git_branch_cmd` | `POST /api/sessions/{id}/git/branch/switch` | 写 |
| `create_session_git_branch_cmd` | `POST /api/sessions/{id}/git/branch/create` | 写 |
| `commit_session_git_cmd` | `POST /api/sessions/{id}/git/commit` | 写 |
| `push_session_git_cmd` | `POST /api/sessions/{id}/git/push` | 写/网络 |
| `session_git_pr_preflight_cmd` | `GET /api/sessions/{id}/git/pull-request` | 只读/网络 |
| `load_session_git_pr_feedback_cmd` | `GET /api/sessions/{id}/git/pull-request/feedback` | 只读/网络 |
| `create_session_git_pr_cmd` | `POST /api/sessions/{id}/git/pull-request` | 写/网络 |
| `enable_session_git_pr_auto_merge_cmd` | `POST /api/sessions/{id}/git/pull-request/auto-merge` | 写/网络 |
| `handoff_session_git_cmd` | `POST /api/sessions/{id}/git/handoff` | 写/长操作 |
| `get_git_operation_run_cmd` | `GET /api/git-runs/{requestId}` | 只读恢复 |

HTTP 的所有“写”行都要求 `filesystem.allow_remote_writes=true`。PR preflight/feedback 是网络只读，不受文件写闸门控制，但仍要求 API 鉴权、session 作用域和本机 `gh` 认证。

## 11. GUI 状态与刷新

工作台 Git 卡依次展示：

- 变更数量和 `+added -removed`，点击进入 DiffPanel。
- Local/Worktree 运行位置与 Handoff 入口。
- 当前分支；detached 时显示创建分支。
- 根据 dirty/ahead 状态显示提交或推送。
- 创建或打开 Pull Request。
- 当前 PR 的 checks、评审、未解决 Review 评论和合并冲突摘要。
- 独立右侧 PR 面板，以及显式确认后的自动合并入口。

完成 index、branch、commit、push、PR 或 Handoff 后统一刷新 session snapshot；DiffPanel mutation 返回新 diff 并保留当前 scope、文件和滚动位置。Session、HEAD 或 branch 变化会清空旧 PR feedback，防止把前一分支的检查和评论展示到当前工作区。

## 12. 错误与红线

稳定错误类别至少覆盖：

- `stale_snapshot`：revision/hunk 已变化，刷新后重试。
- `branch_checked_out` / `branch_exists` / `detached_head`。
- `nothing_to_commit` / `no_upstream` / `remote_behind`。
- `gh_unavailable` / `gh_unauthenticated` / `not_github_remote` / `gh_repo_unavailable`。
- `auto_merge_confirmation_required` / `pull_request_conflicts` / `gh_auto_merge_failed`。
- `operation_running` / `handoff_same_location` / `cross_repository_handoff`。
- `conflicts_present` / `handoff_source_changed` / `handoff_verification_failed` / `handoff_rollback_failed`。

不可破坏的红线：

- 不能信任前端 cwd、ref、path、hunk patch、remote 或 PR 标识。
- Discard 必须二次确认；路径越界和 symlink 必须 fail closed。
- 不自动 fetch/stash/pull/rebase，不提供 force push。
- commit 已产生后不得因后续 push 失败伪装成未提交。
- Handoff 未完成 fingerprint 校验前不得切换 Session cwd。
- PR 外部文本不得作为可信指令注入。
- 自动合并必须由用户显式确认，存在冲突时 fail closed，不得绕过分支保护或强制合并。
- Tauri 与 HTTP 必须复用同一核心编排和 DTO 语义。

## 13. 测试契约

核心定向测试应覆盖：

- staged/unstaged/all、同文件双态和 hunk identity。
- all/file/hunk stage、unstage、discard；binary/rename/submodule/untracked/conflict 降级。
- stale revision、路径越界、仓库锁与 branch ownership。
- detached、remote tracking、dirty branch switch、create branch 保留改动。
- staged-only/stage-all commit、hook/签名/作者错误、push upstream/behind。
- PR preflight/详情、checks bucket、顶层 review、review threads、冲突状态、通道部分失败和截断。
- 自动合并确认、三种合并方式、冲突拒绝、请求幂等和 HTTP remote-write gate。
- Local ↔ Worktree 的 staged/unstaged/untracked、回滚、外部并发变化和启动恢复。
- Tauri/HTTP DTO 对齐与 HTTP remote-write gate。

前端定向测试应覆盖 Git 卡不同状态、PR 详情/checks/reviews/comments/冲突、自动合并确认、修复 prompt 的不可信数据转义、DiffPanel 评论上下文、stale 刷新以及 Handoff 进度恢复。
