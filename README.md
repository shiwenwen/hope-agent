<p align="center">
  <img src="alpha-logo.png" alt="Hope Agent" width="200">
</p>

<h1 align="center">Hope Agent</h1>

<p align="center">
  <strong>深度融合系统的桌面 AI 助手，也能服务化常驻、跑在云上</strong><br/>
  会记忆 · 能成长 · 在你所有的聊天里随叫随到
</p>

<p align="center">
  <a href="https://github.com/shiwenwen/hope-agent/actions/workflows/rust.yml"><img src="https://img.shields.io/github/actions/workflow/status/shiwenwen/hope-agent/rust.yml?branch=main&style=flat-square&logo=githubactions&logoColor=white&label=CI" alt="CI status"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white" alt="macOS"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/Linux-experimental-FFA500?style=flat-square&logo=linux&logoColor=black" alt="Linux (experimental)"></a>
  <a href="https://github.com/shiwenwen/hope-agent/releases"><img src="https://img.shields.io/badge/Windows-experimental-FFA500?style=flat-square&logo=data:image/svg%2Bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI+PHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDMuNDQ5TDkuNzUgMi4xdjkuNDUxSDBtMTAuOTQ5LTkuNjAyTDI0IDB2MTEuNEgxMC45NDlNMCAxMi42aDkuNzV2OS40NTFMMCAyMC42OTlNMTAuOTQ5IDEyLjZIMjRWMjRsLTEyLjktMS44MDEiLz48L3N2Zz4=" alt="Windows (experimental)"></a>
  <a href="#运行模式"><img src="https://img.shields.io/badge/Web%20GUI-browser-4F46E5?style=flat-square&logo=googlechrome&logoColor=white" alt="Web GUI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-edition%202021-dea584?style=flat-square&logo=rust&logoColor=white" alt="Rust"></a>
  <a href="https://tauri.app/"><img src="https://img.shields.io/badge/Tauri-2-24C8DB?style=flat-square&logo=tauri&logoColor=white" alt="Tauri"></a>
  <a href="https://react.dev/"><img src="https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=black" alt="React"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-green?style=flat-square" alt="License: MIT"></a>
</p>

<p align="center">
  <strong>简体中文</strong> · <a href="./README.en.md">English</a>
</p>

---

**Hope Agent** 是一款面向普通人的 AI 助手——我们把重心放在**强大的桌面 GUI 和系统深度融合**上，同时它也能以服务形态常驻后台，部署到 NAS、自家服务器或云主机上，在 IM 渠道里随叫随到。一个原生安装包，主流大模型 GUI 模板内置齐全，填完 API Key 就能开聊。它能跨会话记住你、空闲时自己整理记忆、把做过的任务沉淀成可复用的技能。

## 缘起

普通人需要一个**打开就能用**的 AI 助手——下载安装包双击打开就行，不用先装运行时，也不用先学一套命令行，更不用担心看不懂配置、服务半夜崩了没人管。Hope Agent 不只是桌面 GUI，它还能以 HTTP/WS 服务常驻，放在 NAS、自家服务器或云主机上 7×24 跑着，同时接入 IM 渠道、对接 IDE（ACP）；但我们相信大多数普通人的入口仍然是桌面，所以在**桌面 GUI 和系统深度融合**上投入了最多的精力，同时把性能、稳定性和交互细节一起打磨好。

