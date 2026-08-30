# 对话内分栏工作台（Docked Workbench）

> 返回 [文档索引](../../README.md) · 更新时间 2026-08-15
>
> 关联源码：[`src/components/chat/workbench/`](../../../src/components/chat/workbench/)、[`ChatScreen.tsx`](../../../src/components/chat/ChatScreen.tsx)、[`ChatTitleBar.tsx`](../../../src/components/chat/ChatTitleBar.tsx)、[`useFilePreview.ts`](../../../src/components/chat/files/useFilePreview.ts)、[`RightPanelShell.tsx`](../../../src/components/chat/right-panel/RightPanelShell.tsx)

## 1. 产品心智与边界

主聊天采用「左边持续对话，右边持续工作」的会话级布局。右侧内容不是覆盖聊天的普通浮层，也不是只能显示一个入口的互斥卡片，而是占据真实布局宽度的分栏工作台：

- 对话和工作台通过 `flex` 分栏并列；工作台打开时会挤压对话，不覆盖对话。
- 现有 `h-10` 顶部行是唯一顶层标题区。左段承载会话身份和会话操作，右段承载工作台标签和工作台操作；不增加第二条应用标题栏。
- Files、文件预览、Workspace、Plan、Diff、Pull Request、Browser、Mac Control、Canvas、Team、Background Jobs 和 Subagents 共用同一标签轨道。
- 底部 Terminal 保持独立 dock 和既有多 PTY 标签；工作台开关与切页不得重建或关闭终端。
- Browser / Mac Control 的显式悬浮镜像，以及 Files / Canvas 的桌面独立窗口仍保留；它们是用户主动选择的显示模式，不是普通右侧内容的默认承载方式。
- 完整 [`WorkspacePanel`](workspace.md) 保留为 `workspace` 标签。顶部环境卡只是高频会话信息投影，不能替代或删减完整 Workspace。

工作台只管理布局、标签身份、活动项和视图生命周期。文件、Plan、Browser、Canvas、后台任务等业务数据仍由原有 hook、store、Transport 与后端控制面拥有，工作台不是第二真相源。

## 2. 布局结构

```text
┌──────────────┬────────────────────────────┬──────────────────────────────┐
│ 会话侧栏      │ 对话标题段                  │ 工作台标签段                  │
│              ├────────────────────────────┼──────────────────────────────┤
│ sessions     │ messages + composer        │ active tab content           │
│ projects     │ environment projection     │ optional inner navigation    │
├──────────────┴────────────────────────────┴──────────────────────────────┤
│ Terminal dock（可选；多 PTY；隐藏不杀 shell）                            │
└─────────────────────────────────────────────────────────────────────────┘
```

[`ChatScreen`](../../../src/components/chat/ChatScreen.tsx) 在会话侧栏之后建立一个可测量的内容容器。容器下方的对话列与 [`WorkbenchSurface`](../../../src/components/chat/workbench/WorkbenchSurface.tsx) 使用同一条分隔线；顶部 [`ChatTitleBar`](../../../src/components/chat/ChatTitleBar.tsx) 使用同一个工作台宽度，因此标签段边界与内容分栏边界严格对齐。

工作台收起只把 surface 宽度变成零并设置 `inert`，不关闭标签。重新展开恢复原标签集合与活动项。关闭某个标签才执行该业务面的关闭逻辑；关闭最后一个标签后工作台退出布局。

surface 内每个已打开面板各自挂一个 `integrated` 的 [`RightPanelShell`](../../../src/components/chat/right-panel/RightPanelShell.tsx)，全部 `absolute inset-0` 叠在一起、只靠 `collapsed` 区分。因此**非活动的 shell 绝不能再画自己的不透明底色**：它在 DOM 里排在谁之后就会盖住谁，被盖的活动面板表现为一整片空白（body 已经 `opacity-0`，遮挡来自 shell 外层）。同理，新增面板时别给外层加恒定 `bg-*`，底色由 `WorkbenchSurface` 统一负责。

## 3. 标签模型

### 3.1 当前类型

[`types.ts`](../../../src/components/chat/workbench/types.ts) 定义两层身份：

