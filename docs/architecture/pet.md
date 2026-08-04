# 桌面宠物（Pet）

> 返回 [文档索引](../README.md)

Pet 是一个桌面优先、被动常驻零 LLM 的状态表现层。它把已有主对话的运行、待处理、失败和未读完成状态投影到透明浮窗，不改变 Prompt、Memory、权限、任务状态或会话正文。只有用户明确提交气泡快捷回复时才复用原 Session 的主对话链；Settings 的 Create 则走隔离的媒体生成工作流，后者不属于主对话，也不能成为 Pet activity。

## 模块与数据真相源

| 层 | 唯一职责与入口 |
| --- | --- |
| Core（契约） | `crates/ha-core/src/pet.rs`：`ChatUiSurface`（chat_turns.ui_surface 列 wire 类型）、`emit_activity_changed`、`update_config` trampoline；宠物机器（`activity` 主对话投影 / `atlas` Codex 格式与图片校验 / `import` 发现 + preview capability / `store` 原子安装-删除-恢复-导出 / `asset` 安全资源解析 / `creator` 显式创作流程）随第四刀迁到 `crates/ha-pet/`；这里不依赖 Tauri |
| 桌面壳 | `src-tauri/src/commands/pet.rs` 只做命令适配；`src-tauri/src/pet_window.rs` 负责透明窗口生命周期、原生 geometry、失焦指针桥和位置恢复；`src-tauri/src/pet_deep_link.rs` 只接收 Hope 自有协议并转交预览 |
| HTTP 壳 | `crates/ha-server/src/routes/pet.rs` 只做 Bearer 鉴权、参数/结果映射和同源资源响应，不能重写 Core 的导入、hash、状态资格或优先级 |
| React | `src/PetWindow.tsx` 是独立轻量入口；`src/components/pet/` 承载精灵、气泡、交互卡和布局 hooks；`src/components/settings/PetSettingsPanel.tsx` 是 owner 管理面 |

持久化与临时状态必须保持以下边界：

| 真相源 | 内容与生命周期 |
| --- | --- |
| `config.json` | `pet.enabled`、`pet.selectedPetRef`；所有开关和选择入口都走共享配置 mutation，不维护 localStorage 副本 |
| `sessions.db` | `chat_turns.ui_surface`、turn 状态与消息边界，`sessions.last_read_message_id`，以及 pending `ask_user_question` groups；只有携带 durable `ownerResponse` 的 owner-plane group 可跨重启保持 pending，普通工具 group 的内存 oneshot 无法恢复，启动时由 `expire_pending_ask_user_groups` 标为 answered；`ui_surface` 是 additive migration，历史 NULL 永不按 source 猜测，索引随 migration 一次性建立 |
| `~/.hope-agent/pets/` | 自定义包和 `.trash`；磁盘是真相源，库 revision 只是进程内失效版本，不可替代目录扫描与校验 |
| `pet-window-state.json` | monitor、work area、scale 与宠物脚下归一化锚点；它只属于桌面 UI state，不复制 `enabled` |
| 进程内 approval registry | `tools::approval::PENDING_APPROVALS` 保存完整 request 与响应 sender，供同一进程内的 reload/transport resync 重建卡片；它不写入 `sessions.db`，进程退出即失效，不能用于跨重启恢复 |
| 进程内 capability 表 | Codex candidate 30 分钟、import preview 10 分钟、restore token 10 分钟；重启即失效。preview 最多 128 项/64 MiB，restore token 最多 128 项，客户端不能把 token 当持久 ID |

Pet 的 `activity_snapshot` 只把进程内 approval 数与 SQLite pending Ask group 数相加，用“是否大于零”决定运行中 turn 是否投影为 `NeedsInput`；它不返回交互总数或最早倒计时。侧边栏 `SessionMeta.pending_interaction_count` / `pending_countdown` 的数量和 deadline 合并属于 `session::pending::enrich_pending_interactions`。PetWindow 的交互卡则分别读取当前进程内 approval request 与 live Ask group，不能把任一聚合结果回写数据库，也不能据此假设普通 approval/Ask 可跨重启恢复。

