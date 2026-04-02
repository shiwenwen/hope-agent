# Changelog

All notable changes to OpenComputer will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **Mutex 中毒防护**：修复 52 处 `.lock().unwrap()` 调用（分布在 cron/db.rs、canvas_db.rs、agent providers 等 12 个文件），改用 `map_err` 错误传播或 `unwrap_or_else(|e| e.into_inner())` 恢复，防止 panic 导致的级联 mutex 中毒崩溃
- **无界 Channel 改有界**：将 logging.rs、acp_control/session_manager.rs、channel/worker.rs、chat_engine.rs 中的 4 处 `unbounded_channel` 改为有界 channel（10000/256/512），防止高负载 OOM
- **异步阻塞修复**：tools/cron.rs 中 `spawn_blocking` + `block_on` 改为纯 `tokio::spawn` async，避免阻塞线程池并添加 DB 打开错误处理
- **Channel 轮询超时**：为 Telegram 和 WeChat 长轮询添加 `tokio::time::timeout` 包裹（poll_timeout + 15s），防止服务器无响应时永久阻塞
- **Docker 容器泄漏**：sandbox.rs 中容器启动失败时改为同步等待清理完成后返回错误，避免后台清理任务未执行导致的容器残留
- **Failover Jitter 均匀化**：改进 `rand_simple()` 使用 thread-local counter + XOR 混合，避免快速连续调用时 nanos 相同导致的 jitter 偏差
- **Session 切换竞态**：useChatSession.ts 添加 `switchVersionRef` 版本计数器，快速切换 session 时丢弃过期异步响应
- **Asset Protocol 范围收窄**：tauri.conf.json 的 assetProtocol scope 从 `["**", "$HOME/**"]` 收窄至 `["$RESOURCE/**", "$HOME/.opencomputer/**"]`
- **Memory 批量写入优化**：memory/sqlite.rs 中 embedding 更新操作包裹在 SQLite 事务中，显著减少大量记忆重新嵌入时的磁盘 I/O
- **前端日志丢失修复**：main.tsx 添加 `beforeunload` 事件监听，确保 logger 缓冲区在页面卸载前刷新

### Changed

- **渠道添加流程优化**：添加渠道时先弹出渠道选择界面（各渠道带品牌 Logo），选择后再进入详细配置；编辑渠道时也展示渠道 Logo 和名称替代纯文本

### Added

- **React Error Boundary**：新增 `ErrorBoundary` 组件包裹整个 App，任何子组件渲染错误不再导致白屏，提供友好的错误恢复 UI
- **MessageBubble 性能优化**：使用 `React.memo` 包裹 MessageBubble 组件，避免流式输出时 50+ 条消息的不必要重渲染
- **Memory SQLite 连接池**：将单连接 `Mutex<Connection>` 改为写连接 + 4 个只读连接池（round-robin），WAL 模式下读操作不再阻塞写入，search/list/count 等查询可并发执行
- **前端 Bundle Code-Splitting**：DashboardView 和 CronCalendarView 改为 `React.lazy()` 动态导入，减少首屏加载体积
- **Streamdown 插件懒加载**：math（KaTeX ~300KB）和 mermaid（~200KB）插件改为按需动态导入，仅在消息内容包含数学公式或 Mermaid 图表时加载

### Added

- **Telegram 斜杠命令菜单同步**
  - Bot 启动认证后自动调用 `setMyCommands` 将所有内置斜杠命令同步到 Telegram 的 `/` 命令菜单
  - 用户在 Telegram 中输入 `/` 即可看到所有可用命令及英文描述
  - `SlashCommandDef` 新增 `description_en()` 方法，为渠道 API 提供英文描述（无需 i18n 系统）
  - 同步失败不阻塞 Bot 启动，仅记录警告日志

- **天气地区自动定位**
  - 设置面板城市搜索框旁新增定位按钮（LocateFixed 图标）
  - macOS 优先使用 CoreLocation 系统定位（精确），通过 `objc2` FFI 直接调用，权限对话框显示应用名 "OpenComputer"
  - 跨平台 IP 地理定位兜底（ip-api.com，城市级精度）
  - 系统定位失败时静默降级到 IP 定位，并显示轻提示"已使用网络定位（精度较低）"
  - 系统定位成功后通过 Nominatim 反向地理编码获取城市名，自动填入城市和经纬度

- **PDF 工具视觉分析增强**
  - 三种处理模式：`auto`（默认，智能检测扫描件自动切换）、`text`（纯文本提取）、`vision`（页面渲染为图片直达模型）
  - Vision 模式通过 pdfium 将 PDF 页面渲染为 PNG 图片，以 `__IMAGE_BASE64__` marker 输出，全 4 种 Provider（Anthropic/OpenAI Chat/OpenAI Responses/Codex）均支持视觉分析
  - URL 支持：可直接分析远程 PDF（HTTP/HTTPS），含 SSRF 防护 + PDF 格式校验
  - 多 PDF 支持：`pdfs` 数组参数，单次最多 10 份 PDF
  - Auto 模式：先尝试文本提取，若文本少于 200 字符（扫描件/纯图 PDF）自动切换为 vision 渲染
  - 向后兼容：`path` 参数依然正常工作，行为不变

- **微信 IM 渠道（WeChat Channel）**
  - 后端新增原生 `wechat` Channel 插件，直接兼容 OpenClaw Weixin 使用的 iLink HTTP 协议，不依赖 OpenClaw 宿主
  - 支持二维码登录流程：前端设置面板可发起扫码、轮询登录状态并保存返回的 token / baseUrl
  - 支持 WeChat 私聊长轮询收消息、`context_token` 持久化、会话恢复后继续回复
  - WeChat 账号纳入现有 ChannelRegistry / SessionDB / Channel worker 流水线，与 Telegram 共用 Agent、Slash Command、会话映射与上下文
  - 新增 `~/.opencomputer/channels/` 渠道状态目录，保存 WeChat `get_updates_buf` 和上下文 token 缓存
  - 支持 WeChat typing 指示器：完整生命周期（24h TTL 缓存 + 指数退避重试 + 5 秒心跳 keepalive + 回复时自动 cancel）
  - 支持 WeChat 出站媒体发送（图片/视频/语音/文件）：AES-128-ECB 加密上传至微信 CDN，CDN 5xx 重试 3 次，100MB 体积限制
  - 支持 WeChat 入站媒体接收：自动下载并解密入站图片/视频/语音/文件，转为 `Attachment` 传递给 LLM（支持多模态图片识别）
  - 会话过期处理（errcode -14）：自动暂停 API 调用 1 小时，避免无效重试风暴
  - 二维码登录改进：TTL 延长至 8 分钟，过期后自动刷新（最多 3 次），返回新 QR URL

- **IM Channel 入站媒体管道打通**
  - `ChatEngineParams` 新增 `attachments` 字段，`run_chat_engine()` 将附件传递到 `agent.chat()` 的多模态接口
  - Channel worker 自动将入站 `InboundMedia`（图片读取为 base64，文件传路径）转换为 `Attachment` 送入 LLM
  - 修复 UI 聊天在有 model_chain 时通过 ChatEngineParams 丢失 attachments 的 bug

- **斜杠命令参数选项 (arg_options) 交互增强**
  - `/think` 新增 `xhigh` 超高强度思考等级
  - `/plan` 注册表补齐 `pause`、`resume` 选项
  - 前端 `SlashCommandMenu` 新增可展开子菜单，命令有 `arg_options` 时点击或回车可展开选项列表，键盘导航选择
  - Telegram 等 IM 渠道：无参数发送有 `arg_options` 的命令时返回 inline keyboard 按钮，用户可直接点选
  - `/model` 无参数时在 Telegram 返回所有可用模型的 inline keyboard 按钮（当前模型标记 ✓），点击即切换
  - Telegram polling 新增 `CallbackQuery` 处理，将 `slash:<cmd> <arg>` 格式的回调数据转换为标准斜杠命令执行

### Fixed

- **Telegram/Channel 入站附件归档与可见性修复**
  - Telegram polling 现在会下载入站 photo/document 到本地 `~/.opencomputer/channels/telegram/inbound-temp`，不再仅有 `file_id` 无 `file_url`
  - Channel worker 转换入站媒体为 `Attachment` 时，新增复制归档到会话目录 `~/.opencomputer/attachments/{session_id}/`，避免仅停留在 channel 临时目录
  - 归档后使用会话目录路径参与后续文件提取与多模态输入，提升附件可追溯性与稳定性

- **macOS 自动定位改为原生 CoreLocation**
  - 移除 `osascript` + JXA 桥接，改为 Rust 后端通过 `objc2` 直接调用 `CLLocationManager`
  - 原生实现一次性定位 delegate 与 callback 生命周期，避免 `DelClass.alloc` 这类 JXA bridge 错误
  - CoreLocation 权限请求切回 Tauri 主线程执行，修复后台线程触发时系统授权弹窗不出现的问题
  - `not_determined` 状态下改为先发起 one-shot location，请 macOS 自动弹出定位授权，而不是卡在单独的授权请求阶段
  - 开发态非 `.app` 运行时直接跳过 CoreLocation 并回退到 IP 定位，避免在 `tauri dev` 下长时间等待不会出现的系统授权弹窗
  - 定位等待改为异步 callback 驱动，不再在主线程上手动轮询 run loop
  - 保留现有降级行为：系统定位失败或超时后自动回退到 IP 地理定位

- **对齐斜杠命令在 Channel 对话中的执行行为**
  - `/model`、`/think` 在 Channel 中执行后实际切换模型/推理强度，并通过 `slash:model_switched`、`slash:effort_changed` 事件同步前端 UI
  - `/stop` 支持通过 `ChannelCancelRegistry` 取消 Channel 中正在进行的流式输出
  - `/compact` 在 Channel 中直接执行上下文压缩（之前仅返回文本不执行）
  - `/clear` 执行后 emit `slash:session_cleared` 事件，前端消息列表和侧边栏同步刷新
  - `/export` 在 Channel 中自动写入 `~/.opencomputer/exports/` 目录并返回路径
  - `/permission` 在 Channel 中返回"不适用"提示（Channel 固定 auto-approve）
  - `/plan` 系列命令执行后 emit `slash:plan_changed` 事件同步前端 plan 状态
  - Channel worker 的 `reasoning_effort` 改为从 `AppState` 读取（之前硬编码 `None`）
  - 提取 `set_active_model_core`、`set_reasoning_effort_core`、`compact_context_now_core` 三个 core 函数供 Channel worker 复用

### Added

- **内置天气查询能力 (weather)**：Agent 可通过 `get_weather` 工具查询实时天气，支持在系统提示词中动态注入天气上下文
  - **Open-Meteo 集成**：使用免费无 Key 的 Open-Meteo API 获取天气和地理编码，免除用户配置成本
  - **位置与天气设置**：设置页新增「位置与天气」控制面板，支持城市搜索和手动输入经纬度坐标
  - **智能刷新与缓存**：天气数据 30 分钟缓存（并发安全），支持后台定时刷新（Tauri tick）和前端主动刷新
  - **上下文动态注入**：Agent 系统提示词自动注入当前位置和天气，且仅当天气发生变化（温度/状态码哈希检测）时才更新提示词，保证长期会话的 Prompt Cache 命中率
  - 2 个新 Tauri 命令：`geocode_search`（城市联想），`refresh_weather`（主动刷新）
- **系统提示词查看功能**
  - 新增 `/prompts` 斜杠命令，可在对话中快速查看当前会话的完整系统提示词
  - 对话界面右上角状态面板新增「查看系统提示词」按钮入口
  - 系统提示词以弹窗形式展示，支持一键复制
  - 新增 `get_system_prompt` Tauri 命令，根据当前 Agent 和模型动态构建并返回系统提示词

### Changed

- **聊天 Think/Tool 运行态展示优化（前端）**
  - Think 内容区域新增最大高度限制，超出后在内部滚动，避免思考块无限撑高消息气泡
  - Think 流式更新时自动滚动到底部，便于持续跟踪最新推理片段
  - Think 头部新增耗时显示，流式阶段按 100ms 粒度实时刷新，结束后保留最终耗时
  - Tool 调用项（单条与分组）新增耗时显示：运行中实时更新，完成后展示后端返回的最终 duration
  - Tool 调用流事件补充 `startedAtMs`/`durationMs` 前端字段，统一支持实时耗时与完成态耗时展示
  - Tool 完成时若后端未返回 `duration_ms`，前端会基于 `startedAtMs` 自动补算并写入最终耗时；历史消息回放也会读取 `toolDurationMs` 还原工具耗时