```ts
interface WorkbenchTabItem {
  id: string // 唯一标签 id；文件预览按资源生成
  panelId: WorkbenchPanelId // 归属的业务 surface
  labelKey?: string
  label?: string
  icon: LucideIcon
  fileIcon?: { name: string; mime?: string | null } // 文件预览标签的格式图标
  open: boolean
  badge?: WorkbenchBadge
  windowMode?: "docked" | "floating" | "detached"
}
```

业务面用 `icon`（Lucide 单色图标）。文件预览标签改走 `fileIcon`，由
[`FileTypeIcon`](../../../src/components/icons/FileTypeIcon.tsx) 渲染与 Files 树、消息附件同一套彩色格式图标；扩展名未知时回落到 target 携带的 MIME。

单例业务面使用 `id === panelId`。两类可多开的标签各有自己的 id 空间，都共享一个 `panelId`：

| 标签 | id | 内容 | 由谁产生 |
| --- | --- | --- | --- |
| 文件浏览器 | `files:<n>` | 完整 `FileBrowserPanel`（左树 + 右预览） | 工作台 `+` 菜单、树右键「在新标签中打开」、对话里打开工作目录内的文件 |
| 纯预览 | `preview:<encoded identity>` | 无树的 `FilePreviewPanel` | 树寻址不到的资源：消息附件、Canvas / Artifact、知识笔记、编辑器草稿 |

当前 `WorkbenchPanelId` 包含：

- `workspace`
- `pull-request`
- `diff`
- `plan`
- `files`
- `browser`
- `mac-control`
- `canvas`
- `team`
- `background-jobs`
- `subagent`
- `preview`

### 3.2 打开与复用

**选文件永远不建标签（红线）**：在树里单击一个文件只更新**当前这个**文件标签的右侧预览。新标签只来自用户的显式动作——树右键「在新标签中打开」，或工作台 `+` 菜单里的「文件」。这样连点十个文件不会淹没标签轨道，也不会让 dirty 编辑视图被隐式替换掉。给树传 `onPreviewFile` 会把选中的文件冒泡给宿主再开一个标签，**split 布局的宿主必须留空**（它自己的右栏就是预览位）；该 prop 只留给没有右栏的 stacked 宿主。

- 单例面板重复打开时激活原标签，不创建副本。
- 对话侧的所有「打开这个文件」入口（Markdown 文件链接、消息附件、Workspace Output、Diff 文件）统一走 ChatScreen 的 `openFileTarget`，它按能否被树寻址分流：
  - 落在 `effectiveWorkingDir` 或某个项目源目录内（最长前缀匹配；workspace target 还要求 scope 与当前 Files 面板一致）→ 在**活动文件标签**里 reveal + 选中，没有活动标签就新建一个。
  - 其余（附件、Artifact、笔记、草稿）→ `filePreview.openPreview(target)` 开纯预览标签，因为没有可选中的树行。
- 纯预览的去重键由稳定 `FileTarget` provenance 构造：workspace 使用 `scope + scopeId + relPath`，session path 使用 `sessionId + path`，Knowledge 使用 `kbId + path`，Artifact 使用 opaque artifact id，客户端草稿使用 draft id；再次打开同一资源只刷新 target 并激活原标签。
- 每个文件标签挂一个独立的 `FileBrowserPanel`，各自持有自己的选中项、展开状态与 reveal 队列（`useFileTabs` 的 `FileTabEntry`）。切换标签只切 `collapsed`，保留渲染器、滚动与本地选择状态。
- 标签标题与图标跟随该标签当前选中的文件（`onSelectionChange` 回报），未选中时回落「文件」+ 文件夹图标。
- 文件相对路径始终绑定选择时的 workspace scope；会话、项目或 root 变化不会把旧路径重新解释到新 root。
- 每个文件标签的独立窗口用带 tab id 的 Tauri window label，**不能共用一个固定 label**（否则第二个标签的 detach 会静默失败到第一个的窗口上）。

### 3.3 标签交互

[`WorkbenchHeader`](../../../src/components/chat/workbench/WorkbenchHeader.tsx) 提供：

