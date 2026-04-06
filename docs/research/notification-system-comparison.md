# 通知系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.12

## 一、架构总览

三个项目在通知系统设计上差异显著：

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| 运行环境 | 桌面 GUI（Tauri） | CLI 终端（Ink） | 多渠道守护进程 |
| 通知载体 | macOS 原生桌面通知 | 终端内 TUI 横幅 + 终端专有协议（bell/iTerm2/Kitty/Ghostty） | 渠道回推（Telegram/WeChat/Slack 等） |
| 优先级模型 | 无（单一路径） | 四级优先级队列（low/medium/high/immediate） | 事件驱动（heartbeat + channel delivery） |
| 折叠/去重 | 无 | fold 合并 + invalidates 失效 + key 去重 | lastHeartbeatText 去重 |
| 通知来源数量 | 1（Agent 工具调用） | 16+ hooks | 多种 node event 类型 |

## 二、OpenComputer 实现

### 2.1 工具驱动通知

通知通过内置工具 `send_notification` 触发，定义在 `crates/oc-core/src/tools/notification.rs`：

- Agent 在对话中调用 `send_notification` 工具，传入 `title` 和 `body`
- Rust 后端通过 `tauri::Emitter` 发射 `agent:send_notification` 事件
- 前端 `useNotificationListeners` hook 监听该事件，调用 `notify()` 函数触发 macOS 原生通知

### 2.2 全局开关与 Agent 级配置

- `NotificationPanel.tsx` 提供 GUI 设置面板
- 全局 enabled 开关控制所有通知
- 每个 Agent 可独立配置 `notifyOnComplete`（on/off/default 三态）
- 配置持久化到 `config.json` 和各 Agent 的 `agent.json`

### 2.3 局限性

- 仅支持桌面原生通知一种渠道
- 无优先级区分——所有通知平等对待
- 无折叠/合并机制——重复通知直接发送
- 无通知队列管理——多通知并发时无排序策略
- 通知仅由 Agent 工具主动触发，无系统级事件通知（如 rate limit、model fallback 等）

## 三、Claude Code 实现

### 3.1 优先级队列

`src/context/notifications.tsx` 实现了完整的通知队列系统：

- **四级优先级**：`low`、`medium`、`high`、`immediate`
- `immediate` 优先级直接抢占当前显示的通知，将被抢占的非 immediate 通知放回队列
- 非 immediate 通知按优先级排序入队，依次显示
- 默认超时 8000ms 后自动清除，支持自定义 `timeoutMs`
- 基于 key 去重：队列中已有相同 key 的通知不重复入队

### 3.2 通知折叠（fold）

通知支持 `fold` 函数，类似 `Array.reduce()`：

```typescript
fold?: (accumulator: Notification, incoming: Notification) => Notification
```

- 当同 key 通知已存在（无论在队列中还是正在显示），调用 fold 合并
- 合并后重置显示超时，确保用户看到最新聚合内容
- 适用场景：多次 rate limit 警告合并为一条

### 3.3 通知失效（invalidates）

通知携带 `invalidates` 字段，声明应废止哪些通知：

```typescript
invalidates?: string[]  // 被废止的通知 key 列表
```

- 新通知入队时，自动从队列中移除被废止的通知
- 若被废止的通知正在显示，清除其超时并立即替换
- `immediate` 通知会额外过滤队列中所有被 invalidates 的条目

### 3.4 通知 Hooks

`src/hooks/notifs/` 包含 16 个专用 hook，每个监控特定事件：

| Hook | 监控事件 |
|------|----------|
| `useRateLimitWarningNotification` | API 速率限制逼近/超出 |
| `useFastModeNotification` | Fast Mode 切换提示 |
| `useAutoModeUnavailableNotification` | 自动模式不可用 |
| `useDeprecationWarningNotification` | 模型/功能废弃 |
| `useLspInitializationNotification` | LSP 初始化状态 |
| `useMcpConnectivityStatus` | MCP 服务连接状态 |
| `useModelMigrationNotifications` | 模型迁移提醒 |
| `useNpmDeprecationNotification` | npm 包废弃警告 |
| `usePluginAutoupdateNotification` | 插件自动更新 |
| `usePluginInstallationStatus` | 插件安装进度 |
| `useSettingsErrors` | 配置错误提示 |
| `useIDEStatusIndicator` | IDE 状态指示 |
| `useCanSwitchToExistingSubscription` | 订阅切换提醒 |
| `useTeammateShutdownNotification` | 协作者下线通知 |
| `useInstallMessages` | 安装消息 |
| `useStartupNotification` | 通用启动通知基础 hook |

