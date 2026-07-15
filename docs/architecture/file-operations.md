# 统一文件能力（File Operations）

本文是 Hope Agent 文件展示、预览、打开、下载、编辑与上传生命周期的单一架构真相源。项目文件浏览器、聊天输入框草稿、消息附件、Markdown 文件链接、工具媒体、Workspace 产物、项目 Memory 文件和知识空间文件都必须从本契约派生；业务组件不得自行判断 Tauri/HTTP、拼接文件 URL 或直接调用 `window.open`。

## 1. 两个正交维度

文件的**位置**与**生命周期**分开建模：

```ts
type FileTarget =
  | { kind: "clientDraft"; draft: DraftAttachment; previewId: string }
  | {
      kind: "workspace"
      scope: "session" | "project" | "path"
      scopeId: string
      relPath: string
      name: string
    }
  | { kind: "sessionPath"; sessionId?: string | null; path: string; name: string }
  | { kind: "media"; item: MediaItem }
  | { kind: "knowledgeNote"; kbId: string; path: string; contentHash?: string }
  | { kind: "artifact"; artifactId: string; name: string; projectPath?: string | null };
```

- `clientDraft`：当前 renderer 内存中的浏览器 `File`，从粘贴、拖放或选择器获得；发送前不属于 backend。
- `workspace`：Server/桌面后端解析的受限工作区相对路径，所有访问经 `WorkspaceScope`。
- `sessionPath`：会话中由工具、Markdown 或产物引用的绝对路径；HTTP 仍须按会话授权。
- `media`：已发送的聊天附件或工具媒体，使用 transport 的媒体 URL/路径解析。
- `knowledgeNote`：知识空间 Markdown；写操作始终委托 Note service，不接普通 workspace mutation。
- `artifact`：受管 Canvas/Artifact HTML 投影；以 opaque Artifact ID 解析预览，打开和导出由 Transport 适配当前 runtime。

运行位置只有两种文件主机语义：

| 前端形态 | Transport | `workspaceHost` | 文件实际所在机器 | 打开 | reveal |
|---|---|---|---|---|---|
| 本地桌面 | Tauri | `local` | 当前电脑 | 系统默认应用 | 支持 |
| 桌面远程 | HTTP | `remote` | Server 所在机器 | 浏览器/应用内 | 不支持 |
| Web | HTTP | `remote` | Server 所在机器 | 浏览器/应用内 | 不支持 |

桌面远程与 Web 的文件语义完全相同。`clientDraft` 无论在哪种形态都只位于当前客户端；文件浏览器的“上传”则是用户显式触发的 workspace 写操作，远程时会将客户端文件上传到 Server workspace。

### 大小配置与硬上限

所有可配置的 `MB` 字段实际均按 MiB（`1024 × 1024`）换算。旧 JSON 缺字段时使用默认值；读写、上传 start/complete/claim 与保存入口都调用后端同一组 clamp/bytes helper。

| 配置 | 默认 | 范围 | 覆盖场景 |
|---|---:|---:|---|
| `filesystem.maxChatAttachmentMb` | 20 | 1–512 | 用户聊天附件 + Agent `send_attachment` |
| `filesystem.maxWorkspaceUploadMb` | 20 | 1–512 | 新版 workspace 分块上传 |
| `filesystem.maxTextPreviewMb` | 5 | 1–50 | Workspace、消息附件、未发送附件文本预览 |
| `filesystem.maxTextEditMb` | 5 | 1–20，且 ≤ preview | Workspace/草稿副本/项目 `AGENTS.md` 编辑与保存 |
| `filesystem.maxDocumentPreviewMb` | 50 | 5–100 | PDF/Office 后端预览提取 |
| `filesystem.maxArtifactImportMb` | 25 | 1–100 | Artifact HTML/Markdown/Analysis JSON 来源导入 |
| `knowledgeSourceLimits.maxTextSourceMb` | 5 | 1–20 | 知识空间文本来源 |
| `knowledgeSourceLimits.maxBinarySourceMb` | 24 | 1–100 | 知识空间文档、音视频、图片来源 |
| `knowledgeSourceLimits.maxUrlResponseMb` | 2 | 1–20 | URL 网页响应 |

`PATCH /api/config/filesystem` / `patch_filesystem_config` 只更新显式字段，避免 Server 页修改 `allowRemoteWrites` 覆盖文件限制。`ha-settings` 中 `filesystem` 仍只承载 HIGH 风险开关；`file_limits` 与 `knowledge_source_limits` 为 MEDIUM。