- 单击激活；中键或关闭按钮关闭。
- 原生拖拽重排；顺序按会话保存在 renderer 生命周期内。拖拽必须 `setData` 才能在 WebKit / Gecko 里真正开始，并且**必须有 `onDragEnd` 兜底清理**——取消的拖拽留下的 source 会让之后任意一次 drop（包括从系统拖进来的文件）静默重排。
- roving focus：左右键移动焦点，Enter / Space 激活，Delete 关闭。
- `Alt+Shift+Left/Right` 可用键盘重排；**重排在两端停住不回绕**（焦点移动才回绕），否则一次「往左一格」会把首标签甩到末尾。
- 活动标签自动滚入可见范围；标签过多时标签轨道横向滚动。
- `+` 菜单打开当前上下文可用的工作台面。
- 右上角统一控制整个工作台的最大化 / 恢复和收起；最大化不是当前内容组件的私有状态。
- Browser / Mac Control 标签带窗口模式按钮，在 docked 与 floating 间切换；浮动后标签仍留在轨道中，**点它必须先 dock 回来**再激活，否则激活的是别的面板。
- 工作台完全没有打开标签时，顶部工作台入口直接展示同一启动菜单；收起但仍有标签时，入口一键恢复原工作台。
- **标签轨道与工作台主体同一个开关**：只剩浮动镜像时没有 docked 面板，轨道整条不渲染（否则会留下一条底下什么都没有、收起按钮还会被复位 effect 立刻撤销的空条），此时由启动入口负责把镜像 dock 回来。

标签轨道与 [`ChatTitleBar`](../../../src/components/chat/ChatTitleBar.tsx) 的左半区共同占满标题栏那一行，因此**两者都必须自带 `data-tauri-drag-region`**：Tauri 只认直接接收 mousedown 的那个元素，父级有属性并不会传给子容器，漏标即标签旁的空白无法拖动窗口、也无法双击最大化。

Workspace、Workflow、Background Jobs 和 Subagents 的 badge 继续读取各自真相源。需要处理的状态用 amber，运行态用蓝色；工作台收起后，入口聚合仍可见 badge。

### 3.4 两级标题与工具栏归属

标签存在以后，所有内容都不得机械地保留或机械地删除内部标题行。判断标准是该行是否承载**当前内容上下文或内容专属操作**：

| 类型 | 集成态行为 | 例子 |
| --- | --- | --- |
| frame chrome | 移到顶层工作台，内部删除 | 重复的面板名称、关闭、面板最大化、Browser / Mac float |
| content toolbar | 保留 | 文件路径 / 文件操作、PR 标题与分支、Diff hunk / layout、Plan 版本、Canvas 类型与刷新、Browser URL / 刷新、Subagent 返回 / 状态 / 取消 |
| 纯容器标题 | 删除 | Workspace、Background Jobs、Subagent 列表的重复标题；Team 的重复关闭按钮 |

因此 Files 在工作台内只保留 `FileBrowserView` 的文件工具栏；宿主层的重复 `Files + close + maximize` 行不再渲染，独立窗口入口追加到文件工具栏。文件 Preview 保留文件名 / 路径及打开、下载、编辑、引用等操作，但关闭与最大化由顶层标签和工作台负责。

split 布局的左侧文件列表可整列收起（工具栏的 `PanelLeftClose`）。收起后**必须留一条带展开按钮的窄轨**——展开入口本来就长在被收起的那一列里，只把列宽变成 0 会把它一起藏掉、再也回不来。窄轨同时要带上宿主注入的 `toolbarTrailing`：集成态下「独立窗口」只有文件工具栏这一个入口，跟着列一起收掉就没了。

预览头的路径行是可点面包屑（[`FilePathBreadcrumb`](../../../src/components/chat/files/FilePathBreadcrumb.tsx)，纯拆分逻辑在 [`filePathSegments.ts`](../../../src/components/chat/files/filePathSegments.ts)）：目录段跳到 Files 面板并展开选中该目录，文件名段复制完整路径。**可达性由宿主判定、不可留死链接**——ChatScreen 把绝对路径按最长前缀匹配到 `effectiveWorkingDir` 或项目源目录，workspace 目标则要求 scope 与当前 Files 面板一致；解析不出来的段渲染成纯文本。同一条解析链服务「在新标签中打开」，因此它**必须复用 `openFileTarget` 的预览兜底**：多源文件夹或 worktree 根下的文件解析不成 browsable，没有兜底就是一次完全无反馈的点击。解析结果还要带上 `revealLines`，否则从 Diff 跳行会打开文件却不定位、不高亮。URL / `blob:` / `data:` 一类不透明标识不走面包屑。Diff、PR、Plan、Canvas、Browser、Mac Control 与 Subagent detail 同理保留自己的内容语义行。