`useStartupNotification` 是基础设施 hook，封装了 remote-mode 门控和 once-per-session 防重，其余 hook 基于它构建。

### 3.5 多终端通知渠道

`src/services/notifier.ts` 支持多种终端通知协议：

| 渠道 | 机制 |
|------|------|
| `auto` | 自动检测终端类型（iTerm2/Kitty/Ghostty/Apple Terminal） |
| `iterm2` | iTerm2 专有 escape sequence |
| `iterm2_with_bell` | iTerm2 序列 + 终端 bell |
| `kitty` | Kitty 终端专有协议（带 ID） |
| `ghostty` | Ghostty 终端专有协议 |
| `terminal_bell` | 通用终端 bell（\a） |
| `notifications_disabled` | 禁用 |

- 可通过配置 `preferredNotifChannel` 选择渠道
- Apple Terminal 特殊处理：检测 Bell 是否被禁用（通过 osascript + plist 解析）
- 通知发送前执行用户自定义 hooks（`executeNotificationHooks`）

## 四、OpenClaw 实现

### 4.1 渠道回推通知

OpenClaw 无独立通知子系统，而是利用 IM 渠道本身进行通知：

- `server-node-events.ts` 中的 `notification` 事件类型处理通知推送
- 通知文本限制 120 字符（`MAX_NOTIFICATION_EVENT_TEXT_CHARS`）
- 通过 `CronDelivery` 系统将 cron 任务结果回推到指定渠道
- 支持失败通知独立目的地（`CronFailureDestination`）

### 4.2 心跳去重

- `SessionEntry.lastHeartbeatText` 记录最近一次心跳内容
- `lastHeartbeatSentAt` 记录发送时间戳
- 相同内容不重复推送，避免心跳噪声

### 4.3 APNS 推送注册

- `registerApnsRegistration` 处理 Apple Push Notification 注册
- 支持原生 iOS/macOS 推送通知（通过 native app）
- 与守护进程配合实现后台推送

### 4.4 渠道级通知配置

Cron 系统支持精细的通知配置：

- `deliveryMode`：none / announce / webhook
- 可指定通知目标渠道、账号、线程
- 支持 `bestEffort` 模式（失败不阻塞）

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|:-----------:|:-----------:|:--------:|
| 桌面原生通知 | 有 | 无（CLI） | 有（原生 app） |
| 终端 TUI 通知 | 无 | 有 | 无 |
| IM 渠道回推 | 无 | 无 | 有 |
| 优先级队列 | 无 | 四级 | 无 |
| 通知折叠 | 无 | fold 合并 | 无 |
| 通知失效机制 | 无 | invalidates | 无 |
| Key 去重 | 无 | 有 | heartbeat 去重 |
| 系统事件通知 | 无 | 16 类 | node event 驱动 |
| Rate Limit 预警 | 无 | 有（实时） | 有（failover 通知） |
| 多终端协议 | 无 | 5 种 | 无 |
| 通知 hooks 扩展 | 无 | executeNotificationHooks | 无 |
| 全局开关 | 有 | 有 | 渠道级 |
| Agent 级开关 | 有 | 无 | 无 |
| Cron 结果通知 | 无 | 无 | 有（多渠道） |
| APNS 推送 | 无 | 无 | 有 |

## 六、差距分析与建议

### 6.1 核心差距

1. **无优先级管理**：OpenComputer 所有通知同一优先级，当多通知并发时缺乏排序和抢占机制
2. **无折叠/合并**：重复性通知（如连续多次 rate limit）会产生通知轰炸
3. **无系统事件通知**：仅靠 Agent 工具主动触发，缺少 rate limit、model fallback、context overflow 等系统级预警
4. **单一通知渠道**：仅支持 macOS 桌面通知，无 IM 渠道回推能力
5. **无通知失效机制**：旧通知无法被新状态自动废止

### 6.2 建议

**P0 - 短期**：
- 增加系统事件通知：rate limit 预警、model fallback 提醒、context overflow 警告
- 这些信息后端已有（`failover.rs`、`context_compact/`），仅需通过 Tauri event 前推

**P1 - 中期**：
- 实现通知优先级队列（参考 Claude Code 的四级模型）
- 增加通知折叠/去重机制，避免相同类型通知重复弹出
- 支持通知历史记录面板（GUI 天然适合展示通知历史）

**P2 - 远期**：
- 利用已有 IM Channel 系统（Telegram/WeChat）实现通知回推
- Cron 任务完成通知回推到 IM 渠道
- 通知 hooks 扩展点，允许用户自定义通知处理
