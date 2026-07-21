---
paths:
  - "src/**/*.{ts,tsx,css}"
---

# UI 表单控件与焦点

详见 [ui-interaction-system.md](../../docs/architecture/ui-interaction-system.md)（组件路由表 / 表面 token / 焦点协议 / 登记的例外）。

- **表单控件只走公共入口**（`SearchInput` / `Input` / `Textarea` / Radix `Select` / `ModelSelector` / `ModelChainEditor` / `NumberInput` / `DeferredNumberInput` / `RadioPills`），禁止裸 `<select>`、裸 `<input type="number">`、`Input type="number"`、`NativeSelect`。
- **表面与焦点单一来源**：复用 `FLAT_CONTROL_SURFACE_CLASS`（只许覆盖尺寸 / 布局 / 排版）；组件不得自加 `focus:ring-*` / `focus:border-*`，焦点服从 `src/index.css` 全局协议。
- **hover / selected / open 只加深背景**，不动 border / ring / shadow（`hover:` 前缀由 `interaction-border-audit.test.ts` 兜底，`open` / 持久选中态无测试覆盖）。
- **新增例外必须登记**到 `ui-interaction-system.md`「登记的例外」，禁止用局部样式静默分叉。