`.install-*` staging 超过 24 小时才清理；`.trash` 最长保留 7 天且最多 256 项。清理、导入、删除、恢复与选择校验共享跨进程库锁，任何缓存或 renderer state 都不能成为包、配置、activity 或未读的第二真相源。

## 能力边界与非目标

- 不复制或解包 Codex 应用内置的专有素材；自动发现只扫描用户自己的 Codex current/legacy 自定义目录，用户明确选择的兼容包仍可导入。
- 不让 Pet 数据进入 system prompt、Memory、Awareness 或会话正文，也不新增 scheduler、任务状态或未读系统。
- HTTP/server 提供资源管理与只读 activity API，但不伪造桌面置顶浮层；ACP/IM 同样不能远程唤醒 owner 桌面窗口。
- Pet 与会主动调用模型的 Knowledge Sprite 是两个独立子系统，不共享配置、事件、状态投影或 prompt。Pet 的被动 runtime 零 LLM，只有用户明确快捷回复或 Create Pet 才进入既有模型链。
- 不把透明浮层逻辑塞进 CLI，也不实现 Computer Use 画中画吸附；平台不支持透明或置顶时只做明确的静态降级。

## 主对话边界

是否接入 Pet 由 `chat_turns.ui_surface` 决定，不由 transport、`ChatSource`、窗口名称或历史 session 类型猜测。当前 allowlist 是：

| 一等 UI 表面                      | `ChatUiSurface`  | SessionKind        |
| --------------------------------- | ---------------- | ------------------ |
| 主 ChatScreen                     | `main_chat`      | Regular            |
| QuickChatDialog / QuickChatWindow | `quick_chat`     | Regular            |
| KnowledgeChatPanel                | `knowledge_chat` | Knowledge          |
| DesignChatPanel                   | `design_chat`    | Design             |
| Pet 快捷回复                      | `pet_chat`       | 沿用原 SessionKind |

这几类表面的共同定义是“产品消息列表 + 产品输入框 + 用户可持续多轮对话”。第一方 HTTP transport 有 surface 时走 `/api/chat/ui`；服务端还要求浏览器 Fetch Metadata，并校验请求 `Origin` 与 `Host` 同源或命中显式 CORS allowlist，不能仅凭 route 名或 JSON 字段认定第一方。公共 `/api/chat` 强制清空该字段，因此普通 API 调用不会因误传/伪造 `uiSurface` 被接入。缺少 `ui_surface` 的 side query、automation、compact、Memory、Dreaming、Knowledge Sprite、vision bridge、judge、eval、embedding、STT、媒体生成、Cron、IM、ACP、subagent、ParentInjection 和后台 job 全部排除。Pet 回复对运行中 turn 走既有插话队列，对终态对话以 `pet_chat` 在原 Session 开新主 turn，因此不会创建 Pet 专属会话。内部调用不能继承父 turn 的 surface；未来新增一等对话表面必须显式扩展枚举、固定调用点、Core SQL allowlist 和测试。历史 NULL turn 同样不按 `source` 猜测。Knowledge thread 必须有 `kb_id`，但 `anchor_note_path` 可空：没有打开具体文档的知识空间主对话仍会接入，并通过 KB + session 精确恢复聊天面板。

每个一等表面只有在窗口/面板可见、文档获得焦点、消息列表尾部可见时推进共享的 `sessions.last_read_message_id`。推进值是 React 已取得并经过两帧绘制的最大 `dbId`，绝不在 stream end 时把整场会话直接标已读。因此隐藏面板、历史上翻、迟到消息和后台完成不会被宠物或流结束事件误清。

## 四态投影

`pet::activity_snapshot` 在 `SessionDB::run` 内读取权威状态，每个 session 最多产生一条 activity：

| 优先级 | 状态          | 真相源                                                                       |
| -----: | ------------- | ---------------------------------------------------------------------------- |
|      0 | `needs_input` | session 最新 turn 是合格 UI turn、仍在运行，且有 approval / ask-user pending |
|      1 | `blocked`     | session 最新 turn 是合格 UI turn、为 Failed，且终态消息边界尚未读            |
|      2 | `ready`       | session 最新 turn 是合格 UI turn、为 Completed，且终态消息边界尚未读         |
|      3 | `running`     | session 最新 turn 是合格 UI turn 且尚未终态                                  |

