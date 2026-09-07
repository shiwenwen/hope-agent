# 前端 / UI 编码规范

适用于 `src/**`，修改前读取；根规范见 [../AGENTS.md](../AGENTS.md)。组件入口与已登记例外以 [UI 交互规范](../docs/architecture/core/ui-interaction-system.md) 为准。

操作提供即时反馈（乐观更新 / 加载状态）；动效以 60fps 为目标，优先 CSS `transform` / `opacity`。

## 编码规范

- 函数式组件 + hooks，别名 `@/` → `src/`。业务表单复用 `src/components/ui/`（shadcn/ui），不绕过公共控件另造原生表单；公共控件内部可封装原生元素。
- 静态视觉样式用 Tailwind 与共享设计变量；不在业务组件另建视觉体系。浮层坐标、运行时尺寸可用动态 `style`；终端等第三方组件按已登记例外使用作用域 CSS。
- **动效优先复用 shadcn/ui / Radix / Tailwind 内置 utility**，确认不够用才手写
- **悬浮提示统一走 [tooltip](components/ui/tooltip.tsx)**：图标按钮用 `IconTip`，文本 / 动态提示用 `data-ha-title-tip` 的共享委托桥，控件另设可访问名称。Markdown 链接也不使用原生悬停 `title`，不逐链接增加 `TooltipTrigger`；iframe 的无障碍 `title` 保留。
- 保存按钮统一三态（`saving`→`saved` 绿 2s→`failed` 红 2s）；Think / Tool 流式块设 `max-height` 内滚 + 自动滚底 + 显示耗时
- 沿用现有流式渲染与组件状态边界；仅在稳定引用或性能证据需要时增加记忆化，不批量添加 `memo` / `useMemo` / `useCallback`。

## UI 表单控件与焦点

详见 [../docs/architecture/core/ui-interaction-system.md](../docs/architecture/core/ui-interaction-system.md)（组件路由表 / 表面 token / 焦点协议 / 登记的例外）。

- **表单控件只走公共入口**（`SearchInput` / `Input` / `Textarea` / Radix `Select` / `ModelSelector` / `ModelChainEditor` / `NumberInput` / `DeferredNumberInput` / `RadioPills` / `TogglePills`），禁止裸 `<select>`、裸 `<input type="number">`、`Input type="number"`、`NativeSelect`。
- **表面与焦点单一来源**：复用 `FLAT_CONTROL_SURFACE_CLASS`（只许覆盖尺寸 / 布局 / 排版）；组件不得自加 `focus:ring-*` / `focus:border-*`，焦点服从 `src/index.css` 全局协议。
- **悬停 / 选中 / 展开只加深背景**，不动 border / ring / shadow；拖拽手柄等按已登记例外处理。`interaction-border-audit.test.ts` 覆盖悬停边框及部分选中 / 展开样式，不替代交互验证。
- **新增例外必须登记**到 `ui-interaction-system.md`「登记的例外」，禁止用局部样式静默分叉。
