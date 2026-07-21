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

### 类型判定与可预览集合

文件类型判定的单一来源是 [`src/lib/fileKind.ts`](../../src/lib/fileKind.ts)：`fileKind(name)` 纯按扩展名（含 `Dockerfile` / `README` 这类无扩展名的约定文件名）分桶，`fileKindOf(name, mime, language)` 在有可靠 MIME（附件）时优先按 MIME 判定、再回退扩展名与工具元数据里的语言。输出是 `FileKind` 九元组：`code | markdown | image | pdf | office | text | audio | video | other`。

可预览集合同样只在这里定义：常量 `PREVIEWABLE_KINDS` 与导出函数 `isPreviewableKind(kind)`，当前包含除 `other` 之外的全部八种。它的唯一消费者是 `fileCapabilities.ts` 的 `resolveFileCapabilities`，用来把 `preview` 置为 `enabled` 或 `disabled("not_previewable")`；其余组件不得自行判断"这个类型能不能预览"。

**新增一种可预览类型必须同步四处**，缺任一处都会得到"能力说可以、面板打不开"或"面板能渲染、入口点不出来"的错配：

1. `fileKind.ts` 的 `FileKind` 联合类型加成员。
2. `fileKind()` / `fileKindOf()` 的扩展名集合与 MIME 分支能把该类型识别出来（否则永远落到 `other`）。
3. `PREVIEWABLE_KINDS` 加入该 kind，`isPreviewableKind` 才会返回 `true`。
4. [`FilePreviewPane`](../../src/components/chat/project/file-browser/FilePreviewPane.tsx)（按 `fileKindOf` 结果分派的只读渲染层，注意与外壳 `files/FilePreviewPanel.tsx` 不是同一个文件）增加对应渲染分支，否则 capability 为 `enabled` 但预览面板落到文本尝试或二进制占位。

同时按需在 [`FileTypeIcon`](../../src/components/icons/FileTypeIcon.tsx) 的 `EXT_ICON` / `iconForMime` 补图标，否则新类型只显示默认文件图标。

三类目标不经 `isPreviewableKind`，在 `resolveFileCapabilities` 里无条件把 `preview` 提升为 `enabled`：`clientDraft`（内存 Blob 一律可尝试预览）、`knowledgeNote`、`artifact`。反方向的强制降级是 `workspace` 目录项，它把 `preview` / `open` / `download` / `reveal` 全部关掉。

### 主点击决议的实际位置

`primary` 与 `menu` 在 [`useFileActions.ts`](../../src/components/chat/files/useFileActions.ts) 计算，`useFileResource` 只是它的一层同签名转发：`target && canPreview && capabilities.preview.state === "enabled"` 取 `preview`，否则 `isLocal ? "open" : "download"`。这里的 `isLocal` 取自 `transport.fileRuntime().workspaceHost === "local"`。

`Transport.supportsLocalFileOps()` 是另一个能力位（Tauri `true` / HTTP `false`），**不参与**能力矩阵与主点击决议；它的消费者是设计视图的本机目录选择与 `previewSource.ts` 媒体项的本地路径直读分支。两者不要互相代替。

Markdown 里的本机路径链接不是它的消费者：`MarkdownRenderer` 只用 `localPathFromHref` 判断 href 是否本机路径，命中后交给 `MarkdownFileLink`，由 `useFileResource` 按同一套能力矩阵决议 preview / open / download（该文件内提到 `supportsLocalFileOps()` 的注释已过时）。性能取舍是只有本机路径链接才付 hook + ContextMenu 成本，外链渲染成纯 anchor。

[`src/lib/fileActions.ts`](../../src/lib/fileActions.ts) 只承载 `FILE_ACTION_META`（每个动作的 i18n key、默认标签、lucide 图标）与 `FileAction` 的 re-export，**不含任何决议逻辑**；决议分布在 `fileCapabilities.ts`（纯能力矩阵）与 `useFileActions.ts`（primary / menu / run 派发）。`FILE_ACTION_META` 只被 `FileActionMenu` 与 `FileBrowserTree` 消费，用于渲染菜单项外观。