最新 turn 若来自公开 API、cron、subagent、side query 等非主对话入口，即使同一 session 更早有 UI turn，也不继承 Pet 资格；创建这个 non-UI turn 会让现有投影立即失效。终态边界是 `assistant_message_id ?? user_message_id`，与 `sessions.last_read_message_id` 比较；`Interrupted` 不映射 Blocked。快照最多返回 50 条，稳定排序并携带 `total` / `truncated` / `revision` / `stale`。activity 事件只作失效通知，PetWindow 仍会重查快照；可见时另以 5 秒 reconcile 防事件丢失。

## PetWindow 与动态尺寸

桌面壳用 Tauri 2 `WebviewWindow` 创建 `pet` 窗口：透明、无装饰、置顶、不进任务栏，最小约 120×128 logical px，最大 440×640。React 只挂载轻量 `PetWindow.tsx`，不挂载完整 `App`。

macOS 创建窗口时同时启用 WRY `accept_first_mouse` 与原生 `NSWindow.acceptsMouseMovedEvents`：前者让第一次点击可交互，后者打开原生指针事件入口。WebKit 会按平台惯例抑制后台 WKWebView 的 DOM hover，因此不能假设 tracking area 会自动变成 CSS `:hover`，也不得用 `set_focus()` 绕过，否则宠物仅因鼠标经过就抢走用户当前应用的焦点。

窗口矩形必须始终贴合当前内容，不能用固定的大透明窗口承载气泡：透明区域也会截获桌面点击。布局遵循以下顺序：

1. 气泡栈/交互卡先在 `visibility:hidden` 的 measurement layer 渲染；`ResizeObserver` 与字体 ready 得到 logical size。测量层与正式层共用 scroll viewport 内围留白（左右/顶部 16px、底部 28px），让 shadow 先落入可滚动内容盒再由 native bounds 包住；留白放在 overflow 容器外会让阴影仍被 scrollport 裁掉，因此禁止回退成外层 padding。
2. `usePetWindowLayout` 根据当前显示器 work area、scale factor 和安全边距选择左/右、上/下方向，为请求分配单调 `layoutRevision`。
3. PetOnly → Overlay 时先以目标左/右、上/下朝向挂载 `visible=false` 的正式层并至少保留一个 paint，使宠物 DOM 在原生扩窗前已经对齐目标 foot anchor；否则屏幕边缘首次展开时，native 采用新 anchor 而 renderer 仍按旧朝向排版，会出现一帧宠物跳动/闪烁。已有 overlay 替换或关闭则先完整淡出，并冻结当前卡片/气泡内容直到淡出结束，禁止由 `expanded=false` 提前清空子树。
4. Rust 在一个 mutex 内检查 revision，以原生已提交 anchor（而不是可能过期的 renderer previous anchor）保持宠物脚下屏幕坐标不变；macOS 必须用一次非动画 `NSWindow.setFrame` 原子提交尺寸与位置，禁止 `set_size`/`set_position` 两次提交之间被 WindowServer 绘制出“宠物瞬移到左上角”的中间帧；旧请求返回 `applied=false`。bounds 成功后再保留一个透明 paint，随后把正式层切到 `visible=true`，让 180ms opacity + transform 入场真实发生，而不是以最终状态直接挂载。
5. `ResizeObserver` 对不超过 1px 的变化去重，每个真实的新尺寸只触发一次 latest-wins 更新，因此气泡栈/交互卡可随内容持续变化而不会被固定“校正次数”截断；瞬时失败做两次有界退避重试，仍失败则恢复旧 overlay 可见，不能让宠物或气泡无故消失。关闭使用同一 180ms 淡出后再缩回 PetOnly；reduced-motion 跳过 opacity/transform 动效，但保留准备帧、锚点和错误回退。

