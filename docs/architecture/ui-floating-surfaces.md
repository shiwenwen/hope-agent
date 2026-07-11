# UI 浮层标准

本文定义前端轻量浮层的唯一视觉与动效协议。输入区 `ModelPicker` 是标准参考实现；新增菜单不得在业务组件中重复拼接背景、阴影和动画 class。

## 分类与入口

| 场景 | 统一入口 | 说明 |
|---|---|---|
| 本地锚点菜单 | `FloatingMenu` | 工具栏菜单、状态详情、提及菜单、知识选择等 |
| Radix Dropdown | `DropdownMenuContent variant="floating"` | 需要 Portal、碰撞检测、键盘导航的菜单 |
| Radix Context Menu | `ContextMenuContent variant="floating"` | 右键菜单及其子菜单 |
| 表单 Select | `SelectContent` | 全局继承浮层表面与 Radix 动效，不由业务侧覆盖 |
| Tooltip | `TooltipContent` | 使用同一表面与运动曲线，但采用 120ms / 100ms 的紧凑时长 |
| 模态框 / 抽屉 | `Dialog` / `AlertDialog` / `Sheet` | 独立模态协议，不套用菜单布局 |
| 通知 | Sonner 或专用状态条 | 不伪装成菜单 |

## 视觉与动效契约

- 表面唯一来源：`FLOATING_MENU_SURFACE_CLASS`。
- Radix 动效桥唯一来源：`FLOATING_MENU_RADIX_MOTION_CLASS` / `.ha-radix-menu-motion`。
- 标准菜单表面：`rounded-floating`、`border-border-soft`、`bg-surface-floating/95`、`shadow-floating`、`backdrop-blur-xl`。
- 标准菜单动效：进入 220ms，退出 180ms；方向根据锚点或 Radix `data-side` 决定。
- `default` 变体只用于明确需要高密度旧式紧凑菜单的场景；产品级交互使用 `floating`。

## 生命周期红线

- 使用 `FloatingMenu` 时不得在父组件写 `open && <FloatingMenu ...>` 或关闭时直接 `return null`，否则退场动画无法执行。组件应保持挂载，通过 `open` 控制显示。
- 动态坐标浮层使用 `strategy="fixed"` + `portal` + `style={{top,left}}`；关闭阶段必须保留最后一次有效坐标和内容，避免退场时读到 `null`。
- Portal-backed Radix 菜单不得在业务侧复制整套表面 class；应选择公共组件的 `floating` 变体。
- 业务侧只覆盖尺寸、最大高度、内边距和定位方向，不覆盖背景、边框、阴影及基础动效。

## 代码位置

- `src/components/ui/floating-menu.tsx`
- `src/components/ui/animated-presence.tsx`
- `src/components/ui/dropdown-menu.tsx`
- `src/components/ui/context-menu.tsx`
- `src/components/ui/select.tsx`
- `src/components/ui/tooltip.tsx`
- `src/index.css`
