# OpenComputer vs OpenClaw 内置工具差异分析

> 基线对比时间：2026-04-01
> OpenComputer 当前工具数：36 | OpenClaw 当前工具数：~31（+ 动态 Channel 插件工具）

## 架构差异

| 维度         | OpenComputer                                                                                                                                  | OpenClaw                                                                       |
| ------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| 定位         | 本地桌面 AI 助手（Tauri + Rust）                                                                                                              | 云端 Agent 平台（Node.js）                                                     |
| 编码工具来源 | Rust 自研（`src-tauri/src/tools/`）                                                                                                           | `@mariozechner/pi-coding-agent` 库 + 自研覆盖                                  |
| 工具注册     | Rust `get_available_tools()` + 条件注入                                                                                                       | `pi-tools.ts` 组装编码工具 + `openclaw-tools.ts` 组装平台工具                  |
| 扩展机制     | SKILL.md 技能系统（3 层加载：extra dirs → `~/.opencomputer/skills/` → `.opencomputer/skills/`，frontmatter 声明 + 环境检查 + 系统提示词注入） | `/skills/` 目录动态加载插件工具（`resolvePluginTools`，运行时注入为独立 tool） |
| 记忆工具     | Rust 自研（SQLite + FTS5 + 向量检索），6 个专用工具                                                                                           | `memory-core` 扩展插件（`extensions/memory-core/`），2 个工具 + 文件系统写入   |
| 浏览器       | Rust CDP 直连，核心工具                                                                                                                       | Plugin 注册（`tool-catalog.ts`），sandbox bridge 代理                          |

## 共有工具对比

### 文件系统 & 执行

| 工具        | OpenComputer  | OpenClaw                     | 功能差异                                                                                                                     |
| ----------- | ------------- | ---------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| read        | `read`        | `read`（pi-coding-agent）    | OC 支持图片 base64 读取；OpenClaw 通过 `createOpenClawReadTool` 包装，支持 image sanitization + context window 自适应输出    |
| write       | `write`       | `write`（pi-coding-agent）   | OpenClaw 多 sandbox/host workspace 双模式，支持 memory flush append-only 写入                                                |
| edit        | `edit`        | `edit`（pi-coding-agent）    | OC 支持更多参数别名（old_string/oldText 等）；OpenClaw 多 sandbox/host 双模式                                                |
| apply_patch | `apply_patch` | `apply_patch`（自研）        | OpenClaw 仅 OpenAI provider + 白名单模型启用；OC 始终可用                                                                    |
| ls          | `ls`          | `ls`（pi-coding-agent）      | 基本一致                                                                                                                     |
| grep        | `grep`        | `grep`（pi-coding-agent）    | 基本一致，都遵守 .gitignore                                                                                                  |
| find        | `find`        | `find`（pi-coding-agent）    | 基本一致                                                                                                                     |
| exec        | `exec`        | `exec`（自研 bash-tools）    | OC 多 `pty`、Docker `sandbox` 参数；OpenClaw 多 host 远程执行、safe-bin 策略、sandbox 容器执行、approval 机制、node 远程分发 |
| process     | `process`     | `process`（自研 bash-tools） | OC 更多 action（log/write/clear/remove）；OpenClaw 有 scopeKey 隔离防跨 session 可见                                         |

### Web & 信息

| 工具       | OpenComputer | OpenClaw     | 功能差异                                                         |
| ---------- | ------------ | ------------ | ---------------------------------------------------------------- |
| web_search | `web_search` | `web_search` | 都支持多搜索引擎，基本一致；OpenClaw 额外支持 runtime 动态切换   |
| web_fetch  | `web_fetch`  | `web_fetch`  | 都用 Readability + Markdown；OpenClaw 额外支持 Firecrawl runtime |

### 记忆

| 工具     | OpenComputer    | OpenClaw                            | 功能差异                                                                            |
| -------- | --------------- | ----------------------------------- | ----------------------------------------------------------------------------------- |
| 记忆搜索 | `recall_memory` | `memory_search`（memory-core 插件） | 功能类似（语义/关键词检索）；OC 用 SQLite FTS5 + 向量，OpenClaw 用 manager.search() |
| 记忆读取 | `memory_get`    | `memory_get`（memory-core 插件）    | OC 按 ID 读取完整元数据；OpenClaw 按文件路径 + 行号范围读取                         |

### 定时任务