## 3. 前端资源层

统一入口位于 [`src/components/chat/files/`](../../src/components/chat/files/)：

- [`types.ts`](../../src/components/chat/files/types.ts)：`FileTarget`、`DraftAttachment`、动作与能力 DTO。
- [`fileCapabilities.ts`](../../src/components/chat/files/fileCapabilities.ts)：无副作用能力矩阵；新增目标/动作先更新这里和矩阵测试。
- [`fileResourceAdapter.ts`](../../src/components/chat/files/fileResourceAdapter.ts)：每类目标实现 `capabilities`、`previewSource` 与 `run`。
- [`useFileResource.ts`](../../src/components/chat/files/useFileResource.ts)：React 业务唯一 hook，返回文件类型、主动作、菜单、能力状态和执行函数。
- [`FileActionMenu.tsx`](../../src/components/chat/files/FileActionMenu.tsx)：右键与 `⋯` 的统一视图。
- [`previewSource.ts`](../../src/components/chat/files/previewSource.ts)：将不同存储后端收敛成 `readText` / `extractDoc` / `rawUrl`。
- [`useObjectUrlLease.ts`](../../src/components/chat/files/useObjectUrlLease.ts)：客户端 Blob URL 的唯一租约；替换、移除、关闭预览及卸载时 revoke。

两个共享模块位于 `src/lib/`，供上述资源层与文件浏览器共用：

- [`fileKind.ts`](../../src/lib/fileKind.ts)：`FileKind` 判定、`isPreviewableKind` 可预览集合、`shikiLang` 高亮语言、`extOf` 扩展名原语。
- [`fileActions.ts`](../../src/lib/fileActions.ts)：`FILE_ACTION_META` 动作展示元数据，无决议逻辑。

### 文件图标单一来源

所有文件形态图标走 [`FileTypeIcon`](../../src/components/icons/FileTypeIcon.tsx)（vscode-icons 彩色图标集，`unplugin-icons` 在构建期内联为 SVG——离线、CSP 安全，且只有此文件 import 过的图标会进包）。解析顺序是扩展名优先（`EXT_ICON`，复用 `fileKind.ts` 的 `extOf`），MIME 兜底（`iconForMime` 覆盖 image/audio/video/pdf/office/json/text 等大类），最后 `default-file`。

统一消费点：输入框附件栏、消息附件卡（`FileCard.tsx` 导出的 `FileMimeIcon` 是为旧 `(mime, name)` 调用点保留的薄适配器）、文件浏览器树与搜索结果、Markdown 文件链接图标、输入框 mention chip，以及 `SkillMentionIcon` 借用 office 三件套图标。业务组件不得为文件另选图标；新增文件形态只在 `EXT_ICON` / `iconForMime` 补一条。

单色 lucide 图标 `fileKind.ts::iconForEntry` 是有意保留的**非文件**用途：文件浏览器树用它渲染目录的展开/折叠态（文件行仍走 `FileTypeIcon`），二进制占位页用它作大号灰度插图。它不是 `FileTypeIcon` 的替代品。

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

### Office 富渲染与后端提取的边界

Office 预览有两条独立实现，**必须保持分离**：

前端富渲染由 [`OfficeRichPreview`](../../src/components/chat/files/office/OfficeRichPreview.tsx) 编排。`officeFormatOf` 先把泛化的 `office` kind 收窄到真正能在浏览器里渲染的子格式（`docx` / `xlsx` / `pptx`，`.xls` 走 SheetJS；**旧 OLE 二进制 `.doc` / `.ppt` 刻意返回 `null`**），再经 `source.rawUrl(false)` 取原始字节交给懒加载的 docx-preview / SheetJS / pptxviewjs。命中以下任一情况即翻到 `OfficeTextFallback`：子格式不支持、体积超过 `min(30 MiB, filesystem.maxDocumentPreviewMb)`、字节 fetch 失败、渲染库自身报错。