用户拖动时暂停 layout hook 并冻结当前 bounds 和 overlay，移动超过 4 logical px 才调用 `startDragging()`；drag end 抑制同一 click，再关闭 overlay 并按新显示器重新布局。迟到的字体测量或 activity 更新不能在 OS drag 期间 resize。位置持久化保存的是宠物脚下锚点相对 monitor work area 的归一化坐标、显示器信息和 scale，而不是易受 resize 影响的窗口左上角。move event 由单一 coalescing worker 300ms 去抖，持续拖动不会创建大量线程。macOS 同时保留 `NSWindow.acceptsMouseMovedEvents` 与 WKWebView 的 `NSTrackingActiveAlways` tracking area，并以一对进程生命周期 local/global `MouseMoved | LeftMouseDragged` monitor 补齐 WebKit 的后台 DOM 限制：两条 monitor 必须复用同一个坐标投影，不能在 local 事件上清除 global hover，否则主窗口激活而 PetWindow 非 key 时会逐帧闪烁。只有指针位于当前 PetWindow 矩形内时，才以最多 30Hz 向该窗口发送 logical 坐标，React 用 `elementFromPoint` 映射 pet、activity 和快捷按钮并声明式恢复 hover；命中 Pet 的左键拖动期间，原生层只轮询左键是否仍按下并在释放时发送无坐标的 `pet:native_drag_ended`，因为 AppKit 原生拖拽立即返回且不保证回投 mouse-up。离开矩形只发一次 leave。bridge 不监听按键、mouse-down、普通 click 或窗口外坐标，不持有可失效的 NSWindow 指针，也不得把 PetWindow 设为 key window 或激活主应用。

拖拽 run 的起步方向取越过 4px 阈值时的 pointer delta；进入原生拖拽后以 PetWindow 连续 `Moved` 事件的 x 差实时切换 `run_left` / `run_right`，不能把首次方向锁到 drag end。

方向选择同时计算 anchor 四侧的可用空间和两种朝向的总 overflow：能完整容纳时保持当前朝向避免 1px 抖动，不能完整容纳时选择溢出更少的一侧，最后由 Rust 按 12 logical px（乘当前 scale factor）的安全边距钳位。renderer reload 若丢失本地 revision，会采用 native 返回的 revision 并重试；native size 成功而 position 失败时回滚旧 geometry。

## 多气泡、快捷回复与交互卡

每个 activity 对应独立胶囊气泡：常态保持 52px 高，标题、实心分隔点与摘要像正文一样连续排版，整体最多显示两行，超过后截断且不能继续撑高；标题最多占内容宽度的 52%，并在渲染前按 CJK/ASCII 显示宽度预算生成一个稳定的“前缀…后缀”字符串，不能用两个 flex 片段互相挤压来模拟中间省略。这样既避免关闭自动标题或 LLM 尚未回写时较长的首消息 fallback 吃掉两行摘要空间，也能保留标题结尾的限定词。正文是轻量预览而非完整 Markdown 布局：实时流与完成态统一保留标题、强调、代码、链接等可读内容，去掉 `#`、`*`、反引号、代码围栏等 Markdown 标记并折叠空白；不得在气泡挂载完整 Markdown renderer 导致流式阶段频繁改变窗口高度。背景使用低不透明 surface、`backdrop-blur-xl` 和柔和阴影形成真实毛玻璃，而不是接近不透明的伪 blur。Running 消费既有父主对话 `chat:stream_delta`，显示不断更新的有界正文尾部 + spinner，并复用全局 `animate-text-shimmer` 做文字扫光；reduced-motion 自动退化为静态文本。尚无正文时回退本地状态。Pet 只为 activity snapshot 已准入的 Running session 建立预览，中途打开窗口时先读 `get_session_stream_snapshot` 的 durable prefix，并在握手期间缓存 live delta，按 stream/seq 去重后再 reveal；因此 side-query、工具内部 LLM 和其他非主对话不会产生气泡。Ready 改用 terminal assistant 有界预览 + 完成勾；NeedsInput/Blocked 不泄露问题参数或错误原文。incognito 标题、Agent 和流式/终态预览始终脱敏。