不可配置的安全/协议上限保持独立：头像 10 MiB；Office 富渲染 30 MiB（超限回退文本提取）；代码高亮约 400 KiB（超限无高亮）；Logo、STT、IM 平台、远程图片/PDF、Memory 备份继续使用各子系统硬上限。旧 Base64 知识导入固定 24 MiB，旧聊天 stage/Base64 与旧 Workspace whole-body 上传固定 20 MiB；只有新版分块租约可使用更高配置。

## 2. 统一动作与能力

```ts
type FileAction =
  | "preview" | "open" | "download" | "reveal"
  | "edit" | "remove" | "rename" | "delete"
  | "createFile" | "createFolder" | "upload" | "saveAs";

type CapabilityState = "enabled" | "guided" | "disabled";
```

- `enabled`：可直接执行。
- `guided`：入口保留，点击后解释风险并引导到 Server 设置；不能先发一个注定 403 的 mutation。
- `disabled`：类型、大小或目标本身不允许，不提供解锁引导。

能力优先级固定为：**目标固有只读 > 类型/大小限制 > 远程写开关 > 可执行**。前端能力只控制交互，绝不是鉴权边界；后端每次 mutation 都重新解析 scope 并应用同一最终写策略。

主点击默认行为：可预览目标优先 `preview`；没有预览宿主时，本地使用 `open`，远程使用 `download`。文件左键、右键菜单、`⋯` 菜单和预览面板顶部按钮都必须读取同一个 `FileCapabilitySet`。

## 3. 前端资源层

统一入口位于 [`src/components/chat/files/`](../../src/components/chat/files/)：

- [`types.ts`](../../src/components/chat/files/types.ts)：`FileTarget`、`DraftAttachment`、动作与能力 DTO。
- [`fileCapabilities.ts`](../../src/components/chat/files/fileCapabilities.ts)：无副作用能力矩阵；新增目标/动作先更新这里和矩阵测试。
- [`fileResourceAdapter.ts`](../../src/components/chat/files/fileResourceAdapter.ts)：每类目标实现 `capabilities`、`previewSource` 与 `run`。
- [`useFileResource.ts`](../../src/components/chat/files/useFileResource.ts)：React 业务唯一 hook，返回文件类型、主动作、菜单、能力状态和执行函数。
- [`FileActionMenu.tsx`](../../src/components/chat/files/FileActionMenu.tsx)：右键与 `⋯` 的统一视图。
- [`previewSource.ts`](../../src/components/chat/files/previewSource.ts)：将不同存储后端收敛成 `readText` / `extractDoc` / `rawUrl`。
- [`useObjectUrlLease.ts`](../../src/components/chat/files/useObjectUrlLease.ts)：客户端 Blob URL 的唯一租约；替换、移除、关闭预览及卸载时 revoke。

Transport 在 [`transport.ts`](../../src/lib/transport.ts) 定义：

- `fileRuntime()`：同步返回 `workspaceHost`、`openMode` 与 `canReveal`。
- `getWorkspaceAccess(scope)`：向后端读取最终 workspace 写能力。
- `openWorkspaceFile` / `downloadWorkspaceFile` / `revealWorkspaceFile`。
- `uploadFile(file, purpose, progress?, signal?)` / `discardFileUpload(uploadId)`：聊天、Workspace、知识来源统一分块协议。
- `stageChatAttachment` / `discardChatAttachmentUpload`：聊天调用侧别名，内部委托通用租约。

[`transport-provider.ts`](../../src/lib/transport-provider.ts) 通过 `useSyncExternalStore` 暴露响应式 `useTransport()`；切换本地/远程后所有文件能力立即重算。非 React 代码保留 `getTransport()`。存在脏编辑器时，切换 Transport 必须先确认。

## 4. Workspace 访问与写闸门

Tauri `project_fs_capabilities` 与 HTTP `GET /api/fs/capabilities` 返回：

```ts
interface WorkspaceAccess {
  readable: boolean;
  writeState:
    | "enabled"
    | "remote_writes_disabled"
    | "scope_read_only"
    | "project_archived";
}
```

后端 [`WorkspaceScope`](../../crates/ha-core/src/filesystem/workspace.rs) 是唯一判定点：

- 本地桌面默认可写。
- HTTP（包括桌面远程和 Web）受 `filesystem.allowRemoteWrites` 约束。
- `path` worktree 跳转固定只读。
- archived project 及其 session workspace 固定只读。
- 知识空间外部目录继续服从 `allow_external_writes`；后台自主维护永不写外部。

