# UI 交互与表面设计系统

本文是 Hope Agent 交互控件的单一真相源，统一定义表单控件、焦点反馈、菜单、悬浮弹层和
Tooltip。目标是在普通鼠标操作下保持原生桌面应用式的克制和扁平，同时让键盘、高对比度
和屏幕阅读器用户获得完整反馈。

本文不定义页面信息架构、排版或业务状态颜色；Dialog、Sheet 和通知也有各自的模态协议。

## 总原则

- **语义先行**：先根据搜索、选择、数字编辑、菜单或提示语义选择公共组件，再调整尺寸。
- **表面唯一**：背景、边框、阴影、圆角和基础动效来自公共 token，业务侧不得复制整套 class。
- **状态正交**：Hover、open、selected、checked、invalid 和 focus 分别表达，不互相冒充。
- **输入方式感知**：Pointer 不强调焦点，Keyboard 保留轻量焦点，增强/高对比模式自动加重。
- **系统偏好优先**：`prefers-contrast` 和 `forced-colors` 优先于产品色与应用设置。
- **例外需登记**：确有不同语义的控件必须在本文登记，不能用局部样式静默分叉。

## 组件路由

### 关闭状态与表单控件

| 语义 | 唯一入口 | 说明 |
| --- | --- | --- |
| 搜索 | `SearchInput` | 扁平无边框搜索表面；列表、面板和设置页搜索统一使用 |
| 普通下拉 | `Select` + `SelectTrigger` | Radix Select；选项使用 `SelectContent` / `SelectItem` |
| 分组模型选择 | `ModelSelector` | Provider → Model 二级菜单；触发器复用扁平表面 |
| 模型降级链 | `ModelChainEditor` | 主模型和 fallback 的唯一编辑入口；内部复用 `ModelSelector` |
| 即时数字输入 | `NumberInput` | 保留原生 number 语义和步进按钮，但统一外观 |
| 延迟提交数字输入 | `DeferredNumberInput` | 编辑草稿，失焦或 Enter 后提交，并做 min/max 钳制 |
| 普通文本/密码 | `Input` | 普通编辑字段；不要因为视觉相似误用 `SearchInput` |
| 多行文本 | `Textarea` | 普通多行编辑字段 |

业务组件不得直接使用裸 `<select>`、裸 `<input type="number">`、`Input type="number"`
或重新引入 `NativeSelect`。公共入口表达不了新语义时，应先扩展公共组件。

### 浮层与提示

| 场景 | 统一入口 | 说明 |
| --- | --- | --- |
| 本地锚点菜单 | `FloatingMenu` | 工具栏菜单、状态详情、提及菜单、知识选择等 |
| Radix Dropdown | `DropdownMenuContent variant="floating"` | Portal、碰撞检测和键盘导航 |
| Radix Context Menu | `ContextMenuContent variant="floating"` | 右键菜单及其子菜单 |
| 表单 Select | `SelectContent` | 继承公共浮层表面与 Radix 动效 |
| 分组模型选择 | `ModelSelector` | Trigger 遵守表单标准；Provider/Model 子菜单遵守浮层标准 |
| 图标提示 | `IconTip` | 单个图标按钮的唯一提示入口 |
| 通用 Tooltip | `TooltipContent` | 截断说明或富提示；使用紧凑动效时长 |
| 模态框/抽屉 | `Dialog` / `AlertDialog` / `Sheet` | 独立模态协议，不套菜单布局 |
| 通知 | Sonner 或专用状态条 | 不伪装成菜单或 Tooltip |

## 控件表面

### 选择类控件和数字框

`src/components/ui/control-surface.ts` 的 `FLAT_CONTROL_SURFACE_CLASS` 是唯一来源：

- `rounded-lg`；
- `border-border/60` + `bg-background/40`；
- `shadow-none`，普通状态禁止恢复 `shadow-sm`；
- Hover 仅提升到 `border-border/80` + `bg-muted/40`；
- 禁用态使用统一 cursor 和 opacity；
- `forced-colors` 使用系统 `CanvasText` 边框。

`SelectTrigger`、`ModelSelector` 和 `NumberInput` 必须共享该 token。业务侧只允许覆盖尺寸、
宽度、排版密度和定位，不得覆盖基础背景、边框、阴影或圆角。