- **System Prompt 工具描述重构 + 行为指导增强**（参考 Claude Code System Prompts）
  - 工具描述从单一 60 行常量拆分为 31 个独立 per-tool 常量，每个工具包含详细使用指南、最佳实践和常见陷阱
  - `build_tools_section()` 重写为按 agent allow/deny 配置动态组装，只注入授权工具的描述，减少无关 token 消耗
  - 新增 3 个行为指导段：
    - **Output Efficiency**：简洁输出指引，减少 LLM 冗余回复
    - **Action Safety**：爆炸半径评估，破坏性操作需用户确认
    - **Task Execution Guidelines**：先读后改、避免过度工程、安全编码
  - 新增 8 个之前缺失的工具描述：update_memory、delete_memory、update_core_memory、manage_cron、browser、send_notification、canvas、acp_spawn
- **Plan Mode 架构重构：双模式支持 + 计划质量提升**
  - 支持**子 Agent 模式**（`plan_subagent: true`）和**内联模式**（默认），通过全局设置切换
  - 子 Agent 模式：Planning 阶段由独立子 Agent 执行，探索上下文不污染主 Agent 对话历史
  - 内联模式：与 Claude Code 一致，主 Agent 内联制定计划，保持上下文连续性
  - 新增 `PLAN_SUBAGENT_SESSIONS` 注册表，plan_question 和 submit_plan 事件自动路由到父 session
  - `SpawnParams` 扩展 `plan_agent_mode` / `plan_mode_allow_paths` / `skip_parent_injection` / `extra_system_context` 字段
  - 新增 `cancel_plan_subagent` Tauri 命令，退出 Plan Mode 时自动取消活跃的计划子 Agent
  - 前端新增 `planSubagentRunning` 状态和 "正在制定计划..." 动画指示器
  - **重写计划 system prompt**：以文件为中心组织步骤（非抽象 Phase），要求包含代码块、结构体定义、函数签名、file:line 引用等实现细节
  - 子 Agent 模式追加 `PLAN_SUBAGENT_CONTEXT_NOTICE`，要求计划自包含所有执行所需上下文

### Fixed

- **web_fetch 稳定性与性能优化**：重写 HTML 基础提取的标签跳过逻辑，移除基于字节索引的字符串切片，避免多字节 UTF-8 页面触发 panic；同时减少整页 `to_lowercase()` 复制带来的额外内存开销，并将 `max_chars` 截断改为按字符边界处理，防止截断时破坏 UTF-8 有效性
- **后端 UTF-8 截断稳定性加固**：修复多个输出截断路径的字节切片风险，统一改为 UTF-8 安全边界处理
  - `process_registry` 的 `aggregated_output` 与 `tail` 维护改为按字符数限制，避免多字节输出触发 panic，并新增 UTF-8 边界单测
  - Embedding Provider 测试失败信息 `detail` 截断改为 `truncate_utf8`，避免错误响应包含多字节字符时崩溃
- **web_fetch 安全校验增强**：新增 URL 解析与 scheme 白名单校验（仅允许 HTTP/HTTPS），并在重定向阶段增加本地/私网目标拦截（如 `localhost`、`127.0.0.1`、`::1`），降低 SSRF 绕过风险
- **Provider API Key 掩码安全性增强**：`masked()` 改为按字符而非字节截断，避免多字节字符导致 panic，并新增单元测试覆盖
- **Telegram 流式回复修复**：修正 `text_delta` 字段解析与 `sendMessageDraft` 调用参数，私聊优先使用 Telegram 官方 draft streaming，群聊/论坛自动回退到 `sendMessage` + `editMessageText` 预览链路，不再只在最后收到整条消息
- **系统托盘交互修复**：菜单栏图标恢复预期点击行为，左键显示主窗口、右键弹出菜单
  - `TrayIconBuilder` 现在显式关闭 `show_menu_on_left_click`，避免和自定义左键打开主窗口逻辑互相打架
  - tray icon 改为专用小图 `menuIconTray.png`，不再把 `1830x1830` 的大 PNG 直接嵌入 Tauri 二进制
  - 新增 tray setup / click / menu item 调试日志，便于继续排查 macOS 菜单栏事件异常
- **本地 loopback 代理绕行修复**：访问 Docker SearXNG 和本地 Chrome CDP 时，`localhost` / `127.0.0.1` / `::1` 目标现在会自动直连，不再误走系统代理导致 503 或连接失败
  - `web_search` 的 SearXNG client 改为按目标 URL 判断是否绕过代理，修复本地 Docker 实例搜索 503，同时保留远程 SearXNG 走代理能力
  - SearXNG Docker 部署返回地址、默认回退地址和设置面板填充地址统一改为 `127.0.0.1`
  - 浏览器 CDP 自动连接与默认连接地址统一改为 `127.0.0.1:9222`，避免系统代理拦截本地调试端口
  - SearXNG `start()` 前会先刷新挂载的 `settings.yml`，代理配置变更后无需重新部署容器，直接重启即可生效
  - 新增 SearXNG Docker “向容器注入代理”开关；关闭后不会写入 `settings.yml` 的 `outgoing.proxies`，适合系统 VPN 已接管出网的场景
- **Plan Mode 内联评论提示词包装**：评论消息使用 `<plan-inline-comment>` 结构化标签包裹，后端 system prompt 同步补充内联评论处理说明，模型能正确理解"对计划的修改意见"意图
- **Plan Mode 选中文本高亮**：计划面板评论时选中区域以蓝色 `<mark>` 高亮显示，弹窗关闭后自动清除，支持跨元素选区降级处理
- **Plan Mode 问答回溯**：`plan_question` 工具调用结果不再隐藏，改为在消息流中渲染绿色 Q&A 摘要卡片，与 Think/Tool Call 保持时序，不再因状态清除而消失

### Added

- **提示词系统技术文档** (`docs/prompt-system.md`)：完整记录 System Prompt 13 段组装流程、31 个 per-tool 描述清单、3 个行为指导段、Plan Mode 提示词、上下文压缩提示词、条件注入段、缓存优化策略等
- **温度配置三层覆盖**：支持全局、Agent、会话三个层级的 LLM 温度（Temperature）配置，覆盖优先级：会话 > Agent > 全局
  - 全局设置面板（GlobalModelPanel）新增温度滑块，范围 0.0–2.0，存储在 `config.json` 的 `temperature` 字段
  - Agent 模型配置（ModelTab）新增温度覆盖选项，继承/自定义模式，存储在 `agent.json` 的 `model.temperature` 字段
  - 聊天输入框（ChatInput）新增温度弹出菜单，会话级即时调整，通过 `temperatureOverride` 参数传递给后端
  - 后端 `AssistantAgent` 新增 `temperature` 字段，四种 Provider（Anthropic/OpenAI Chat/OpenAI Responses/Codex）均已适配
  - 新增 Tauri 命令 `get_global_temperature` / `set_global_temperature`
  - 新增 `src/components/ui/slider.tsx` Radix UI Slider 组件

### Changed

- **Plan Mode 计划面板协同编辑重构**：
  - 计划面板不再在进入计划模式时立即显示，仅在计划 Markdown 内容生成后自动展示
  - 移除计划面板中的手动编辑 textarea，所有状态下均为只读 Markdown 渲染
  - 新增选中文本评论功能（CommentPopover），用户选中计划文本后弹出评论框，评论以引用格式发送给模型进行修订
  - 移除独立的"请求修改"按钮，改为更精准的内联评论协同编辑方式
- **Plan Mode 双 Agent 架构重构**：从单 Agent 状态机切换改为 Plan Agent / Build Agent 双 Agent 架构
  - 新增 `PlanAgentConfig` 声明式配置，Plan Agent 使用工具白名单（替代 denied_tools 黑名单）
  - 新增 `PlanAgentMode` 枚举（Off/PlanAgent/BuildAgent）统一控制工具注入
  - 新增 `apply_plan_tools()` 共享方法，消除 4 个 Provider 各自重复的 8 行条件注入代码
  - 移除 `AssistantAgent` 上的 3 个 plan 专用字段（plan_ask_tools/plan_executing/plan_tools_enabled）
  - `commands/chat.rs` plan 分支从 100+ 行 if/else 简化为 ~50 行 match 表达式
  - PlanCardBlock Phase 列表支持点击展开显示步骤详情

### Added

- **图片生成能力增强**：追平 OpenClaw，全面增强图片生成工具
  - **新增 MiniMax Provider**：支持 image-01 模型，最多生成 9 张图片，支持 aspectRatio 和参考图编辑
  - **图片编辑支持**：Google（最多 5 张参考图）、Fal（1 张）、MiniMax（1 张）均支持参考图输入编辑
  - **aspectRatio 参数**：支持 10 种比例（1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9），Google/Fal/MiniMax 可用
  - **resolution 参数**：支持 1K/2K/4K 分辨率，Google/Fal 可用，编辑时自动从参考图推断
  - **action=list 查询**：Agent 可查询所有 Provider 的模型和能力详情
  - **Provider Capabilities 系统**：每个 Provider 声明 generate/edit 能力和几何约束，自动跳过不兼容的 Provider
  - **新增尺寸**：Google/Fal 支持 1024x1792 和 1792x1024
- **新增 3 个国产图片生成 Provider**：
  - **硅基流动 SiliconFlow**：聚合 Qwen-Image/Kolors 等多模型，支持 Qwen-Image-Edit 图片编辑，OpenAI 兼容 API
  - **智谱 ZhipuAI CogView-4**：中文文字渲染能力最强，支持 2048x2048 分辨率，OpenAI 兼容 API
  - **通义万相 Tongyi Wanxiang**：DashScope 异步 API，支持文生图和 wanx2.1-imageedit 描述编辑，自动轮询任务结果
- **数据大盘详情列表**：Overview 卡片点击展开详情列表面板
  - 新增 5 个后端查询命令：`dashboard_session_list` / `dashboard_message_list` / `dashboard_tool_call_list` / `dashboard_error_list` / `dashboard_agent_list`
  - 6 种详情列表：会话列表、消息列表、工具调用列表、错误日志列表、Agent 列表、定时任务列表
  - 卡片点击 toggle 展开/收起，活跃卡片高亮边框，列表复用全局 DashboardFilter
- **Plan Mode 深度增强**：对标 OpenCode/Claude Code，全面提升计划模式的可靠性、灵活性和智能水平
  - **步骤进度持久化**：plan_steps 列持久化到 SessionDB，崩溃/重启后步骤进度完整恢复（P0）
  - **子 Agent 安全继承**：Planning/Review 状态下 spawn 的子 Agent 自动继承 PLAN_MODE_DENIED_TOOLS 限制，修复工具限制泄漏安全漏洞（P0）
  - **exec 审批激活**：Planning/Review 状态下 exec 工具需要用户审批，激活原有定义但从未生效的 PLAN_MODE_ASK_TOOLS（P0）
  - **Plan/Build 独立模型**：Agent 配置新增 `planModel` 字段，Planning 阶段可使用更便宜/快速的模型探索，执行阶段用强模型生成代码（P1）
  - **Completed 状态系统提示词**：计划执行完成后注入 PLAN_COMPLETED_SYSTEM_PROMPT，指导 LLM 总结执行结果、标注失败步骤、建议后续操作（P1）
  - **项目本地化计划文件**：git 仓库内计划存储到 `.opencomputer/plans/`（可随项目版本控制），非 VCS 项目回退到全局目录（P1）
  - **5 阶段规划流程**：全新 PLAN_MODE_SYSTEM_PROMPT，引入 Deep Exploration → Requirements Clarification → Design & Architecture → Plan Composition → Review & Refinement 五阶段工作流，推荐使用子 Agent 并行探索代码库（P1）
  - **细粒度路径权限**：Planning 阶段 write/edit 工具仅允许编辑 `.opencomputer/plans/` 下的计划文件，通过 `plan_mode_allow_paths` 在 ToolExecContext 中传播路径白名单（P2）
  - **计划版本管理**：保存计划时自动备份旧版本为 `plan-xxx-v{N}.md`，PlanPanel 支持版本历史浏览与一键恢复（P2）
  - **执行中修改计划**：新增 `amend_plan` 工具，Executing/Paused 状态下支持 insert/delete/update 步骤，自动重编号 + 计划文件再生成 + `plan_amended` 事件驱动前端实时更新（P3）
  - **Git Checkpoint 回滚**：进入 Executing 状态时自动创建 git 分支 checkpoint，执行失败后 PlanPanel 显示回滚按钮（`git reset --hard`），成功完成后自动清理 checkpoint 分支（P3）
  - **plan_question 增强**：选项支持 `recommended` 标记（琥珀色星标高亮），问题支持 `template` 模板分类（scope/tech_choice/priority 对应不同图标）
  - **Review 请求修改**：PlanPanel Review 状态新增"请求修改"按钮，用户输入反馈文本后自动转回 Planning 状态，将反馈发送给 LLM 修订计划
  - **Plan Model 前端配置**：Agent 设置面板新增 Plan Mode Model 选择器，琥珀色 Lightbulb 图标标识
  - **自定义 plansDirectory**：ProviderStore 新增 `plans_directory` 配置项，支持覆盖默认计划文件存储路径
