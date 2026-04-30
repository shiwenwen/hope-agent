# AI 回复滚动控制 implementation plan

## 需求重述

修复聊天页面在 AI 回复流式输出时无条件自动滚到底部的问题。目标体验对齐 Codex App / Claude Code：默认跟随最新输出；用户主动向上滚动后暂停 auto-follow；用户滚回底部或点击圆形向下箭头按钮后恢复。该需求涉及前端 UI 与滚动交互，目前未提供 Figma 设计稿，规划按现有聊天界面风格实现。

需求来源：`docs/requirements/PRD：AI回复滚动控制.md`

## 调研摘要

- Claude Code 官方文档明确支持 auto-follow 暂停/恢复：向上滚动会暂停跟随，滚回底部或按 Esc 恢复。键位文档也有“滚到底部并重新启用 auto-follow”的动作。这说明目标产品把滚动跟随作为显式状态，而不是输出时无条件写 `scrollTop`。
- ChatGPT 内部实现未公开。只能借鉴产品行为：底部时自动跟随，用户离开底部时不打扰阅读，并提供跳回最新内容的入口。
- Web 实现可参考三类机制：
  - `scrollTop` / `scrollHeight` 距离阈值：简单、与当前代码一致。
  - bottom sentinel + `IntersectionObserver`：更稳地判断是否贴底。
  - 专用 hook：Vercel AI Chatbot 的 `use-scroll-to-bottom` 与 StackBlitz `use-stick-to-bottom` 都把“贴底”状态封装到 hook 中。

## 当前代码定位

- 共享滚动 hook：`src/components/common/useVirtualFeed.ts`
- 主聊天调用：`src/components/chat/MessageList.tsx`
- Quick Chat 调用：`src/components/chat/QuickChatMessages.tsx`
- 当前根因候选：
  - `useVirtualFeed` 在 `followOutput` 第一次变为 true 时强制 `isUserScrolledUpRef.current = false`。
  - `followOutput` 结束时可能再次调用 `scrollToBottom("auto")`。
  - loading 期间每帧循环把 `scrollTop` 写到底部，只要 detached 状态被重置就会抢回滚动权。

## 设计原则

1. 用户意图优先：用户主动上滚后，AI 输出不得抢滚动位置。
2. 显式状态机：用 `following` / `detached` 表达是否贴底，而不是隐式依赖 `loading`。
3. 单一实现点：优先在 `useVirtualFeed` 统一修复，主聊天与 Quick Chat 复用。
4. 保持历史加载锚点：不破坏 `captureAnchor` / `pendingAnchorRef` 逻辑。
5. 最小可验证改动：先修滚动行为，再加轻量 UI 入口和测试。

## 实施阶段

### Phase 1：重构 `useVirtualFeed` 的 auto-follow 状态

- 将 `isUserScrolledUpRef` 升级为更明确的内部状态：
  - `isFollowingRef`
  - `isDetachedRef`
  - `hasUnseenOutputRef`
- 增加可返回给调用方的状态：
  - `isAtBottom`
  - `isAutoFollowPaused`
  - `hasUnseenOutput`
  - `scrollToBottom`
  - `resumeAutoFollow`
- 修改 `followOutput` 行为：
  - 不再在 streaming 开始时无条件清零用户上滚状态。
  - 仅当当前本来就在底部阈值内时自动跟随。
  - streaming 结束时不强制滚到底部。
- 保留 `resetKey` 行为：会话切换、清空、首次进入仍滚到底部并恢复 following。
- 保留历史加载锚点逻辑，确保加载旧消息后视口不跳动。

### Phase 2：补充回到底部入口

- 在 `MessageList` 底部区域增加悬浮按钮：
  - 显示条件：`isAutoFollowPaused && (loading || hasUnseenOutput)`。
  - 点击：调用 `resumeAutoFollow()` 或 `scrollToBottom("smooth")`，恢复 following。
  - 图标：使用 `lucide-react` 不带横线的向下箭头。
  - 文案/i18n：`chat.scrollToBottom`，中文“回到底部”，英文“Scroll to bottom”，仅用于 tooltip / aria-label。
- 在 `QuickChatMessages` 使用同一状态和 icon-only + tooltip/aria-label。
- 按钮定位需避开输入框、底部 padding、审批/ask_user/plan card。

### Phase 3：处理显式导航与边界场景

- 搜索跳转 `pendingScrollTarget` 后，应暂停自动跟随，直到用户回到底部、点击按钮或发送新消息。
- 用户每次发送新消息时都必须滚动到底部并恢复 following，因为这是明确开始新一轮对话，优先级高于此前的 detached 阅读状态。
- ask_user、plan card、tool approval 等底部交互卡片出现时：
  - 若用户在 following，自动展示到可见区域。
  - 若用户 detached，不抢占视口，但可通过圆形向下箭头按钮抵达。
