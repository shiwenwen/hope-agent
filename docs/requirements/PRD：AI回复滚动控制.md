# PRD：AI 回复滚动控制

## 背景

当前聊天页面在 AI 回复流式输出时会持续自动滚动到底部。用户尝试向上滚动查看前文、工具结果或正在生成回答的早期内容时，视口会被新的 token 更新拉回底部，导致无法稳定阅读。

用户期望行为与 Codex App / Claude Code 这类开发型 AI 聊天体验一致：默认跟随最新输出，但一旦用户主动向上滚动，系统应尊重用户阅读意图，暂停自动跟随；用户回到底部或点击圆形向下箭头按钮后再恢复跟随。

## 调研结论

### Claude Code / Codex 类体验

- Claude Code 官方 fullscreen 文档将该行为称为 auto-follow：默认跟随最新输出；用户向上滚动后暂停；滚回底部或按 Esc 恢复跟随。来源：https://code.claude.com/docs/en/fullscreen
- Claude Code 快捷键文档提供“滚到底部并重新启用 auto-follow”的快捷操作，说明该能力被建模为显式的“跟随/暂停跟随”状态，而不是每次输出都无条件滚到底部。来源：https://code.claude.com/docs/en/keybindings
- 借鉴点：把“是否自动滚动”从简单的 `loading === true` 改成用户意图驱动的状态机。

### ChatGPT

- ChatGPT Web 的内部实现未在官方公开资料中披露，不能断言其代码结构。
- 从产品行为和同类聊天 UI 的公开实现看，合理抽象是：用户处于底部附近时自动跟随；用户主动向上滚动时停止跟随；底部浮出圆形向下箭头入口；用户点击或滚回底部后恢复。
- 因此本需求不声称复刻 ChatGPT 内部实现，只借鉴其用户体验模式。

### Web 实现参考

- `Element.scrollIntoView()` 是常见的滚到底部实现方式，但必须受“是否仍在跟随底部”的状态控制。来源：https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollIntoView
- `IntersectionObserver` 可用底部 sentinel 判断用户是否在底部附近，适合避免纯 scrollTop 计算在动态内容高度变化时误判。来源：https://developer.mozilla.org/en-US/docs/Web/API/Intersection_Observer_API
- CSS `overflow-anchor` 会影响浏览器自动保持滚动位置，虚拟列表和手动滚动控制场景需要明确评估是否启用或禁用。来源：https://developer.mozilla.org/en-US/docs/Web/CSS/overflow-anchor
- Vercel AI Chatbot 的开源 `use-scroll-to-bottom` hook 使用容器 ref + end ref + observer/scroll 操作来封装聊天滚动控制，适合作为 React 结构参考。来源：https://raw.githubusercontent.com/vercel/ai-chatbot/main/hooks/use-scroll-to-bottom.tsx
- StackBlitz `use-stick-to-bottom` 把“贴底/不贴底”做成 hook 状态，适合作为“自动跟随状态机”的参考。来源：https://github.com/stackblitz-labs/use-stick-to-bottom

## 当前项目观察

- 主聊天列表与 Quick Chat 都复用 `src/components/common/useVirtualFeed.ts`。
- `MessageList` 通过 `followOutput: loading` 与 `followKey` 驱动滚动，`followKey` 会随最后一条消息内容长度变化而变化。
- `QuickChatMessages` 也以同样方式传入 `followOutput: loading`。
- `useVirtualFeed` 已有部分“用户向上滚动暂停”的逻辑：wheel 上滚、touchmove、PageUp/ArrowUp/Home、scrollTop 变小且离底部超过阈值时会设置 `isUserScrolledUpRef = true`。
- 主要问题在 `followOutput` 生命周期：
  - 流式输出刚开始时会把 `isUserScrolledUpRef` 强制设回 `false`。
  - 流式输出结束时，如果没有被判定为用户上滚，会再次滚到底部。
  - streaming 期间通过 `requestAnimationFrame` 循环持续设置 `scrollTop = scrollHeight - clientHeight`。
- 因此只要“用户已脱离底部”的状态被重置或未被正确识别，视口就会持续被拉回底部。

## 目标

1. AI 回复过程中，用户可以向上滚动并稳定停留在阅读位置。
2. 用户处于底部或接近底部时，AI 回复仍自然自动跟随最新输出。
3. 用户离开底部后，新 token、新工具状态、thinking block 展开、markdown 高度变化都不能强制拉回底部。
4. 用户可通过明确动作恢复跟随：发送新消息、滚回底部、点击圆形向下箭头按钮、或切换/重置会话。
5. 保留现有虚拟列表、历史消息向上加载和搜索跳转能力。
6. 主聊天与 Quick Chat 的体验一致。

## 非目标