- **系统托盘常驻（System Tray）**：应用关闭窗口后常驻系统托盘，不再退出
  - 菜单栏/系统托盘图标，提供快捷菜单（显示主窗口/快捷对话/新建对话/设置/退出）
  - 关闭主窗口仅隐藏，应用在后台持续运行，全局快捷键始终可用
  - 左键单击托盘图标直接显示主窗口
  - macOS: 点击 Dock 图标恢复主窗口（`RunEvent::Reopen`）
  - 托盘菜单"退出"才会真正退出应用
- **快捷对话快捷键（Quick Chat Shortcut）**：全局快捷键 Option+Space（Alt+Space）快速唤起 Spotlight 风格浮动对话框
  - 居中浮层对话框，包含聊天输入、消息预览、Agent 快捷选择
  - 连续唤起默认加载上一次快捷会话，支持新建会话
  - 复用 ChatInput 组件，保留模型选择、斜杠命令、文件附件等完整功能
  - Agent 切换自动保存/恢复对应会话
  - "查看完整对话"一键跳转到主聊天界面
  - 使用 `tauri-plugin-global-shortcut` 实现系统级全局快捷键
- **Plan Mode 重构（交互式计划模式）**：完全重新设计的 Plan Mode，支持交互式问答制定计划
  - 六态状态机：Off → Planning → Review → Executing → Paused/Completed
  - **交互式计划制定**：`plan_question` 工具发送结构化问题（含建议选项），前端渲染可视化选择卡片，用户选择/自定义输入后提交继续
  - **计划提交**：`submit_plan` 工具提交最终计划，自动转入 Review 状态
  - **计划卡片**：消息流中嵌入 PlanCardBlock 计划摘要卡片（标题/阶段/步骤数/进度），点击查看完整计划
  - **执行控制**：可暂停/恢复执行，`/plan pause` 和 `/plan resume` 斜杠命令
  - ChatInput Plan 按钮五色状态（灰/蓝/紫/绿/黄）对应不同阶段
  - PlanPanel 右侧面板支持 Review（只读 Markdown 渲染）、Paused（暂停标识）、Completed（完成统计）视图
  - 复用 approval.rs 的 oneshot channel 阻塞模式实现前后端问答交互
  - Plan 文件持久化到 `~/.opencomputer/plans/`，会话状态持久化到 DB
  - 子 Agent 继承 Plan Mode 工具限制（防止逃逸）
- **Core Memory（核心记忆）**：全局 `~/.opencomputer/memory.md` 和 Agent 级 `agents/{id}/memory.md` 文件全文注入系统提示词，用于长期规则/偏好/指令。用户可在设置面板编辑，Agent 可通过 `update_core_memory` 工具主动修改（支持 append/replace + global/agent 作用域）
- **Pinned（置顶）记忆**：记忆条目支持置顶功能，pinned 记忆在系统提示词中优先注入并带 ★ 标记，不受时间排序影响。前端记忆面板添加 Pin 按钮
- **Memory Flush（压缩前记忆保存）**：上下文压缩 Tier 3 摘要前自动提取即将被丢弃消息中的重要信息保存为记忆，防止信息丢失。可通过 `flushBeforeCompact` 配置开启
- **历史会话搜索**：messages 表添加 FTS5 全文索引，`recall_memory` 工具新增 `include_history` 参数，支持搜索历史对话消息（排除 cron 和子 Agent 会话）

### Changed

- **图片生成系统重构（image_generate）**：Provider 抽象 + 排序降级 + 动态工具描述
  - 引入 `ImageGenProviderImpl` trait 抽象，支持可扩展的 Provider 架构
  - Provider id 从枚举改为 String（向后兼容，自动 normalize "OpenAI" → "openai"）
  - 实现自动降级（Failover）循环：按优先级遍历 Provider，retryable 错误自动重试，失败后降级到下一个
  - 复用 `failover::classify_error` + `failover::retry_delay_ms` 指数退避重试
  - 工具描述动态生成：只列出已启用的模型名称和优先级顺序
  - 工具参数简化：去掉 `provider` 参数，改为 `model` 参数（默认 auto），LLM 视角更简洁
  - 结果透明度：返回实际使用的模型信息，如发生降级则详细记录过程
  - 前端设置面板添加 Provider 排序功能（上下箭头 + 优先级序号）

### Added

- **ACP 控制面（ACP Control Plane）**：让模型能启动和管理外部 ACP Agent（Claude Code、Codex CLI、Gemini CLI 等）
  - `AcpRuntime` trait 可插拔后端抽象 + `StdioAcpRuntime` 子进程 stdio/NDJSON 实现
  - `AcpRuntimeRegistry` 全局后端注册表 + 自动发现（扫描 $PATH 中的 claude/codex/gemini）
  - `AcpSessionManager` 会话生命周期管理（spawn/check/kill/steer + 异步 tokio::spawn 执行）
  - `acp_spawn` 工具（8 种 action：spawn/check/list/result/kill/kill_all/steer/backends）
  - 系统提示词 Section ⑬ 条件注入 ACP 外部 Agent 委派说明
  - `acp_runs` SQLite 表持久化运行记录（自动迁移）
  - 8 个 Tauri 命令（acp_list_backends/acp_health_check/acp_refresh_backends/acp_list_runs/acp_kill_run/acp_get_run_result/acp_get_config/acp_set_config）
  - 前端设置面板 `AcpControlPanel.tsx`（启用开关、后端列表、健康状态、配置管理）
  - 聊天嵌入组件 `AcpSpawnBlock.tsx`（流式输出、工具调用、状态、Kill 按钮）
  - `AcpControlConfig` 全局配置 + `AgentAcpConfig` per-Agent 配置（allowed_backends/denied_backends/max_concurrent）
  - 流式事件实时推送到前端（Tauri 全局事件 `acp_control_event`）
  - 新增 `src-tauri/src/acp_control/` 模块目录（8 个文件）+ `src-tauri/src/tools/acp_spawn.rs`
  - 新增依赖：`async-trait`、`which`
- **ACP 协议支持（Agent Client Protocol）**：原生 Rust 实现 ACP 服务器，IDE（Zed/VS Code 等）可通过 stdio + NDJSON 直接连接 OpenComputer Agent
  - 通过 `opencomputer acp` 子命令启动 ACP 服务器（支持 `--verbose`/`--agent-id`/`--help` 参数）
  - 完整的 JSON-RPC 2.0 协议实现（NDJSON stdio 传输层）
  - 会话管理：`session/new`、`session/load`（完整历史重放）、`session/list`、`session/close`
  - Prompt 执行：流式事件映射（text_delta→agent_message_chunk、thinking_delta→agent_thought_chunk、tool_call/tool_result→tool_call/tool_call_update）
  - 多 Agent 模式切换（`session/setMode`）+ 动态配置选项（`session/setConfigOption`）
  - 完整 failover 支持：复用现有模型链降级策略（RateLimit 重试 + 多模型降级）
  - 会话持久化：共享 SessionDB，ACP 会话与桌面端会话数据互通
  - 新增 `src-tauri/src/acp/` 模块目录（7 个文件：`mod.rs`/`types.rs`/`protocol.rs`/`event_mapper.rs`/`session.rs`/`agent.rs`/`server.rs`）
- **技能系统全面升级**：追平并超越 OpenClaw 的 skill 系统能力
  - **懒加载 Prompt 注入**：系统提示词仅注入技能目录（名称+描述+路径），LLM 按需 read SKILL.md 全文，大幅节省 token
  - **三层预算降级**：Full（名称+描述+路径）→ Compact（名称+路径）→ 二分搜索截断，确保技能数量增长不会溢出 prompt
  - **路径压缩**：home 目录替换为 `~`，每个技能节省 ~5-6 tokens
  - **Requirements 增强**：新增 anyBins（OR 逻辑）、always（跳过所有检查）、config（配置路径检查）、primaryEnv（apiKey 满足主环境变量）
  - **调用策略**：`user-invocable` 控制是否注册为斜杠命令，`disable-model-invocation` 控制是否注入 prompt
  - **Skill 与斜杠命令统一**：user-invocable 的技能自动注册为 `/skillname` 斜杠命令（Skill 分类），支持 `command-dispatch: tool` 绑定工具直接调用
  - **安装引导**：SKILL.md `install:` 块支持 brew/node/go/uv/download 五种安装方式，设置面板一键安装 + 二进制验证
  - **健康检查**：`get_skills_status` 命令返回结构化诊断（eligible/disabled/blocked/missing_bins/missing_env），前端状态徽章
  - **嵌套目录检测**：自动发现 `dir/skills/*/SKILL.md` 嵌套结构
  - **Skill 缓存**：AtomicU64 版本号 + 30 秒 TTL，配置变更自动失效
  - **可配置预算限制**：`SkillPromptBudget`（max_count/max_chars/max_file_bytes/max_candidates_per_root）
  - **Bundled Allowlist**：`skill_allow_bundled` 限制可用的 bundled 技能集

### Changed

- **API 请求/响应全链路日志增强**：大幅提升所有外部 API 调用的 debug 级别日志详细度，覆盖 Agent Provider、Embedding、图片生成三大模块
  - **Agent Provider（4 个）**：原始请求体（脱敏+截断 32KB）、响应头（rate limit/model version/request-id/retry-after）、工具执行全链路（参数/结果/耗时/错误标记）
  - **Embedding API（OpenAI/Google）**：请求参数（model/text_count/dimensions/body）、响应状态（status/ttfb/body 摘要）、Google 逐条请求日志
  - **图片生成 API（OpenAI/Google/Fal）**：请求参数（model/prompt 预览/size/n）、响应状态（status/ttfb/request-id）、错误响应体完整记录

### Added

- **系统监控大盘（System Metrics Dashboard）**：数据大盘新增「系统监控」Tab，实时展示本机 CPU、内存、网络等系统资源使用情况
  - **CPU 监控**：全局使用率 + 每核心使用率柱状图，支持多核心可视化
  - **内存监控**：总内存/已用/可用 + RAM/Swap 双环形图，百分比实时展示
  - **网络流量**：按网卡分组统计接收/发送流量，水平柱状图 + 详情表格
  - **系统信息**：操作系统、主机名、运行时间、CPU 核心数概览卡片
  - 后端使用 `sysinfo` crate 采集系统指标，通过 `dashboard_system_metrics` Tauri 命令暴露
- **仪表盘分析模块（dashboard）**：新增 `dashboard.rs` 后端模块 + 6 个 Tauri 命令，提供会话/Token/工具/错误/任务多维度统计分析
  - **概览统计**：会话数、消息数、Token 用量、工具调用、错误数、活跃 Agent/定时任务数、预估费用
  - **Token 用量**：按日趋势 + 按模型分组统计 + 硬编码 20+ 模型定价预估费用（Claude/GPT/Gemini/DeepSeek/Qwen）
  - **工具使用**：按工具名分组统计调用次数、错误次数、平均/总耗时
  - **会话分析**：按日趋势 + 按 Agent 分组统计（会话数/消息数/Token 总量）
  - **错误分析**：从日志库按日趋势 + 按分类分组统计（error/warn 双维度）
  - **任务统计**：定时任务（总数/活跃/成功/失败/平均耗时）+ 子 Agent（总数/完成/失败/终止/Token/耗时）
  - 所有查询支持 `DashboardFilter`（时间范围/Agent/Provider/模型过滤），自动排除 cron 会话和子 Agent 会话
- **画布工具（canvas）**：新增第 29 个内置工具，支持交互式可视化内容创作
  - **7 种内容类型**：HTML/CSS/JS（Web 应用、游戏、动画）、Markdown（富文档）、Code（语法高亮）、SVG（矢量图形）、Mermaid（图表）、Chart（Chart.js 数据可视化）、Slides（演示文稿）
  - **11 个操作**：create/update/show/hide/snapshot/eval_js/list/delete/versions/restore/export
  - **实时预览**：右侧 CanvasPanel 面板（iframe 沙箱渲染），通过 Tauri asset protocol 加载，零网络依赖
  - **视觉反馈循环**：html2canvas 截图 → base64 → IMAGE_BASE64_PREFIX 回传 LLM，实现 AI 视觉验证与迭代
  - **JavaScript 执行**：eval_js 操作通过 postMessage 双向通信在 canvas iframe 中执行代码
  - **版本历史**：每次 update 自动创建版本快照，支持查看历史和恢复到指定版本（SQLite 持久化）
  - **文档协作**：类似 Gemini 的 AI 文档创建/编辑/预览体验
  - **条件注入**：全局开关控制，配置存储在 config.json 的 `canvas` 字段
  - **设置面板**：新增 Canvas 设置页面（启用开关、自动显示、默认类型、项目/版本上限）
  - **存储**：项目文件在 `~/.opencomputer/canvas/projects/{id}/`，元数据在 `~/.opencomputer/canvas/canvas.db`