### 搜索框

`SearchInput` 使用独立的无边框搜索表面：

- 普通状态 `border-0`、`bg-muted/50`、`shadow-none`；
- Hover 使用 `bg-muted/70`；
- placeholder 降低对比度，不与真实内容争抢注意力；
- WebKit 原生 search cancel button 隐藏，避免与业务清除按钮重复；
- `forced-colors` 恢复 1px `CanvasText` 系统边框，防止背景被强制调色板抹平后失去边界。

搜索图标和清除按钮由业务外壳定位；不得为了放图标重新复制一套输入框表面 class。

### 模型与数字输入边界

设置页的全尺寸模型选择需要 Provider → Model 二级菜单，因此使用 `ModelSelector` 的 Radix
DropdownMenu，而不是普通 `Select`；弹出结构不同，但 Trigger 仍遵守同一个表面 token。
默认模型、视觉模型、fallback 和 `ModelChainEditor` 都通过它继承统一外观。

`NumberInput` 继续使用原生 `<input type="number">`，保留 `min`、`max`、`step`、
ArrowUp/ArrowDown、移动端数字键盘提示和屏幕阅读器数值语义；业务侧不得隐藏步进按钮。

Radix `SelectItem` 不允许空字符串值。继承/默认项使用内部哨兵值；无可用选项使用空 Root
value + `SelectValue` placeholder，并禁用 Trigger。

## 浮层表面与动效

- 表面唯一来源：`FLOATING_MENU_SURFACE_CLASS`。
- Radix 动效桥唯一来源：`FLOATING_MENU_RADIX_MOTION_CLASS` / `.ha-radix-menu-motion`。
- 标准表面：`rounded-floating`、`border-border-soft`、`bg-surface-floating/95`、
  `shadow-floating`、`backdrop-blur-xl`。
- 标准菜单进入 220ms、退出 180ms；方向由锚点或 Radix `data-side` 决定。
- Tooltip 使用同一视觉体系，但采用 120ms / 100ms 的紧凑进入/退出时长。
- `default` 变体只用于明确需要高密度旧式菜单的场景；产品级交互使用 `floating`。

### 生命周期红线

- 使用 `FloatingMenu` 时不得在父组件写 `open && <FloatingMenu ...>` 或关闭时直接
  `return null`，否则退场动画无法执行。组件应保持挂载，只通过 `open` 控制状态。
- 动态坐标浮层使用 `strategy="fixed"` + `portal` + `style={{top,left}}`；关闭阶段保留
  最后一次有效坐标和内容，避免退场时读取 `null`。
- Portal-backed Radix 菜单不得在业务侧复制表面 class，应选择公共 `floating` 变体。
- 业务侧只覆盖尺寸、最大高度、内边距和定位方向，不覆盖背景、边框、阴影及基础动效。

## Tooltip 与可访问名称

- `IconTip` 是图标按钮提示的唯一入口；不得同时保留原生 `title`，否则会显示双重提示。
- 截断文本、动态状态和禁用原因使用 `data-ha-title-tip`，由 `TooltipProvider` 的单例委托桥渲染。
- 交互控件必须有自身 `aria-label`；Tooltip 不是可访问名称的替代品。
- 生产 JSX 禁止原生悬停 `title`，仅 iframe 的无障碍标题例外。
- Tooltip 只承载补充说明，完成任务所必需的信息不能只在 Hover 后出现。

## 焦点可见性

### 状态模型

- `html[data-input-modality="pointer"]`：鼠标或触摸是最近输入方式，不画焦点轮廓。
- `html[data-input-modality="keyboard"]`：Tab 或非文本控件键盘交互，画轻量焦点轮廓。
- `html[data-focus-indicators="enhanced"]`：用户手动开启增强提示，所有输入方式都画增强轮廓。
- `prefers-contrast: more` / `forced-colors: active`：系统偏好优先，自动增强。

鼠标聚焦文本框后输入文字、移动光标或使用编辑快捷键（包括打开搜索）不会切换到 Keyboard；
文本框内的 Tab 和非文本控件上的键盘交互仍会切换。键盘用户通过 Tab 进入文本框时已经处于
Keyboard，因此编辑期间会持续保留焦点提示。
运行时只在 `src/main.tsx` 安装一次，因此主窗口、Quick Chat 和分离窗口行为一致。首屏偏好
读取有 2 秒上限；后端无响应时回退普通自动模式，不阻塞窗口挂载。