- 收起时宠物右上角以固定 28×28 正圆显示 activity 数量（超过 9 显示 `9+`）；点击展开后同一控件变为向下箭头。若 Ask/审批待处理，收起态数字改为黄色但仍表示 activity 总数；风险等级只在卡片内表达，不把严格审批映射成红色数字。栈按优先级排列并在最大高度内滚动。自动展开本身不推进 read watermark；Ready/Blocked 气泡停留阅读至少 700ms 后移开、提交快捷回复、关闭，或用户主动收起已展开的栈，才由 Pet 按该气泡的 `boundary` 标记已读。点击气泡只发 typed navigation，不在 PetWindow 提前推进 watermark；目标消息列表成功加载并真实渲染正文后，再由该表面的 read receipt 标记已读，导航失败或目标已删除时必须保留未读。所有推进都必须使用 `mark_session_read_cmd(throughMessageId)`，不得无边界清空，确保并发到达但尚未渲染的新消息继续保持未读；Running 与 Ask/审批等待态不因展示被标记已读。成功推进会同时触发 `session:unread_changed` 与 `pet:activity_changed` 失效通知，侧栏未读聚合和 Pet activity 数字都必须重新查询权威值，不能在 renderer 内各减一。
- hover 单条气泡才显示左上角关闭与右侧快捷动作：Running 同时显示回复与停止，其他可回复状态只显示回复；回复使用不带消息框的单线条箭头，操作按钮常态保持低对比、无阴影，只在 hover 时加深背景。动作覆盖状态位并只改变内部文本截断，不改变气泡外框。停止必须复用 `stop_chat(sessionId, turnId:null)`，只中止该 activity 的权威主 turn，不使用全局 stop 或另造取消通道。点击关闭 Running/NeedsInput 只隐藏当前 activity 投影，状态或 turn boundary 改变后可重新出现，不取消执行或撤销权威交互卡；关闭 Ready/Blocked 则用该 activity 的 terminal boundary 推进共享 read watermark。点击回复后仅展开这一条 composer。运行中回复走 durable turn-message 插话，终态回复以 `pet_chat` 在同 Session 开新主 turn。
- `ask_user_question`、工具审批和计划确认使用 Pet 专属紧凑卡片，但提交仍复用既有命令与权威 pending queue。Ask group 一次只渲染一道题，答案保存在分页 state，上一题/下一题可往返，最后一题才原子提交完整 `answers[]`；选项只保留标签、单行说明、推荐标记和 Other，不加载消息列表的 Markdown/方向预览。卡片将类型、题号、队列位置和倒计时合并为单行元信息，选项说明同样单行截断，导航使用 28px 紧凑操作区，避免复制消息列表的多层标题与大段留白。审批卡保留 reason、command、cwd、倒计时及 deny/once/always 语义，严格审批与 cron delete 继续禁止 standing grant。用户收起时卡片与普通气泡一起收起；新的 request id 到达才再次自动展开，`ask_user:resolved` / `approval:resolved` 保证跨表面同步撤销。
- 气泡正文点击按 typed target 打开完整对话；数字/箭头只控制气泡栈；单击 Pet 播放 Jump 并以 `pet_focus_target_cmd(target:null)` 唤起 Hope 主窗口，不擅自切换会话。拖拽超过 4px 时必须抑制同一手势合成的 click，不能误唤起主窗口。右键 Pet 时收起气泡栈，并在宠物本体中心覆盖一个紧凑的「设置 / 关闭」胶囊：设置先唤起主窗口，再发出既有 `open-settings(section:pets)` 事件；菜单不扩张原生窗口，也不在宠物外另开卡片。
- 每个新的 activity 投影（含同一会话的新 turn/状态/boundary）和新的 Ask/审批 request 都自动展开；用户手动收起后，已有内容更新不得反复重开，只有新的稳定 key 才可再次展开。自动出现不抢 OS 焦点；Escape 先关闭回复 composer，再收起整个信息层，不会 Tuck Away。

typed navigation 由主 App 壳消费：Regular 回主聊天 session；Knowledge 恢复知识空间 thread；Design 恢复 design project + thread。PetWindow 不拼 URL，也不把专属对话伪装成 Regular。

