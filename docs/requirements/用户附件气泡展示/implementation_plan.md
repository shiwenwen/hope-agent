# 用户附件气泡展示实现计划

## 需求重述

当前用户在聊天输入框发送图文消息后，模型可以正常读取并分析图片，但聊天界面的用户气泡只展示文本，不展示随消息发送的图片或文件附件。

需要修复桌面 GUI 和 HTTP/server 模式下的用户附件展示链路：

- 发送瞬间的 optimistic 用户消息应立即展示上传附件。
- 历史消息从数据库加载后，应从 `attachmentsMeta` 恢复用户上传附件。
- 用户消息气泡应展示图片缩略图与文件附件卡片，并支持打开、下载或本地 reveal 等现有能力。
- 不影响助手消息工具附件、Plan Mode 消息、cron/subagent/channel inbound 等现有特殊消息。

该需求涉及前端 UI 改动。未提供 Figma 设计稿，按现有聊天气泡、`ToolMediaPreview`、`FileAttachments` 和项目 Tailwind/shadcn 风格实现。

## 现状证据

- `src/components/chat/hooks/useChatStream.ts` 中 `optimisticUserMessage` 只包含文本、时间戳和 plan 字段，没有携带 `attachedFiles` 对应的展示数据。
- `src/components/chat/chatUtils.ts::parseSessionMessages` 解析用户消息时只消费 `subagent_result`、`cron_trigger`、`plan_trigger`、`plan_comment`、`channel_inbound` 等元数据，没有解析用户上传附件数组。
- `src/components/chat/message/MessageBubble.tsx` 中 `messageFiles` 只从 `msg.role === "assistant" && msg.contentBlocks` 提取工具附件，用户消息不会进入附件渲染路径。
- 桌面 Tauri `src-tauri/src/commands/chat.rs` 会把上传附件保存到 session attachments 目录，并把 `{ name, mime_type, size, path }[]` 写入用户消息 `attachments_meta`。
- HTTP/server `crates/ha-server/src/routes/chat.rs` 当前保存用户消息时传入 `None`，未把 `body.attachments` 转成可恢复的 `attachments_meta`。

## 风险评估

- `attachments_meta` 是多用途字段，已经承载 Plan Mode、subagent、cron、tool media 等标记。新增解析必须只识别“数组形态的用户附件”，避免误判已有对象形态元数据。
- 桌面保存的用户附件字段是 snake_case-ish 旧形态：`mime_type`、`size`、`path`；现有 `MediaItem` 是 camelCase：`mimeType`、`sizeBytes`、`localPath`、`url`、`kind`。需要归一化，不能直接 cast。
- HTTP/web 客户端不能直接读取服务器绝对路径。要么后端生成 `/api/attachments/{sessionId}/{filename}` URL，要么前端在 HTTP 模式退化为通过 `/api/sessions/{sessionId}/files/by-path` 打开/下载文件。图片 inline 预览需要可解析 URL。
- Optimistic 阶段如果直接使用 `File` object 预览，需要管理 `URL.createObjectURL` 生命周期，避免泄漏。更稳妥是发送前创建轻量 preview URL，并在消息被 DB 行替换后由 React 卸载释放。
- 现有 `FileAttachments` 文案是 `chat.modifiedFiles`，用于助手工具产物。用户上传附件继续复用可能显示语义不准，建议拆出附件预览组件或允许 label 覆盖。

## 实现策略

### Phase 1: 类型与解析测试

1. 在 `src/types/chat.ts` 为 `Message` 增加用户附件展示字段，例如：
   - `attachments?: MessageAttachment[]`
   - `MessageAttachment` 可复用/扩展为 UI 友好的结构：`name`、`mimeType`、`sizeBytes`、`kind`、`localPath?`、`url?`、`previewUrl?`
2. 在 `src/components/chat/chatUtils.ts` 增加纯函数解析用户附件元数据：
   - 仅当 `attachmentsMeta` JSON 是数组时解析。
   - 支持 `{ name, mime_type, size, path }`。
   - 归一化为 UI 类型，并根据 MIME 推断 `kind: "image" | "file"`。
   - 对 malformed JSON、对象形态 meta、空数组返回空。