`WorkspaceScope::access` 与 `resolve_effective_writable` 使用同一策略，防止 capability 与实际 403 漂移。路径必须是 scope 内相对路径；`..`、绝对路径、symlink escape 与非当前仓库 worktree 跳转均 fail closed。

远程写关闭时，UI 将写动作标记为 `guided`，弹出风险说明并提供“前往 Server 设置”；文件浏览器不能直接修改高风险开关。设置事件、Transport 重连和 event-stream resync 后重新读取能力。

## 5. 文本读取、编辑与并发保存

`project_fs_read_text` / `FileTextContent` 除原字段外返回：

```ts
interface FileTextContent {
  contentHash: string | null; // 磁盘原始 bytes 的 BLAKE3
  isUtf8: boolean;
  lineEnding: "lf" | "crlf" | "cr" | "mixed";
  hasUtf8Bom: boolean;
}
```

只有有效 UTF-8、非二进制、非截断且不超过 `filesystem.maxTextEditMb` 的文件可编辑（默认 5 MiB）。编辑器复用 CodeMirror 6，按扩展名识别语言；Markdown 可在源码与渲染视图间切换。Office、PDF、图片及其他二进制文件不编辑。

保存必须显式触发（按钮或 Cmd/Ctrl+S）：

- 编辑已有文件传 `expectedFileHash`。
- 新建/另存为传 `createOnly=true`。
- 保存保留 UTF-8 BOM 与原换行格式；混合换行首次保存会提示，并统一到占比最高的格式。
- 写入经 `platform::write_atomic`，不存在普通 `fs::write` 回退。

返回值在 Tauri/HTTP 保持相同结构：

```ts
type FileWriteOutcome =
  | { status: "saved"; relPath: string; sizeBytes: number; contentHash: string }
  | { status: "conflict"; reason: "changed" | "deleted"; currentContentHash?: string };
```

冲突只提供“重新加载”“另存为”“取消”，禁止强制覆盖。另存为只能留在当前 scope，且 `createOnly` 防止覆盖已有文件。

收到 `project:fs_changed` 时：编辑器干净则重读并自动刷新；有脏修改则显示外部变化提示，不覆盖编辑区。切文件、关闭面板、切 Transport 与离开页面都必须拦截未保存修改。

## 6. 客户端草稿附件

```ts
interface DraftAttachment {
  id: string;
  file: File;
  acquisition: "paste" | "drop" | "picker";
  semanticSource: "upload" | "pasted_text";
  status: "ready" | "uploading" | "error";
  error?: string;
}
```

草稿按会话保存在 renderer 内存；切换会话可恢复，刷新/退出不持久化。发送前不得发出附件上传请求。

- 图片、音视频、PDF、Office、文本直接从 Blob/File 预览。
- “打开”只打开 Blob URL，不创建临时磁盘文件。
- 有效 UTF-8 且不超过 `filesystem.maxTextEditMb`（默认 5 MiB）的文本、代码、Markdown 和长粘贴文本可编辑内存副本；保存以新 `File` 替换草稿，绝不修改用户原始磁盘文件。
- 支持预览、打开、下载副本、编辑副本、移除和替换。
- Object URL 由统一租约管理。

## 7. 发送与 upload lease

点击发送后才开始上传，并固定当时的 Transport 与草稿快照：

1. 前端读取当前后端的 `filesystem.maxChatAttachmentMb` 并校验单文件大小；默认 20 MiB，可配置范围 1–512 MiB。单消息最多 64 个附件。
2. 最多 3 个文件并发调用 `uploadFile(..., "chat_attachment")`；每个文件内部按 4 MiB 严格顺序发送，图片和普通文件不再转 Base64。
3. 任一失败时等待在途任务结束，回收全部成功 lease；文字和所有草稿保留，并标出错误，消息不发送。
4. 全部成功后生成只含 `upload_id` 的 `ChatAttachment`，再清空输入并启动/入队消息。
5. normal chat 在保存用户消息时 claim；durable queue 在保存 queue row 时 claim。未 claim lease 可显式 discard。
6. lease id 为 UUID，HTTP 不暴露服务端磁盘路径；`.part` 与原子 metadata sidecar 位于内部 pending 目录。lease 1 小时过期，启动时及每 15 分钟清理；全局最多 256 个、8 GiB 声明数据。
7. 后端用同一配置再次校验附件大小、64 个、UUID、来源和 `upload_id` 与 `data`/`file_path` 互斥；客户端值不能绕过后端。
8. claim 先复制并准备全部目标，任一失败回滚所有目标且保留原 lease；准备全部成功后才删除源 lease，保证可重试。

