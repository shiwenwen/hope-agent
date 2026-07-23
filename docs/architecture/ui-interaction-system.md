# UI 交互与表面设计系统

本文是 Hope Agent 交互控件的单一真相源，统一定义空间级标题栏、表单控件、交互状态、
焦点反馈、菜单、悬浮弹层和 Tooltip。目标是在普通鼠标操作下保持原生桌面应用式的克制和
扁平，同时让键盘、高对比度和屏幕阅读器用户获得完整反馈。

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
| 强互斥分类标签 | `RadioPills variant="strong"` | 单选；支持图标、固定网格或自动换行；选中反白 |
| 多选标签 | `TogglePills` | 多选；选中使用深色反白，未选中保留中性实色底，不使用边框、阴影或额外勾选 |

业务组件不得直接使用裸 `<select>`、裸 `<input type="number">`、`Input type="number"`
或重新引入 `NativeSelect`。公共入口表达不了新语义时，应先扩展公共组件。

## 空间级标题栏

知识空间、设计空间、产物库、仪表盘、Plan 和定时任务等一级工作区共用紧凑单行标题栏：

- 固定 `h-10`，`shrink-0`，标题、可选副标题和右侧操作不得撑出第二行；
- 返回按钮位于最左侧；存在侧栏展开/收起时，该按钮紧跟返回按钮，不能散落到内容工具条；
- 标题使用紧凑字号；副标题与标题同行、允许截断，使用弱化前景色，不再占据独立行高；
- 右侧刷新、设置、创建等操作统一使用紧凑按钮，窄宽度下优先压缩或隐藏次要说明；
- 标题栏可保留固定结构分隔线，但 hover、selected、open 等状态不得改变该分隔线。

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

### 普通表单控件

`src/components/ui/control-surface.ts` 的 `FLAT_CONTROL_SURFACE_CLASS` 是唯一来源：

- `rounded-lg`；
- `border-border/60` + `bg-background/40`；
- `shadow-none`，普通状态禁止恢复 `shadow-sm`；
- Hover 仅提升背景到 `bg-muted/40`，边框保持静态 `border-border/60`；
- 禁用态使用统一 cursor 和 opacity；
- `forced-colors` 使用系统 `CanvasText` 边框。

`Input`、`Textarea`、`SelectTrigger`、`ModelSelector` 和 `NumberInput` 必须共享该 token。
业务侧只允许覆盖尺寸、宽度、排版密度、textarea resize 行为和定位，不得覆盖基础背景、
边框、阴影或圆角。普通文本、密码、日期及多行输入不再保留旧的 `border-input`、
`bg-transparent` 或 `shadow-sm` 表面。

`Input` / `Textarea` 默认使用 `surface="default"`。视觉边界由外壳承担的复合控件必须显式使用
`surface="embedded"`；该变体从组件入口整体移除背景、边框、圆角、阴影及 hover 表面，避免
仅覆盖静态 class 后仍泄漏 `hover:bg-*` 等状态。业务侧不得靠零散 Tailwind class 模拟该变体。

### 搜索框

`SearchInput` 使用独立的无边框搜索表面：

- 普通状态 `border-0`、`bg-muted/50`、`shadow-none`；
- Hover 使用 `bg-muted/70`；
- placeholder 降低对比度，不与真实内容争抢注意力；
- WebKit 原生 search cancel button 隐藏，避免与业务清除按钮重复；
- `forced-colors` 恢复 1px `CanvasText` 系统边框，防止背景被强制调色板抹平后失去边界。

搜索图标和清除按钮由业务外壳定位；不得为了放图标重新复制一套输入框表面 class。
组件内部基于 `surface="embedded"` 构建完整搜索表面，不继承普通字段的静态或 hover 表面。

### 通用 Hover 与选中反馈

- 普通容器、卡片、列表行、分段选择和工具按钮的 hover 只加深背景；禁止新增或加深
  `border` / `ring` / `shadow`，也禁止通过 `group-hover` / `peer-hover` 间接改变子元素边框；
- 控件原有的静态结构边框可以保留，但 hover、active、selected、checked 和 open 不得用边框
  变化表达状态；普通持久选中使用 `bg-secondary`，未选中 hover 使用 `bg-secondary/40`；
