# 用户附件气泡展示完成总结

## 变更摘要

- 前端 `Message` 增加 `attachments` 展示字段，并新增 `MessageAttachment` 类型。
- `parseSessionMessages` 支持从用户消息 `attachmentsMeta` 的数组形态恢复用户上传附件。
- 发送消息时为用户上传文件生成 optimistic 附件展示数据，图片使用 blob preview URL。
- 新增 `UserAttachments` 组件：
  - 图片缩略图展示在用户气泡内，点击进入现有 lightbox。
  - 文件以 compact chip 展示，支持 open/download/reveal（按 transport 能力降级）。
  - 自动 revoke optimistic blob URL。
- 桌面 Tauri 和 HTTP/server 统一通过 `ha_core::attachments::persist_chat_user_attachments_meta` 持久化用户附件元数据。
- HTTP 历史消息返回前会把用户附件绝对路径改写为 `/api/attachments/{session}/{file}` URL，并移除服务器绝对 path。

## 关键文件

- `src/types/chat.ts`
- `src/components/chat/chatUtils.ts`
- `src/components/chat/chatUtils.test.ts`
- `src/components/chat/hooks/useChatStream.ts`
- `src/components/chat/message/MessageBubble.tsx`
- `src/components/chat/message/UserAttachments.tsx`
- `crates/ha-core/src/attachments.rs`
- `src-tauri/src/commands/chat.rs`
- `crates/ha-server/src/routes/chat.rs`
- `crates/ha-server/src/routes/sessions.rs`

## 验证结果

- `pnpm exec vitest run src/components/chat/chatUtils.test.ts`：通过
- `pnpm typecheck`：通过
- `cargo check -p ha-core`：通过
- `cargo check -p ha-server`：通过

未主动运行全量 `pnpm test`、`pnpm lint`、`cargo test`、clippy，遵守项目开发期检查约束。

## 注意事项

- HTTP/server 历史消息会剥离用户附件中的服务器绝对 path，只返回可访问的附件 URL。
- Plan Mode、cron、subagent 等对象形态 `attachmentsMeta` 不会被误解析为用户附件。
- `Attachment.source` 是可选 serde 字段：旧 payload 缺省时仍可反序列化；前端上传显式写
  `upload`，file/plan mention 写 `mention` / `plan_mention`，后端只把用户上传持久化进聊天
  历史附件元数据。HTTP chat 对携带 `file_path` 的附件要求 `source = upload`，避免远端请求把任意
  本地路径伪装成历史附件。
- IM inbound 媒体在 channel worker 中已先搬到 session 附件目录，并直接作为 chat engine
  `attachments` 使用；该路径只写 `channel_inbound` 元数据，不调用
  `persist_chat_user_attachments_meta`。