`ChatAttachment.upload_id` 与 `data`、`file_path` 互斥。旧字段仍用于 ACP、IM、历史客户端和历史消息，但 HTTP 传入的旧 `file_path` 必须 canonicalize 后位于该 session 或 `_temp` 附件目录，否则 403。远程客户端不能借 `source: "upload"` 伪造任意主机路径。

通用协议为 `file_upload_start/status/chunk/complete/discard`：chunk 必须携带精确 offset，最多 4 MiB；响应丢失时客户端查 status 从已收 offset 继续；单块最多 3 次指数退避；完成时流式计算 BLAKE3 并验证声明大小。start、complete 和最终业务 claim 都重读当前配置，配置在上传途中降低会使 finalize/claim 失败。Tauri chunk 使用 raw binary IPC body，HTTP chunk 使用 Blob request body，renderer 与 Server 在上传阶段都不缓冲完整文件。

附件上限属于后端配置：本地桌面读取本机 `config.json`，桌面远程与 Web 读取 Server 的 `config.json`。旧配置缺少字段时按 20 MiB 处理；设置保存时钳制到 1–512 MiB。旧 multipart/stage/Base64 入口维持 20 MiB 静态兼容上限。

发送 API 返回失败时，前端 discard 尚未 claim 的 lease；已 claim 文件由 session 删除和 incognito 焚毁流程管理。

## 8. 预览、打开与知识空间边界

[`FilePreviewPane`](../../src/components/chat/project/file-browser/FilePreviewPane.tsx) 是统一预览视图：

- code/text/Markdown：文本与语法高亮；Markdown 可切换渲染/源码。
- image/PDF/audio/video：浏览器原生预览。
- Office：docx-preview / SheetJS / pptxviewjs 富预览，失败时回退后端抽取文本。
- 二进制/失败状态：显示原因，并从同一能力层提供打开或下载。
- 顶部按钮按 capability 显示打开、下载和编辑。

HTTP `sessionPath` 的 read/extract/raw 共用会话授权：路径必须被会话工具消息引用，或 canonical path 位于会话 workspace 内；否则统一 403。不存在的已授权路径可返回 404。Tauri 本地路径由本机 owner 信任边界处理。

知识空间文件只统一读侧预览、打开、下载、reveal 与能力展示；编辑仍由 NoteEditor 和 Note service 承担，并保留其 `expectedFileHash` stale-write、外部 root read-only、external/remote write 双闸门。禁止把知识空间 mutation 接到普通 `project_fs_write_text`。

消息附件的“归档到知识空间”是 media adapter 之外的显式扩展动作，不能混进通用 `FileAction` 权限语义。

Workspace 上传先完成 `workspace_upload` lease，再由 `project_fs_claim_upload` / `POST /api/fs/upload-claim` 在最终可写 scope 中复制、fsync、原子 publish；claim 时重新检查远程写开关、归档/只读、路径逃逸、symlink、覆盖策略和动态大小。知识来源本地文件使用 `knowledge_source` lease，`KnowledgeSourceImportInput.uploadId` 与 `content` / `dataBase64` / `url` 互斥。客户端本地 Artifact 来源使用 `artifact_source` lease，`ArtifactImportRequest.uploadId` 与 runtime-host `filePath` 互斥。知识来源与 Artifact 均在成功导入后消费 lease，失败保留至过期以支持重试。

## 9. 接入与验证清单

新增文件入口必须满足：

1. 创建合适的 `FileTarget`。
2. 使用 `useFileResource`；左键执行 `run(primary)`。
3. 右键使用 `FileContextMenu`，可发现入口使用 `FileActionsMoreButton`。
4. 不直接调用 `window.open`、`openFilePath`、`downloadFilePath`、`reveal_in_folder`，不拼 raw URL。
5. 新 Transport 命令同时实现 Tauri + HTTP，并更新 [`api-reference.md`](api-reference.md)。
6. mutation 的 UI capability 与 backend guard 必须来自同一后端判定。
7. 覆盖本地桌面、桌面远程、Web、固有只读、远程写关闭与 transport 切换。

最低测试面：能力纯函数矩阵、Tauri/HTTP 适配对齐、路径逃逸/symlink/worktree/archive/远程闸门、CAS 保存与冲突、BOM/换行、脏状态与外部变化、草稿 acquisition/Object URL、upload lease 成功/部分失败回滚/claim/discard/限制/过期清理，以及文件入口不再局部直连系统打开。