独立窗口不是普通 close / maximize：它改变资源承载窗口并有自己的生命周期，所以 Files / Plan / Canvas 仍可从内容工具栏 detach / reattach；Browser / Mac Control 的轻量浮动属于工作台布局模式，入口放在标签上。

## 4. 尺寸与响应式

尺寸算法只读取会话侧栏之后的实际内容宽度 `L`，实现见 [`useWorkbenchSizing.ts`](../../../src/components/chat/workbench/useWorkbenchSizing.ts)。

| Token                     |      值 | 含义                                       |
| ------------------------- | ------: | ------------------------------------------ |
| `CHAT_IDEAL_MIN`          |  560 px | 对话列的**理想**下限（卡片打开时另加通道） |
| `CHAT_HARD_MIN`           |  360 px | 对话列绝对下限，stage 阈值用               |
| `WORKBENCH_IDEAL_MIN`     |  560 px | 工作台的**理想**下限                       |
| `WORKBENCH_MIN`           |  420 px | 工作台绝对下限，低于此即收起               |
| `WORKBENCH_MAX`           | 1280 px | 超宽屏绝对上限                             |
| `WORKBENCH_DEFAULT_RATIO` |     0.5 | 未拖拽时的分配比例（对半分）               |

`resolveWorkbenchLayout({ available, ratio, chatIdeal })` 是**两栏宽度与「该收起了」的唯一裁决**：

```ts
if (L < chatIdeal + WORKBENCH_MIN) return { collapse: true }
upper = min(WORKBENCH_MAX, L - chatIdeal) // 对话列的理想永远优先扣除
lower = L >= chatIdeal + WORKBENCH_IDEAL_MIN ? WORKBENCH_IDEAL_MIN : WORKBENCH_MIN
width = clamp(round(L * ratio), min(lower, upper), upper)
```

`lower` 这一项就是「两栏同时缩」的实现：只要两边都还撑得住理想下限，工作台的下限就是 `WORKBENCH_IDEAL_MIN`，`ratio` 在这个区间里自由分配；一旦 `L` 掉到两个理想之和以下，下限才松到 `WORKBENCH_MIN`，于是**只有工作台继续变窄**、对话列稳在自己的理想值。

**手动宽度必须存成比例而不是像素**（`hope.chat.workbench.manualRatio`）。存像素的话窗口一变窄工作台纹丝不动，收缩全由对话列买单——用户看到的就是「聊天区一路被压，右侧一直很大，然后突然消失」。同理，比例也不能给工作台更高的默认值（曾用 0.68），否则宽屏上它一直顶着 `WORKBENCH_MAX`，效果一样。拖拽落点先过一遍上面同一个函数再换算成比例存盘，所以手动值同样守两条理想下限；双击分隔线恢复 auto。

`widthMode` 与该比例保存在窗口级 localStorage，不进入 `AppConfig`。

### 收缩让位顺序

窗口变窄时按固定顺序放弃空间。前三步都由 `resolveWorkbenchLayout` 在实测的 `L` 上裁决，第四步归 ChatScreen（侧栏贴着窗口左缘，只能按 viewport 量）：

1. **两栏同时缩**，直到各自的理想下限。
2. **卡片先让**：会话信息卡片浮在对话列右缘，打开时它的通道要计进对话列的理想下限（`chatIdeal = CHAT_IDEAL_MIN + lane`）。`L` 不足 `CHAT_IDEAL_MIN + lane + WORKBENCH_IDEAL_MIN` 时先关掉卡片——它腾出的空间立刻回到两栏，所以两栏会一起变宽一截。
3. **工作台单独让**：从 `WORKBENCH_IDEAL_MIN` 一路缩到 `WORKBENCH_MIN`，再窄就**自动收起工作台**（不是进 stage）。
4. **最后动侧栏**：先连续压到 `CHAT_SIDEBAR_MIN_WIDTH`，压不动了再自动收起。挤压只是**渲染宽度**（`renderedWidth`），拖拽仍以存储的偏好 `panelWidth` 为基准——拿被挤压后的值当基准，窄窗口里随手一拖就把偏好永久改小了。