- 不修改后端 streaming 协议。
- 不改变消息渲染格式、markdown 渲染、工具结果卡片结构。
- 不引入新的全局设置项，除非后续用户明确要求“关闭自动跟随”。
- 不重写虚拟列表为非虚拟列表。

## 用户故事

1. 作为用户，我在 AI 回复过程中向上滚动查看上一段内容时，页面不应继续跳回底部。
2. 作为用户，我在底部等待回复时，新内容应持续可见，不需要手动滚动。
3. 作为用户，我离开底部后，应看到一个轻量的圆形向下箭头入口，点击后回到底部并继续跟随。
4. 作为用户，我滚回底部后，系统应自动恢复跟随后续输出。
5. 作为用户，我搜索并跳转到历史消息时，不应被仍在生成的回复打断阅读位置。
6. 作为用户，我发送一条新消息时，无论当前正在历史位置还是 detached 状态，聊天视口都应立即回到底部，确保我能看到刚发送的消息和随后的 AI 回复。

## 交互规则

### 状态定义

- `following`：视口在底部附近，允许 AI 输出驱动滚到底部。
- `detached`：用户主动离开底部，禁止 AI 输出驱动滚动。
- `manualJump`：用户点击圆形向下箭头按钮或显式调用滚到底部后，切回 `following`。

### 状态切换

- 初始进入会话、切换会话、清空会话、创建新会话：进入 `following` 并滚到底部。
- 用户发送新消息：必须滚动到底部并进入 `following`。这是用户明确开启新一轮对话的动作，优先级高于此前的 `detached` 阅读状态。
- 用户向上滚动、触摸上滑、PageUp/ArrowUp/Home，且距离底部超过阈值：进入 `detached`。
- 用户滚动到底部阈值内：进入 `following`。
- 用户点击圆形向下箭头按钮：滚到底部并进入 `following`。
- 搜索跳转、历史加载锚点恢复：视为显式导航，不应被 streaming 的跟随逻辑覆盖。

### 阈值建议

- 底部判定阈值：80px，与当前 `useVirtualFeed` 默认值保持一致。
- 历史加载顶部阈值：保持当前主聊天 50px、Quick Chat 默认配置。
- 流式滚动：只在 `following` 状态下每帧最多滚动一次，避免每个 token 同步触发布局。

## UI 要求

- 当 `detached` 且有 streaming 输出或新内容时，在聊天列表底部上方显示一个悬浮按钮。
- 按钮使用不带横线的向下箭头图标，带 tooltip / aria-label：“回到底部”。
- 主聊天和 Quick Chat 均使用圆形 icon-only 按钮，不显示“跳到最新”可见文案。
- 按钮 hover 时显示小手光标。
- 按钮位置不能遮挡输入框、审批弹窗、ask_user 卡片和 plan card。
- 不需要在页面内解释功能；行为应符合用户直觉。

## 验收标准

1. AI 回复生成中，用户向上滚动超过 80px 后，后续 token 不会改变当前 `scrollTop` 到底部。
2. 用户离开底部后，浮动圆形向下箭头入口出现；点击后滚到底部，并恢复自动跟随。
3. 用户滚回底部阈值内后，后续 token 自动跟随。
4. 流式输出结束时，如果用户处于 `detached`，页面不会自动跳到底部。
5. 历史消息向上加载后，当前阅读锚点不丢失。
6. 搜索跳转到历史消息后，若当前会话仍在 loading，视口不会立即被 streaming 拉走。
7. 用户每次发送新消息后，视口都会自动滚动到底部，并恢复自动跟随后续 AI 输出。
8. 主聊天和 Quick Chat 均满足上述行为。
9. 不出现明显布局抖动、按钮遮挡输入区或移动端无法点击的问题。

## 测试要求

- 提取滚动状态判定为可单测的纯函数或小 hook，覆盖 `following -> detached -> following` 的状态切换。
- 为 `MessageList` 或 `useVirtualFeed` 增加回归测试：模拟 loading 中用户上滚，后续 followKey 变化不触发滚到底部。
- 为“点击圆形向下箭头按钮”增加测试：点击后调用滚到底部并恢复 following。
- 为“发送新消息”增加测试：即使当前处于 detached 状态，也会强制滚到底部并恢复 following。
- 至少运行前端类型检查；涉及测试文件时运行对应 Vitest 单点测试。

## 风险

- TanStack Virtual 的动态测量可能在 markdown、高亮代码块、图片/工具卡片高度变化时触发布局变化，需要避免误判用户意图。
- 当前 `useVirtualFeed` 同时承担虚拟列表、历史加载、锚点恢复和底部跟随，改动需要保持边界清晰，避免修复自动滚动时破坏历史加载。
- i18n 有 12 种语言，新增按钮文案需要补齐或通过现有同步脚本检查。
- Quick Chat 空间更紧，按钮样式需要单独验证。