精灵动画分两层仲裁：业务状态循环（Idle/Working/Waiting/Sad/Celebrate）和指针一次性动作（hover Wave、click Jump）；拖拽左右 Run 优先级最高。固定顺序是 Drag > Click > Hover > 业务状态 > Idle，一次性动作完整播放后恢复业务状态，Pet 内部移动不重复触发。

## 精灵图、存储与导入

Core 显式支持 Codex v1 `1536×1872`（8×9）和 v2 `1536×2288`（8×11），单格 `192×208`。渲染使用 SVG `viewBox` + atlas 坐标，不用 Canvas/WebGL，不复制 Codex 内置专有素材。内置 Hope pet 编译进应用；自定义包位于 `~/.hope-agent/pets/`，`pet.json` 只保留 Codex 兼容的 `id`、`displayName`、`description`、`spriteVersionNumber`、`spritesheetPath`，后两项缺省为 `1` 和 `spritesheet.webp`；Hope provenance、source 与 hash 单独放 `hope.json`。v2 末两行保持原样，不赋予未经确认的业务语义。

Debug 构建额外注入内置 `builtin:hope-debug`，其 v1 atlas 每格使用纯色背景，并精确标注英文状态、中文状态、零基 row/frame；同一行的各帧用同色系明暗变化，便于同时观察 action 仲裁和计时器是否推进。行契约固定为 `Idle/空闲`、`Run Right/向右跑`、`Run Left/向左跑`、`Wave/挥手`、`Jump/跳跃`、`Sad/难过`、`Waiting/等待`、`Working/工作中`、`Celebrate/庆祝`。资源由 `scripts/generate-debug-pet.py` 确定性生成。Core 的注册、内嵌 asset resolver 和导出分支必须受 Rust `debug_assertions` 编译门控；renderer 的直连 asset 必须受 `import.meta.env.DEV` 门控。Release library、选择校验和 asset API 均不能识别该 pet；若开发配置残留其引用，Release 按既有 selected-unavailable 逻辑回退 Hope，不迁移用户配置。

### Create Pet 生成管线

Create 是 Settings 内 owner 显式触发的媒体生成，不注册 Agent tool/skill，也不从一张图继续发起逐帧模型调用。它先经 `media_gen::execute_image` 生成单个角色源图，再由 Core 确定性构造 Codex v1 atlas：

1. 按 magic bytes 解码并限制源图尺寸；已有明显透明度时保留原 alpha。
2. 只有四角背景色一致且图像基本不透明时，才移除与图像边缘连通的近似背景色；不做全图颜色抠除，避免误删角色内部的同色细节。
3. 按 alpha 内容边界裁剪并缩放到单格安全区，随后使用固定的位移、缩放、翻转和 bob 参数合成 9 行 × 8 帧动作。动作行顺序与上面的 v1 契约一致，结果恒为 `1536×1872` PNG。
4. 生成包继续走与外部导入相同的 atlas validator、preview capability 和人工确认；校验失败不安装，用户取消不留下最终包。

因此 Creator 的“动画”是本地可复现的 pose 合成，不表示媒体模型分别生成了 72 帧；修改背景判定、裁剪、pose 或行顺序时必须同步 creator/atlas 单测和本节。

所有导入入口都走 preview → validate → commit：Codex current/legacy 扫描、目录、zip、manifest + image、PNG/WebP、浏览器 upload、HTTPS sprite、粘贴 `codex://` / `hope-agent://`，以及系统注册的 `hope-agent://` 协议。系统协议只把主窗口带到 Settings 的预览确认页；`codex://` 只支持粘贴解析，因为该 scheme 属于 Codex。任何入口都不能静默安装或启用。

一个 drop 含 manifest 时作为一个 loose-file 包；否则多个目录、zip 或独立 atlas 分别生成 preview card。标准 WebView drop 通过 `DataTransferItem.webkitGetAsEntry()` 有界递归顶层目录（最大深度 8、最多 64 个文件），再用通用分块上传 lease 进入同一 preview 流程；Tauri native path drop 仅作可用时的快速路径。批量 commit 独立处理，成功项消失，失败项留在界面重试；失败的 preview source lease 立即释放。HTTP/Web 只接收 staged upload id，拒绝客户端本机路径。