`cardFits` 刻意**不依赖卡片当前是否打开**，否则窄窗口里点开卡片会在同一次点击里把自己关掉。窄窗口下用户仍可手动打开，此时卡片直接盖在正文上（见 [`environmentInset.ts`](../../../src/components/chat/environmentInset.ts)）。

自动与手动必须分开记账：只有自动收起的才允许自动展开（`autoCollapsedRightPanelRef`），窄屏下用户手动展开工作台要能扛住下一次 resize（`manualRightPanelExpandedOverrideRef`），此时由 stage 负责呈现。收起/展开与侧栏两级都带 `RESPONSIVE_PANEL_HYSTERESIS` 回滞，避免在阈值上抖动。

### Stage

自动收起之后用户仍可手动展开工作台。此时 `L < WORKBENCH_MIN + CHAT_HARD_MIN` 就进入 `stage`：工作台占满会话内容舞台，对话节点保持挂载但隐藏并 `inert`。关闭 / 收起工作台即返回对话。退出 stage 需要比进入阈值多 80 px，避免窗口动画和滚动条变化导致来回抖动。

### 工作台最大化

最大化是 workbench frame 的统一状态，不再由 Files、Preview、Plan、Canvas 等集成内容各自覆盖窗口。最大化时：

- `WorkbenchHeader` 固定在窗口顶端，macOS overlay 区保留 28 px，标签和统一控制仍在唯一顶部行。
- `WorkbenchSurface` 覆盖 header 下方的全部应用内容，活动标签保持原组件实例；其它标签继续 warm mount + `inert`。
- resize handle 暂停渲染，恢复后回到原 docked 宽度；按 `Escape`（**已被弹层消费的按键不算**）或右上角恢复按钮退出。
- **收起前一律先退出最大化**，响应式自动收起也不例外：留着 `maximized` 会让下次展开变成盖住整屏的浮层而不是原来的分栏；同理手动最大化要记 `manualRightPanelExpandedOverrideRef`，否则下一次 resize tick 会把它自动收走。
- 收起、关闭最后一个标签或切换会话都会先退出最大化，不能把 frame 状态泄漏给下一会话。

### 分隔线

工作台分隔线的定位祖先是「标题栏 + 分栏区」那一层，**不含底部 Terminal dock**：dock 是通栏的，把 handle 的 `inset-y-0` 挂到整个会话容器上会让线从终端中间穿过去。新增通栏底部区域时同理——放在这层之外。

列边界本身是 1 px `border-border-soft` 结构线，**任何时候都在**；拖拽反馈另有一层视觉宽度 1 px、命中宽度 10 px 的 [`ResizeHandleGlow`](../../../src/components/ui/resize-handle-glow.tsx) 叠在它上面，idle 完全透明，只有 hover、键盘 focus 或正在拖拽时才显示细蓝色光晕。两者职责分开：**别用「有拖拽手柄」当作省掉结构线的理由**（Files 树右缘曾因此看不到边界）。拖拽以 rAF 合并宽度更新，拖拽期间临时关闭 iframe pointer events，并在 pointer up、cancel、窗口失焦和卸载时恢复。

分隔线具有 `role="separator"`、`aria-valuemin/max/now`，可访问名是「调整工作台宽度」而不是标签轨道的停靠栏名称（两者曾共用一个 key）。方向键每次 16 px，Shift + 方向键 48 px，Home 到最小值，End 到当前允许上限。拖拽要过 3 px 阈值才算数——1 px 抖动会把 `widthMode` 永久钉成 `manual`。

`widthMode` 与比例的读写一律走 try/catch 包装：`useState` 初始化里裸读 localStorage，在被策略禁用或配额爆掉的环境里会直接把整个 ChatScreen 打成白屏。

## 5. 环境信息投影

环境卡是**可常驻面板，不是 popover**：按钮是开关，点别处不关闭，入口紧挨终端。标题栏右端所有图标动作共用 [`titleBarStyles.ts`](../../../src/components/chat/titleBarStyles.ts) 的同一个 28×28 方钮外壳，靠 `gap-1` 排成一条均匀的节奏；一次性动作与开关的唯一差别是**开关多一层选中填充**（搜索、无痕、环境卡、终端、工作台入口）。新增标题栏图标按钮直接复用该常量，别再手写一套尺寸或补 `ml-*`。因此 `ChatTitleBar` 里只有 `statusPinned`（用户意图）是状态：`showStatus = statusPinned`。窗口太窄时**只是不再预留通道**（`environmentInsetWidth` 返回 0），卡片照常打开、直接盖在正文上——按钮点了没反应比盖住正文更糟，而 `cardFits` 的阈值（约 1468 px）在常见笔记本宽度上根本达不到。也**别改成「收起时清掉 statusPinned」**，那是「缩小后拉大不出现」的老 bug。卡片内那几个跳走的操作（compact、查看上下文 / 系统提示、打开 Workspace）显式 unpin。