Hope Agent 早期曾受 [openclaw](https://github.com/openclaw/openclaw) 影响，感谢他们在本地 AI 助手方向上的先行工作——我们选择了不同的实现路径。

## 亮点

### 🎯 日常使用

<table>
<tr><td width="220"><b>🖥️ 桌面原生 GUI</b></td><td>macOS / Linux / Windows 三端原生应用，下载即用。12 种界面语言（简/繁中、英、日、韩、西、葡、俄、阿、土、越、马），深色主题与精心调校的字体排版。</td></tr>
<tr><td><b>🧙 傻瓜式 Provider 配置</b></td><td>36 个内置 Provider 模板，覆盖 166 个预设模型。Anthropic / OpenAI / Gemini / Codex / OpenRouter / DeepSeek / Kimi / Qwen / 豆包 / GLM / MiniMax / xAI / Mistral / Ollama 一站式覆盖；同一 Provider 支持多 API Key 自动轮换，遇到限流或额度用尽无缝切换下一把钥匙。</td></tr>
<tr><td><b>🦙 本地小模型一键安装</b></td><td><b>不用账号、不用 API Key、不用终端</b>——设置 → 模型页面按硬件挑一个能跑得动的 Qwen3.6 / Gemma 4 尺寸，一键完成 <a href="https://ollama.com">Ollama</a> 安装、模型下载、Provider 注册与切换。同一流程也覆盖本地 Embedding 模型。</td></tr>
<tr><td><b>💬 12 个 IM 渠道一站接入</b></td><td>Telegram、Discord、Slack、飞书、Google Chat、LINE、QQ Bot、Signal、iMessage、IRC、WeChat、WhatsApp。图片 / 语音 / 文件入站自动转多模态上下文；工具审批直接在聊天窗按按钮决定；每个群聊 / 账号可绑定独立 Agent 和权限策略。</td></tr>
<tr><td><b>🌐 独立服务 · 浏览器即客户端</b></td><td><b>不止是桌面应用</b>——可以完全脱离 GUI 单独作为服务运行。一条命令 <code>hope-agent server start</code> 就能起一个 HTTP/WS 守护进程，<code>server install</code> 注册成 launchd / systemd 开机自启，放家里 NAS / 云服务器 / 旧笔记本上 24 小时在线。<b>Server 内嵌完整 Web GUI</b>（前端用 <code>rust-embed</code> 打进二进制），<b>手机、平板、浏览器、另一台电脑打开 <code>http://&lt;server&gt;:port</code> 就是完整 React 界面</b>——不用装客户端、不用配前端。Bearer Token 鉴权 + SSRF 三档策略保证公网暴露也可控；会话、记忆、Cron、IM 渠道全在服务端跑，客户端只是窗口。</td></tr>
<tr><td><b>🔁 三种运行模式同核</b></td><td>桌面 GUI（默认）、HTTP/WS 守护进程 + 内嵌 Web GUI（浏览器直连）、ACP stdio（给 IDE 当 agent 后端）。三种模式共用 Rust <code>ha-core</code> 核心库，零 Tauri 依赖——同一份代码既能当桌面 app，也能当服务器，也能嵌进 IDE。</td></tr>
</table>

### 🧠 记忆与学习

<table>
<tr><td width="220"><b>🧠 跨会话持久记忆</b></td><td>SQLite + FTS5 全文检索 + 向量语义检索三位一体。记忆可按全局 / 项目 / Agent 三层 scope 组织；system prompt 注入按联合预算分配，不会因为某一层过长挤掉其他层。</td></tr>
<tr><td><b>🕶 无痕对话</b></td><td>会话级开关，首条消息就能无痕。开启后当前对话不注入任何记忆或跨会话感知，也不自动收集记忆；只有你明确说“记住这个”或“回忆一下”时，才会主动调用记忆工具。</td></tr>
<tr><td><b>💤 离线"做梦"整理</b></td><td>空闲时自动跑一遍"过去这两天最有价值的记忆是哪些"，把入选条目 pin 住并写成 markdown 日记，可在设置 → Dream Diary 回看。每天工作完帮你把今天学到的知识沉淀下来，下次对话用得上。</td></tr>
<tr><td><b>🔍 主动召回 + 反省画像</b></td><td>每轮对话开始前，按你刚打的那句话主动捞出最相关的记忆注入 prompt（Active Memory）；另外反省式地从历史对话里提炼沟通风格 / 工作习惯 / 长期偏好，单独以"用户画像"段落进 prompt，越用越懂你。</td></tr>
<tr><td><b>🛠 会成长的技能系统</b></td><td>执行完复杂任务后自动生成技能草稿（Draft），你审核通过下次就能复用。技能支持条件激活（比如只在编辑 Python 文件时加载）、fork 子 Agent 执行、工具白名单隔离；兼容 <a href="https://agentskills.io">agentskills.io</a> 开放标准，社区技能即插即用。</td></tr>
<tr><td><b>👁 跨会话行为感知</b></td><td>它知道你别的对话里在做什么。每轮对话开始前自动感知其他活跃会话的最近动作、目标、摩擦点，需要时把相关信息同步到当前会话——不打扰主线，只在上下文相关时出现。默认零 LLM 成本的结构化模式，可选切到 LLM 自然语言摘要模式。</td></tr>
<tr><td><b>💾 长对话不失忆</b></td><td>上下文五层渐进式压缩，不管聊多久前文都不会被强切丢失。tool 调用配对永远不拆散；摘要过的消息还会自动从磁盘恢复最近编辑过的文件内容，省去你反复粘贴的麻烦。与 Prompt Caching 配合，长会话的 API 成本明显低于朴素调用。</td></tr>
</table>

### 🛠 工作流 & 工具

<table>
<tr><td width="220"><b>📋 Plan Mode 计划执行</b></td><td>面对复杂任务先出一份可修改 / 可承接的计划书，六态状态机管理执行进度。计划可跨会话存档，下次继续只要一句"继续上次的计划"。执行期间严格按白名单工具操作，避免模型跑飞。</td></tr>
<tr><td><b>📁 Project 项目容器</b></td><td>把相关会话归到同一项目下，继承项目级记忆 / 项目指令 / 共享文件。上传的文件自动文本抽取并三层注入（目录清单 / 小文件自动内联 / 大文件按需读取），不用手动 @ 文件也不怕吃爆上下文。</td></tr>
<tr><td><b>👥 Agent Team 多 Agent 协作</b></td><td>在设置里预置团队模板（成员角色、绑定 Agent、默认任务模板），模型按需一句话就能组建专家团。成员间可互发消息、协同推进，完成后自动把 transcript 汇总回主对话。</td></tr>
<tr><td><b>🗓 自然语言定时任务</b></td><td>"每天早 8 点给我写日报"、"每周一整理上周待办"、"工作日每小时扫一次邮箱"——到点自动跑，结果可选投递到任一 IM 渠道。Cron 在守护进程 / 桌面 GUI 下都能稳定运行。</td></tr>
<tr><td><b>📊 Dashboard + Recap 复盘</b></td><td>内置数据大盘：成本 / Token / 活跃度热力图 / 健康度四维可视化。<code>/recap</code> 深度复盘一键跑过去 N 天会话，生成 11 个 AI 章节报告（含 Agent 工具优化建议、记忆与技能推荐、成本优化等），可导出独立 HTML 分享。</td></tr>
<tr><td><b>🔌 MCP 客户端（OAuth 2.1）</b></td><td>内置 Model Context Protocol 客户端，四种 transport 全支持：stdio / Streamable HTTP / SSE / WebSocket。完整 OAuth 2.1 + PKCE 流程（自动 discovery、RFC 7591 动态注册、loopback 回调），凭据 0600 原子写落盘，Notion / Linear 等标准 OAuth server 可一键授权；所有出站 URL 硬过 SSRF 策略。GUI 里一键从 <code>claude_desktop_config.json</code> 导入，工具自动以 <code>mcp__&lt;server&gt;__&lt;tool&gt;</code> 接入主对话；另配 <code>mcp_resource</code> / <code>mcp_prompt</code> 工具访问被动数据，长跑工具自动后台化。</td></tr>
<tr><td><b>🔧 工具箱</b></td><td>可控浏览器（CDP）、Canvas 画布、AI 画图（7 个 Provider）、Web 搜索（8 个 Provider failover）、bash 执行（可选 Docker 沙箱隔离）、文件读写 / grep / find、URL 预览、崩溃日志、自诊断。</td></tr>
<tr><td><b>⚡ 后台跑长任务</b></td><td>耗时的 shell 命令 / Web 搜索 / AI 画图可以让 Agent "丢到后台跑"，立即返回 <code>job_id</code> 继续对话不阻塞。后台完成后结果自动注入回主对话，也可以让模型主动 <code>job_status</code> poll 结果。再长的任务都不会卡住你的聊天窗。</td></tr>
</table>

### 🛡 安全与本地化

<table>
<tr><td width="220"><b>🔒 工具审批 + Docker 沙箱</b></td><td>敏感工具调用走审批门控（支持超时后自动 deny / proceed 策略，也支持渠道级自动批准）；高危的 bash / 文件写入可选择跑在 Docker 沙箱里隔离执行。给 Agent 高权限也不怕翻车。</td></tr>
<tr><td><b>🏠 本地优先 · 零第三方中转</b></td><td>所有数据在 <code>~/.hope-agent/</code>：配置、会话、记忆、附件、技能、日志全部本地 SQLite / 文件存储；API Key 直连模型厂商。服务模式下 Bearer Token 鉴权 + SSRF 三档策略，远程访问也可控。</td></tr>
<tr><td><b>🛟 配置自动快照 · 一键回滚</b></td><td>任何配置变更都自动快照到本地 <code>backups/autosave/</code>，保留最近 50 份。就算模型通过设置工具帮你改乱了参数，也能随时还原到任意历史时间点。</td></tr>
<tr><td><b>♻️ 崩溃自愈 · 三层保活</b></td><td>父子进程 Guardian 监控子进程异常退出，指数退避（1s → 3s → 9s → 15s → 30s）自动拉起；连续崩溃 5 次自动备份配置 + LLM 自诊断 + 尝试自动修复，崩溃历史在「设置 → 崩溃历史」里可回看。<code>server install</code> 后再叠加 launchd <code>KeepAlive</code> / systemd <code>Restart=on-failure</code> OS 级二次保险——即使 Guardian 本身被 <code>kill -9</code>，操作系统也会把它拉回来。Cron / IM 渠道 / MCP 连接各自独立 watchdog 自动重连。</td></tr>
</table>

> 更多细节亮点请查看 [CHANGELOG.md](CHANGELOG.md)。

## 快速开始

### 普通用户

> 🛠 **Release 状态**：0.1.0 已可用但未发布二进制——我们想再打磨一轮首次体验再发。代码随时可用，如果你感兴趣，`pnpm tauri build` 即可得到与 Release 完全一致的产物，详见下方「开发者」。

1. 到 [Releases](https://github.com/shiwenwen/hope-agent/releases) 下载对应平台安装包
   - macOS：`Hope-Agent_*.dmg`
   - Linux：`hope-agent_*.AppImage`
   - Windows：`Hope-Agent_*.exe` / `Hope-Agent_*.msi`（尚未完成充分测试）
2. 首次启动向导：**选 Provider 模板 → 填 API Key → 开聊**
3. 桌面安装包内置 GitHub Releases 自动更新；应用内可在 **设置 → 关于** 里检查更新并一键安装

### 开发者

```bash
git clone https://github.com/shiwenwen/hope-agent.git
cd hope-agent
pnpm install
pnpm tauri dev         # 桌面开发模式（前端 + Rust 热重载）

# 其他常用命令
pnpm typecheck         # 前端类型检查（tsc -b）
pnpm lint              # Lint
pnpm tauri build       # 打生产包
```

本地开发时如果想在浏览器里看“网页版”并实时刷新，运行 `pnpm tauri dev` 后打开 `http://localhost:1420`。这是 Vite dev server，和 Tauri 窗口共用前端热更新；`http://localhost:8420` 是内嵌 HTTP/WS 服务提供的静态 Web GUI（来自 `dist/` / embedded bundle），用于模拟打包后的浏览器入口，不会跟随源码 HMR。若本地 Server 开了 API Key，`1420` 页面请求 `8420` 可能返回 401，开发时可先在设置里临时清空 Server API Key 后重启。

## 运行模式

| 模式                        | 启动方式                                                                         | 场景                                                                                                                                                                                              |
| --------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 桌面 GUI                    | 双击图标 / `pnpm tauri dev`                                                      | 功能最全的入口：完整 GUI 体验，并内嵌 HTTP/WS 服务，桌面在用的同时可对外提供接入                                                                                                                  |
| Server + Web GUI（HTTP/WS） | 通过 `server start` 子命令；`server install` 可注册成 launchd / systemd 开机自启 | 无 GUI 守护进程，24 小时在线，IM 渠道 / Cron 不断线；**前端 React UI 通过 `rust-embed` 内嵌进 server 二进制，浏览器打开 `http://<server>:port` 即得完整 Web GUI**，手机 / 平板 / 任意电脑都能直连 |
| ACP（stdio）                | 通过 `acp` 子命令                                                                | IDE 直连，兼容 ACP 协议的编辑器把 Hope Agent 当 agent 后端调                                                                                                                                      |

三种模式共用同一套 `ha-core` 核心逻辑；配置、会话、记忆全部落在 `~/.hope-agent/` 下。

## 生态一览

<table>
<tr>
  <td width="140"><b>📦 模型 Provider</b></td>
  <td>
    <b>36 个模板 · 166 个预设模型</b><br/>
    <b>国际</b> · Anthropic · OpenAI · Codex · Google Gemini · OpenRouter · Azure OpenAI · Groq · Together AI · Fireworks · Perplexity · xAI Grok · Mistral · Cohere<br/>
    <b>国内</b> · DeepSeek · Moonshot (Kimi) · 通义千问 (Qwen) · 豆包 (火山引擎) · 智谱 GLM · MiniMax · 小米 MiMo<br/>
    <b>本地</b> · Ollama · 任意 OpenAI 兼容端点
  </td>
</tr>
<tr>
  <td><b>💬 IM 渠道</b></td>
  <td><b>12 个</b> · Telegram · Discord · Slack · 飞书 · Google Chat · LINE · QQ Bot · Signal · iMessage · IRC · WeChat · WhatsApp</td>
</tr>
<tr>
  <td><b>🌐 界面语言</b></td>
  <td><b>12 种</b> · 简体中文 · 繁體中文 · English · 日本語 · 한국어 · Español · Português · Русский · العربية · Türkçe · Tiếng Việt · Bahasa Melayu</td>
</tr>
</table>

## 项目结构

Cargo Workspace 三 Crate 架构，核心业务逻辑全部在 `ha-core`：

```
crates/
  ha-core/       Rust 核心库（零 Tauri 依赖）— 所有业务逻辑在这里
  ha-server/     axum HTTP/WS 守护进程（薄壳）
src-tauri/       Tauri 桌面 Shell（薄壳）
src/             React 19 + TypeScript 前端
skills/          内置技能（随应用发行）
```

完整的模块拓扑、架构约定、编码规范见 [AGENTS.md](AGENTS.md)。

## 文档

详见 [docs/](docs/)。

## 贡献

主分支处于活跃开发阶段，欢迎 issue / PR。贡献前请先读一遍 [AGENTS.md](AGENTS.md) 的 "架构约定" 和 "编码规范" 两节。

常用命令：

```bash
pnpm tauri dev                    # 桌面开发
cargo check --workspace              # Rust 依赖 / 类型检查
cargo test -p ha-core -p ha-server   # 核心测试
node scripts/sync-i18n.mjs --check   # 检查翻译缺失
```

## 社区

- 🐛 [Issues](https://github.com/shiwenwen/hope-agent/issues) — Bug 报告、功能请求
- 💡 [Discussions](https://github.com/shiwenwen/hope-agent/discussions) — 用法分享、想法讨论、提问答疑
- ⭐ 如果 Hope Agent 帮到了你，欢迎在 GitHub 上点个 Star
- 📮 路线图、正式文档站和更多社区渠道正在筹备中

## 致谢

- [openclaw](https://github.com/openclaw/openclaw)：在本地 AI 助手方向上的启发
- [Ollama](https://ollama.com/)：本地大模型一键安装能力建立在 Ollama 的本地运行时与 OpenAI 兼容端点之上；Hope Agent 仅作 GUI 层包装，Qwen / Gemma 等模型由 Ollama 模型库分发
- [ClawHub](https://www.clawhub.com/) / [SkillHub](https://skillhub.cn/)：为 Hope Agent 提供公开的 skill 搜索与发现来源
- [Hermes Agent](https://github.com/NousResearch/hermes)（间接溯源 [obra/superpowers](https://github.com/obra/superpowers)）：部分内置编程方法论 skill 改编自此（MIT），详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
- [Tauri](https://tauri.app/)、[axum](https://github.com/tokio-rs/axum)、[React](https://react.dev/)、[shadcn/ui](https://ui.shadcn.com/)、[Streamdown](https://github.com/streamdown/streamdown)、[Radix UI](https://www.radix-ui.com/) 等开源基础设施
- 所有为这个项目做过反馈、测试、提交 issue 的朋友

## Star History

<a href="https://www.star-history.com/#shiwenwen/hope-agent&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=shiwenwen/hope-agent&type=Date" />
  </picture>
</a>

## License

[MIT](LICENSE)
