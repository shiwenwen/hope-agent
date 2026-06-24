# 文件操作统一（File Operations）

聊天里文件出现在三处：**Markdown 超链文件**（[`MarkdownRenderer`](../../src/components/common/MarkdownRenderer.tsx) 的 `MarkdownLink`）、**消息下挂文件**（[`FileCard`](../../src/components/chat/message/FileCard.tsx) / [`FileAttachments`](../../src/components/chat/message/FileAttachments.tsx)）、**工作台产物文件**（[`WorkspacePanel`](../../src/components/chat/workspace/WorkspacePanel.tsx) 的 `FileRow`）。本文档是这三处「点击 / 右键做什么」的单一真相源——统一的策略、按运行模式分流的行为矩阵、右侧内置预览面板，以及支撑预览的 preview-by-path 双壳后端。

## 行为矩阵（契约）

唯一分流信号是 `transport.supportsLocalFileOps()`：桌面 Tauri = **本机**（true），HTTP/Web = **远端**（false）。文件类别由 [`fileKind.ts`](../../src/lib/fileKind.ts) 的 `fileKind(name)` / `fileKindOf(name, mime)` 给出，`isPreviewableKind` 判定是否可预览。

| 类别 | 主点击（本机） | 主点击（远端） | 菜单（本机） | 菜单（远端） |
| --- | --- | --- | --- | --- |
| 可预览：text / code / markdown / pdf / office / audio / video / image | 右侧面板预览 | 右侧面板预览 | 预览 · 打开 · 在文件夹中打开 | 预览 · 下载 |
| 其余（archive / 未知 / 二进制 = `other`） | 打开（系统默认应用） | 下载 | 打开 · 在文件夹中打开 | 下载 |

菜单同时由**右键**（`ContextMenu`）和**「⋯」按钮**（`DropdownMenu`）触发；Markdown 链接只挂右键（保持内联简洁），卡片 / 行两者都给。

## 前端策略层（`src/components/chat/files/`）

- **纯策略** [`fileActions.ts`](../../src/lib/fileActions.ts)：`resolvePrimaryFileAction(kind, isLocal)` 与 `resolveFileMenuActions(kind, isLocal)` 是无副作用纯函数（可单测），外加 `FILE_ACTION_META`（i18n key + 图标）。
- **分类器** [`fileKind.ts`](../../src/lib/fileKind.ts)：在原有 code/markdown/image/pdf/office/text 基础上加 `audio` / `video` / `other`；`fileKindOf` 优先信任 MIME（下挂文件 `MediaItem.mimeType` 比扩展名可靠）。**新增可预览类型只改这里的 `isPreviewableKind`。**
- **派发 hook** [`useFileActions`](../../src/components/chat/files/useFileActions.ts)：吃一个 `PreviewTarget`（`{kind:"path",path,name,mime?}` | `{kind:"media",item}`），返回 `{ primary, menu, run, … }`。`run(action)` 把 `preview` 派给 `onPreviewFile`、其余派给 transport（`openMedia`/`openFilePath` · `downloadMedia`/`downloadFilePath` · `revealMedia`/`reveal_in_folder`）。
- **环境注入** [`fileActionsContext.ts`](../../src/components/chat/files/fileActionsContext.ts)：消息树深，`sessionId` + `onPreviewFile` 经 `FileActionsContext` 注入（ChatScreen 在 `MessageList` 外用 `FileActionsContext.Provider` 包裹 + memoized value），叶子组件不用层层 prop 透传。消息树**外**的调用方（工作台面板）用 `useFileActions(target, { sessionId, onPreviewFile })` overrides 显式传入。无 provider 时 `onPreviewFile` 缺失 → 预览降级为打开/下载。
- **菜单组件** [`FileActionMenu.tsx`](../../src/components/chat/files/FileActionMenu.tsx)：`FileContextMenu`（右键包裹）+ `FileActionsMoreButton`（⋯）。
- **文件类型图标** [`FileTypeIcon`](../../src/components/icons/FileTypeIcon.tsx)：vscode-icons 彩色格式图标（`unplugin-icons` 构建期内联、仅打包所需、离线 CSP 安全）；`FileMimeIcon` 是其 `(mime, name)` 薄适配。**新增可视化文件图标处一律复用它**、按扩展名 / MIME 扩 `EXT_ICON`，**勿再用单色 lucide `File*`**。

四处接入都只做：主点击 `run(primary)`，外面包 `FileContextMenu` / 加 `FileActionsMoreButton`。工作台 `FileRow` 额外保留「查看 diff」按钮（独有）。

## 预览面板