弹层由整个右侧操作组的相对容器定位（不是按钮自己，否则会比预留通道多探出几个按钮的宽度），不 portal 到 `document.body`；宽度上限用对话标题段的 container query，右边界不会越过对话 / 工作台 divider。

正文让位有两种呈现，判定见 [`environmentInset.ts`](../../../src/components/chat/environmentInset.ts)：

- 居中正文列会撞上卡片、且让位后仍留得下可读宽度 → 预留 348 px（316 卡片 + 16 偏移 + 16 间距）。`MessageList` 与 composer 用**同一个盒子**、以 inline padding 施加，两者才会同轴移动；写成 class 会被 `cn` 的 padding 合并吃掉。
- 让位后剩余宽度不足 → 保持不动，卡片直接盖在正文上。

环境卡继续提供版本、模型 / 鉴权类型、Context / compact、Memory policy、Agent、会话 ID、消息数、reasoning effort、更新时间和系统提示入口。

环境卡与 Workspace 读取相同会话 / 模型 / memory / context 状态，不保存凭据；API Key、OAuth token 和 Owner token 不进入投影 props 或 DOM。

## 6. 面板生命周期与非退化契约

工作台打开过的业务面默认 warm mount。[`RightPanelShell`](../../../src/components/chat/right-panel/RightPanelShell.tsx) 是绝对定位的兼容宿主：活动项可见，非活动项 `aria-hidden + inert + pointer-events-none`。宽度、分隔线、圆角卡片、panel shadow 与最大化全部归 WorkbenchSurface，**这些能力已从 shell 的 API 里删除**（`width` / `onWidthChange` / `resizeLabel` / `minWidth` / `maxWidth` / `reservedMainWidth` / `overlay` / `maximized` / `integrated`），面板不得再自带一套；同理各面板自己的最大化按钮与 FLIP 过渡也已随之下线。

**warm mount 不等于后台照跑**：hidden 标签没有读者，它的轮询就该停。shell 通过 [`PanelVisibilityContext`](../../../src/components/chat/right-panel/panelVisibility.ts) 向下广播「当前内容是否可见」，`usePanelVisible()` 是唯一读取入口，shell 之外（浮窗、设置页、对话框）默认可见、行为不变。规则：

- **有事件源兜底的轮询**（Workflow / Loop / Review / Verification / Domain Quality、后台任务 1Hz 输出尾）hidden 即停，靠既有 `transport.listen` 保持新鲜，重新显示时 effect 自然重跑。
- **没有事件源的纯轮询**（PR 检查 30s，含 Workspace 里的 Git 卡片）hidden 即停，并用 `usePanelRevealRefresh` 在「重新可见」那一刻补一次——**只在 hidden→visible 的沿上触发，不在挂载时触发**，否则每次挂载都会和初始加载打架、多打一次请求。
- 标签条上的 badge 不受影响：它们的数据源在 ChatScreen 层，不在面板内部。

| 能力                    | 工作台行为             | 保留的原生命周期                                                         |
| ----------------------- | ---------------------- | ------------------------------------------------------------------------ |
| Workspace               | 会话单例标签           | section、深链、自动打开一次、dismissed、控制操作                         |
| Diff                    | 会话单例标签           | staged / unstaged / all、file / hunk mutation、Git snapshot              |
| Pull Request            | 会话单例标签           | checks / comments 独立错误、stale、修复填 composer、自动合并确认         |
| Plan                    | 会话单例标签           | 计划状态、版本、评论、Approve / Resume / Rollback / Exit                 |
| Files                   | workspace 单例标签     | tree / search、linked roots、编辑、quote reveal、内容工具栏、detach      |
| File / Artifact preview | 每稳定 target 一个标签 | 双 Transport 授权、Office / PDF / media / code、引用、打开 / 下载 / 编辑 |
| Browser                 | 会话单例标签           | frame 过滤、事件 + polling、历史、QuickBar、dismissed、float / dock      |
| Mac Control             | 会话单例标签           | 当前帧、dismissed、float / dock                                          |
| Canvas                  | 当前会话 Canvas 标签   | iframe 不重挂、streaming、quote、内容工具栏、detach                      |
| Team                    | team 单例标签          | team 状态与子会话入口                                                    |
| Background Jobs         | 会话单例标签           | running badge、取消、展开态、后台打开不抢活动标签                        |
| Subagents               | 会话单例标签           | run 选择、live child transcript、running badge                           |
| Terminal                | 工作台之外的底部 dock  | 多 PTY、快捷键、隐藏不杀 shell、拖高、最大化、输出重放                   |