- 动态高度内容（markdown code block、thinking 展开、tool result 展开）只在 following 状态下推进到底部。

### Phase 4：测试与验证

- 新增或扩展前端测试：
  - loading 中用户上滚后，followKey 变化不触发滚到底部。
  - 点击圆形向下箭头按钮后恢复 following。
  - detached 状态下发送新消息后强制滚到底部并恢复 following。
  - streaming 结束时 detached 状态不强制滚到底部。
  - 历史加载时锚点仍保持。
- 建议验证命令：
  - `pnpm typecheck`
  - 针对改动测试文件运行 Vitest 单点测试。
- 按项目约定，不默认运行全量 `pnpm lint` / `pnpm test`，除非用户确认或改动范围扩大。

## 风险评估

- 复杂度：Medium。改动集中在前端共享滚动 hook，但影响主聊天和 Quick Chat 两处入口。
- 主要风险：
  - 虚拟列表测量与滚动状态互相影响，导致 near-bottom 误判。
  - 历史消息 prepend 的锚点恢复被新的 auto-follow 状态干扰。
  - 搜索跳转、会话切换、loading 结束三种显式滚动行为边界混淆。
  - 新增 i18n key 未覆盖所有语言导致检查失败。

## 开放问题

1. 是否需要严格复刻 Codex App 的按钮样式，还是按 Hope Agent 现有视觉风格实现即可？
2. “用户发送新消息”是否总是恢复 auto-follow？本计划默认恢复，因为用户通常希望看到新回复。
3. Quick Chat 是否需要显示文字，还是 icon-only 更合适？本计划默认主聊天和 Quick Chat 都使用 icon-only。

## 变更计划：浮动按钮 UI 收敛（2026-04-30）

### 需求补充

用户希望调整 auto-follow 恢复按钮的视觉和交互：

- 按钮不显示“跳到最新”这类可见文案。
- 按钮保持圆形，而不是带文字的胶囊形按钮。
- 向下箭头图标不要带底部横线，避免视觉上像“下载/落到底线”。
- 鼠标悬浮到按钮时需要显示小手光标。

该需求涉及前端 UI 微调。当前未提供 Figma 设计稿；若没有额外设计稿，默认按 Hope Agent 现有圆形 icon button 风格实现。

### 实现方案

1. 更新需求文档与任务清单
   - 将 PRD / task 中“跳到最新”可见文案的描述改为“圆形向下箭头按钮”。
   - 保留可访问性语义，但避免用户可见位置继续出现“跳到最新”。
   - 将本次 UI 调整作为追加任务记录到 `task.md`。

2. 调整主聊天 `MessageList`
   - 将当前带文字的主聊天按钮改为 icon-only 圆形按钮。
   - 图标从 `ArrowDownToLine` 替换为不带横线的 `ArrowDown` 或同类 lucide 图标。
   - 样式使用固定宽高（如 `h-9 w-9`）、`rounded-full`、`items-center justify-center`。
   - 增加 `cursor-pointer`，确保 hover 时为小手。
   - 移除 visible `<span>{t("chat.jumpToLatest")}</span>`。

3. 调整 Quick Chat
   - 与主聊天使用一致的无横线向下箭头图标。
   - 保持 icon-only 圆形按钮。
   - 增加 `cursor-pointer`。

4. i18n 与可访问性
   - 若仍需要 tooltip / aria-label，改用“回到底部 / Scroll to bottom”这类语义，避免用户看到“跳到最新”。
   - 同步 12 个 locale，并运行 i18n 缺失检查。

5. 测试更新
   - 更新 `MessageList.test.tsx` / `QuickChatMessages.test.tsx` 中按钮 accessible name 的断言。
   - 增加或调整测试，确认主聊天按钮不再渲染“跳到最新”文本。
   - 运行相关 Vitest 单点测试、`pnpm typecheck`、`node scripts/sync-i18n.mjs --check`。

### 风险

- 删除 visible 文案后，按钮含义依赖图标与 tooltip / aria-label，需要确保无障碍语义仍清晰。
- 如果保留 tooltip，用户 hover 时仍会看到某个文案；本计划默认使用“回到底部”而不是“跳到最新”。
- 当前 PR 已创建，确认后默认在现有分支 `fix/chat-auto-follow-scroll` 上追加提交并更新同一个 PR。

## 等待确认

请回复 `CONFIRM` 或 `确认执行` 后再进入实现阶段。确认后会创建 `docs/requirements/AI回复滚动控制/task.md` 并按任务清单推进。