3. 先补 `src/components/chat/chatUtils.test.ts`：
   - 用户消息从数组 `attachmentsMeta` 恢复图片附件。
   - 对象形态 `plan_trigger` / `plan_comment` 不产生附件。
   - 非图片文件恢复为 file kind。

### Phase 2: Optimistic 用户附件

1. 在 `useChatStream.ts` 发送时基于 `filesToSend` 构建 optimistic 附件列表。
2. 图片使用 `URL.createObjectURL(file)` 作为 `previewUrl`，文件使用名称/MIME/大小展示。
3. 把该附件列表挂到 `optimisticUserMessage.attachments`。
4. 确保 `mergeMessagesByDbId` 在 DB 行到达后保留 `_clientId`，附件最终以 DB 解析结果为准；如果 DB 行还没带附件，则不误删 optimistic 展示直到 reload merge 完成。

### Phase 3: 用户气泡渲染

1. 新增或抽取一个通用附件展示组件，优先复用现有能力：
   - 图片：使用 `getTransport().resolveMediaUrl(...)`、`resolveAssetUrl(...)` 或 optimistic `previewUrl` 渲染缩略图；点击进入现有 `ImageLightbox`。
   - 文件：复用 `FileMimeIcon` 和 `FileAttachments` 的 open/download/reveal 行为。
2. 在 `MessageBubble.tsx` 用户分支中，在文本上方或下方展示附件。建议顺序：
   - 图片缩略图先展示，形成图文消息直观预览。
   - 文件 chips/卡片跟在图片后。
   - 文本保持当前 Markdown/Text renderer。
3. 保持助手消息工具附件现有表现不变。
4. 确保空文本+附件消息可渲染，不因为 `handleSend` 的 `rawText.trim()` 限制被挡住；如当前产品允许纯图片发送，应调整 guard 为“文本或附件至少一个”。如果当前不支持纯附件发送，本次至少不破坏现状。

### Phase 4: HTTP/server 附件元数据补齐

1. 检查 `crates/ha-server/src/routes/chat.rs` 的 `ChatRequest.attachments` 数据来源与 `POST /api/chat/attachment` 暂存行为。
2. 对 HTTP `body.attachments` 生成与 Tauri 兼容的用户附件 `attachments_meta`：
   - 对 `file_path` 附件，读取大小，写入 `name`、`mime_type`、`size`、`path`。
   - 对 `data` 图片附件，保存到 session attachments 目录后写入同样字段。
   - 尽量复用/抽取 ha-core helper，避免 Tauri 和 server 继续漂移。
3. 如果重构后会影响 Tauri，增加小范围 Rust 单测覆盖元数据构造逻辑；否则保持最小变更。

### Phase 5: 验证

按项目约束，开发过程中只做单点验证：

1. 前端类型检查：`pnpm typecheck`。
2. 相关 Vitest 单测：优先运行 `pnpm vitest run src/components/chat/chatUtils.test.ts` 或项目实际可用的等价命令。
3. 如改了 Rust helper，运行对应 crate 的 `cargo check -p ha-server` 或 `cargo check -p ha-core`。
4. 不主动跑 `pnpm lint`、全量 `pnpm test`、`cargo test`、clippy；需要时先询问。

## 任务拆分

1. 创建 `task.md`，按计划维护执行状态。
2. 为用户附件元数据解析补前端单测。
3. 实现前端类型与解析函数。
4. 实现 optimistic 附件字段。
5. 实现用户气泡附件 UI。
6. 补齐 HTTP/server 用户附件 `attachments_meta`。
7. 运行单点验证，修复发现的问题。
8. 创建 `walkthrough.md` 总结变更与验证结果。

## 复杂度

Medium。

主要复杂度不在 UI 本身，而在三种附件形态的兼容：

- Optimistic 阶段的 browser `File`。
- Tauri 历史消息中的 `{ name, mime_type, size, path }`。
- 工具/助手消息中已有的 `MediaItem`。

## 等待确认

请回复 `CONFIRM` 后开始执行。未确认前不会修改实现代码。