- **图片生成工具（image_generate）**：新增第 28 个内置工具，支持 3 个 AI 图片生成 Provider
  - **OpenAI**：DALL-E / gpt-image-1，通过 `/v1/images/generations` API 生成，支持 b64_json 返回
  - **Google**：Gemini 图片生成（`gemini-2.0-flash-preview-image-generation`），通过 `generateContent` API 的 `responseModalities: ["IMAGE"]` 模式
  - **Fal**：Flux 模型（`fal-ai/flux/dev`），通过 CDN URL 下载生成结果
  - **条件注入**：仅在有配置了 API Key 的 Provider 时才向 Agent 注入该工具（同 `send_notification` 模式）
  - **图片持久化**：生成的图片自动保存到 `~/.opencomputer/generated-images/`，带时间戳文件名
  - **视觉反馈**：通过 `IMAGE_BASE64_PREFIX` 机制将生成图片回传给 LLM，实现 Agent 视觉确认
  - **设置面板**：工具设置新增"图片生成"Tab，支持 Provider 开关、API Key、Base URL、Model、默认尺寸、超时配置
- **Docker 沙箱安全加强**：全面提升容器隔离安全性
  - **P0 修复断连**：Agent 设置面板的 `behavior.sandbox` 开关现在真正生效，`ToolExecContext` 新增 `force_sandbox` 字段自动注入
  - **P1 安全加固**：默认镜像从 `ubuntu:22.04` 更换为 `debian:bookworm-slim`（更小更快）；新增 6 项 Docker 安全配置——只读根文件系统（`--read-only`）、移除所有 capability（`--cap-drop ALL`）、禁止新权限（`--no-new-privileges`）、网络隔离（`--network none`）、进程数限制（`--pids-limit 256`）、tmpfs 可写临时目录（`/tmp`、`/var/tmp`、`/run`）
  - **P2 环境变量过滤**：`sanitize_env()` 拦截 API Key、Token、Password 等 20+ 种敏感环境变量模式，白名单放行 PATH/HOME/LANG 等安全变量
  - **P3 挂载路径校验**：`validate_bind_mount()` 禁止挂载 `/etc`、`/proc`、`/sys`、`/dev`、`/root`、Docker socket 等系统关键路径，防止 symlink 逃逸
  - **P4 系统提示词**：当 `behavior.sandbox` 启用时，自动注入 Section ⑪ 告知 LLM 沙箱特性和限制
  - **P5 设置面板**：新增 Sandbox 设置页面（Docker 可用性检测、镜像配置、资源限制、安全开关），3 个 Tauri 命令（`get_sandbox_config`、`set_sandbox_config`、`check_sandbox_available`）
- **斜杠命令系统（Slash Commands）**：输入框键入 `/` 自动展开命令菜单，支持 16 个内置命令
  - **架构**：命令解析和执行在 Rust 后端实现（`slash_commands/` 模块），channel-agnostic 设计，未来可复用于 Telegram/Discord/Slack 等渠道
  - **5 个命令类别**：会话（`/new` `/clear` `/compact` `/stop` `/rename`）、模型（`/model` `/think`）、记忆（`/remember` `/forget` `/memories`）、Agent（`/agent` `/agents`）、工具（`/help` `/status` `/export` `/usage` `/search`）
  - **后端**：3 个 Tauri 命令（`list_slash_commands` / `execute_slash_command` / `is_slash_command`），返回 `CommandResult`（content + CommandAction 枚举），各 channel 按 action 类型执行副作用
  - **前端**：弹出菜单 UI（按分类分组、键盘 ↑↓ 导航、模糊过滤）、`/` 按钮触发、集成到 ChatInput 键盘事件拦截
  - **i18n**：中/英双语命令描述和分类标签
- **P1 工具能力增强**：新增 8 个内置工具（工具总数 19 → 27）
  - `memory_get`：按 ID 精确读取记忆完整内容和元数据
  - `agents_list`：列出所有可用 Agent 及其配置信息
  - `sessions_list`：列出所有会话元数据（标题、Agent、模型、消息数）
  - `session_status`：查询单个会话的详细状态
  - `sessions_history`：分页读取会话聊天历史（支持分页游标、工具消息过滤、80KB 输出上限）
  - `sessions_send`：跨会话消息发送（支持同步等待和异步投递两种模式）
  - `image`：独立图像分析工具（支持 prompt 参数指定分析内容，复用 read.rs 的图像检测和缩放逻辑）
  - `pdf`：PDF 文档文本提取（支持页码范围过滤、字符数上限、按页分隔输出）
  - 前端：8 个新工具图标 + i18n（中/英）+ 参数摘要显示 + 工具分组归类
  - 系统提示词：Section ⑥ 新增 8 个工具描述
  - 内部工具（无需审批）：memory_get、agents_list、sessions_list、session_status、sessions_history
- **记忆系统优化（Phase 1.5）**：5 项优化增强
  - **Prompt Summary 优先级加权**：`build_prompt_summary` 改为逐条添加直到超出 budget，避免在记忆内容中间截断，保持 `updated_at DESC` 排序优先展示最近更新的记忆
  - **提取模型可配**：`MemoryConfig` 新增 `extractProviderId`/`extractModelId` 字段，auto-extract 可使用独立的便宜模型，前端 MemoryPanel 展示模型选择器和最少轮数配置
  - **memory_extracted Toast 通知**：聊天界面监听 `memory_extracted` 事件，显示轻量 banner "从对话中提取了 N 条新记忆"，4 秒后自动消失
  - **去重阈值可配置**：`DedupConfig` 存储在 `config.json` 的 `dedup` 字段，Embedding 设置页新增可折叠"去重高级设置"区域，支持调节重复/合并阈值
  - **记忆统计仪表板**：新增 `memory_stats` 命令返回 `MemoryStats`（总数/按类型/向量覆盖率），MemoryPanel list 视图顶部显示统计行
- **子 Agent 系统全面升级**：9 种操作 + Steer 干预 + 附件传递 + 标签 + 工具策略 + 批量操作
  - **Steer 运行中干预**：新增 `steer` action，通过 `SubagentMailbox` 消息邮箱模式在子 Agent tool loop 每轮注入消息，改变运行方向而无需 kill 重来
  - **文件附件传递**：spawn 时可传递 `files` 参数（支持 utf8/base64），自动转为 Attachment 传入子 Agent
  - **Label 标签系统**：每个 run 可附带 `label` 便于追踪、定位和按标签操作
  - **深度分层工具策略**：`SubagentConfig.deniedTools` 可限制子 Agent 可用工具集，支持 orchestrator vs leaf worker 差异化
  - **批量操作**：`batch_spawn` 一次 spawn 最多 10 个任务，`wait_all` 等待多个 run 完成
  - **Token 统计**：记录 `input_tokens`/`output_tokens` 到 DB，前端 SubagentBlock 展示统计
  - **可配置最大嵌套深度**：`maxSpawnDepth`（1-5，默认 3），per-Agent 配置
  - **可配置结果注入超时**：`announceTimeoutSecs`（10-600，默认 120）
  - 系统提示词 Section ⑩ 更新：含 steer/files/label/batch 用法说明
  - 前端 SubagentBlock 增强：显示 label、model、token 统计、附件角标
  - 前端 SubagentPanel 增强：新增 maxSpawnDepth 和 announceTimeout 配置

### Changed

- **重构 `agent.rs` 为模块目录**：将 2940 行的 `agent.rs` 拆分为 `agent/` 模块目录，提升可维护性
  - `agent/mod.rs`：模块声明 + 公共 API 重导出 + 构造器/setter/chat 分发器
  - `agent/types.rs`：核心类型定义（`AssistantAgent`、`LlmProvider`、`Attachment`、`ChatUsage`、`CodexModel`、`ThinkTagFilter`）
  - `agent/config.rs`：常量、系统提示词构建、API URL 构建、thinking 风格映射
  - `agent/content.rs`：多模态内容构建器（Anthropic/OpenAI Chat/Responses 三种格式）
  - `agent/events.rs`：前端事件发射函数（text_delta/tool_call/tool_result/thinking_delta/usage）
  - `agent/api_types.rs`：SSE/请求/响应 DTO 类型（15+ struct）
  - `agent/context.rs`：上下文管理（compaction、summarization、conversation history）
  - `agent/errors.rs`：错误处理与重试判断
  - `agent/providers/`：四种 Provider 独立实现（anthropic.rs、openai_chat.rs、openai_responses.rs、codex.rs）
  - 公共 API 保持不变，外部调用方无需修改

### Added

- **子 Agent 配置、调度与协作通讯系统**：Agent 可通过 `subagent` 工具委派子任务给其他 Agent
  - 新增 `subagent` 工具：spawn（委派任务）、check（轮询状态）、list（查看所有子 Agent）、result（获取完整结果）、kill/kill_all（终止）
  - 非阻塞异步执行：spawn 立即返回 run_id，子 Agent 在隔离 session 中独立运行
  - 最大嵌套深度 3 层，每个父 session 最多 5 个并发子 Agent
  - 完整模型链降级：子 Agent 复用 cron 的 `build_and_run_agent` 模式（load agent → resolve model chain → failover retry）
  - `SubagentConfig` per-Agent 配置：启用/禁用、允许/禁止委派的 Agent 列表、最大并发数、默认超时、模型覆盖
  - SQLite 持久化 `subagent_runs` 表：记录所有子 Agent 运行状态、结果、耗时
  - 取消注册表（`SubagentCancelRegistry`）：基于 `AtomicBool` 的运行时取消机制
  - Tauri 全局事件 `subagent_event`：前端实时收到 spawned/completed/error/killed/timeout 通知
  - 系统提示词自动注入子 Agent 委派说明（section ⑩），包含可用 Agent 列表和用法
  - 前端组件：`SubagentBlock.tsx`（聊天内嵌实时状态）、`SubagentPanel.tsx`（Agent 设置面板子 Agent 配置）
  - Tauri 命令：`list_subagent_runs`、`get_subagent_run`、`kill_subagent`
  - Cron 任务也支持生成子 Agent（depth=0）
- **系统消息通知功能**：macOS 原生桌面通知，支持三级粒度控制
  - 全局通知开关（默认开启），通过 `tauri-plugin-notification` 实现原生通知
  - 按 Agent 级别通知覆盖配置（默认/开启/关闭）
  - 按定时任务级别通知开关，在定时任务创建/编辑表单中配置
  - 非当前会话的模型回复完成或异常时发送通知
  - 定时任务执行成功/失败后发送通知
  - Agent 可自主调用 `send_notification` 工具发送通知（仅在通知开启时注入）
  - 通知设置面板（设置 → 通知），支持全局开关 + 按 Agent 独立配置
- **自愈式自动重启系统**：Guardian Process 架构实现全类型崩溃检测与自动恢复
  - Guardian/Child 双模式进程架构：同一二进制通过 `OPENCOMPUTER_CHILD` 环境变量区分模式，Guardian 作为父进程监控子进程退出码
  - 捕获所有崩溃类型：Rust panic、segfault（SIGSEGV）、OOM kill（SIGKILL）、abort（SIGABRT）等
  - 智能重启策略：指数退避（1s→3s→9s→15s→30s）、10 分钟窗口自动重置崩溃计数
  - 信号转发：SIGTERM/SIGINT 正确转发给子进程，macOS Force Quit 不会被误判为崩溃
  - 退出码约定：0=正常退出、42=请求重启、其他=崩溃
  - 配置备份系统：连续崩溃 5 次后自动备份 config.json、user.json、agents/、credentials/ 到 `~/.opencomputer/backups/`，保留最近 5 份
  - LLM 自诊断：读取崩溃日志 + 纯文本日志，遍历所有可用 Provider（按 cost 排序）调用 LLM 分析崩溃原因，全部失败降级为基于退出码/信号的基础分析
  - 保守自动修复：仅修复 config.json 损坏、logs.db 损坏、compact 配置异常，绝不动凭证和会话数据
  - 崩溃日志（crash_journal.json）：JSON 格式持久化崩溃记录（最近 50 条），记录退出码、信号名、诊断结果
  - 新增设置 → 系统健康面板：崩溃历史、诊断结果展示、手动创建/恢复备份、一键重启
  - 崩溃恢复横幅：应用从崩溃恢复后在聊天界面顶部显示通知横幅
  - 新增 `crash_journal.rs`、`backup.rs`、`self_diagnosis.rs` 后端模块
  - 7 个 Tauri 命令：`get_crash_recovery_info` / `get_crash_history` / `clear_crash_history` / `request_app_restart` / `list_backups_cmd` / `restore_backup_cmd` / `create_backup_cmd`
