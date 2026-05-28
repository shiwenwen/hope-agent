# Mac Control v3 Peekaboo Gap Checklist

临时清单：用于在 v3 开发期间保留 Peekaboo 对齐项，避免上下文压缩丢失。全部完成后删除本文件。

## Progress

- [x] 1. 标注截图 / UI Map 可视化
  - Peekaboo `see --annotate` 会生成带元素 ID/边框的 annotated screenshot，并持久化 `ui_map`。
  - Hope Agent 已完成：`visual.observe annotate=true`、`uiMapLimit`、标注截图、紧凑 `uiMap`、标注失败 warning 保留。

- [x] 2. 更完整的 AX action
  - Peekaboo 有 `perform-action --action AXPress/AXShowMenu/...`，可以对元素执行命名 AX action。
  - Hope Agent 已完成：`act.perform_action` + `axAction`，对常用别名规范化，其它合法 AX action 字符串直接尝试执行，不再依赖不可靠的 `actions[]` 广告；已接入权限审批、schema、Tauri bridge、文档与测试。

- [x] 3. Dock / Spaces
  - Peekaboo 有 Dock 和 Space 能力：列 Dock、启动 Dock app、隐藏/显示 Dock、切换 Space、移动窗口到 Space。
  - Hope Agent 已完成：`dock.list/launch/hide/show/menu/select_menu`、`spaces.list/switch/move_window`，接入 schema、权限审批、Tauri bridge、文档与测试。
  - 已知边界：`spaces.move_window` 依赖 SkyLight/CGS 私有 API；CGS 不可用或系统行为变化时会返回错误/verification warning。

- [x] 4. Dialog 专项能力细化
  - Peekaboo 的 dialog 能做 `list` / `click` / `input` / `file` / `dismiss`。
  - Hope Agent 已完成：`dialog.list/click/input/file`，其中 `list` 返回按钮与字段摘要，`click` 按按钮文本点击，`input` 支持 field/fieldIndex/elementId 填写文本字段，`file` 支持文件选择器路径/文件名输入与指定按钮确认。
  - 已知边界：`dialog.file` v1 采用 macOS Go to Folder 快捷键 + AX filename field，未实现 Peekaboo 那套 Show Details/路径字段多策略验证。

- [x] 5. 人类化输入/鼠标动作
  - Peekaboo 有 `press`、`swipe`、move cursor、drag with duration/steps、人类输入 delay/profile。
  - Hope Agent 已完成：`act.move_cursor`、`act.press`、`act.swipe`、`act.drag`/`act.move_cursor`/`act.swipe` 的 `durationMs` + `steps` + `motionProfile=linear|human` 轨迹控制，drag/swipe 坐标或 AX 元素双端点、拖拽期间 `modifiers`，以及 `act.type` 的 `typingProfile` / `typingDelayMs` 逐字符 CGEvent 输入。
  - 已知边界：默认 `act.type` 仍保持 `AXSetValue` 语义，只有显式传 typing profile/delay 才走真实键盘事件；swipe 是鼠标拖拽语义，不模拟触控板惯性手势。

- [x] 6. Web 内容聚焦 fallback
  - Peekaboo `see` 对浏览器页面没有暴露 text field 时，会尝试聚焦 dominant `AXWebArea` 后重新遍历。
  - Hope Agent 已完成：snapshot/visual/elements/act target 解析共享 AX 采集路径；当树里有 `AXWebArea` 但没有文本输入控件时，会 best-effort 聚焦面积最大的 WebArea 后重新遍历，并在 `warnings[]` 记录 fallback。
  - 已知边界：只使用 Accessibility focus，不做坐标点击 WebArea；如果应用不允许 `AXFocused`，会保留 warning 并让模型回退到视觉/OCR。

- [x] 7. 菜单栏 popover 专项识别
  - Peekaboo 对 menubar popover 有专门路径：窗口列表 + OCR + app hint。
  - 已补 `menu.popover`：遍历 all-app AX windows，结合靠近菜单栏/面板形态、状态栏 host App、`appHint` 和可选 Vision OCR 文本给候选排序。

## Deferred

- [ ] 8. 暂时不用
- [ ] 9. 暂时不用
- [ ] 10. 暂时不用

## Stability Hardening

- [x] A. Target 解析稳定层（第一块）
  - `target` 支持 `snapshotId + elementId` 锚定。
  - `elements.find` 产生的 snapshot 会进入短生命周期 cache。
  - mutation 前会用旧元素的 role/label/value/window/bounds/actions 指纹在当前 AX 树中重定位；snapshot 过期、旧 id 缺失、指纹无法唯一匹配时拒绝执行，避免 stale `el_N` 误点。

- [x] B. Observe → Act → Verify 事务层（首批）
  - `act.type/paste/set_value` 返回 AXValue verification；append 型 typing/paste 需要执行后值发生变化且包含本次文本。
  - `act.move_cursor/drag/swipe` 返回最终指针位置 verification。
  - `windows.focus/move/resize/close` 返回焦点、bounds 或窗口消失 verification。
  - 点击等无明确业务期望的动作不假装完成，仍要求调用方用 `wait/snapshot/elements.find/dialog.inspect` 做业务级确认。
- [x] C. 动作 fallback 策略统一化（首批）
  - `act.click` / dialog 按钮：优先 `AXPress`，失败且有 bounds 时回退 CGEvent 中心点点击，并在 execution 标记 fallback。
  - `menu.click`：统一为 `AXShowMenu -> AXPress -> CGEvent center click`，避免单个 AX action 不可靠时直接失败。
  - `act.type/set_value`、`dialog.input clear=true`、`dialog.file` 文件名：`AXSetValue` 失败后聚焦、Cmd+A、pasteboard replace；可验证的 act 路径继续返回 `verification`。
  - 后续仍可继续加强：视觉/OCR 目标多策略、dialog 文件面板路径字段多策略、失败回放工具。
- [x] D. 状态恢复与焦点保护加强（首批）
  - 审批前 `mac_control` focus anchor 从 App 级扩展到 focused window 级。
  - 审批通过 / AllowAlways / timeout proceed 后，执行前先按 `pid -> bundleId -> appName` 恢复 App，再按 pid-scoped window id 恢复窗口，失败时用窗口标题兜底。
  - 窗口恢复失败只写 warning，不阻断原工具执行；后续链式动作仍应按 skill 要求用 `frontmost` / fresh observe 验证。
- [x] E. 测试与回放工具（首批）
  - 新增 `diagnostics.summary/export`：返回 readiness/status、compact snapshot cache 摘要、recent errors 和当前 focus anchor。
  - `export` 会把同一份诊断 bundle 写到 `~/.hope-agent/mac-control/diagnostics/`，用于失败现场复盘；bundle 不包含截图 base64、完整 AX tree 或剪贴板原文。
- [x] F. dry_run / explain 体验层（首批）
  - `act.dry_run` 新增 `dryRunOp`，按目标真实 op 预演 target 解析，但不触发 AX action、CGEvent、键盘或剪贴板。
  - `MacControlActResult.preview` 返回 executionPlan、fallbackPlan、verificationPlan、warnings 和 nextStep；`explain=true` 可把同一 preview 附到真实 act 结果上。