| 工具 | OpenComputer  | OpenClaw | 功能差异                                |
| ---- | ------------- | -------- | --------------------------------------- |
| cron | `manage_cron` | `cron`   | 基本一致，都支持一次性/周期/cron 表达式 |

### 浏览器

| 工具    | OpenComputer          | OpenClaw                 | 功能差异                                                                             |
| ------- | --------------------- | ------------------------ | ------------------------------------------------------------------------------------ |
| browser | `browser`（核心工具） | `browser`（plugin 注册） | OC 用 CDP 直连，核心工具；OpenClaw 支持 sandbox bridge URL + node 远程浏览器代理路由 |

### 多模态 / 媒体

| 工具           | OpenComputer     | OpenClaw         | 功能差异                                                                                 |
| -------------- | ---------------- | ---------------- | ---------------------------------------------------------------------------------------- |
| image          | `image`          | `image`          | OC 单图分析 + base64；OpenClaw 支持多图（最多 20 张）+ URL                               |
| image_generate | `image_generate` | `image_generate` | OC 支持 OpenAI/Google/Fal 三 Provider；OpenClaw 按配置推断 Provider                      |
| pdf            | `pdf`            | `pdf`            | OC 用 pdf-extract 文本提取；OpenClaw 支持 Anthropic/Google 原生 PDF 分析 + 文本/图像回退 |

### Canvas

| 工具   | OpenComputer                        | OpenClaw                                             | 功能差异                                                 |
| ------ | ----------------------------------- | ---------------------------------------------------- | -------------------------------------------------------- |
| canvas | `canvas`（11 action，7 种内容类型） | `canvas`（present/hide/navigate/eval/snapshot/A2UI） | OC 功能更丰富（版本历史、导出等）；OpenClaw 多 A2UI 模式 |

### 子 Agent & 会话管理

| 工具              | OpenComputer                  | OpenClaw                                          | 功能差异                                                                                                             |
| ----------------- | ----------------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 子 Agent 生命周期 | `subagent`（单工具 9 action） | `sessions_spawn` + `subagents` + `sessions_yield` | OpenClaw 拆分为 3 个独立工具；OC 合并为 1 个工具（spawn/check/list/result/kill/kill_all/steer/batch_spawn/wait_all） |
| ACP Agent         | `acp_spawn`（独立工具）       | `sessions_spawn` 的 `runtime="acp"` 模式          | OC 单独拆出 ACP 启动；OpenClaw 统一在 sessions_spawn 中                                                              |
| 会话列表          | `sessions_list`               | `sessions_list`                                   | 基本一致                                                                                                             |
| 会话历史          | `sessions_history`            | `sessions_history`                                | 基本一致                                                                                                             |
| 跨会话消息        | `sessions_send`               | `sessions_send`                                   | OC 支持同步等待 + 异步投递；OpenClaw 通过 sessionKey/label 定位                                                      |
| 会话状态          | `session_status`              | `session_status`                                  | 基本一致                                                                                                             |
| Agent 列表        | `agents_list`                 | `agents_list`                                     | 基本一致                                                                                                             |

## OpenComputer 独有工具

| 工具                 | 说明                                                  | 备注                                                   |
| -------------------- | ----------------------------------------------------- | ------------------------------------------------------ |
| `save_memory`        | 显式保存记忆（4 种类型 + 2 种作用域）                 | OpenClaw 记忆写入通过文件系统（MEMORY.md）而非专用工具 |
| `update_memory`      | 按 ID 更新记忆内容和标签                              | OpenClaw 无此细粒度操作                                |
| `delete_memory`      | 按 ID 删除记忆                                        | OpenClaw 无此细粒度操作                                |
| `update_core_memory` | 更新核心记忆文件（memory.md），直接反映在系统提示词中 | OpenClaw 通过 write 工具写 MEMORY.md 实现类似效果      |
| `send_notification`  | macOS 原生桌面通知（条件注入）                        | OpenClaw 用 `message` 工具覆盖通知场景（多渠道）       |
| `get_weather`        | 天气查询（Open-Meteo API，免费无 key）                | OpenClaw 无对应工具                                    |
| `plan_question`      | Plan Mode：向用户发送结构化问题（选项 + 自定义输入）  | OpenClaw 无对应的计划系统                              |
| `submit_plan`        | Plan Mode：提交最终实施计划，进入 Review 状态         | 同上                                                   |
| `update_plan_step`   | Plan Mode：更新计划步骤状态（进行中/完成/跳过/失败）  | 同上                                                   |
| `amend_plan`         | Plan Mode：执行中修改计划（插入/删除/更新步骤）       | 同上                                                   |