- **对话上下文压缩系统**：4 层渐进式上下文压缩，防止 context overflow 卡死会话。参考 openclaw 方案优化适配桌面场景
  - Tier 1：工具结果截断 — 单个结果超过 context 30% 时 head+tail 截断（结构感知边界切割）
  - Tier 2：上下文裁剪 — 软裁剪旧工具结果 → 硬替换为占位符，基于 age×size 优先级评分
  - Tier 3：LLM 摘要 — 调用当前模型摘要旧消息，保留最近 N 轮，3 级 fallback
  - Tier 4：溢出恢复 — ContextOverflow 不再是 terminal 错误，触发紧急压缩后自动重试
  - Token 估算校准器：利用 API 返回的实际 input_tokens 做 EMA 滑动平均校准
  - 新增 `context_compact.rs` 后端模块，`CompactConfig` 配置存储在 `config.json` 的 `compact` 字段
  - 新增设置面板「上下文管理」：3 个可折叠区域（工具裁剪 / 摘要压缩 / 高级设置），15 个可配置参数
  - 修复 `tool_context()` 始终传 `None` 的问题，工具输出现在自适应 context window
  - 2 个 Tauri 命令：`get_compact_config` / `save_compact_config`
- **系统权限管理页面**：新增设置 → 系统权限面板，检测并引导用户授权 macOS 辅助功能、屏幕录制、自动化、应用管理、完全磁盘访问、文件和文件夹、定位服务、通讯录、日历、提醒事项、照片、相机、麦克风、本地网络、蓝牙（共 15 项）。新增 `permissions.rs` 后端模块（`check_all_permissions` / `check_permission` / `request_permission` 三个 Tauri 命令），支持自动检测权限状态、跳转系统设置授权、窗口聚焦时自动刷新
- **浏览器 Profile 隔离**：`browser` 工具 `launch` action 新增 `profile` 参数，支持多配置档隔离（独立 cookies/存储/登录状态）。新增 `list_profiles` action 列出已有配置档。Profile 数据存储在 `~/.opencomputer/browser-profiles/{name}/`
- **浏览器 PDF 导出**：`browser` 工具新增 `save_pdf` action，将当前页面导出为 PDF 文件。支持 `paper_format`（a3/a4/a5/letter/legal/tabloid）、`landscape`、`print_background` 参数，默认输出到 `~/.opencomputer/share/`
- **记忆 Embedding Provider 测试功能**：向量搜索设置新增"测试 Embedding"按钮，支持 OpenAI 兼容 API、Google Gemini、本地 ONNX 模型三种类型的连接测试，复用 `TestResultDisplay` 组件展示测试结果（状态码、延迟、返回维度）
- **记忆系统增强 — Embedder 自动初始化**：应用启动时若 embedding 已配置并启用，自动初始化 embedder，无需用户手动触发。`save_embedding_config` 保存后立即 apply 到运行中的后端
- **记忆系统增强 — 去重检测**：新增 `find_similar` / `add_with_dedup` trait 方法，Agent 保存记忆时自动检测相似条目（RRF 混合评分），高相似度跳过、中等相似度合并。前端手动添加时弹出确认对话框。新增 `memory_find_similar` Tauri 命令
- **记忆系统增强 — 导入 + 批量操作**：
  - 支持从 JSON / Markdown 文件导入记忆（含可选去重），新增 `parse_import_json` / `parse_import_markdown` 解析函数
  - 列表多选模式（checkbox），批量删除、批量重新生成 Embedding
  - Embedding 设置页新增"重新生成全部向量"按钮
  - 新增 `memory_delete_batch` / `memory_import` / `memory_reembed` Tauri 命令
- **记忆系统增强 — 自动记忆提取**：对话完成后异步提取值得记住的信息（用户事实、偏好、项目上下文），通过 `tokio::spawn` 后台执行不阻塞交互
  - 新增 `memory_extract.rs` 模块：提取 prompt、JSON 解析、事件通知
  - Per-Agent 配置：`autoExtract`（默认关闭）、`extractMinTurns`（最少轮数）
  - 复用当前 Provider 做 LLM 调用，结合去重系统避免重复提取
  - 前端：Agent Memory 设置区新增"自动提取记忆"开关

### Refactored

- **`tools/web.rs` 拆分为独立模块**：`web_search.rs`（搜索 Provider 配置 + 8 个搜索引擎实现 + 搜索缓存）和 `web_fetch.rs`（网页抓取配置 + SSRF 防护 + Readability 提取 + 抓取缓存），职责分离更清晰

### Changed

- **web_fetch 工具全面升级**：从简单正则 HTML 清理升级为生产级网页抓取工具
  - Mozilla Readability（`readability` crate）正文提取 + `htmd` crate HTML→Markdown 转换
  - 新增 `extract_mode` 参数：`markdown`（默认）保留格式结构，`text` 纯文本
  - 内存缓存：15 分钟 TTL，100 条上限，自动淘汰过期/最早条目
  - SSRF 防护：DNS 解析 + 私有/保留 IP 地址拦截（IPv4 + IPv6）
  - 流式字节限制读取：默认 2MB，防止大页面 OOM
  - 结构化 JSON 响应：url/finalUrl/status/title/extractor/tookMs/cached/truncated 等元数据
  - 外部内容标记：`<web_fetch_result>` 标签包装，标识不可信外部来源
  - 可视化配置面板 `WebFetchPanel`：8 项配置（字符限制/网络/缓存/安全）
  - 2 个 Tauri 命令：`get_web_fetch_config` / `save_web_fetch_config`
  - 配置持久化在 `config.json` 的 `webFetch` 字段
  - i18n：中英文翻译

### Added

- **记忆工具完善**：新增 `update_memory` 和 `delete_memory` AI 工具
  - `update_memory`：根据 ID 修改记忆内容和标签
  - `delete_memory`：根据 ID 删除记忆
  - `recall_memory` 输出中增加 ID 显示，便于修改和删除操作
- **Web 搜索多 Provider 支持**：web_search 工具支持 7 个搜索引擎，可拖拽排序 + 独立开关
  - 零成本 Provider：DuckDuckGo（默认开启）、SearXNG（自托管元搜索）
  - 付费 Provider：Brave Search、Perplexity、Google Custom Search、Grok (X.AI)、Kimi (Moonshot)
  - 有序优先级：按列表顺序使用第一个已开启的引擎，拖拽调整优先级
  - 智能约束：需要 API Key 的引擎必须填写密钥后才能开启，清空密钥自动关闭
  - 新增设置面板 `WebSearchPanel`：@dnd-kit 拖拽排序 + 展开编辑 + 开关切换
  - 数据模型：`WebSearchProviderEntry[]`（id/enabled/apiKey/apiKey2/baseUrl）
  - 2 个 Tauri 命令：`get_web_search_config` / `save_web_search_config`
  - 配置持久化在 `config.json` 的 `webSearch.providers` 有序数组
  - i18n：中英文翻译
- **SearXNG Docker 一键部署**：选择 SearXNG 时提供 Docker 一键部署功能
  - 新增 `docker.rs` 模块：Docker CLI 交互（检测/拉取镜像/启动/停止/删除容器）
  - 自动注入 `settings.yml`（禁用 limiter + 启用 JSON 格式）
  - 端口冲突检测（8080-8089 自动递增）+ 健康检查轮询
  - 前端状态指示灯（运行中/已停止）+ 启动/停止/删除按钮
  - 5 个 Tauri 命令：`searxng_docker_status/deploy/start/stop/remove`
- **开机自动启动**：设置面板「系统」分类，一键开启/关闭登录时自动启动
  - 集成 `tauri-plugin-autostart`，macOS 使用 LaunchAgent 方式注册
  - 2 个 Tauri 命令：`get_autostart_enabled` / `set_autostart_enabled`
  - 新增设置面板 `SystemPanel`（系统设置入口）
  - i18n：中英文翻译
- **单实例保护**：集成 `tauri-plugin-single-instance`，防止重复启动，第二次启动自动聚焦已有窗口
- **崩溃自动恢复**：`main.rs` 实现 panic 捕获 + 自动重启循环（最多 3 次），1 秒间隔防止频繁重启
  - 集成 `tauri-plugin-process` 支持应用内重启能力
- **定时任务系统 (cron)**：支持 AI Agent 按计划自动执行任务
  - 新增 `cron.rs` 模块：3 种调度类型（一次性 At / 固定间隔 Every / Cron 表达式）
  - `CronDB`：基于 `~/.opencomputer/cron.db`（SQLite + WAL），持久化任务和运行日志
  - 后台调度器：tokio 定时任务每 15 秒轮询，到期任务自动 spawn 执行
  - 任务执行：创建隔离 session，构建 AssistantAgent，支持模型链降级
  - 错误处理：指数退避重试（30s → 1h），连续失败 N 次自动禁用
  - 启动恢复：孤立运行标记为 error，过期一次性任务标记为 missed
  - 日历范围查询：展开 Cron/Every 表达式计算月度事件，关联运行日志
  - 9 个 Tauri 命令：`cron_list_jobs` / `cron_get_job` / `cron_create_job` / `cron_update_job` / `cron_delete_job` / `cron_toggle_job` / `cron_run_now` / `cron_get_run_logs` / `cron_get_calendar_events`
  - Agent 工具 `manage_cron`：AI 可直接创建/管理定时任务（7 个 action）
  - **日历视图页面**：侧边栏入口，月历网格显示任务圆点，点击日期展开任务列表
  - **设置面板 CronPanel**：列表管理视图，搜索/筛选/批量操作
  - 共享组件：`CronJobForm`（新建/编辑表单 + Cron 预设）、`CronJobDetail`（详情 + 运行历史）
  - 实时刷新：Tauri 事件 `cron:run_completed` 通知前端
  - 依赖：`cron` crate 0.13（Cron 表达式解析）
  - i18n：中英文翻译（70+ 翻译键）
- **浏览器控制工具 (browser)**：通过 Chrome DevTools Protocol 直接控制浏览器
  - 新增 `browser_state.rs`：全局浏览器连接管理（OnceLock 单例，支持连接已运行 Chrome 或启动托管实例）
  - 新增 `tools/browser.rs`：24 个 action 的 browser tool（connect/launch/disconnect/navigate/go_back/go_forward/take_snapshot/take_screenshot/click/fill/fill_form/hover/drag/press_key/upload_file/evaluate/wait_for/handle_dialog/resize/scroll/list_pages/new_page/select_page/close_page）
  - 页面可访问性快照：注入 JS 提取元素树，生成 LLM 友好文本格式，ref ID 用于后续交互
  - 截图返回 base64 image content block（Anthropic multimodal 格式）
  - 自动连接：tool 调用时自动尝试连接 localhost:9222
  - 依赖：`chromiumoxide` crate（tokio-runtime），纯 Rust 实现无 Node.js 依赖
- **记忆系统后端（Phase 2A）**：实现持久化、可搜索的 Agent 记忆系统
  - 新增 `memory.rs` 模块：`MemoryBackend` trait 可插拔架构（MVP 使用 SQLite + FTS5）
  - `SqliteMemoryBackend`：基于 `~/.opencomputer/memory.db`，WAL 模式，FTS5 全文搜索（BM25 排序）
  - 4 种记忆类型：`user`（用户信息）/ `feedback`（行为偏好）/ `project`（项目上下文）/ `reference`（外部资源）
  - 2 种作用域：`Global`（所有 Agent 共享）/ `Agent`（私有）
  - 记忆自动注入系统提示词 section ⑧（按类型分组格式化，可配置字符预算，默认 5000）
  - `MemoryConfig`：per-Agent 配置（enabled / shared / promptBudget），`serde(default)` 零破坏性
  - 12 个新 Tauri 命令：`memory_add` / `memory_update` / `memory_delete` / `memory_get` / `memory_list` / `memory_search` / `memory_count` / `memory_export` / `get_embedding_config` / `save_embedding_config` / `get_embedding_presets` / `list_local_embedding_models`
  - `AgentSummary` 新增 `memory_count` 字段
  - Embedding 配置系统：支持 API 模式（OpenAI / Google Gemini / Jina / Cohere / 硅基流动 / 自定义）和本地 ONNX 模型，类 Provider 设计
  - `EmbeddingConfig` 存储在 `config.json`（ProviderStore），内置 5 个 API 预设 + 4 个本地模型预设
  - SQLite FTS5 通过 build.rs 编译时启用
