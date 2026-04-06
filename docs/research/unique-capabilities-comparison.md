# 其他独有能力对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.14

## 一、架构总览

三个项目各有独特能力，反映了不同的产品定位：

| 项目 | 定位 | 独有能力侧重 |
|------|------|-------------|
| OpenComputer | 本地桌面 AI 助手 | 数据可视化、环境感知（天气）、多语言 GUI、定时任务 |
| Claude Code | 开发者 CLI 工具 | Proactive 记忆整理、结构化输出、Advisor 分级模型、上下文裁剪 |
| OpenClaw | 多渠道 AI 守护进程 | Canvas 工作台、原生移动/桌面 App、渠道级会话策略 |

## 二、OpenComputer 独有能力

### 2.1 数据大盘（Dashboard）

`crates/oc-core/src/dashboard/` 实现完整的数据分析大盘：

**模块组成**：
- `types.rs`：数据类型定义
- `queries.rs`：聚合查询
- `detail_queries.rs`：详情查询
- `cost.rs`：成本计算
- `filters.rs`：过滤器

**OverviewStats 聚合指标**：
- `total_sessions`、`total_messages`：会话和消息总量
- `total_input_tokens`、`total_output_tokens`：Token 消耗
- `total_tool_calls`、`total_errors`：工具调用统计
- `active_agents`、`active_cron_jobs`：活跃资源
- `estimated_cost_usd`：估算成本
- `avg_ttft_ms`：平均首 Token 延迟

**多维分析**：
- `TokenUsageTrend`：按日期的 Token 消耗趋势（含日均 TTFT）
- `TokenByModel`：按模型的 Token 分布和成本
- `ToolUsageStats`：工具调用频次、错误率、平均/总耗时
- `SessionTrend`：会话数和消息数趋势
- `SessionByAgent`：按 Agent 的会话分布

**DashboardFilter 多维过滤**：
- 时间范围（`start_date` / `end_date`）
- Agent / Provider / Model 维度

**跨数据源聚合**：
- SessionDB（sessions + messages + subagent_runs）
- LogDB（日志数据库）
- CronDB（cron_jobs + cron_run_logs）

前端使用 recharts 图表库渲染，`src/components/dashboard/` 提供完整 GUI。

### 2.2 天气系统

`crates/oc-core/src/weather.rs` + `weather_location_macos.rs` 实现环境感知：

**天气数据**：
- Open-Meteo API 获取实时天气和预报
- `WeatherData`：温度、体感温度、湿度、WMO 天气码、风速、坐标
- `DailyForecast`：每日预报（温度范围、降水量、风速）
- 天气码到描述的映射

**地理定位**：
- macOS CoreLocation 原生定位（`weather_location_macos.rs`，使用 objc2）
- 支持手动设置位置（城市名搜索 → 经纬度）
- `GeoResult` 地理编码搜索

**缓存策略**：
- 全局缓存（`OnceLock<Mutex<...>>`）
- 避免频繁 API 调用

**用途**：
- 作为 Agent 工具注入系统提示，让 AI 感知用户环境
- 天气问候集成到 Dashboard

### 2.3 i18n 多语言（12 种）

`src/i18n/locales/` 支持 12 种语言：

| 语言 | 文件 |
|------|------|
| 中文（简体） | `zh.json` |
| 中文（繁体） | `zh-TW.json` |
| 英文 | `en.json` |
| 日文 | `ja.json` |
| 韩文 | `ko.json` |
| 西班牙文 | `es.json` |
| 葡萄牙文 | `pt.json` |
| 俄文 | `ru.json` |
| 阿拉伯文 | `ar.json` |
| 越南文 | `vi.json` |
| 马来文 | `ms.json` |
| 土耳其文 | `tr.json` |

**工具链**：
- i18next 运行时
- `scripts/sync-i18n.mjs --check`：检查翻译缺失
- `scripts/sync-i18n.mjs --apply`：自动补齐缺失翻译
- 开发约定：新功能只实现 zh + en，其余通过脚本同步

Claude Code 和 OpenClaw 均为英文单语言，无 i18n 系统。

### 2.4 Canvas 画布

OpenComputer 暂无独立 Canvas 画布功能（与 OpenClaw 的 A2UI 不同）。但 GUI 天然支持富交互：
- Markdown 渲染（Streamdown + Shiki + KaTeX + Mermaid）
- 附件预览（图片 base64 展示）
- 工具结果结构化展示

### 2.5 Cron 调度

`crates/oc-core/src/cron/` 实现完整的定时任务系统：

**三种调度类型**（`CronSchedule`）：
- `At`：单次定时触发（ISO 8601 时间戳）
- `Every`：固定间隔（毫秒级）
- `Cron`：标准 cron 表达式 + 时区支持

