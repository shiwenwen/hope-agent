---
paths:
  - "src/**/*.{ts,tsx,css}"
---

# 前端编码规范

**性能和用户体验是最高优先级**：操作即时反馈（乐观更新 / loading 态），动效 60fps（优先 CSS transform/opacity）。

- 函数式组件 + hooks；UI 一律用 `src/components/ui/`（shadcn/ui），不用原生表单控件；样式只用 Tailwind utility class，**不写行内 style / 自定义 CSS**；别名 `@/` → `src/`
- **动效优先复用 shadcn/ui / Radix / Tailwind 内置 utility**，确认不够用才手写
- **Tooltip 必须用 [@/components/ui/tooltip](../../src/components/ui/tooltip.tsx)**（优先 `<IconTip>`），**禁止原生 `title`**——唯一例外是 markdown 绝对路径链接的 anchor（一条消息可渲染上百个），见 [prompt-system.md](../../docs/architecture/prompt-system.md)
- 保存按钮统一三态（`saving`→`saved` 绿 2s→`failed` 红 2s）；Think / Tool 流式块设 `max-height` 内滚 + 自动滚底 + 显示耗时
- 避免不必要的重渲染（`React.memo` / `useMemo` / `useCallback`）