- **向量语义搜索（Phase 2B）**：在 FTS5 关键词搜索基础上增加向量相似度搜索
  - 集成 `fastembed`（本地 ONNX embedding）+ `sqlite-vec`（SQLite 向量扩展）
  - `EmbeddingProvider` trait + `ApiEmbeddingProvider`（OpenAI/Google/Jina/Cohere 兼容）+ `LocalEmbeddingProvider`（fastembed-rs）
  - RRF（Reciprocal Rank Fusion）混合检索：FTS5 BM25 + 向量余弦相似度融合排序
  - 记忆 `add()`/`update()` 自动生成向量，`delete()` 自动清理 vec0 表
  - memories 表新增 `embedding BLOB` 列 + `memories_vec` vec0 虚拟表
- **记忆管理前端 UI（Phase 2C）**：完整的 GUI 记忆管理界面
  - 新增 `MemoryPanel.tsx` 设置面板：记忆列表（按类型图标 + 搜索 + 过滤）、添加/编辑/删除、导出 Markdown
  - Embedding 配置子页面：API 模式（5 个预设一键切换）/ 本地模型选择、API Key + Model + Dimensions 配置
  - 设置侧边栏新增 "Memory"（Brain 图标）入口
  - i18n 支持（中文 + 英文，32 个翻译 key）
- **完整的日志记录系统**：记录应用执行全流程的详细日志，支持可视化查看和检索
  - 新增 `logging.rs` 模块：SQLite 持久化日志（`~/.opencomputer/logs.db`），WAL 模式
  - `LogDB`：支持分页查询、多条件过滤（级别/分类/关键词/时间/会话）、统计、导出（JSON/CSV）、自动清理过期日志
  - `AppLogger`：基于 `tokio::sync::mpsc` 异步写入通道，批量攒 buffer（100条/200ms），不阻塞主流程
  - 日志分类：agent（LLM 请求/token 用量）、tool（工具执行/耗时）、provider（降级/重试）、system（启动）、session（会话创建）
  - 敏感信息自动脱敏（API Key、Token 等替换为 `[REDACTED]`）
  - 可配置：启用/禁用、日志级别（error/warn/info/debug）、最大保留天数、最大存储大小
  - 配置持久化至 `~/.opencomputer/log_config.json`
  - 6 个新 Tauri 命令：`query_logs_cmd` / `get_log_stats_cmd` / `clear_logs_cmd` / `get_log_config_cmd` / `save_log_config_cmd` / `export_logs_cmd`
  - 新增 `LogPanel.tsx` 设置面板：日志浏览器（过滤栏 + 日志列表 + 分页 + 详情展开 + 导出）+ 可折叠配置区
  - 在 `tools/mod.rs`、`tools/exec.rs`、`tools/approval.rs`、`agent.rs`、`lib.rs` 中添加结构化日志埋点
  - **纯文本日志文件输出**：SQLite + 文件双写，日志同时输出到 `~/.opencomputer/logs/opencomputer-YYYY-MM-DD.log`
  - 日志文件按日期切分、按大小轮转（默认单文件 10MB），支持 `tail -f`、`grep` 等外部工具直接查看
  - Agent 可通过内置 `read`/`grep` 工具读取日志文件实现自我排查
  - 新增 3 个 Tauri 命令：`list_log_files_cmd` / `read_log_file_cmd` / `get_log_file_path_cmd`
  - `LogPanel` 新增双视图模式：结构化查询视图（SQLite）+ 文件浏览视图（左侧文件列表 + 右侧内容查看器）
  - 配置面板新增文件日志开关和单文件大小上限，SQLite 和文件日志可独立开关
- **Agent 执行全链路日志**：后端 `agent.rs` 和 `lib.rs` 新增 30+ 个结构化日志点，覆盖 chat 入口调度（provider/model/history）、API 请求详情（URL/消息数/body 大小/TTFB）、API 响应状态（HTTP status/request-id）、SSE 流解析结果（text 长度/tool_calls/usage）、Tool Loop 进度、chat 完成总结（rounds/tokens）、模型链解析、模型降级尝试、会话上下文恢复、系统提示词组装
- **前端统一日志**：新增 `src/lib/logger.ts` 前端日志工具，通过 `frontend_log` / `frontend_log_batch` Tauri 命令将前端日志写入后端统一日志系统，支持批量缓冲（500ms/20 条），error/warn 级别同时镜像到 console。替换全部 10 个组件中 ~45 处 `console.error` 为结构化 logger 调用

### Changed

- **`SettingsView.tsx` 拆分为独立面板组件**：原 2831 行单文件拆分为 `types.ts`（共享类型）+ 8 个独立面板组件（ChatSettingsPanel / AppearancePanel / LanguagePanel / GlobalModelPanel / SkillsPanel / AgentPanel / UserProfilePanel / AboutPanel）+ 瘦身后的 SettingsView 编排入口（~170 行）
- **`tools.rs` 拆分为子模块目录**：原 2927 行单文件拆分为 `src-tauri/src/tools/` 目录下 12 个模块（mod.rs / approval.rs / exec.rs / process.rs / read.rs / write.rs / edit.rs / ls.rs / grep.rs / find.rs / apply_patch.rs / web.rs），公共 API 保持不变
- **前端组件目录重构**：`src/components/` 按功能模块拆分为三个子目录
  - `chat/`：ChatScreen / ChatInput / ChatSidebar / ThinkingBlock / ToolCallBlock / FallbackDetailsPopover / ApprovalDialog
  - `settings/`：SettingsView / ProviderSettings / ProviderSetup / ProviderEditPage / TestResultDisplay / AvatarCropDialog
  - `common/`：MarkdownRenderer / ProviderIcon / IconSidebar
  - `ui/` 保持不变（shadcn/ui 基础组件）
  - 所有跨组件 import 路径同步更新

### Added

- **文件附件内容提取**：非图片文件（PDF/Word/Excel/PPT/文本代码）发送给 LLM 前自动提取内容
  - 新增 `file_extract.rs` 模块，统一文件内容提取逻辑
  - PDF：`pdf-extract` 提取文本 + `pdfium-render` 渲染页面为 PNG 图片
  - Word (.docx)：zip + quick-xml 解析提取段落文本
  - Excel (.xlsx/.xls)：`calamine` 读取所有 sheet 转 TSV 文本
  - PPT (.pptx)：提取幻灯片文本 + 嵌入图片（ppt/media/）
  - 文本/代码文件：直接 UTF-8 读取，20 万字符截断
  - 所有文件类型始终透传磁盘路径（`<file name="x" path="/path">`），模型可通过 tools 自行决策进一步处理
  - 未知二进制文件仅透传路径，不做"不支持"提示
  - 新增依赖：pdf-extract、pdfium-render、calamine、zip、quick-xml

- **Thinking/Reasoning 推理过程展示**：流式显示模型推理内容，支持三种 Provider
  - 后端 `agent.rs` 新增 `emit_thinking_delta` 事件，Anthropic（thinking_delta content block）/ OpenAI Chat（delta.reasoning_content，适配 DeepSeek/o-series）/ OpenAI Responses（reasoning_summary_text.delta）均支持
  - 前端新增 `ThinkingBlock.tsx` 折叠展示组件：流式生成中紫色脉冲自动展开，完成后自动折叠；左侧紫色竖线 + MarkdownRenderer 渲染
  - `Message` 类型新增 `thinking` 字段，`ChatScreen` 处理 `thinking_delta` 事件
- **头像裁剪功能**：用户头像和 Agent 头像均支持选图后裁剪
  - 新增 `AvatarCropDialog` 组件（基于 `react-easy-crop`，圆形裁剪、缩放滑条）
  - 后端新增 `save_avatar` Tauri 命令，裁剪后图片保存至 `~/.opencomputer/avatars/`
  - `paths.rs` 新增 `avatars_dir()`，`ensure_dirs` 自动创建
  - `tauri.conf.json` 扩展 asset protocol scope 支持 `$HOME` 路径
- **会话管理增强**：新增 `get_session_cmd` / `rename_session_cmd` Tauri 命令
  - 会话列表在新消息发送后立即自动刷新（按更新时间重排序）
  - 右键菜单或双击支持会话重命名
- **Agent 列表交互升级**：
  - 点击 Agent 项切换选中态，选中后过滤下方会话列表（支持多选）
  - Agents 标题栏显示清除过滤按钮（X + 选中数量）
  - 双击 Agent 项直接快速新建会话（跳过选择菜单）
  - 仅一个 Agent 时点击新建按钮直接创建会话（跳过选择菜单）
  - Agent 项悬浮时显示 MessageSquarePlus 新建对话图标
- **侧边栏用户头像**：`IconSidebar` 顶部展示当前用户头像（无头像时显示 User 图标）
- **暗黑模式配色优化**：从纯黑背景调整为柔和深蓝灰色调，提升长时间使用舒适度；修复拖拽窗口时出现黑色闪烁背景的问题
- **对话气泡 UI 优化**：消息气泡宽度调整至 95%，用户/助手消息颜色对比度增强

- **会话上下文持久化与恢复**：完整的 conversation_history 序列化/反序列化机制
  - `session.rs`：sessions 表新增 `context_json` 列，新增 `save_context()` / `load_context()` 方法
  - `agent.rs`：新增 `set_conversation_history()` / `get_conversation_history()` 方法
  - `lib.rs`：`chat` 命令中恢复历史上下文（`restore_agent_context`）+ 成功后保存（`save_agent_context`）
  - 数据库文件重命名为 `sessions.db`
- **Event 消息角色**：新增 `MessageRole::Event`（替代 `System`，避免与 LLM API 的 system role 冲突）
  - 错误消息、降级通知等系统事件统一使用 `event` 角色落库和渲染
  - 前端 event 消息居中显示，柔和样式，与用户/助手消息区分
- **消息排队与自动发送**：loading 中可继续输入并发送，消息进入 pending 队列
  - 回复结束后自动发送（可在「设置 → 对话」中关闭，改为回填输入框）
  - pending 消息指示器：琥珀色脉冲圆点 + 消息预览 + 取消按钮
- **打断回复（Stop Chat）**：loading 中显示红色停止按钮，可随时中断 LLM 回复
  - `AppState` 新增 `chat_cancel: Arc<AtomicBool>` + `stop_chat` Tauri 命令
  - 3 个 SSE 解析器 + 4 个工具循环中检查取消标志，取消后保存部分回复
- **连续 user 消息兼容**：`agent.rs` 新增 `push_user_message()` 方法，合并连续 user 消息
  - 避免 Anthropic API 的 role 交替校验错误（打断发送、异常等场景）
- **多会话独立支持**：切换会话时各会话的消息状态独立保存和恢复
  - `sessionCacheRef`（Map）缓存每个会话的消息，`loadingSessionsRef`（Set）跟踪加载中的会话
  - 流式回调通过 `updateSessionMessages` 按 session ID 更新，支持后台会话继续接收数据
- **对话设置面板**：设置页新增「对话」分区（MessageSquare 图标）
  - 自动发送排队消息开关，存储到 `~/.opencomputer/user.json`
  - `UserConfig` 新增 `auto_send_pending: bool` 字段（默认 true）
- **全局默认模型 + 降级模型系统**：支持设置有序降级链，每个 Agent 可继承全局设置或自定义覆盖
  - `provider.rs`：`ProviderStore` 新增 `fallback_models` 字段 + `resolve_model_chain()` / `parse_model_ref()` / `find_provider()` 辅助函数
  - 新增 Tauri 命令：`get_fallback_models` / `set_fallback_models`
  - `chat` 命令重构为支持 primary + fallback 模型链按序尝试
- **智能降级错误分类**（参考 OpenClaw）：新增 `failover.rs` 模块
  - `FailoverReason` 枚举：RateLimit / Overloaded / Timeout / Auth / Billing / ModelNotFound / ContextOverflow / Unknown
  - `classify_error()`：基于 HTTP 状态码 + 错误消息模式匹配，自动分类 API 错误
  - `ContextOverflow` 错误终止返回，不降级（小窗口模型会更差）
  - `RateLimit` / `Overloaded` / `Timeout` 先重试 2 次（指数退避 1s→2s + jitter），再降级
  - `Auth` / `Billing` / `ModelNotFound` 直接跳到下一模型
  - 11 个单元测试覆盖所有错误分类场景
- **降级通知增强**：`model_fallback` 事件新增 `reason` / `from_model` / `attempt` / `total` / `error` 字段
  - 前端显示富通知：`⚠️ Fallback → Model ← From (reason) [2/3]`
- **全局模型设置 UI**：`SettingsView.tsx` 新增 `GlobalModelPanel` 组件
  - 默认模型下拉选择（按 Provider 分组）
  - 降级模型有序列表（优先级标签、上移/下移/删除/添加）
  - 导航新增 "模型" 分区（Layers 图标）
