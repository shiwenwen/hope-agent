# OpenComputer vs OpenClaw 内置工具差异分析

> 基线对比时间：2026-03-22
> OpenComputer 当前工具数：17 | OpenClaw 当前工具数：~27

## 共有工具对比

| 分类 | 工具 | OpenComputer | OpenClaw | 功能差异 |
|------|------|-------------|----------|----------|
| 文件系统 | read | `read` | `read` | OC 支持图片 base64 读取 |
| 文件系统 | write | `write` | `write` | 基本一致 |
| 文件系统 | edit | `edit` | `edit` | OC 支持更多参数别名（old_string/oldText 等） |
| 文件系统 | apply_patch | `apply_patch` | `apply_patch` | 一致 |
| 执行 | exec | `exec` | `exec` | OC 多 `pty`、`sandbox` 参数 |
| 执行 | process | `process` | `process` | OC 更多 action（log/write/clear/remove） |
| Web | web_search | `web_search` | `web_search` | 都支持 8+ 搜索引擎，基本一致 |
| Web | web_fetch | `web_fetch` | `web_fetch` | 都用 Readability + Markdown，基本一致 |
| 记忆 | 记忆搜索 | `recall_memory` | `memory_search` | 名称不同，功能类似 |
| 定时任务 | cron | `manage_cron` | `cron` | 基本一致 |
| 浏览器 | browser | `browser` | `browser` | OC 用 CDP 直连；OpenClaw 多 profiles（多配置档隔离）和 pdf（页面导出 PDF） |

## OpenComputer 独有工具

这些工具 OpenClaw 没有单独提供，或通过其他方式实现：

| 工具 | 说明 | 备注 |
|------|------|------|
| `ls` | 列出目录内容 | OpenClaw 通过 exec 实现 |
| `grep` | 正则搜索文件内容 | OpenClaw 通过 exec 实现 |
| `find` | Glob 模式查找文件 | OpenClaw 通过 exec 实现 |
| `save_memory` | 显式保存记忆 | OpenClaw 记忆写入方式不同 |
| `update_memory` | 按 ID 更新记忆 | OpenClaw 无此细粒度操作 |
| `delete_memory` | 按 ID 删除记忆 | OpenClaw 无此细粒度操作 |

## OpenClaw 独有工具（待补齐）

### 优先级 P0 — 核心能力缺失

| 工具 | 说明 | 补齐建议 |
|------|------|----------|
| `sessions_spawn` | 创建子 Agent（子会话） | 实现 Agent 编排的基础，支持并行任务分发 |
| `sessions_yield` | 结束当前轮次，等待子 Agent 结果 | 配合 spawn 使用，实现异步等待 |
| `sessions_send` | 向其他会话发送消息 | 会话间通信 |
| `subagents` | 管理子 Agent（创建/列表/停止） | 子 Agent 生命周期管理 |

### 优先级 P1 — 重要增强

| 工具 | 说明 | 补齐建议 |
|------|------|----------|
| `sessions_list` | 列出所有会话及元数据 | 会话管理基础设施 |
| `sessions_history` | 获取会话聊天历史（分页） | 跨会话上下文引用 |
| `session_status` | 查询会话状态 | 任务状态监控 |
| `agents_list` | 列出可用 Agent | 多 Agent 系统基础 |
| `image` | 图片理解 / 视觉分析 | 多模态能力 |
| ~~`browser.profiles`~~ | ~~浏览器多配置档隔离~~ | ✅ 已实现：`list_profiles` action + `launch` 的 `profile` 参数 |
| ~~`browser.pdf`~~ | ~~将当前页面导出为 PDF~~ | ✅ 已实现：`save_pdf` action，支持 paper_format/landscape/print_background |

### 优先级 P2 — 扩展能力

| 工具 | 说明 | 补齐建议 |
|------|------|----------|
| `message` | 多渠道消息发送（Slack/Discord/Telegram/WhatsApp 等 10+） | 需要先设计通道抽象层 |
| `nodes` | 设备控制（摄像头/截屏/定位/通知） | IoT/移动设备集成 |
| `gateway` | 网关配置管理（restart/config） | 平台运维能力 |
| `canvas` | UI Canvas 控制 | 前端交互增强 |
| `image_generate` | 图片生成（DALL-E 等） | 创意工具 |
| `tts` | 文本转语音 | 语音输出能力 |
| `pdf` | PDF 文档提取分析 | 文档处理工具（非浏览器导出） |
| `memory_get` | 记忆分页读取 | 当前 recall_memory 已部分覆盖 |

## 补齐路线建议

1. **Phase 1**：子 Agent 编排（spawn/yield/send/subagents）— 这是 OpenClaw 最核心的差异化能力
2. **Phase 2**：会话管理（sessions_list/history/status）+ agents_list
3. **Phase 3**：浏览器增强（profiles + pdf 导出）+ 图片理解
4. **Phase 4**：消息通道 + 设备控制 + 媒体工具