- 多选标签必须使用 `TogglePills`，以 `aria-pressed` 和
  `bg-primary text-primary-foreground` 深色反白表达选中，未选中使用
  `bg-secondary text-secondary-foreground` 与页面底色分层，hover 使用
  `bg-foreground/15` 保证明暗两种主题都有反馈；保留原图标，不另加勾选；
- 小型 checkbox / radio 的内部勾选标记可以使用 `bg-primary`，但选中时不叠加 primary 边框；
- 键盘焦点、`prefers-contrast` 与 `forced-colors` 的系统轮廓/边框属于可访问性反馈，不受上述
  视觉限制；错误、警告、拖拽落点等语义状态也按各自协议处理；
- 需要黑底反白的强互斥分类标签必须使用 `RadioPills variant="strong"`，不能复制 class，也不能
  把该样式扩散到普通列表、Tab、视图切换或多选筛选。

### 列表条目

首页聊天会话列表是普通列表行状态的视觉基准：

- 未选中条目 hover 使用 `bg-secondary/40`；
- 持久选中条目使用 `bg-secondary`，文字保持正常 `text-foreground`；
- 普通选中禁止使用 `bg-primary/*`、`text-primary` 或硬编码蓝色，避免把“当前项”误读为
  信息提示、链接或主要操作；
- 文件树中没有持久选中语义的文件夹只应用 hover；当前打开的文件、空间、任务或运行记录
  按上述 selected 标准显示；
- 错误、警告、未读、运行状态、危险操作和拖拽落点具有独立语义，可使用红、黄、绿或
  primary 强调色，但这些颜色只在对应状态存在时出现，不替代普通 hover/selected。

新增知识空间、定时任务、产物、设置或其他 master-detail 列表时应直接复用这组状态类；
若确需不同视觉，必须在本文“登记的例外”中说明语义和原因。

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

## 布局面板最大化动效

Canvas、文件浏览器、单文件预览、Plan、产物阅读器等从局部布局切换到应用内最大化时，
统一使用 `useFullscreenTransition`：

- 动画基于切换前后的真实 `getBoundingClientRect()` 做 FLIP，不硬编码起止坐标；
- 尺寸变化时缩放原点固定为左上角，使矩形差值与 CSS transform 坐标系一致；
- 展开和恢复必须双向平滑，恢复前重新测量 flex 布局，窗口缩放后仍回到正确位置；
- 动画期间保持正文、iframe 和滚动节点挂载，禁止为了动效复制或替换内容树；
- 统一使用 `UI_MOTION.panelSurface` 与 `UI_EASING.emphasized`；
- 遵守 `prefers-reduced-motion: reduce`，此时直接切换布局；
- 共用 `RightPanelShell` 的面板通过 `fullscreenTransitionRef` 接入，业务组件不得再复制一套
  `Element.animate` / `flushSync` 编排。

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

聊天输入区的 `chat/input/ModelPicker`、权限入口，以及设计空间首页生成器 prompt dock 内的
`ModelSelector` 是工具栏 ghost action，不是表单字段：它们保持无边框、紧凑按钮样式；
展开后的菜单仍遵守本文的浮层协议。不得把工具栏按钮强行包成全宽表单选择器，也不得用
该例外让设置页字段绕过公共表面。

Tab 有独立的层级协议，不套普通列表选中背景：公共 `TabsList` 是 `bg-muted` 容器，
`TabsTrigger` 选中恢复 `bg-background`，依靠轨道与选中面的明度差形成层级，不加阴影；不得改成
与容器接近的半透明背景。无外壳的线型 Tab（当前仅 Agent 编辑页）可使用底部 primary 强调线。
两类 Tab 都不得在 hover 时改变边框；线型 Tab 的底线只在持久选中时出现。

`RadioPills variant="strong"` 是强互斥分类标签的唯一入口：选中项使用
`bg-primary text-primary-foreground`（深色主题下仍使用对应反白 token），图标继承前景色；
未选中使用 `bg-secondary text-secondary-foreground`，hover 使用 `bg-foreground/15`；
选中前后均不得增加或改变边框。它适用于设计空间产物
类型、定时频率、导出格式/倍率、审批策略、Memory 学习模式和模型能力分类等“从并列标签中
确定一个值”的场景。Tab 使用上面的独立表面协议；页面导航、视图切换、权限等级继续使用
普通 `bg-secondary`；多选筛选继续使用普通选中背景或勾选标记，不能借强标签制造多个
并列黑块。