- **会话持久化**：新增 `session.rs` 模块，基于 SQLite（WAL 模式）存储会话历史
  - `SessionDB`：管理 sessions / messages 两张表，支持 user / assistant / system / tool 四种消息角色
  - `chat` 命令自动创建/关联会话，保存用户消息、助手回复、工具调用结果
  - 降级事件（`model_fallback`）以 `role=system` JSON 消息落库，恢复会话时可回显
  - 首条消息自动生成会话标题（`auto_title`）
  - `paths.rs` 新增 `attachments_dir()` 管理附件存储
  - 新增 Tauri 命令：`create_session_cmd` / `list_sessions_cmd` / `load_session_messages_cmd` / `delete_session_cmd`
  - 新增依赖：`rusqlite`（bundled）、`chrono`、`uuid`
- **会话侧边栏 UI**：App.tsx 侧边栏重构为 Agents + Sessions 双区域
  - 可折叠 Agents 网格：按钮点击创建新会话
  - 会话列表：按更新时间倒序，显示标题、Agent 头像、相对时间
  - 支持会话切换和删除
  - 新建聊天弹出菜单：选择 Agent 后创建新会话
- **Agent 定义系统**：支持创建和管理多个 AI Agent，每个 Agent 可独立配置身份、性格和行为
  - 设置页新增 Agent section，支持列表/新建/编辑/删除
  - Agent 编辑 4 个 Tab：身份（名称/描述/Emoji/头像/角色定位）、性格（气质/语气/特质/准则/边界/个性/沟通方式）、行为（工具轮数/审批工具/沙箱/工具指导）、自定义提示词
  - 结构化配置模式：GUI 表单填写，自动组装系统提示词（PersonalityConfig 8 个字段）
  - 自定义提示词模式：开启后忽略结构化设置，直接编辑 Markdown（agent.md / persona.md）
  - 身份和性格页底部均支持「补充说明」自由文本
  - 首次开启自定义模式自动从模板文件预填内容
  - 新增 `agent_config.rs`：AgentConfig / PersonalityConfig / AgentDefinition / AgentSummary 数据结构
  - 新增 `agent_loader.rs`：Agent 文件 CRUD + 多语言模板（`include_str!` 嵌入 12 种语言）
  - 新增 `system_prompt.rs`：模块化提示词组装，支持结构化/自定义双模式
  - 新增 `user_config.rs`：用户个人配置（昵称/性别/年龄/角色/时区/语言/AI 经验/回复风格）
  - 新增 Tauri 命令：`list_agents` / `get_agent_config` / `get_agent_markdown` / `save_agent_config_cmd` / `save_agent_markdown` / `delete_agent` / `get_agent_template` / `get_user_config` / `save_user_config` / `get_system_timezone`
- **多语言 Agent 模板**：12 种语言的 `agent.*.md`（身份说明）和 `persona.*.md`（人设骨架），编译时嵌入二进制
  - 默认 Agent 按系统语言创建（名称/描述/agent.md 本地化）
  - 空字段加载时自动按当前 UI 语言填充模板
- **Agent 头像支持**：通过 `tauri-plugin-dialog` 文件选择器选择本地图片，使用 `convertFileSrc` 展示
  - `tauri.conf.json` 开启 `assetProtocol`
- **聊天界面 Agent 集成**：
  - 对话列表显示当前 Agent 头像 + 名称 + Emoji
  - 聊天页头部显示 Agent 名称
  - 右上角 Settings 图标可跳转 Agent 设置页
- **Agent 行为设置增强**：
  - Markdown 输入框（agent.md / persona.md / tools.md）显示字符计数（20,000 上限），超 80% 黄色警告，超限红色提示
  - 最大工具调用轮数支持「不限制」选项（0 = 无上限）
  - 新增 per-Agent 技能配置：全局开启的技能默认可用，可单独禁用不需要的
  - 工具审批改为三种模式：全部审批 / 无需审批 / 自定义（可逐个选择工具）
  - 内置工具名称和描述支持 12 种语言本地化显示
  - 新增 `list_builtin_tools` Tauri 命令，前端动态加载工具列表
- **用户个人配置 UI**：设置页「个人信息」面板，支持头像/昵称/性别/年龄/角色/AI 经验/时区/语言/回复风格/补充说明
- **Markdown 消息渲染**：用户和 AI 消息均支持完整 Markdown 渲染（基于 Streamdown）
  - 流式场景优化：正确处理未闭合语法（加粗、代码块等），渐进式渲染无闪烁
  - 代码块语法高亮（Shiki）、CJK 中文标点优化
  - KaTeX 数学公式渲染（LaTeX 语法）
  - Mermaid 图表渲染（流程图、时序图等）
  - 新增 `MarkdownRenderer` 组件（`src/components/MarkdownRenderer.tsx`）
  - 新增依赖：`streamdown`、`@streamdown/code`、`@streamdown/cjk`、`@streamdown/math`、`@streamdown/mermaid`、`katex`
- **统一数据存储架构**：所有数据落盘集中到 `~/.opencomputer/` 目录
  - 新增 `paths.rs` 模块：集中管理 root、config、credentials、home、share 等路径
  - 目录结构：`config.json`（通用配置）、`credentials/auth.json`（OAuth 凭证）、`home/`（主 Agent Home）、`share/`（共享目录）
  - 启动时自动创建所有必要目录
  - 启动时自动从旧路径迁移数据（`providers.json` 和 `auth.json`）
- **Provider 品牌 Logo**：所有 24 个内置 Provider 模板和 Provider 管理面板使用官方品牌 SVG 图标（基于 `@lobehub/icons`），替换原来的 emoji 字符
  - 新增 `ProviderIcon` 组件（`src/components/ProviderIcon.tsx`），支持 provider key 直接映射和 provider name 模糊匹配
- **多语言支持 (i18n)**：使用 `i18next` + `react-i18next` 实现完整的国际化支持
  - 支持 12 种语言：简体中文、繁體中文、English、日本語、Türkçe、Tiếng Việt、Português、한국어、Русский、العربية、Español、Bahasa Melayu
  - 自动检测系统语言，无法识别时回退到英文
  - 侧边栏语言切换菜单，切换后立即生效
  - 语言偏好持久化到 localStorage
  - 新增 `src/i18n/` 模块：12 个翻译文件 + i18n 初始化配置
- **Think 等级按 Provider 差异化映射**：不同 API 类型使用各自原生的 thinking 参数格式
  - Anthropic：`thinking: { type: "enabled", budget_tokens: N }`（low→1024 / medium→4096 / high→8192 / xhigh→16384）
  - OpenAI Chat Completions：`reasoning_effort` 字段（low/medium/high，xhigh 自动降级为 high）
  - OpenAI Responses / Codex：保持现有 `reasoning.effort` 格式（支持 xhigh）
- **思考类型（Thinking Style）配置**：Provider 级别的 `thinking_style` 字段，控制向不同 API 发送思考参数的格式
  - 支持 5 种风格：`openai`（reasoning_effort）、`anthropic`（thinking budget）、`zai`（thinking budget）、`qwen`（enable_thinking）、`none`（不发送）
  - 各内置模板自动设置默认值：千问/DashScope → `qwen`，智谱 → `zai`，Anthropic → `anthropic`
  - 新增/编辑 Provider 时可通过下拉菜单选择
- **动态 Think 选项**：前端根据当前模型的 API 类型显示不同的 effort 选项列表
- **切换模型自动修正**：当切换到不支持当前 effort 等级的 Provider 时，自动回退到有效值
- **模型 Provider 管理系统**：支持多个自定义模型服务商，GUI 傻瓜式配置
- **24 个内置 Provider 模板**：选择模板后只需填 API Key，Base URL 和模型列表自动预填
  - 国际：Anthropic、OpenAI (Responses)、OpenAI (Chat)、DeepSeek、Google Gemini、xAI、Mistral、OpenRouter、Groq、NVIDIA、Together AI
  - 国内：Moonshot (Kimi)、Kimi Coding、通义千问、ModelStudio (DashScope)、火山引擎、智谱 AI、MiniMax、小米 MiMo、百度千帆
  - 本地：Ollama、vLLM、LM Studio
- **Provider 三步引导向导** (`ProviderSetup.tsx`)：模板网格 + 自定义入口（API 类型选择 → 连接配置 → 模型配置）
- **Provider 管理面板** (`ProviderSettings.tsx`)：查看/编辑/删除/启用禁用，从侧边栏设置按钮进入
- **自定义 User-Agent**：支持在配置 Provider 时指定 `User-Agent` HTTP 头部（默认 `claude-code/0.1.0`），以兼容特定 WAF（如 DashScope CodingPlan）
- **三种 API 类型支持**：Anthropic Messages API、OpenAI Chat Completions、OpenAI Responses API
- **API Key 可选**：本地服务（Ollama/vLLM/LM Studio）和自定义 Provider 的 API Key 为可选项
- **OpenAI Chat Completions 流式调用**：完整的 SSE 解析和 tool calling 支持
- **OpenAI Responses API 自定义 Base URL**：可用于兼容 OpenAI API 的第三方服务
- **Provider 持久化**：配置保存至 `providers.json`，重启自动恢复
- **模型属性配置**：支持名称、输入类型(文本/图片/视频)、Context Window、Max Tokens、推理支持、成本
- **连通性测试**：添加 Provider 时可验证 API Key 和 Base URL 是否有效
- 新增 `provider.rs` 模块：`ApiType`、`ModelConfig`、`ProviderConfig` 数据结构 + JSON 持久化
- 新增 Tauri 命令：`get_providers`、`add_provider`、`update_provider`、`delete_provider`、`test_provider`、`get_available_models`、`get_active_model`、`set_active_model`、`has_providers`
- **统一 Tool Calling 支持**：Anthropic 和 OpenAI 双 Provider 均支持 tool 调用（exec、read_file、write_file、patch_file、list_dir、web_search、web_fetch）
- **新增 `web_search` 工具**：AI 可搜索网页获取最新信息（基于 DuckDuckGo，无需 API Key）
- **新增 `web_fetch` 工具**：AI 可抓取网页内容，自动提取正文并清理 HTML 标签
- **新增 `patch_file` 工具**：基于搜索替换的精确文件编辑，比 write_file 覆写更安全
- **`exec` 工具全面升级**（对齐 OpenClaw）：
  - 默认超时从 120s 调整为 1800s（30 分钟），最大支持 7200s（2 小时）
  - 新增 `env` 参数支持自定义环境变量
  - 新增 `background` 参数支持后台执行，立即返回 session ID
  - 新增 `yield_ms` 参数支持自动后台化（等待指定毫秒后若未完成则后台）
  - 启动时自动解析 login shell PATH，确保 npm/python 等工具可用
  - 输出截断动态调整：根据模型上下文窗口自动计算（默认 200K chars，最小 8K）
- **新增 `process` 工具**：管理后台执行的 exec 会话
  - `list`：列出所有运行/已结束的会话
  - `poll`：获取会话新输出，支持 timeout 等待
  - `log`：查看完整输出日志，支持 offset/limit 分页
  - `write`：向后台进程 stdin 写入（Phase 3 完善）
  - `kill`：终止后台进程
  - `clear`/`remove`：清理已结束会话
- **新增 `process_registry.rs` 模块**：进程会话注册表，全局单例管理所有 exec 产生的后台进程
- **PTY 支持**：exec 新增 `pty` 参数，基于 `portable-pty` crate 实现伪终端执行
  - 适用于需要 TTY 的交互式命令（REPL、编辑器等）
  - PTY 不可用时自动回退到普通模式
  - 输出自动清理 ANSI 转义序列
- **命令审批系统**：exec 执行前检查命令是否在 allowlist 中
  - 不在 allowlist 中的命令触发审批流程（Tauri `approval_required` 事件）
  - 支持 AllowOnce / AllowAlways / Deny 三种响应
  - AllowAlways 自动将命令前缀加入 allowlist（持久化至 `~/.opencomputer/exec-approvals.json`）
  - 新增 `respond_to_approval` Tauri 命令
  - 全局 `APP_HANDLE` 存储用于事件发射
- **`read_file` 工具增强**（对齐 OpenClaw）：
  - 自适应分页：根据模型 context window 自动计算单页大小（20% 上下文），循环拼接最多 8 页
  - 新增 `offset`/`limit` 参数支持行级分页读取（1-based 行号），大文件可分段读取
  - 自动检测图片文件（PNG/JPEG/GIF/WebP/BMP/TIFF/ICO）并返回 base64 编码数据
  - 图片 MIME 二次校验：base64 编码后解码头部 re-sniff 验证实际类型
  - 超大图片自动缩放（最大 1200px、5MB 限制），渐进 JPEG 质量降级
  - 结构化参数解析：支持 `{type:"text", text:"..."}` 嵌套格式
  - 兼容 `file_path` 参数别名
  - 文本输出带行号格式，截断时提示行范围/字节数/续读偏移量
  - 新增 `image` crate 依赖（v0.25）用于图片解码和缩放
  - 工具名从 `read_file` 改为 `read`（保留 `read_file` 别名兼容）