文本提取由 `OfficeTextFallback` 在**真正降级发生时**才调 `source.extractDoc()` 触发，具体落到哪里由 `previewSource.ts` 的目标适配器决定，**不是恒定走后端**：

| 目标 | `extractDoc` 实现 | 提取发生在 |
|---|---|---|
| `sessionPath` | `transport.previewExtractDoc` → Tauri `preview_extract` / HTTP `GET /api/sessions/{id}/files/extract` | 后端 |
| `workspace` | `project_fs_extract`（文件浏览器侧等价于 `useProjectFs.extractDoc`） | 后端 |
| `media` | `transport.extractMediaDocument` | 后端 |
| `clientDraft` | `extractOfficeFileInBrowser`（`browserOfficeExtract.ts`） | **浏览器** |
| `artifact` | 抛错（产物不是文档提取源） | — |

前四行里的后端路径统一进入 `filesystem::ops::extract_at` → [`file_extract::extract`](../../crates/ha-core/src/file_extract.rs)，返回纯文本加内嵌图片，并受 `filesystem.maxDocumentPreviewMb` 约束。`clientDraft` 是唯一例外：尚未发送的草稿在 backend 无对应文件，提取只能在 renderer 里做，因此**改后端 `file_extract` 不会影响草稿预览的降级文本**，反之亦然。

**LLM 注入是第三条路径，与上面两条都不相交**：用户附件在发送期由 `agent/content.rs` 直接调同一个 `file_extract::extract`，把正文包成 `<file name=… path=…>` 文本块、抽出的图片并入多模态内容。它不经 `previewSource`、不经 `OfficeRichPreview`、也不看前端是否降级。

两者必须分开的原因是产物性质不同：前端富渲染产出的是 DOM / canvas **视图**，既不可序列化进 prompt，其渲染库也只存在于 renderer；后端提取产出的是确定性**语义文本**，可入上下文、可复用。由此得到两条方向相反的约束：

- 前端富渲染的成败、降级与 30 MiB 视图预算，**不改变模型看到的内容**——调整渲染库或视图上限时不必担心影响模型输入。
- 反过来，`file_extract` 是三方共用的（预览回退文本、LLM 注入、知识空间导入 `knowledge/source.rs`），改动它的提取逻辑必须同时评估这三个消费方，不能只按预览效果验收。

HTTP `sessionPath` 的 read / extract / raw 三个端点**必须共用同一个授权 helper** `authorized_canonical_file_path`（`crates/ha-server/src/routes/sessions.rs`），禁止各端点自写谓词：路径必须被会话工具消息引用，或 canonical path 位于会话 workspace 内；两者皆非的主机任意路径一律 403（否则等于开放远程任意文件读）。不存在的已授权路径可返回 404。Tauri 本地路径由本机 owner 信任边界处理。

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
8. 文件类型判定只调 `fileKind` / `fileKindOf`，图标只用 `FileTypeIcon`，不得自建扩展名表。

新增一种可预览类型时，按「类型判定与可预览集合」一节的四处同步清单逐项确认（`FileKind` 成员 → 分桶识别 → `PREVIEWABLE_KINDS` → `FilePreviewPane` 渲染分支），并在 `FileTypeIcon` 补图标。改动 `file_extract` 的提取逻辑时，同时验证预览回退文本、LLM 附件注入与知识空间导入三个消费方。

最低测试面：能力纯函数矩阵、Tauri/HTTP 适配对齐、路径逃逸/symlink/worktree/archive/远程闸门、CAS 保存与冲突、BOM/换行、脏状态与外部变化、草稿 acquisition/Object URL、upload lease 成功/部分失败回滚/claim/discard/限制/过期清理，以及文件入口不再局部直连系统打开。