### 控件契约

- 原生交互元素和常用 ARIA role 由 `src/index.css` 统一覆盖。组件不得自行添加
  `focus:ring-*`、深色 `focus:border-*` 或另一套 outline。
- 全局焦点规则刻意保持为非分层 CSS，以覆盖历史 Tailwind `focus:outline-none`；不得移回
  `@layer base`。
- Hover、active、selected、checked 和菜单当前项继续使用背景或颜色，不用焦点框表达。
- CodeMirror 等复合编辑器在外壳标记 `data-focus-scope`，内部实际焦点节点标记
  `data-focus-ring="none"`，保证只画一层轮廓。
- 菜单项和 option 在普通键盘模式使用背景高亮；非 ARIA 菜单项使用 `ha-focus-item`；
  增强/高对比模式增加 1px 内描边。
- 原生 disabled 和 `aria-disabled="true"` 控件不绘制焦点提示。
- `forced-colors` 使用系统 `Highlight`，不得用产品色覆盖用户的强制调色板。

### 持久化与跨运行模式

`AppConfig.enhanced_focus_indicators` 是手动增强开关，默认关闭。桌面通过 Tauri 命令、
Web GUI 通过 `/api/config/enhanced-focus-indicators` 读写；两者都通过
`config:changed { category: "focus_indicator" }` 热更新现有窗口。对话式设置通过
`ha-settings` 的 `focus_indicator.enhancedFocusIndicators` 读取和修改，风险级别为 low。

## 登记的例外

聊天输入区的 `chat/input/ModelPicker` 和权限入口是工具栏 ghost action，不是表单字段：
它们保持无边框、紧凑按钮样式；展开后的菜单仍遵守本文的浮层协议。不得把工具栏按钮
强行包成全宽表单选择器，也不得用该例外让设置页字段绕过公共表面。

## 代码审查清单

- 搜索是否使用 `SearchInput`？
- 普通下拉是否使用 Radix `Select`，而不是裸 `<select>`？
- 模型选择是否复用 `ModelSelector` / `ModelChainEditor`？
- 数字字段是否使用 `NumberInput` 或 `DeferredNumberInput`？
- 是否出现局部 `shadow-sm`、深色边框或重复的表面 class？
- 浮层是否复用公共表面和动效，关闭时是否仍保持挂载？
- 图标提示是否只使用 `IconTip`，控件是否同时拥有 `aria-label`？
- 是否保留 disabled、placeholder、空选项、键盘导航和 `forced-colors` 行为？
- 复合控件是否只显示一层焦点反馈？
- 是否误把工具栏 ghost action 当成表单字段，或反过来？

建议审查时执行：

```bash
rg -n 'NativeSelect|<select\b' src/components -g '*.tsx' -g '!**/*.test.tsx'
rg -n -U '<Input[^>]*type="number"|<input[^>]*type="number"' src/components -g '*.tsx' -g '!**/ui/number-input.tsx'
rg -n 'FLAT_CONTROL_SURFACE_CLASS' src/components/ui
rg -n 'title=' src/components -g '*.tsx'
```

前两条在业务组件中应无结果；原生 DOM 只能封装在公共 UI 组件内部。`title=` 的结果应逐项
确认是否为允许的 iframe 标题，而不是悬停提示。

## 代码位置

- `src/components/ui/control-surface.ts`
- `src/components/ui/search-input.tsx`
- `src/components/ui/input.tsx`
- `src/components/ui/number-input.tsx`
- `src/components/ui/deferred-number-input.tsx`
- `src/components/ui/select.tsx`
- `src/components/ui/model-selector.tsx`
- `src/components/ui/model-chain-editor.tsx`
- `src/components/ui/floating-menu.tsx`
- `src/components/ui/animated-presence.tsx`
- `src/components/ui/dropdown-menu.tsx`
- `src/components/ui/context-menu.tsx`
- `src/components/ui/tooltip.tsx`
- `src/lib/input-modality.ts`
- `src/lib/focus-indicator-preference.ts`
- `src/index.css`
