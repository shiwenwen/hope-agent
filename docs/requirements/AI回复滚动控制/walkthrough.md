# AI 回复滚动控制 walkthrough

## 变更总结

- 在 `useVirtualFeed` 中把原先隐式的 `isUserScrolledUpRef` 升级为显式 auto-follow 状态：
  - `isAutoFollowPaused`
  - `hasUnseenOutput`
  - `isAtBottom`
  - `pauseAutoFollow`
  - `resumeAutoFollow`
- streaming 开始时不再无条件恢复跟随；streaming 结束时也不再强制滚到底部。
- 用户向上滚动、触摸向历史方向拖动或键盘向上导航后进入 detached 状态；滚回底部或点击按钮后恢复 following。
- 新增 `forceFollowKey`，用于“用户发送新消息”这类显式动作恢复跟随。
- 主聊天 `MessageList` 增加圆形 icon-only 向下箭头悬浮按钮。
- Quick Chat 增加紧凑版圆形 icon-only 向下箭头按钮。
- auto-follow 恢复按钮不再显示“跳到最新”可见文案；主聊天与 Quick Chat 均使用无横线 `ArrowDown` 图标、圆形按钮和 `cursor-pointer` hover 小手。
- 主聊天和 Quick Chat 都补充了组件层测试，确保“最新 user turn”变化时会传入 `forceFollowKey`；即使发送后已追加 assistant 占位消息，也会强制恢复跟随。
- Quick Chat 的 icon-only 按钮使用 `IconTip`，避免 HTML 原生 `title` 属性。
- 搜索跳转会暂停 auto-follow，避免仍在 streaming 的回复把视口抢回底部。
- `resetKey` 只在 key 实际变化时恢复 following，避免 hook 内部重渲染误恢复 auto-follow。
- detached 后 streaming 的 rAF 跟随循环会停止，用户恢复 following 后再启动。
- 修复两个继续下滚的根因：
  - 已排队的 `scrollToBottom` rAF 在用户 detached 后会在调用 `virtualizer.scrollToIndex` 前退出。
  - TanStack Virtual 的动态高度补偿在 detached 状态下关闭，避免流式回答行变高时修正 `scrollTop`。
- 修复发送新消息不自动到底部的根因：发送流程会在同一批 React 更新中追加 user 消息和空 assistant 占位，旧逻辑只检查最后一条消息是否为 user，导致 `forceFollowKey` 被置空。
- `forceFollowKey` 的 user turn key 抽到共享工具，并只使用消息位置 + `dbId` / `timestamp` 等稳定身份，不再拼接用户消息全文。
- touch 交互增加方向判断，手指向上滑动、内容朝底部移动时不再短暂进入 detached。
- 补齐 12 个 locale 的 `chat.scrollToBottom` 文案，用于 aria-label / tooltip。

## 关键文件

- `src/components/common/useVirtualFeed.ts`
- `src/components/common/useVirtualFeed.test.tsx`
- `src/components/chat/MessageList.tsx`
- `src/components/chat/MessageList.test.tsx`
- `src/components/chat/QuickChatMessages.tsx`
- `src/components/chat/QuickChatMessages.test.tsx`
- `src/components/chat/chatScrollKeys.ts`
- `src/components/chat/chatScrollKeys.test.ts`
- `src/i18n/locales/*.json`

## 验证结果

- `pnpm exec vitest run src/components/common/useVirtualFeed.test.tsx src/components/chat/MessageList.test.tsx src/components/chat/QuickChatMessages.test.tsx src/components/chat/chatScrollKeys.test.ts`
  - 4 files passed
  - 19 tests passed
- `pnpm typecheck`
  - exit 0
- `pnpm lint`
  - exit 0
- `node scripts/sync-i18n.mjs --check`
  - 总计缺失：0 条
- `git diff --check`
  - exit 0
- `pnpm install --frozen-lockfile`
  - exit 0，用于恢复当前本地缺失的 `diff@9.0.0` 依赖链接；未修改 `package.json` / `pnpm-lock.yaml`

## 未运行

- 未运行全量 `pnpm test`，遵循项目“开发过程中默认只跑单点验证”的约定。
