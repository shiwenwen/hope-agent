# 前端 / UI 编码规范

> `src/` 目录的嵌套 AGENTS.md，改前端（`src/**`）时自动生效；根规范见 [../AGENTS.md](../AGENTS.md)。

**性能和用户体验是最高优先级**：操作即时反馈（乐观更新 / loading 态），动效 60fps（优先 CSS transform/opacity）。

## 编码规范

- 函数式组件 + hooks；UI 一律用 `src/components/ui/`（shadcn/ui），不用原生表单控件；样式只用 Tailwind utility class，**不写行内 style / 自定义 CSS**；别名 `@/` → `src/`
- **动效优先复用 shadcn/ui / Radix / Tailwind 内置 utility**，确认不够用才手写
- **Tooltip 必须用 [@/components/ui/tooltip](components/ui/tooltip.tsx)**（优先 `<IconTip>`），**禁止原生 `title`**——唯一例外是 markdown 绝对路径链接的 anchor（一条消息可渲染上百个），见 [../docs/architecture/prompt-system.md](../docs/architecture/prompt-system.md)
- 保存按钮统一三态（`saving`→`saved` 绿 2s→`failed` 红 2s）；Think / Tool 流式块设 `max-height` 内滚 + 自动滚底 + 显示耗时
- 避免不必要的重渲染（`React.memo` / `useMemo` / `useCallback`）

## UI 表单控件与焦点

详见 [../docs/architecture/ui-interaction-system.md](../docs/architecture/ui-interaction-system.md)（组件路由表 / 表面 token / 焦点协议 / 登记的例外）。

- **表单控件只走公共入口**（`SearchInput` / `Input` / `Textarea` / Radix `Select` / `ModelSelector` / `ModelChainEditor` / `NumberInput` / `DeferredNumberInput` / `RadioPills`），禁止裸 `<select>`、裸 `<input type="number">`、`Input type="number"`、`NativeSelect`。
- **表面与焦点单一来源**：复用 `FLAT_CONTROL_SURFACE_CLASS`（只许覆盖尺寸 / 布局 / 排版）；组件不得自加 `focus:ring-*` / `focus:border-*`，焦点服从 `src/index.css` 全局协议。
- **hover / selected / open 只加深背景**，不动 border / ring / shadow（`hover:` 前缀由 `interaction-border-audit.test.ts` 兜底，`open` / 持久选中态无测试覆盖）。
- **新增例外必须登记**到 `ui-interaction-system.md`「登记的例外」，禁止用局部样式静默分叉。