- **`write` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `write_file` 改为 `write`（保留 `write_file` 别名兼容）
  - 兼容 `file_path` 参数别名
  - 结构化参数解析：`path` 和 `content` 均支持 `{type:"text", text:"..."}` 嵌套格式
- **`edit` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `patch_file` 改为 `edit`（保留 `patch_file` 别名兼容）
  - 兼容 `oldText`/`old_string`/`newText`/`new_string`/`file_path` 参数别名
  - 结构化参数解析：所有参数均支持 `{type:"text", text:"..."}` 嵌套格式
  - `new_text` 参数未提供时默认为空字符串（删除模式）
  - 写后恢复（Post-write Recovery）：两层防护
    - 写入错误恢复：写操作报错后检查文件是否已正确更新，避免假失败
    - 重复编辑恢复：old_text 不存在但 new_text 已存在时视为已应用，避免重试报错
- **`ls` 工具增强**（对齐 OpenClaw）：
  - 工具名从 `list_dir` 改为 `ls`（保留 `list_dir` 别名兼容）
  - 新增 `limit` 参数（默认 500 条）
  - 新增 50KB 输出字节上限，防止超大目录撑爆上下文
  - 支持 `~` 和 `~/` 路径展开
  - 大小写不敏感排序
  - 路径验证：检查路径存在性和是否为目录
  - 跳过无法 stat 的条目（不报错）
  - 空目录返回 "(empty directory)"
  - 兼容 `file_path` 参数别名 + 结构化参数解析
- **新增 `grep` 工具**（对齐 OpenClaw）：搜索文件内容
  - 原生 Rust 实现（`ignore` + `regex` crate），无需系统安装 ripgrep
  - 支持正则和字面量搜索（`literal` 参数）
  - 支持 `glob` 文件过滤、`ignore_case` 大小写、`context` 上下文行
  - 默认 100 条匹配限制，每行最长 500 字符，50KB 输出上限
  - 自动尊重 `.gitignore`，跳过二进制文件
- **新增 `find` 工具**（对齐 OpenClaw）：按 glob 模式查找文件
  - 原生 Rust 实现（`ignore` + `glob` crate），无需系统安装 fd
  - 默认 1000 条结果限制，50KB 输出上限
  - 自动尊重 `.gitignore`，支持 `~` 路径展开
  - 输出相对路径，匹配文件名和完整路径
- **新增 `apply_patch` 工具**（对齐 OpenClaw）：多文件补丁操作
  - 支持 `*** Begin Patch` / `*** End Patch` 格式
  - `*** Add File: <path>` — 创建新文件
  - `*** Update File: <path>` — 修改文件（`@@` 上下文 + `-`/`+` 行）
  - `*** Delete File: <path>` — 删除文件
  - `*** Move to: <path>` — 在 Update 中移动文件
  - 3-pass fuzzy matching（精确 → 去尾空白 → 全 trim），容忍空白差异
  - 不限 Provider（OpenClaw 限 OpenAI only，我们全 Provider 可用）
- **新增依赖**：`regex`、`ignore`、`glob` crate
- **命令审批对话框 UI**：前端 `ApprovalDialog` 组件
  - 监听 Tauri `approval_required` 事件，弹出全屏遮罩审批对话框
  - 显示待执行命令内容和工作目录
  - 三按钮：拒绝（红色）/ 允许一次 / 始终允许
  - 支持多请求队列（FIFO），显示队列指示器
  - 全 12 语言 i18n 支持
- **Docker 沙箱模式**：exec 新增 `sandbox` 参数，支持在 Docker 容器内隔离执行命令
  - 基于 `bollard` crate 异步 Docker API 客户端
  - 新增 `sandbox.rs` 模块：容器生命周期管理（创建 → 启动 → 等待 → 收集日志 → 清理）
  - 自动挂载工作目录到容器 `/workspace`
  - 可配置镜像（默认 `ubuntu:22.04`）、内存限制（默认 512MB）、CPU 限制（默认 1 核）
  - 配置持久化至 `~/.opencomputer/sandbox.json`
  - 支持 `background=true` + `sandbox=true` 组合
  - Docker 不可用时返回清晰错误提示，不崩溃
- **Anthropic Messages API 直接调用**：支持 Claude tool_use 流式响应与多轮 tool 循环
- **OpenAI Tool Loop**：完整的 function_call SSE 事件解析与 agent loop 实现
- **Provider Schema 适配层**：`tools.rs` 引入 `ToolProvider` 枚举，同一套 tool 定义自动转换为 Anthropic / OpenAI 格式
- **微信风格三栏布局**：图标侧边栏 + 可拖拽会话/Agent 列表 + 对话区
- **可拖拽会话面板**：会话列表面板宽度可在 180px ~ 400px 范围内拖拽调整
- **模型选择器重构**：从原生 select 改为定制的**级联菜单**（Cascading Submenu）
  - Provider 列表向上弹出可见，鼠标悬停时从右侧展开该 Provider 下的模型列表
  - 支持单模型 Provider 直接点击选中
  - 增加半透明毛玻璃背景、精致阴影、圆角列表项等对齐参考图的质感设计
- **Think 思考模式选择器优化**：同步升级为向上弹出的自定义弹层，样式与模型选择器保持一致
- **可拖拽多行输入框**：类似微信的 Textarea 输入区域，支持拖拽调整高度（80~400px）
- **图片和文件附件**：输入工具栏新增图片（📷）和文件（📎）选择按钮，支持多选
- **粘贴图片/文件**：输入框支持直接从剪贴板粘贴图片和文件
- **附件预览与删除**：已添加的附件显示在输入框上方，支持图片缩略图预览和单独删除
- **后端多模态支持**：`agent.rs` 新增 `Attachment` 结构体和三种 API 格式的图片内容构建函数（Anthropic base64 source / OpenAI Chat image_url / OpenAI Responses input_image）
- **图片消息发送**：前端读取图片为 base64 传递给 Rust 后端，后端按各 Provider API 格式构建多模态请求

### Changed

- **App.tsx 组件化重构**：将 1583 行的 `App.tsx` 拆分为 6 个独立模块，主文件精简至约 110 行
  - `types/chat.ts`：共享类型定义（Message / Attachment / LlmApiType）+ `getEffortOptionsForType`
  - `ChatInput.tsx`：底部输入区（附件 / 模型选择器 / 思考模式 / 发送按钮）
  - `ChatScreen.tsx`：聊天主屏幕（消息列表 + ThinkingBlock + ToolCallBlock + 流式渲染）
  - `ChatSidebar.tsx`：左侧 Agent 网格 + 会话列表面板
  - `IconSidebar.tsx`：左侧图标导航栏
  - `ToolCallBlock.tsx`：工具调用折叠块
- **默认工具审批模式**：新建 Agent 默认改为所有工具均需审批（`requireApproval: ["*"]`），原为仅 `exec` 需审批
- **全面替换原生 HTML 表单组件**：`SettingsView`、各对话框中所有原生 `<select>` / `<input>` / `<textarea>` 统一替换为 shadcn/ui 封装组件（Select / Input / Textarea），保证 UI 和交互一致性
- **i18n 翻译补全**：所有 12 种语言补齐缺失的翻译键，Provider 模板名称和描述完整国际化
- **内置 Provider 模板升级**（同步 OpenClaw 最新变更）：
  - xAI：Grok 3 → Grok 4，base URL 加 `/v1`
  - 智谱 AI：base URL 升级到 `/v4`，模型扩展为 5 个（GLM-5 / GLM-5 Turbo / GLM-4.7 / GLM-4.7 Flash / GLM-4.7 FlashX），全部支持 reasoning
  - Kimi Coding：新增推荐模型 `kimi-code`，保留 `k2p5` 兼容
  - Mistral：base URL 加 `/v1`，移除 Codestral，Mistral Large 支持 image 输入，contextWindow/maxTokens 提升至 262144
  - Moonshot：精简为 `kimi-k2.5` 单模型
  - OpenRouter：新增 `auto` 自动模型选择
  - Together AI：新增 Llama 4 Maverick 17B
  - Ollama：默认模型从 `llama3.3` 改为 `glm-4.7-flash`
- `agent.rs` `LlmProvider` 从 2 种（Anthropic/OpenAI）扩展到 4 种（Anthropic/OpenAIChat/OpenAIResponses/Codex），全部支持自定义 base_url
- `lib.rs` `AppState` 使用 `ProviderStore` 替代独立的 codex_model 字段
- `lib.rs` `initialize_agent` 命令改为自动创建 Anthropic Provider
- `lib.rs` `finalize_codex_auth` 改为自动创建/更新内置 Codex Provider
- `App.tsx` 模型选择器改为显示 `Provider / Model` 组合格式
- `App.tsx` 侧边栏底部新增「设置」按钮，可进入 Provider 管理面板
- `App.tsx` 启动流程改为检查 Provider 列表决定显示引导页或聊天界面
- `App.tsx` 底部输入框从单行 `<Input>` 改为多行 `<textarea>`，默认 Enter 发送，Shift+Enter 换行
- `App.tsx` 顶部 Header 简化为仅显示 Agent 名称
- `agent.rs` Anthropic 调用从 `rig-core` Prompt trait 改为直接 HTTP 调用 Messages API
- `tools.rs` `ToolDefinition` 重构为 provider-agnostic 格式，新增 `to_anthropic_schema()` / `to_openai_schema()` 方法
- `LlmProvider::Anthropic` 从包装 `rig-core::Client` 改为存储 API key 字符串
- 对话界面从单栏改为三栏布局（图标侧边栏 / Agent 列表 / 对话区）

### Fixed

- 修复对话上下文丢失问题：`AssistantAgent` 新增 `conversation_history` 字段保存多轮对话历史
- 修复发送消息时出现两个气泡的问题：将独立 loading 指示器合并到 assistant 气泡中
- 修复三栏顶部分割线高度不对齐问题

## [0.2.0] - 2026-03-14

### Added

- **Codex OAuth 登录**：支持通过 ChatGPT 账号 OAuth 2.0（PKCE）登录，使用 OpenAI Codex 模型
- **多模型选择**：顶栏模型下拉菜单，支持 GPT-5.4 / GPT-5.3 Codex / GPT-5.2 / GPT-5.1 等系列模型
- **流式输出**：基于 Tauri Channel + SSE 的流式响应，实时显示 AI 回复
- **思考力度控制**：支持 None / Low / Medium / High / XHigh 五档 reasoning effort 调节
- **会话持久化与自动恢复**：OAuth token 持久化存储，启动时自动恢复上次登录状态
- **Token 自动刷新**：过期 token 自动使用 refresh_token 刷新
- **登出功能**：支持退出登录并清除本地 token
- 新增 `oauth.rs` 模块：完整的 OAuth 2.0 PKCE 流程、本地回调服务器、token 管理
- 新增 Tauri 命令：`start_codex_auth`、`check_auth_status`、`finalize_codex_auth`、`try_restore_session`、`logout_codex`、`get_codex_models`、`set_codex_model`、`set_reasoning_effort`、`get_current_settings`

### Changed

- `rig-core` 从 0.9 升级至 0.32
- `AssistantAgent` 重构为多 Provider 架构（`LlmProvider::Anthropic` / `LlmProvider::OpenAI`）
- `chat` 命令新增 `on_event: Channel<String>` 参数以支持流式输出
- `AppState` 从 `std::sync::Mutex` 迁移到 `tokio::sync::Mutex`
- `SetupScreen` 新增 Codex OAuth 登录按钮（"使用 ChatGPT 登录"）
- `index.css` 主题变量从 `@layer base :root` 迁移到 Tailwind CSS v4 的 `@theme` 语法
- `vite.config.ts` 固定开发端口为 1420

### Added (Dependencies)

- `sha2`、`base64`、`uuid`、`dirs`、`open`、`rand`、`tiny_http`、`reqwest`、`futures-util`

## [0.1.0] - 2026-03-14

### Added

- Initial scaffold: Tauri 2 + React 19 + TypeScript + Vite
- Setup screen with Anthropic API key input
- Chat screen with message history and streaming-style UX
- Rust backend with `rig-core` for Claude claude-sonnet-4-6 integration
- Tailwind CSS v4 + shadcn/ui component library
- `AssistantAgent` abstraction over Anthropic client
- Tauri commands: `initialize_agent`, `chat`