**任务载荷**（`CronPayload`）：
- `AgentTurn`：触发 Agent 对话轮次，指定 prompt 和可选 Agent ID

**任务状态**（`CronJobStatus`）：
- `Active` / `Paused` / `Disabled` / `Completed` / `Missed`
- `consecutive_failures` + `max_failures`：连续失败自动禁用
- `running_at`：执行锁（防并发）

**组件**：
- `db.rs`：SQLite 持久化
- `scheduler.rs`：调度器（tokio 异步）
- `executor.rs`：任务执行器
- `schedule.rs`：cron 表达式校验

**GUI 管理**：
- `src/components/cron/CronJobForm.tsx` 提供创建/编辑界面
- Dashboard 展示活跃 cron 数和运行日志

## 三、Claude Code 独有能力

### 3.1 DreamTask（Proactive 记忆整理）

`src/tasks/DreamTask/DreamTask.ts` + `src/services/autoDream/autoDream.ts`：

**核心机制**：
- 空闲时自动触发记忆整理（"做梦"），无需用户干预
- 以 forked subagent 运行，不污染主对话

**门控策略（cheapest-first）**：
1. **时间门控**：距上次整理 >= minHours（一次 stat 调用）
2. **会话门控**：新增 transcript 数 >= minSessions
3. **锁门控**：无其他进程正在整理

**四阶段流程**：
1. Orient（定向）
2. Gather（收集）
3. Consolidate（整理）
4. Prune（裁剪）

**UI 集成**：
- `DreamTaskState`：phase（starting/updating）、sessionsReviewing、filesTouched、turns
- 通过 Task 系统在页脚显示进度
- 支持 Shift+Down 打开详情对话框
- 可 kill（回滚 consolidation lock）

**节流**：
- `SESSION_SCAN_INTERVAL_MS = 10 * 60 * 1000`：时间门控通过但会话不足时，10 分钟内不重复扫描

### 3.2 SyntheticOutput（结构化输出）

`src/tools/SyntheticOutputTool/SyntheticOutputTool.ts`：

- 工具名：`StructuredOutput`
- 仅在非交互会话（`isNonInteractiveSession`）中启用
- 接受任意 JSON schema 输入，通过 Ajv 验证
- 用途：API/SDK 调用时强制 Agent 返回结构化 JSON
- `maxResultSizeChars = 100_000`
- 并发安全、只读

### 3.3 Advisor Model

`src/commands/advisor.ts`：

- `/advisor <model>` 命令设置顾问模型
- 允许用主模型+顾问模型的分级架构（如 Sonnet 主模型 + Opus 顾问）
- `modelSupportsAdvisor`：检查当前主模型是否支持 advisor
- `isValidAdvisorModel`：检查目标模型是否可用作 advisor
- `/advisor unset` 或 `/advisor off` 禁用
- 持久化到 userSettings

### 3.4 History Snip（SnipTool）

通过 `src/utils/collapseReadSearch.ts` 和 `src/utils/messages.ts` 实现：

- 对话历史中的大型 Read/Search 结果自动折叠
- 保留 head + tail 预览，中间内容替换为摘要
- 减少上下文 token 消耗，同时保留关键信息

### 3.5 Coordinator Mode

`src/coordinator/coordinatorMode.ts`：

- 协调多个工具/子任务的执行模式
- 与 SyntheticOutputTool 配合实现复杂任务编排

## 四、OpenClaw 独有能力

### 4.1 A2UI Canvas 工作台

`src/canvas-host/` 实现 Web Canvas 工作台：

**架构**：
- HTTP 服务器（Node.js `http` + chokidar 文件监控 + WebSocket）
- 路由：`/__openclaw__/a2ui`（A2UI 界面）、`/__openclaw__/canvas`（画布内容）、`/__openclaw__/ws`（WebSocket）
- A2UI bundle（`a2ui/index.html` + `a2ui.bundle.js`）独立打包

**功能**：
- Agent 生成的 HTML/JS/CSS 内容实时渲染
- WebSocket 实时推送更新
- Live Reload 支持（chokidar 文件监控）
- 安全文件解析（`resolveFileWithinRoot` 防路径穿越）
- 多候选目录自动发现 A2UI 资源（source/dist/entry 等 10+ 路径）
- iOS/Android 原生 App 内嵌（`openclaw:a2ui-action-status` 事件桥接）
- 默认 index.html 提供 Canvas 状态页

**CanvasHostHandler**：
- `handleHttpRequest`：静态文件服务 + MIME 类型检测
- `handleUpgrade`：WebSocket 升级
- 可配置 `rootDir`、`basePath`、`port`、`listenHost`
- 支持测试模式（`allowInTests`）

### 4.2 原生 App（macOS/iOS/Android）