安全与一致性约束：

- manifest/path canonicalize，拒绝 absolute/`..`/symlink escape/设备文件；zip 有 entry、depth、单文件和总展开量限制。
- 图片按 magic + bounded decode 校验，sprite 上限 20 MiB；URL 仅 HTTPS，每一跳走严格 SSRF 检查，最多 5 次 redirect，流式读取限制解压后 bytes。
- preview token 为短期随机 capability；本地 commit 重读并检查 hash，URL/upload commit 使用已缓存 bytes，不二次联网。token 只在所有请求副作用成功后消费，失败时可幂等重试。Settings 关闭、替换或移除 preview 时调用幂等 cancel，立即释放 cache 与绑定 upload lease；过期清理也执行同一释放逻辑，不能只清 React state 等 1 小时 upload TTL。cancel/commit token 只放 JSON body；thumbnail 因资源 URL 必须带 token，HTTP access log 对该 path segment 固定脱敏。
- pet root 变更持 OS 独占锁；同 root staging 完整写入后原子发布。删除移动到 `.trash`，expected package hash 防陈旧写，restore token 10 分钟内可撤销。
- `assetHash` 标识原始 sprite；`packageHash` 标识 canonical manifest + asset。相同 package 幂等，相同名称不同内容不覆盖。
- 导出生成最小 Codex-compatible zip。Create Pet 走统一 `media_gen::execute_image` 并以 `pet.create` 入账，生成结果仍先经过相同 validator 和人工确认。
- import preview 的 invoke rejection 在 Settings 调用边界写统一 `pet` warn，因为 Tauri 参数反序列化失败发生在 command handler 之前。持久日志只含固定 command、source kind、失败数、稳定 `pet_*`/错误类别，以及 `invalid_args` 的安全字段名；禁止写 raw error、路径、URL、upload/candidate id 或请求参数。失败 upload lease 的清理异常另写只含数量的 warn。

## 配置、事件与接口

`AppConfig.pet` 含 `enabled` 与 `selectedPetRef`，默认关闭。它同时有 Settings GUI、侧边栏底部快捷开关、`ha-settings` category/risk 和 skill 风险表；各入口都监听 `pet:config_changed`，不得维护独立可见性状态。配置写入是字段级 patch，选择校验与持久化持有宠物库锁，避免并发开关/选择互相回滚或删除/选择 TOCTOU。HTTP 可以管理宠物库与选择，但不能声称拥有桌面 overlay；HTTP/ACP 的 `ha-settings` 也拒绝改变 `enabled`，窗口命令返回 desktop-only。

桌面端在 Pet 配置首次就绪且仍关闭时，于侧边栏蛋图标旁延迟 700ms 显示一次非模态 discovery popover；它不抢焦点，提供直接开启与进入 Pets 设置两个动作。开启过、进入过设置或明确关闭提示后，以版本化 localStorage key `hope-agent.pet-discovery.v1` 记为已发现；outside click / Escape 只 snooze 当前挂载周期，避免误触让用户永久错过。该 key 只记录引导曝光，不复制 `enabled` 或其它 Pet 配置。

关键事件：

| 事件                   | 作用                                              |
| ---------------------- | ------------------------------------------------- |
| `pet:config_changed`   | 配置失效；主 renderer 同步 PetWindow 生命周期     |
| `pet:library_changed`  | 安装、删除或恢复后刷新 library                    |
| `pet:activity_changed` | 对话状态失效；PetWindow 重查 snapshot             |
| `session:title_updated` | 首消息 fallback 或 LLM 标题回写后立即重查 snapshot |
| `pet:navigate`         | Tauri-only；PetWindow 请求主 App 做 typed navigation |
| `pet:install_link`     | Tauri-only；OS `hope-agent://` 路由到 Settings import preview |
| `pet:inactive_pointer` | macOS Tauri-only；`{ inside, x, y }`，进入/移动时为 logical pointer，离开固定发 `{ inside: false, x: 0, y: 0 }`，最多 30 Hz |
| `pet:native_drag_ended` | macOS Tauri-only；补齐原生拖拽释放，不携带窗口外坐标 |