## OpenClaw 独有工具

### 优先级 P2 — 扩展能力（尚未补齐）

| 工具      | 说明                                                 | 补齐建议                                                                                                                |
| --------- | ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `message` | 多渠道消息发送（Slack/Discord/Telegram/WhatsApp 等） | 需要先设计通道抽象层，OC 的 `send_notification` 仅覆盖桌面通知；OpenClaw 支持 auto-threading、reply-to 模式、group 路由 |
| `tts`     | 文本转语音                                           | 语音输出能力，OpenClaw 按 channel provider 条件启用                                                                     |
| `nodes`   | 设备控制（摄像头/截屏/定位/通知/invoke）             | IoT/设备集成，OpenClaw 支持 node 远程路由 + media invoke                                                                |
| `gateway` | 网关配置管理（restart/config/update）                | 平台运维能力，owner-only 权限控制                                                                                       |

## 数量统计

| 分类                                     | OpenComputer                                               | OpenClaw                                              |
| ---------------------------------------- | ---------------------------------------------------------- | ----------------------------------------------------- |
| **总工具数**                             | **36**                                                     | **~31** + Channel 插件                                |
| 文件系统（read/write/edit/ls/grep/find） | 6                                                          | 6（pi-coding-agent）                                  |
| 执行（exec/process）                     | 2                                                          | 2（bash-tools）                                       |
| 补丁（apply_patch）                      | 1                                                          | 1（条件启用）                                         |
| Web（search/fetch）                      | 2                                                          | 2                                                     |
| 记忆                                     | 6（recall/save/update/delete/get/update_core）             | 2（search/get，memory-core 插件）                     |
| 定时任务                                 | 1                                                          | 1                                                     |
| 浏览器                                   | 1                                                          | 1（plugin）                                           |
| 子 Agent / 会话                          | 6（subagent + acp*spawn + sessions*\*4）                   | 7（spawn/yield/send/list/history/status + subagents） |
| 通知 / 消息                              | 1（桌面通知）                                              | 1（多渠道消息）                                       |
| Agent 管理                               | 1（agents_list）                                           | 1（agents_list）                                      |
| 多模态 / 媒体                            | 3（image/image_generate/pdf）                              | 4（image/image_generate/tts/pdf）                     |
| 画布 / Canvas                            | 1（canvas）                                                | 1（canvas）                                           |
| 计划 / Plan                              | 4（plan_question/submit_plan/update_plan_step/amend_plan） | 0                                                     |
| 天气                                     | 1（get_weather）                                           | 0                                                     |
| 平台特有                                 | 0                                                          | 2（nodes/gateway）                                    |

## 差异总结

### OpenComputer 领先的领域

- **记忆系统**：6 个专用工具（save/recall/update/delete/get/update_core），SQLite + FTS5 + 向量检索，细粒度 CRUD；OpenClaw 仅 2 个工具（search/get）+ 文件系统写入
- **Plan Mode**：完整的 4 工具计划系统（六态状态机），OpenClaw 无对应能力
- **天气查询**：内置免费天气 API，OpenClaw 无对应
- **Canvas**：11 个 action + 7 种内容类型 + 版本历史，比 OpenClaw 更丰富

### OpenClaw 领先的领域

- **多渠道消息**：`message` 工具支持 Slack/Discord/Telegram/WhatsApp 等多渠道，auto-threading、group 路由
- **语音输出**：`tts` 文字转语音
- **设备控制**：`nodes` 工具支持 IoT 远程设备（摄像头/截屏/定位）
- **PDF 分析**：支持 Anthropic/Google 原生 PDF 理解，不仅仅是文本提取
- **网关运维**：`gateway` 平台级配置管理

### 尚未补齐的 OpenClaw 工具

| 优先级 | 工具      | 理由                         |
| ------ | --------- | ---------------------------- |
| P2     | `message` | 需设计通道抽象层，工程量较大 |
| P3     | `tts`     | 语音场景在桌面端需求有限     |
| P3     | `nodes`   | IoT 场景与桌面端定位不同     |
| P4     | `gateway` | 平台运维能力，桌面端不适用   |
