# Mac Control v3 Peekaboo Gap Checklist

临时清单：用于在 v3 开发期间保留 Peekaboo 对齐项，避免上下文压缩丢失。全部完成后删除本文件。

## Progress

- [x] 1. 标注截图 / UI Map 可视化
  - Peekaboo `see --annotate` 会生成带元素 ID/边框的 annotated screenshot，并持久化 `ui_map`。
  - Hope Agent 已完成：`visual.observe annotate=true`、`uiMapLimit`、标注截图、紧凑 `uiMap`、标注失败 warning 保留。

- [x] 2. 更完整的 AX action
  - Peekaboo 有 `perform-action --action AXPress/AXShowMenu/...`，可以对元素执行命名 AX action。
  - Hope Agent 现状：主要是 `click` / `set_value` / `type` / `paste` / `hotkey` / `scroll` / `drag` 等封装动作。
  - Hope Agent 已完成：`act.perform_action` + `axAction`，白名单 AX action，要求目标 `actions[]` 声明支持，已接入权限审批、schema、Tauri bridge、文档与测试。

- [x] 3. Dock / Spaces
  - Peekaboo 有 Dock 和 Space 能力：列 Dock、启动 Dock app、隐藏/显示 Dock、切换 Space、移动窗口到 Space。
  - Hope Agent 已完成：`dock.list/launch/hide/show`、`spaces.list/switch`，接入 schema、权限审批、Tauri bridge、文档与测试。
  - 已知边界：`spaces.move_window` 因 macOS 无稳定公开 API 先返回显式 unsupported；后续若找到可靠路径再单独补。

- [ ] 4. Dialog 专项能力细化
  - Peekaboo 的 dialog 能做 `list` / `click` / `input` / `file` / `dismiss`。
  - Hope Agent 现状：已有 `inspect` / `accept` / `dismiss`。
  - 缺口：文件选择器路径输入、按按钮文本点击、弹窗字段填写等高层封装。

- [ ] 5. 人类化输入/鼠标动作
  - Peekaboo 有 `press`、`swipe`、move cursor、drag with duration/steps、人类输入 delay/profile。
  - Hope Agent 现状：已有 `hotkey` / `type` / `paste` / `scroll` / `drag` / `click_point`。
  - 缺口：专门的 cursor move、swipe、平滑鼠标轨迹、带节奏 typing profile。

- [ ] 6. Web 内容聚焦 fallback
  - Peekaboo `see` 对浏览器页面没有暴露 text field 时，会尝试聚焦 dominant `AXWebArea` 后重新遍历。
  - Hope Agent 现状：还没有面向浏览器/复杂 WebView 的自动修复逻辑。

- [ ] 7. 菜单栏 popover 专项识别
  - Peekaboo 对 menubar popover 有专门路径：窗口列表 + OCR + app hint。
  - Hope Agent 现状：已有 system menu bar 菜单和 OCR，但缺“状态栏弹出面板”的专门选择策略。

## Deferred

- [ ] 8. 暂时不用
- [ ] 9. 暂时不用
- [ ] 10. 暂时不用