前三个 `pet:*_changed` 是 Core EventBus 失效通知，Tauri/HTTP 两条桥都会转发；其余四个是桌面壳内部事件，不经过 HTTP bridge。Tauri commands 与 HTTP routes 一一对应，详见 [API 参考](api-reference.md)。只有 `pet_apply_window_bounds_cmd`、`pet_sync_window_cmd`、`pet_focus_target_cmd` 的 HTTP 适配明确返回 overlay unsupported；`pet_take_install_link_cmd` 在 HTTP 恒为 `null`。

## 失败与性能契约

- 坏自定义包逐项跳过，selected 不存在回退内置 pet；PetWindow 创建或 snapshot 失败不阻止主应用。
- snapshot 失败保留最近成功值并标 stale；asset/decode 失败回退内置静态帧，不白屏。
- activity 查询只为未读 Ready 读取 terminal assistant row，并在 Core 折叠空白、按有效 UTF-8 边界截断为 240 bytes；incognito、Running、NeedsInput、Blocked 不返回正文。候选列表只读 header，thumbnail 进 viewport 才生成；候选与安装 preview 返回单行 idle 动画条而非整张 atlas，sprite URL 使用可撤销 Blob lease。
- 动画 timer 按 `performance.now()` 跳过后台积压帧；逐帧状态只影响 `PetSprite`，不让气泡栈重渲染。
- 布局 IPC 只在 overlay mode/测量变化时触发，不做逐帧窗口尺寸动画；revision 与 generation 双重 latest-wins。

## 验证与变更检查

Pet 的回归矩阵分三层，不能只验证设置页能选中图片：

| 层级 | 必须覆盖的契约 |
| --- | --- |
| Core 确定性测试 | v1/v2 尺寸、manifest 缺省与动作行；路径/symlink/zip/URL 上限和 SSRF；preview→commit stale 检测、hash 去重、并发锁、staging、trash/restore；四态优先级、最新 `ui_surface` allowlist、历史 NULL 和所有非主对话排除；terminal boundary/read watermark；asset resolver 与 HTTP 路径不泄露 |
| React 测试 | atlas 裁切与 reduced-motion；Running snapshot+delta/seq 去重、Markdown 纯文本和完成状态；多气泡、黄色待处理数字、hover 回复/停止、Ask 单题分页、审批撤窗；PetOnly/Overlay/Dragging、latest-wins bounds、180ms 收展、阴影安全区、4px click/drag 抑制、失焦 pointer bridge；逐 boundary 未读刷新；批量拖拽 preview 与 Blob URL revoke |
| 桌面 smoke | macOS 多 Space/全屏/Retina/失焦 hover，Windows 多显示器/DPI/透明点击区，Linux compositor 透明与置顶降级；四角、长 CJK/英文、字体放大、多活动滚动、拖拽中更新、拔插显示器和重启恢复；主窗口隐藏时常驻、应用退出无孤儿窗口 |

以下变更必须同步检查：

- 新增一等主对话表面：扩展 `ChatUiSurface`、固定 UI 调用点、Core SQL allowlist、typed navigation/read receipt 与纳入/排除测试；不得用 `ChatSource` 或 session 类型推断。
- 新增或修改 Pet command/route/event：同步 Tauri、Bearer-auth HTTP adapter、前端 transport map 和 [API 参考](api-reference.md)；纯桌面能力必须明确返回 unsupported，而不是假成功。
- 修改 `AppConfig.pet`：同步 Settings GUI、`ha-settings` 读写与 risk、skill 风险表；窗口坐标仍保持 GUI-only。
- 修改 Codex manifest/atlas、Creator pose 或安全限制：同步 Core validator、导入/导出 fixtures、debug pet 与本节兼容契约。
- 修改用户可见交互：同步中英文 `docs/user-guide/` 与全语言 i18n，并运行 docs parity 和 i18n check。