Browser / Mac 转成 floating 后离开 docked surface，但标签仍留在顶部用于定位；点击标签上的 reattach 或直接选择该标签会 dock 回工作台。浮窗继续使用共享 frame store，切换容器不重新订阅或中断帧。

WorkbenchSurface **恒挂载**（没有打开面板时 `empty` → `hidden`，不占布局也不占那 1 px 列边框）。Canvas 面板自己持有 `canvas_show` 监听与会话 canvas 恢复，`canvasPanelOpen` 又由它回调驱动：把它挂在「有面板打开」这个条件下就成了自引用——默认状态下没人听 `canvas_show`，canvas 再也打不开；而它随其它面板重新挂载时又会重新认领最新 canvas，把用户刚关掉的标签弹回来。同理，Canvas 把主窗口最小宽度顶到 1280 的那个 effect **必须有卸载兜底**，否则组件先被卸载、复位分支永远跑不到，窗口就一直缩不回去了。

Files / Canvas 独立窗口的 handle 保持在原组件内。关闭顶部 Files 标签会先触发同一个外部关闭事件，使独立窗口 reattach / close 并复位 fullscreen；不能绕过已有窗口清理。

关闭 Files 或文件标签前统一调用 `confirmDiscardDirtyFileEditors`，**并带上被关标签的 owner**（`FileBrowserPanel` 的 `instanceKey`）：这是进程级注册表，不带 owner 的话关掉一个干净标签会把其它标签的未保存缓冲一起 discard，弹窗还指着一个用户没在离开的文件。用户取消后标签和编辑器保持不变；会话导航、新建会话与 Transport 切换继续使用同一个全局（无 owner）guard。

文件浏览器的 reveal 也是导航：`revealFile` / `revealDirectory` 两条 render-phase 通道都必须走 `editorDirty` 判定，命中就转成 `pendingNavigation` 等用户确认——尤其目录 reveal 还得清 `editing`，否则编辑器会被指到一个目录上 `readFile`。

## 7. 自动打开与会话隔离

现有自动打开策略不因工作台改造而改变：

- 当前会话首个 Browser / Mac frame 可激活对应标签；用户关闭后，本会话继续遵守 dismissed。
- Workspace 首次出现任务、文件或来源时可自动打开一次；用户关闭后不反复抢回。
- Background Jobs 从 0 进入 running 时，如果已有活动标签则后台打开并显示 badge，不抢焦点。
- 文件、Diff、PR、Plan 和深链属于用户显式前景打开。

普通会话切换时，renderer 缓存以下会话级视图状态：安全标签集合、标签顺序、活动标签、工作台收起状态、文件预览集合以及 Browser / Mac / Workspace / Jobs 的 dismissed 状态。返回原会话时恢复这些状态。

浏览器与 Mac 镜像的显示、关闭状态独立由 `useMirrorPanelSessionScope` 按当前主对话或侧聊的真实会话标识保存；父会话工作台缓存不重复保存这两项。切换对话面会关闭旧悬浮镜像，草稿转正保留自身状态，无痕状态不缓存。

草稿会话用 `__draft__` 作 scope key。**首条消息把草稿变成真会话时是重命名、不是切换**：`session_created` 经 `onSessionPromoted` 把工作台缓存、标签顺序与两个文件 scope 一起改址（[`useScopedTabState`](../../../src/components/chat/files/useScopedTabState.ts) 的 `renameScope`）。当成普通切换处理会有两个后果——回合一开始所有已开文件标签凭空消失，而遗留的 `__draft__` 桶会在下一次「新建对话」里被恢复出来，预览还指着上一个会话的工作目录。