**macOS App**（`apps/macos/`）：
- Swift Package（`Package.swift`）
- `Sources/`：OpenClaw 主应用、Discovery（设备发现）、IPC（进程间通信）、MacCLI（命令行）、Protocol（协议定义）
- 原生 macOS 菜单栏 App

**iOS App**（`apps/ios/`）：
- 完整 iOS 应用（`project.yml` + Xcode 配置）
- `Sources/`：主应用 + SwiftUI
- `ShareExtension/`：系统分享扩展（向 AI 发送内容）
- `WatchApp/` + `WatchExtension/`：Apple Watch 支持
- `ActivityWidget/`：iOS 动态岛/锁屏 Widget
- fastlane 自动化构建
- 截图测试

**Android App**（`apps/android/`）：
- 原生 Android 应用

**共享代码**（`apps/shared/`）：
- 跨平台共享逻辑

### 4.3 消息队列策略

`SessionEntry.queueMode` 支持 8 种模式：
- `steer`：引导当前运行
- `followup`：追加后续
- `collect`：收集批量
- `steer-backlog` / `steer+backlog`：引导+积压
- `queue`：排队
- `interrupt`：中断当前

配合 `queueDebounceMs`、`queueCap`、`queueDrop`（old/new/summarize）实现精细的消息流控。

### 4.4 Cron 增强

相比 OpenComputer 的 Cron，OpenClaw 增加了：
- `CronSessionTarget`：main / isolated / current / `session:{id}`
- `CronWakeMode`：next-heartbeat / now
- `CronDelivery`：多渠道通知（announce/webhook + 独立失败通知）
- `staggerMs`：确定性错开窗口（避免多任务同时触发）
- `CronRunTelemetry`：运行遥测（model/provider/usage）

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| 数据大盘 | 完整 GUI（recharts） | 无 | 无 |
| Token 成本追踪 | Dashboard 聚合 | 会话级 | 会话级 |
| TTFT 分析 | 有（按模型/按日期） | 无 | 无 |
| 工具使用统计 | 有（频次/错误率/耗时） | 无 | 无 |
| 天气系统 | 有（Open-Meteo + CoreLocation） | 无 | 无 |
| 多语言 GUI | 12 种语言 | 英文 | 英文 |
| i18n 工具链 | sync-i18n 脚本 | 无 | 无 |
| Cron 调度 | 三种类型 | 无 | 增强版（5 种模式） |
| Cron GUI | 有 | 无 | CLI/API |
| Proactive 记忆整理 | 无 | DreamTask | 无 |
| 结构化输出工具 | 无 | SyntheticOutput | 无 |
| Advisor 分级模型 | 无 | 有 | 有（modelOverride） |
| Canvas 工作台 | 无 | 无 | A2UI + WebSocket |
| 原生 iOS App | 无 | 无 | 有（含 Watch/Widget） |
| 原生 macOS App | Tauri 桌面 App | 无 | Swift 原生 App |
| Android App | 无 | 无 | 有 |
| 消息队列策略 | 无 | 无 | 8 种模式 |
| History 折叠 | 无 | SnipTool/collapse | 无 |
| Coordinator Mode | 无 | 有 | 无 |
| Markdown 富渲染 | Streamdown+Shiki+KaTeX+Mermaid | 终端 Markdown | 渠道原生格式 |

## 六、差距分析与建议

### 6.1 OpenComputer 独有优势

1. **数据大盘**：三者中唯一提供完整数据可视化的项目，可直观分析 Token 消耗、成本、工具使用
2. **多语言 GUI**：12 种语言支持，国际化程度最高
3. **天气感知**：唯一集成环境感知能力的项目
4. **TTFT 追踪**：唯一在消息级别记录并在 Dashboard 展示首 Token 延迟的实现

### 6.2 需补齐的差距

**P0 - 短期**：
- **Proactive 记忆整理**：参考 DreamTask 实现空闲时自动整理记忆文件，已有 `memory/` 模块和 `memory_extract.rs` 基础设施，缺少触发调度
- **结构化输出**：在 Agent SDK/API 模式下支持 StructuredOutput 工具，提高程序化调用可靠性

**P1 - 中期**：
- **Canvas 画布**：利用 Tauri webview 优势实现内嵌 Canvas，Agent 生成的 HTML 可直接渲染
- **Advisor 分级模型**：支持主模型 + 顾问模型配置，廉价模型日常使用 + 高端模型关键决策
- **History 智能折叠**：对话历史中大型工具结果自动折叠，减少上下文消耗

**P2 - 远期**：
- **移动端 App**：利用 Tauri 2 的移动端支持（iOS/Android beta），或评估 Swift/Kotlin 原生方案
- **Cron 增强**：增加会话目标类型、错开窗口、运行遥测
- **消息队列策略**：为 IM 渠道场景增加精细的消息流控模式