- **复用** 文件浏览器的 [`FilePreviewPane`](../../src/components/chat/project/file-browser/FilePreviewPane.tsx)，已重构为吃一个 **`PreviewSource`**（[`previewSource.ts`](../../src/components/chat/files/previewSource.ts)：`readText` / `extractDoc` / `rawUrl`）而非 scope 绑定的 `(fs, entry)`。三个适配器：`projectFsPreviewSource`（文件浏览器，relPath）/ `pathPreviewSource`（绝对路径）/ `mediaPreviewSource`（`MediaItem`）。渲染按 kind 派发：code/text/markdown → Shiki，markdown 可切渲染/源码，image → `<img>`，pdf → `<iframe>`，**audio/video → 原生 `<audio>/<video>`**，office → **前端富渲染**（[`files/office/`](../../src/components/chat/files/office/)：docx 用 docx-preview、xlsx/xls 用 SheetJS、pptx 用 pptxviewjs，均懒加载 `await import`，解析 `rawUrl` 取得的原始字节渲染近原版式），不支持子格式 / 损坏 / 超大 / 渲染失败 → 回退后端 extract 文本（`OfficeTextFallback`，顶部小字提示「排版可能与原文件有差异」），其余 → 二进制占位。
- **面板与控制器**：[`FilePreviewPanel`](../../src/components/chat/files/FilePreviewPanel.tsx) 把 `PreviewTarget` 转 `PreviewSource` 后渲染 `FilePreviewPane`；[`useFilePreview`](../../src/components/chat/files/useFilePreview.ts) 是 ChatScreen 的控制器（`showPanel` / `target` / `openPreview` / `closePreview`），作为右侧 `ExclusiveRightPanel` 的 `"preview"` 接入（与 diff/workspace 同套：类型并集 / ORDER / ICONS / visibility memo / label / 渲染分支 / 切会话重置）。

## preview-by-path 后端（核心进 ha-core）

预览的三处文件多为**绝对路径**（Markdown 链接、工作台产物、下挂 path 项），不在任何单一 workspace scope 下，故走按绝对路径的读取通道：

- **ha-core**（[`filesystem/ops.rs`](../../crates/ha-core/src/filesystem/ops.rs)）：`read_text_abs(abs)` / `extract_abs(abs)`，沿用 5MB/50MB 上限、二进制嗅探、`mime_for_path`；原 scope 版重构成「scope+rel → abs 再调 abs 版」单一实现。**abs 版不做 scope 容器校验**——授权由调用边界负责。
- **桌面（Tauri）**：`preview_read_text(path)` / `preview_extract(path)`（[`commands/project_fs.rs`](../../src-tauri/src/commands/project_fs.rs)）信任本机，直接读（与 `open_directory` 一致）；raw url 客户端 `convertFileSrc`（`resolveAssetUrl`）。
- **远端（HTTP）**：`GET /api/sessions/{id}/files/{read,extract}`（[`routes/sessions.rs`](../../crates/ha-server/src/routes/sessions.rs)），raw 复用既有 `/files/by-path`（inline 供 `<img>/<iframe>/<video>`，`?download=1` 强制下载）。
- **Transport**：`previewReadText` / `previewExtractDoc` / `previewRawUrl`（[`transport.ts`](../../src/lib/transport.ts) + tauri/http）。

### 授权红线

HTTP 三端点共用 `authorized_canonical_file_path(sessionId, path, messages)`：

1. 路径**被会话某条 tool 消息引用**（`collect_authorized_session_file_paths`，精确字符串匹配），或
2. 路径 canonicalize 后**落在会话工作目录内**（`WorkspaceScope::for_session` + [`WorkspaceScope::contains`](../../crates/ha-core/src/filesystem/workspace.rs)，即文件浏览器 `/api/fs/*` 暴露的同一作用域）。

二者皆非的主机任意路径一律 `403`——**远端放行任意主机路径 = 远程任意文件读，安全红线**。为防探测，被引用但已删除的文件返 `404`，其余未授权统一返 `403`（不区分"不存在"与"在作用域外"）。这条拓宽同时惠及既有 `/files/by-path` 下载。

## 已知局限

- HTTP 下 Markdown 绝对路径链接只有**被工具引用过或落在工作目录内**才可预览/下载；纯文中提及、又在工作目录外的主机路径仍 403。
- Office 预览优先**前端富渲染**（docx/xlsx/pptx 解析 `rawUrl` 取得的原始字节渲染近原版式）；旧二进制格式（`.doc`/`.ppt`）/ 损坏 / 超大（>30MB）/ 渲染失败时**回退** extract 文本近似（顶部小字提示差异）。Excel 仅还原结构 + 合并单元格（不含单元格填充色 / 边框，SheetJS 社区版限制），PPT 走 Canvas（文本不可选、无动画——纯前端 pptx 固有局限）。菜单仍可「打开/下载」原文件看完整版式。远端下挂的 office `MediaItem` 富渲染需可 `fetch` 的 `rawUrl`，否则回退文本 / 下载。
- 桌面 asset 协议须允许任意绝对路径（`pickLocalImage` + 文件浏览器预览早已在用 `convertFileSrc(abs)`）。