设计空间首页 recipe 模板卡仍按普通卡片选中规则使用 `bg-secondary`，并通过
`aria-pressed` 暴露状态；它不是强互斥分类标签。

行内改名、标签输入、复合搜索和整页源码 / 指令编辑器的视觉边界由外壳承担，内部
`Input` / `Textarea` 必须使用 `surface="embedded"`；典型入口包括
`SessionSearchBar`、`AllowlistTagInput`、项目指令 / 自动记忆编辑器及各列表行内改名。
不得在普通表单字段上复用该变体；复合控件仍须保留清晰外壳和统一焦点协议。

内嵌终端是由 xterm.js 管理 canvas、viewport 和输入层的第三方复合控件：允许导入上游
`xterm.css`，并在 `chat/terminal/terminal.css` 内使用 `.hope-terminal` 作用域补齐内部层尺寸、
主题 token 和滚动条适配。面板拖拽高度依赖运行时测量，可设置动态像素 `height`；拖拽期间
也可临时设置 `document.body` 的 `cursor` / `user-select`，结束时必须清理。该例外不允许扩展到
终端外的业务表面，也不允许在作用域样式中硬编码亮色或暗色。

## 代码审查清单

- 搜索是否使用 `SearchInput`？
- 普通下拉是否使用 Radix `Select`，而不是裸 `<select>`？
- 模型选择是否复用 `ModelSelector` / `ModelChainEditor`？
- 数字字段是否使用 `NumberInput` 或 `DeferredNumberInput`？
- 普通 `Input` / `Textarea` 是否继承公共表面，而不是局部覆盖背景、边框、阴影或圆角？
- embedded 控件是否显式使用 `surface="embedded"`，并由外壳提供边界与焦点反馈？
- 是否出现局部 `shadow-sm`、深色边框或重复的表面 class？
- 浮层是否复用公共表面和动效，关闭时是否仍保持挂载？
- 图标提示是否只使用 `IconTip`，控件是否同时拥有 `aria-label`？
- 是否保留 disabled、placeholder、空选项、键盘导航和 `forced-colors` 行为？
- 复合控件是否只显示一层焦点反馈？
- 是否误把工具栏 ghost action 当成表单字段，或反过来？
- 强互斥分类标签是否复用 `RadioPills variant="strong"`，并避免用于 Tab、视图切换或多选？
- 容器型 Tab 是否使用 `bg-background` 区分选中面，并保持无阴影？
- 一级工作区标题栏是否保持 `h-10` 单行，并把返回、侧栏开关按顺序放在最左侧？
- hover / selected / open 是否只改变背景，没有引入 `hover:border-*`、`hover:ring-*`、
  `group-hover:border-*` 或状态阴影？
- 普通列表行是否使用 `hover:bg-secondary/40` 和 `bg-secondary`，并把语义强调色限制在
  错误、警告、未读或拖拽等真实状态？
- 应用内最大化是否复用 `useFullscreenTransition`，并同时覆盖展开与恢复？

建议审查时执行：

```bash
rg -n 'NativeSelect|<select\b' src/components -g '*.tsx' -g '!**/*.test.tsx'
rg -n -U '<Input[^>]*type="number"|<input[^>]*type="number"' src/components -g '*.tsx' -g '!**/ui/number-input.tsx'
rg -n 'FLAT_CONTROL_SURFACE_CLASS' src/components/ui
rg -n 'title=' src/components -g '*.tsx'
pnpm exec vitest run src/components/ui/interaction-border-audit.test.ts
```

前两条在业务组件中应无结果；原生 DOM 只能封装在公共 UI 组件内部。`title=` 的结果应逐项
确认是否为允许的 iframe 标题，而不是悬停提示。

## 代码位置

- `src/components/ui/control-surface.ts`
- `src/components/ui/search-input.tsx`
- `src/components/ui/input.tsx`
- `src/components/ui/radio-pills.tsx`
- `src/components/ui/toggle-pills.tsx`
- `src/components/ui/selection-pill-styles.ts`
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
- `src/hooks/useFullscreenTransition.ts`
- `src/components/chat/right-panel/RightPanelShell.tsx`
- `src/lib/input-modality.ts`
- `src/lib/focus-indicator-preference.ts`
- `src/index.css`
