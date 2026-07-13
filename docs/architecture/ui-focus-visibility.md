# UI 焦点可见性

Hope Agent 的焦点反馈按输入方式分层，目标是在鼠标操作时保持桌面应用式的克制，
同时让键盘和高对比度用户始终能定位当前控件。

## 状态模型

- `html[data-input-modality="pointer"]`：鼠标或触摸是最近输入方式，不画焦点轮廓。
- `html[data-input-modality="keyboard"]`：Tab、快捷键或非文本控件的键盘交互，画轻量焦点轮廓。
- `html[data-focus-indicators="enhanced"]`：用户手动开启增强提示，所有输入方式都画增强轮廓。
- `prefers-contrast: more` / `forced-colors: active`：系统偏好优先于应用设置，自动增强。

文本框经鼠标聚焦后输入文字不会切换到 keyboard；Tab 和带修饰键的快捷键仍会切换。
运行时只在 `src/main.tsx` 安装一次，因此主窗口、Quick Chat 和分离窗口行为一致。
首屏偏好读取有 2 秒上限；后端无响应时回退普通自动模式，不阻塞窗口挂载。

## 控件契约

- 原生交互元素和常用 ARIA role 由 `src/index.css` 统一覆盖，组件不得自行添加
  `focus:ring-*`、深色 `focus:border-*` 或另一套 outline。全局规则刻意保持为非分层 CSS，
  以覆盖历史 Tailwind `focus:outline-none`；不得把它移回 `@layer base`。
- hover、active、selected、checked 和菜单当前项是独立状态，可以继续使用背景或颜色反馈。
- CodeMirror 等复合编辑器在外壳标记 `data-focus-scope`，内部实际焦点节点标记
  `data-focus-ring="none"`，确保只画一层轮廓。
- 菜单项和 option 在普通键盘模式使用已有背景高亮；非 ARIA 菜单项使用 `ha-focus-item`
  参与同一规则；增强/高对比模式增加 1px 内描边。
- 原生 disabled 和 `aria-disabled="true"` 控件不绘制焦点提示。
- `forced-colors` 使用系统 `Highlight`，不得用产品色覆盖用户的强制调色板。

## 持久化与跨运行模式

`AppConfig.enhanced_focus_indicators` 是手动增强开关，默认关闭。桌面通过 Tauri 命令、
Web GUI 通过 `/api/config/enhanced-focus-indicators` 读写，两者都以
`config:changed { category: "focus_indicator" }` 热更新现有窗口。
