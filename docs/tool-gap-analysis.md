# OpenComputer vs OpenClaw 内置工具差异分析

> 基线对比时间：2026-03-24
> OpenComputer 当前工具数：19 | OpenClaw 当前工具数：~28（+ 动态插件工具）

## 架构差异

| 维度 | OpenComputer | OpenClaw |
|------|-------------|----------|
| 定位 | 本地桌面 AI 助手（Tauri + Rust） | 云端 Agent 平台（Node.js） |
| 编码工具来源 | Rust 自研（`src-tauri/src/tools/`） | `@mariozechner/pi-coding-agent` 库 + 自研覆盖 |
| 工具注册 | Rust `get_available_tools()` + 条件注入 | `pi-tools.ts` 组装编码工具 + `openclaw-tools.ts` 组装平台工具 |
| 扩展机制 | SKILL.md 技能系统（3 层加载：extra dirs → `~/.opencomputer/skills/` → `.opencomputer/skills/`，frontmatter 声明 + 环境检查 + 系统提示词注入） | `/skills/` 目录动态加载插件工具（`resolvePluginTools`，运行时注入为独立 tool） |

## 共有工具对比

### 文件系统 & 执行

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| read | `read` | `read`（pi-coding-agent） | OC 支持图片 base64 读取；OpenClaw 通过 `createOpenClawReadTool` 包装，支持 image sanitization + context window 自适应输出 |
| write | `write` | `write`（pi-coding-agent） | OpenClaw 多 sandbox/host workspace 双模式，支持 memory flush append-only 写入 |
| edit | `edit` | `edit`（pi-coding-agent） | OC 支持更多参数别名（old_string/oldText 等）；OpenClaw 多 sandbox/host 双模式 |
| apply_patch | `apply_patch` | `apply_patch`（自研） | OpenClaw 仅 OpenAI provider + 白名单模型启用；OC 始终可用 |
| ls | `ls` | `ls`（pi-coding-agent） | 基本一致 |
| grep | `grep` | `grep`（pi-coding-agent） | 基本一致，都遵守 .gitignore |
| find | `find` | `find`（pi-coding-agent） | 基本一致 |
| exec | `exec` | `exec`（自研 bash-tools） | OC 多 `pty`、Docker `sandbox` 参数；OpenClaw 多 host 远程执行、safe-bin 策略、sandbox 容器执行、approval 机制、node 远程分发 |
| process | `process` | `process`（自研 bash-tools） | OC 更多 action（log/write/clear/remove）；OpenClaw 有 scopeKey 隔离防跨 session 可见 |

### Web & 信息

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| web_search | `web_search` | `web_search` | 都支持多搜索引擎，基本一致；OpenClaw 额外支持 runtime 动态切换 |
| web_fetch | `web_fetch` | `web_fetch` | 都用 Readability + Markdown；OpenClaw 额外支持 Firecrawl runtime |

### 记忆

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| 记忆搜索 | `recall_memory` | `memory_search` | 名称不同，功能类似（语义/关键词检索） |

### 定时任务

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| cron | `manage_cron` | `cron` | 基本一致，都支持一次性/周期/cron 表达式 |

### 浏览器

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| browser | `browser` | `browser` | OC 用 CDP 直连；OpenClaw 支持 sandbox bridge URL + node 远程浏览器代理路由 |

### 子 Agent

| 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|-------------|----------|----------|
| 子 Agent 生命周期 | `subagent`（单工具 9 action） | `sessions_spawn` + `subagents` + `sessions_yield` | OpenClaw 拆分为 3 个独立工具；OC 合并为 1 个工具（spawn/check/list/result/kill/kill_all/steer/batch_spawn/wait_all） |

## OpenComputer 独有工具

| 工具 | 说明 | 备注 |
|------|------|------|
| `save_memory` | 显式保存记忆（4 种类型 + 2 种作用域） | OpenClaw 记忆写入通过文件系统（MEMORY.md）而非专用工具 |
| `update_memory` | 按 ID 更新记忆内容和标签 | OpenClaw 无此细粒度操作 |
| `delete_memory` | 按 ID 删除记忆 | OpenClaw 无此细粒度操作 |
| `send_notification` | macOS 原生桌面通知（条件注入） | OpenClaw 用 `message` 工具覆盖通知场景（多渠道） |

## OpenClaw 独有工具

### ~~已补齐~~