Browser / Mac 浮窗在会话切换时仍关闭。PR 是 HEAD / branch 相关网络状态，切换时关闭并由目标会话重新打开。文件 target 保存原 provenance，不能重绑定到目标会话。

Incognito 会话不写入会话工作台缓存；切离时不留下文件资源集合或 dismissed 记录。工作台隐藏、stage 切换或 HTTP 页面断线都不得取消服务端持有的主对话 turn、Workflow、Subagent、后台任务或终端。

## 8. 可访问性与视觉

- 标签轨道使用 `role="tablist"`，标签使用 `role="tab" + aria-selected`。
- 非活动内容 `aria-hidden + inert`，键盘焦点不能进入隐藏面板。
- 打开工作台不主动抢 composer 焦点；用户点击或键盘激活标签后才进入工作面。
- 边界始终画 1 px `border-border-soft` 结构线：工作台左缘、侧栏右缘、Files 树右缘，以及 `WorkbenchSurface` 顶缘（标签轨道与面板内容同底色，靠它收口）；[`ResizeHandleGlow`](../../../src/components/ui/resize-handle-glow.tsx) 是**叠在结构线之上的拖拽反馈**，不替代它——idle 只有结构线、没有光晕，hover / focus / drag 才显示同一 1 px 蓝色光晕，且不通过加粗边框改变布局。
- hover / selected 只改变背景和文字，不增加状态边框、ring 或阴影。
- 工作台底色是 `bg-background`，与对话列、标题栏完全一致——它是同一个窗口的另一半，不是浮层，所以不用 `surface-app` / `surface-panel` 那套偏冷的面板底色；两侧只靠 1 px `border-border-soft` 分隔。`RightPanelShell` 的 `integrated` 分支与切换遮罩同样跟随该 token。
- resize 与 surface 动效遵守 reduced motion；iframe 在 resize 手势中不可截获指针。

## 9. 安全与运行模式

- 工作台不绕过 `useFileResource`、preview-by-path 授权、`WorkspaceScope` 或远端写闸门。
- HTTP 文件路径仍由后端 canonicalize；标签只保存 `FileTarget`，不把相对路径解析结果跨 root 复用。
- Browser QuickBar、raw CDP strict、SSRF、Chrome claim 和 Stop 契约不变。
- 环境投影只展示脱敏状态，不展示任何 Key / Token。
- Tauri 与 Bundled HTTP UI 共用布局与标签模型；detach 按既有 Tauri runtime 门控。
- 普通工作台 surface 不使用 `position: fixed` 模拟分栏。只有显式 maximize、float 或 detach 可以离开 docked 几何边界。

## 10. 验证入口

关键回归用例：

- [`useWorkbenchSizing.test.ts`](../../../src/components/chat/workbench/useWorkbenchSizing.test.ts)：auto / manual clamp 与 stage hysteresis。
- [`useFilePreview.test.ts`](../../../src/components/chat/files/useFilePreview.test.ts)：稳定身份去重、多文件排序、关闭 fallback、会话恢复。
- [`ChatTitleBar.test.tsx`](../../../src/components/chat/ChatTitleBar.test.tsx)：单顶部行标签、badge、多文件标签与收起恢复入口。
- [`WorkbenchSurface.test.tsx`](../../../src/components/chat/workbench/WorkbenchSurface.test.tsx)：docked / collapsed / maximized 的几何与无障碍状态。
- [`RightPanelShell.test.tsx`](../../../src/components/chat/right-panel/RightPanelShell.test.tsx)：兼容 shell 的 resize / mount / fullscreen 行为。
- [`fileDirtyRegistry.test.ts`](../../../src/components/chat/files/fileDirtyRegistry.test.ts)：按 owner 收窄的未保存 guard，以及仍然覆盖全部编辑器的全局 guard。
- [`useFileTabs.test.ts`](../../../src/components/chat/files/useFileTabs.test.ts)：reveal 复用活动标签、会话保留 / 无痕丢弃，以及草稿转正的 scope 改址。

人工验收至少覆盖：2560、1920、1440、1280、1024、800 px；侧栏开关；工作台 auto / manual / stage；环境卡 reserved / overlay；Terminal；Browser / Mac float；Files / Canvas detach；文件 dirty guard；正常与 Incognito 会话切换。
