<p align="center">
  <img src="assets/alpha-logo.png" alt="Hope Agent" width="200">
</p>

<h1 align="center">Hope Agent</h1>

<p align="center">
  <strong>跨端交接、越用越懂你的桌面 AI 助手，也能服务化常驻、跑在云上</strong><br/>
  会记忆 · 能成长 · 能自主推进目标 · 会动态编排任务 · 在你所有的聊天里随叫随到
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

**Hope Agent** 是一个本地优先、桌面优先，也能服务化常驻的个人 AI Agent。它像成熟的桌面软件一样打开即用，又真正具备 Agent 的行动力：理解长期上下文、调用工具完成工作，并在你离开后继续可靠地推进目标。

**Hope Agent 很早就开始探索桌面 AI Agent。** 在桌面 Agent 还很少见、Codex 尚处早期形态的阶段，我们就已经开始构建这个项目。我们一直专注于产品本身，而不是制造声量。随着桌面 AI Agent 逐渐成为热门方向，越来越多产品开始探索相似路径，也印证了我们最初的判断：**AI 助手终将走出聊天框，成为可以长期托付工作的个人软件。**

## 目录

- [核心能力](#核心能力)
- [能力全景](#能力全景)
- [快速开始](#快速开始)
  - [下载安装](#下载安装)
  - [自托管（Docker）](#自托管docker)
  - [开发者](#开发者)
- [运行模式](#运行模式)
- [生态一览](#生态一览)
- [项目结构](#项目结构)
- [文档](#文档)
- [贡献](#贡献)
- [社区](#社区)
- [致谢](#致谢)
- [Star History](#star-history)
- [License](#license)

## 核心能力

<table>
<tr><td width="220"><b>🖥️ 开箱即用的桌面 Agent</b></td><td>原生 GUI、丰富的 Provider 模板与预设模型，并支持本地模型一键安装。填入 API Key 或登录账号即可开始，不要求先搭环境、学命令行或维护复杂配置。</td></tr>
<tr><td><b>🧭 持续完成，而不只回答</b></td><td>Goal 定义结果，Workflow 动态组织执行，Loop 决定何时继续。复杂任务可以在后台推进，并随时查看、暂停、恢复或调整。</td></tr>
<tr><td><b>🧠 长期记忆与知识空间</b></td><td>跨会话记忆、按需召回、离线整理、用户画像与可复用技能逐步积累；知识空间让你和 AI 共同读写真实 Markdown，形成可追溯的第二大脑。</td></tr>
<tr><td><b>🎨 从想法到可交付设计</b></td><td>设计空间从一句话或参考图生成网页、移动原型、演示文稿等 10+ 类产物，支持可视化微调、版本管理、多格式导出，并能继续交付到真实代码工程。</td></tr>
<tr><td><b>🛠️ 真正能够操作和执行</b></td><td>在授权与审批下控制电脑和浏览器，运行命令、读写文件、调用 MCP、连接工作空间，并通过多 Agent 协作完成复杂工作。</td></tr>
<tr><td><b>🌐 跨端交接，随时在线</b></td><td>Desktop / Server / Web / ACP 共用同一套核心；同一份会话、记忆与任务状态可以在桌面、浏览器和常用 IM 渠道之间继续。</td></tr>
<tr><td><b>🛡️ 本地优先、可控可靠</b></td><td>数据默认保存在本机，模型请求直连 Provider；工具审批、Docker 沙箱、配置回滚、崩溃恢复与后台保活共同守住长期运行边界。</td></tr>
</table>

## 能力全景

### 🎨 设计与知识

<table>
<tr><td width="220"><b>🎨 设计空间</b></td><td>从一句话或参考图生成网页、移动原型、演示文稿、仪表盘、海报、文档、邮件、图像、动效、音频与交互组件。生成过程实时预览，项目内置 AI 对话、元素微调、批注、撤销重做、设备预览、版本历史与产物库，可导出 HTML / PNG / PDF / PPTX / MP4 / ZIP。</td></tr>
<tr><td><b>🧩 设计系统到代码</b></td><td>可从截图、网址、Figma 或现有代码仓库提取品牌设计系统，跨产物复用并导出多平台 Design Token 与代码交付包。设计项目可绑定真实仓库，把选定产物交给主对话实现到代码，并在代码变化后提示回灌设计稿。</td></tr>
<tr><td><b>🧠 知识空间 · 第二大脑</b></td><td>AI 与你共同读写真实 Markdown 笔记，支持资料归档、全文与向量检索、双链、图谱、原子笔记和可审阅的 AI 整理建议。可绑定现有 Obsidian 库，外部改动实时同步；来源与 Evidence 保留回溯。</td></tr>
</table>

### 🧭 长期任务与自主执行

<table>
<tr><td width="220"><b>🎯 Goal · 朝结果持续推进</b></td><td>给出最终目标和完成标准后，Agent 会持续拆解、执行、检查和推进。目标支持预算、进度、暂停与恢复；完成前经过保守审计并展示结果证据。</td></tr>
<tr><td><b>🧩 Workflow · 动态编排</b></td><td>模型按任务需要组织阶段、条件、并行、多 Agent、工具、Diff、Review 与验证。每次执行都有持久记录，可暂停、恢复、取消，并在异常退出后保守恢复。</td></tr>
<tr><td><b>🔁 Loop · 按时间或事件继续</b></td><td>支持固定间隔、条件、内部事件和模型自定唤醒时间；每轮既可继续当前会话，也可触发绑定 Goal 的 Workflow，并带预算、退避和无进展保护。</td></tr>
<tr><td><b>📋 Plan、Task 与后台任务</b></td><td>复杂工作可以先形成可修改的计划，再用 Task 展示实时进度。耗时工具和子 Agent 可在后台运行，阶段结果逐步回到主对话，不必阻塞继续交流。</td></tr>
</table>

> 心智模型：<b>Goal</b> 定义最终结果，<b>Workflow</b> 负责一次具体执行，<b>Loop</b> 决定何时再次推进，<b>Task</b> 呈现当前进度，<b>Mode</b> 控制自主执行强度。它们可以组合，也可以独立使用。

### 🧠 记忆与成长

<table>
<tr><td width="220"><b>🧠 跨会话持久记忆</b></td><td>记忆按全局、项目与 Agent 分层组织，精简 Core 稳定进入上下文，详细内容通过全文与向量检索按需取回，避免每轮重复塞入全部历史。</td></tr>
<tr><td><b>🔍 召回、整理与反省</b></td><td>模型可按任务主动召回记忆；用户也可开启 Fast / Deep Recall。空闲时可整理重要内容、生成 Dream Diary，并从历史中提炼可审阅的沟通风格、工作习惯和长期偏好。</td></tr>
<tr><td><b>🛠 会成长的技能系统</b></td><td>复杂任务完成后可以沉淀为技能草稿，经你审核后复用。技能支持条件激活、子 Agent 执行与工具白名单，并兼容 <a href="https://agentskills.io">agentskills.io</a> 标准。</td></tr>
<tr><td><b>💾 长对话与无痕模式</b></td><td>渐进式上下文压缩保留长对话中的关键事实与工具调用关系；无痕会话则关闭长期记忆、跨会话感知与持久化旁路，结束后不保留会话数据。</td></tr>
</table>

### 🛠 工具与连接

<table>
<tr><td width="220"><b>🖱️ 电脑与浏览器控制</b></td><td>在 macOS 授权后观察并操作桌面、窗口、菜单、键盘与鼠标；可控浏览器提供实时镜像，让你看到 Agent 正在访问和操作的页面。副作用动作统一经过审批。</td></tr>
<tr><td><b>👥 多 Agent 与自然语言定时</b></td><td>通过预设团队或动态子 Agent 并行协作，结果自动汇总回主对话；也可以用自然语言创建定时任务，并把结果投递到指定 IM 渠道。</td></tr>
<tr><td><b>📁 Project 项目容器</b></td><td>把相关会话、项目指令、记忆与共享文件组织在一起。上传文件自动提取文本，并按内容规模选择目录、内联或按需读取，控制上下文占用。</td></tr>
<tr><td><b>🔌 MCP 与 Hooks</b></td><td>内置 MCP 客户端，覆盖主流 transport 与 OAuth 2.1；Hooks 可在 20+ 生命周期事件上接入 command / HTTP / MCP / prompt / agent 处理器，并支持分层配置与热重载。</td></tr>
<tr><td><b>🔧 工具箱与工作空间</b></td><td>内置 AI 画图、语音与音乐音效生成、Web 搜索、bash、文件操作、Canvas、URL 预览与自诊断；飞书工作空间提供 40+ 工具，覆盖文档、多维表格、云盘、知识库、审批、日历、联系人和招聘。</td></tr>
<tr><td><b>📊 Dashboard + Recap</b></td><td>统一查看成本、Token、活跃度、健康度、Plan 与长期任务状态；Recap 可复盘一段时间内的会话，生成多章节报告并导出独立 HTML。</td></tr>
</table>

### 🌐 桌面、服务与跨端

<table>
<tr><td width="220"><b>🖥️ 原生 GUI 与模型配置</b></td><td>macOS 提供完整桌面体验，Linux / Windows 当前为实验性支持；界面支持多语言。内置主流 Provider 模板与丰富的预设模型，同一 Provider 可配置多把 API Key 自动轮换。</td></tr>
<tr><td><b>🦙 本地模型一键安装</b></td><td>不需要账号、API Key 或终端，在设置中选择适合硬件的模型，即可完成 Ollama 安装、模型下载、Provider 注册与切换；本地 Embedding 使用同一套流程。</td></tr>
<tr><td><b>🤝 IM 渠道与会话交接</b></td><td>接入 Telegram、Discord、Slack、飞书等常用 IM 渠道，图片、语音和文件可直接进入多模态上下文。会话可在桌面、浏览器与 IM 之间接管或交接，运行中的回复也能流式镜像到聊天工具。</td></tr>
<tr><td><b>🌐 独立服务与多种运行形态</b></td><td><code>hope-agent server</code> 可常驻 NAS、家用服务器或云主机，并内嵌完整 Web GUI；<code>hope-agent acp</code> 可作为 IDE 的 Agent 后端。不同入口共享同一套核心、会话、记忆与配置。</td></tr>
</table>

### 🛡 安全与可靠性

<table>
<tr><td width="220"><b>🔒 工具审批 + Docker 沙箱</b></td><td>敏感工具调用进入统一审批，高风险命令和文件写入可选择在 Docker 沙箱中执行，降低高权限误操作的影响范围。</td></tr>
<tr><td><b>🏠 本地优先 · 零第三方中转</b></td><td>配置、会话、记忆、附件、技能与日志默认保存在 <code>~/.hope-agent/</code>，API Key 直连模型 Provider。服务模式提供 Bearer Token 鉴权与 SSRF 防护策略。</td></tr>
<tr><td><b>🛟 回滚、自愈与保活</b></td><td>配置变更自动保存本地快照，可一键回滚；Guardian、系统服务与子系统 watchdog 负责异常重启、诊断和自动重连，让长期任务在故障后以可观察的方式恢复。</td></tr>
</table>

> 更完整的版本变化请查看 [CHANGELOG.md](CHANGELOG.md)，实现细节见 [docs/architecture/](docs/architecture/)。

## 快速开始

### 下载安装

> 📦 各平台完整安装包列表：[Releases](https://github.com/shiwenwen/hope-agent/releases)

#### macOS

##### Homebrew（推荐）

```bash
brew tap shiwenwen/hope-agent
brew install --cask hope-agent
```

> 已经手动装过 `Hope Agent.app`？在 `brew install` 后面加 `--adopt`（接管同版本现有应用，不重新下载）或 `--force`（强制重下覆盖）。

##### 手动安装（DMG）

到 [Releases](https://github.com/shiwenwen/hope-agent/releases) 下载 `Hope.Agent_*.dmg`，拖到「应用程序」即可。

> 若启动时提示"已损坏"或"无法验证开发者"，请在终端执行：
>
> ```bash
> sudo xattr -cr /Applications/Hope\ Agent.app
> sudo codesign --force --deep --sign - /Applications/Hope\ Agent.app
> ```

Apple Silicon 与 Intel Mac 均提供原生构建（arm64 / x64 DMG），Homebrew 与手动下载都会按你的硬件自动选对版本。

##### 启动方式

- **桌面 GUI**：Launchpad / 应用程序文件夹（点 Hope Agent 图标），或终端 `open -a "Hope Agent"` / `hope-agent`
- **浏览器 Web GUI**：打开桌面应用后访问 <http://127.0.0.1:8420>；也可以运行 `hope-agent server start`，只启动服务、不打开桌面窗口
- **ACP（IDE 集成）**：`hope-agent acp`

#### Windows

##### Scoop（推荐）

```powershell
scoop bucket add hope-agent https://github.com/shiwenwen/scoop-hope-agent
scoop install hope-agent
```

##### 手动安装（installer）

到 [Releases](https://github.com/shiwenwen/hope-agent/releases) 下载 `Hope.Agent_*-setup.exe` 双击安装。**Windows 端尚未完成充分测试**，欢迎反馈问题。

> 若启动时提示"由于找不到 MSVCP140_1.dll，无法继续执行代码"或类似缺失 `VCRUNTIME140.dll` / `MSVCP140.dll`，请安装 [Microsoft Visual C++ 2015–2022 运行库（x64）](https://aka.ms/vs/17/release/vc_redist.x64.exe)后重启应用。

当前仅 x64。

##### 启动方式

- **桌面 GUI**：Start 菜单点「Hope Agent」启动，或 PowerShell `hope-agent`
- **浏览器 Web GUI**：打开桌面应用后访问 <http://127.0.0.1:8420>；也可以在 PowerShell / cmd 运行 `hope-agent server start`，只启动服务、不打开桌面窗口
- **ACP（IDE 集成）**：`hope-agent acp`

#### Linux

##### Arch Linux / Manjaro（AUR）

```bash
yay -S hope-agent-bin   # 或 paru / 任意 AUR helper
```

预编译二进制版（沿用 GitHub Release 的 `.deb`），不从源码编译。

##### Debian / Ubuntu（apt）

```bash
curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/pubkey.gpg | \
  sudo gpg --dearmor -o /usr/share/keyrings/hope-agent.gpg
echo "deb [signed-by=/usr/share/keyrings/hope-agent.gpg] https://shiwenwen.github.io/hope-agent-linux-repo/apt stable main" | \
  sudo tee /etc/apt/sources.list.d/hope-agent.list
sudo apt update
sudo apt install hope-agent
```

##### Fedora / RHEL / CentOS（dnf / yum）

```bash
sudo curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/rpm/hope-agent.repo \
  -o /etc/yum.repos.d/hope-agent.repo
sudo dnf install hope-agent     # 或 sudo yum install hope-agent
```

> 历史命令 `sudo dnf config-manager --add-repo …` 在 dnf5（Fedora 41+）已经废弃，用上面的 `curl` 写法对 dnf4 / dnf5 / yum / zypper 都兼容。

openSUSE 用户：

```bash
sudo zypper addrepo https://shiwenwen.github.io/hope-agent-linux-repo/rpm/hope-agent.repo
sudo zypper install hope-agent
```

##### 手动安装（AppImage / deb / rpm）

到 [Releases](https://github.com/shiwenwen/hope-agent/releases) 下载（包名含架构后缀，按你的机器选 `_amd64` / `_arm64` 或 `.x86_64` / `.aarch64`）：

- AppImage：`Hope.Agent_*.AppImage` —— `chmod +x` 后直接运行
- Debian / Ubuntu：`Hope.Agent_*.deb` —— `sudo dpkg -i Hope.Agent_*.deb`
- Fedora / RHEL：`Hope.Agent_*.rpm` —— `sudo rpm -i Hope.Agent_*.rpm`

提供 amd64 (x86_64) 与 arm64 (aarch64) 两种原生构建，覆盖普通 PC、树莓派 4/5、Apple Silicon 跑 Asahi Linux、Graviton / Ampere 云主机。apt 与 dnf 都会按 `dpkg --print-architecture` / `$basearch` 自动选对版本。

##### 启动方式

- **桌面 GUI**：应用菜单点「Hope Agent」启动，或终端 `hope-agent`
- **浏览器 Web GUI**：打开桌面应用后访问 <http://127.0.0.1:8420>；也可以运行 `hope-agent server start`，只启动服务、不打开桌面窗口
- **ACP（IDE 集成）**：`hope-agent acp`

#### 首次启动 & 自动更新

1. 首次启动向导：**选 Provider 模板 → 填 API Key / Codex OAuth 登录 → 开聊**
2. 桌面应用内置 GitHub Releases 自动更新，应用内 **设置 → 关于** 检查更新并一键安装；或者直接在对话里说「升级」或「检查更新」
3. 通过 Homebrew / AUR / Scoop 装的版本同样走应用内置 updater；包管理器视角的版本号会保持初装时的，不影响功能

> 要从手机或另一台电脑访问，在「设置 → 服务器」中设置 API Key，并把监听地址改为 `0.0.0.0:8420`；重启后访问 `http://<运行 Hope Agent 的设备 IP>:8420`。不要在没有鉴权的情况下把端口暴露到局域网或公网；公网使用请前置 HTTPS 反向代理，详见 [Docker 部署指南](docs/deployment/docker.md)。

### 自托管（Docker）

把 Hope Agent 跑在家用 NAS / VPS / homelab 上、用浏览器访问 Web GUI 的场景：

```bash
docker run -d \
  --name hope-agent \
  -p 127.0.0.1:8420:8420 \
  -v hope-data:/data \
  ghcr.io/shiwenwen/hope-agent:latest
```

容器跑起来后浏览器打开 <http://127.0.0.1:8420>，按 Onboarding 向导配 Provider API Key。镜像覆盖 `linux/amd64` + `linux/arm64`（含 Apple Silicon / 树莓派），随每次 Release Tag 自动构建。

要用 docker compose / 配合 Ollama 本地 LLM / 暴露到 LAN / 反向代理与 TLS / 升级流程，见 [`docs/deployment/docker.md`](docs/deployment/docker.md)。

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
    <b>40+ 个模板 · 300+ 个预设模型</b><br/>
    <b>国际</b> · Anthropic · OpenAI · Codex · GitHub Copilot · Google Gemini · OpenRouter · Azure OpenAI · Groq · Together AI · Fireworks · Novita · Perplexity · xAI Grok · Mistral · Cohere<br/>
    <b>国内</b> · DeepSeek · Moonshot (Kimi) · 通义千问 (Qwen) · 豆包 (火山引擎) · 智谱 GLM · MiniMax · 小米 MiMo<br/>
    <b>本地</b> · Ollama · 任意 OpenAI 兼容端点
  </td>
</tr>
<tr>
  <td><b>💬 IM 渠道</b></td>
  <td><b>10+ 个</b> · Telegram · Discord · Slack · 飞书 · Google Chat · LINE · QQ Bot · Signal · iMessage · IRC · WeChat · WhatsApp</td>
</tr>
<tr>
  <td><b>🌐 界面语言</b></td>
  <td><b>10+ 种</b> · 简体中文 · 繁體中文 · English · 日本語 · 한국어 · Español · Português · Русский · العربية · Türkçe · Tiếng Việt · Bahasa Melayu</td>
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

- 📖 **[使用手册](docs/user-guide/README.md)** —— 面向用户的完整中文文档:安装上手、每项功能的用法与设置([English](docs/user-guide/en/README.md))
- 🏗️ [技术文档](docs/) —— 架构设计与实现细节(面向开发者)

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

- [Ollama](https://ollama.com/)：本地大模型一键安装能力建立在 Ollama 的本地运行时与 OpenAI 兼容端点之上；Hope Agent 仅作 GUI 层包装，Qwen / Gemma 等模型由 Ollama 模型库分发
- [ClawHub](https://www.clawhub.com/) / [SkillHub](https://skillhub.cn/)：为 Hope Agent 提供公开的 skill 搜索与发现来源
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