| 工具 | 说明 | OpenComputer 对应实现 |
|------|------|----------------------|
| ~~`sessions_spawn`~~ | ~~创建子 Agent~~ | ✅ `subagent` 的 `spawn`/`batch_spawn` action |
| ~~`subagents`~~ | ~~管理子 Agent~~ | ✅ `subagent` 的 `list`/`kill`/`kill_all`/`steer` action |
| ~~`sessions_yield`~~ | ~~等待子 Agent 结果~~ | ✅ `subagent` 的 `check`/`result`/`wait_all` action |
| ~~`browser.profiles`~~ | ~~浏览器多配置档~~ | ✅ `browser` 的 `list_profiles` action + `launch` 的 `profile` 参数 |
| ~~`browser.pdf`~~ | ~~页面导出 PDF~~ | ✅ `browser` 的 `save_pdf` action |

### 优先级 P1 — 重要增强

| 工具 | 说明 | 补齐建议 |
|------|------|----------|
| `sessions_send` | 向其他会话发送消息 | 跨会话通信，OC 的 subagent 仅支持 steer 干预运行中的子 Agent，不支持向任意会话发消息 |
| `sessions_list` | 列出所有会话及元数据 | 会话管理基础设施，可结合前端侧边栏会话列表暴露给 Agent |
| `sessions_history` | 获取会话聊天历史（分页） | 跨会话上下文引用，Agent 无法读取其他会话的历史 |
| `session_status` | 查询会话状态和模型配置 | 任务状态监控，当前 subagent 的 `check` 仅查子 Agent 状态 |
| `agents_list` | 列出可用 Agent | 多 Agent 场景下动态选择合适的 Agent 委派任务 |
| `image` | 图片理解 / 视觉分析 | 多模态能力，OC 的 `read` 已支持图片 base64 但未暴露为独立视觉分析工具；OpenClaw 支持 sandbox 隔离 + agentDir 存储 |
| `memory_get` | 记忆文件分页读取（行级精确定位） | `recall_memory` 已覆盖搜索场景，但缺少 memory_search → memory_get 的两步精确读取流程 |
| `pdf` | PDF 文档提取分析 | 文档处理工具（非浏览器导出 PDF），OpenClaw 支持 sandbox + agentDir 存储 |

### 优先级 P2 — 扩展能力

| 工具 | 说明 | 补齐建议 |
|------|------|----------|
| `message` | 多渠道消息发送（Slack/Discord/Telegram/WhatsApp 等） | 需要先设计通道抽象层，OC 的 `send_notification` 仅覆盖桌面通知；OpenClaw 支持 auto-threading、reply-to 模式、group 路由 |
| `image_generate` | 图片生成（DALL-E 等） | 创意工具，可集成 OpenAI Image API；OpenClaw 支持 sandbox + agentDir 存储生成结果 |
| `tts` | 文本转语音 | 语音输出能力，OpenClaw 按 channel provider 条件启用 |
| `canvas` | UI Canvas 控制 | 前端交互增强，动态渲染展示 |
| `nodes` | 设备控制（摄像头/截屏/定位/通知/invoke） | IoT/设备集成，OpenClaw 支持 node 远程路由 + media invoke |
| `gateway` | 网关配置管理（restart/config） | 平台运维能力，owner-only 权限控制 |

## 数量统计

| 分类 | OpenComputer | OpenClaw |
|------|-------------|----------|
| 总工具数 | **19** | **~28** + 插件 |
| 文件系统（read/write/edit/ls/grep/find） | 6 | 6（pi-coding-agent） |
| 执行（exec/process） | 2 | 2（bash-tools） |
| 补丁（apply_patch） | 1 | 1（条件启用） |
| Web（search/fetch） | 2 | 2 |
| 记忆 | 4（recall/save/update/delete） | 2（search/get） |
| 定时任务 | 1 | 1 |
| 浏览器 | 1 | 1 |
| 子 Agent / 会话 | 1（9 种 action） | 6（spawn/yield/send/list/history/status + subagents） |
| 通知 / 消息 | 1（桌面通知） | 1（多渠道消息） |
| Agent 管理 | 0 | 1（agents_list） |
| 多模态 / 媒体 | 0 | 4（image/image_generate/tts/pdf） |
| 平台特有 | 0 | 3（nodes/gateway/canvas） |

## 补齐路线建议

1. **Phase 1**：会话管理能力（sessions_list/history/send/status + agents_list）— subagent 已覆盖 spawn/管理，但跨会话通信和状态查询仍缺失
2. **Phase 2**：多模态工具（image 视觉分析 + pdf 文档提取）— 利用已有的多模态 API 能力
3. **Phase 3**：消息通道（message）+ 图片生成（image_generate）— 扩展输出形式
4. **Phase 4**：语音（tts）+ UI 交互（canvas）— 增强用户体验
