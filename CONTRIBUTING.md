# 贡献指南

> [English](CONTRIBUTING.en.md) · 简体中文

欢迎贡献 Hope Agent！本文档面向**首次贡献者和日常贡献者**——介绍如何报 Bug、提 PR、参与翻译 / 技能 / Provider / Channel 等常见方向。

如果你是**资深 maintainer 或 AI 编码助手**（Claude Code / Codex / Cursor），项目跨 PR 的契约面在 [AGENTS.md](AGENTS.md)，子系统设计在 [`docs/architecture/`](docs/architecture/)。本文档只覆盖流程，不重复那些。

## 行为准则

参与本项目即同意遵守 [Contributor Covenant](https://www.contributor-covenant.org/zh-cn/version/2/1/code-of-conduct/) 行为准则。简言之：**专业讨论、互相尊重、对事不对人**。

## 你想做什么？

| 你想 | 路径 |
|---|---|
| 报告 Bug | [新建 issue](https://github.com/shiwenwen/hope-agent/issues/new/choose) → Bug report 模板 |
| 报告**安全漏洞** | **请勿公开 issue**，走 [SECURITY.md](SECURITY.md) 私密渠道 |
| 提议新功能 | 先开 [discussion](https://github.com/shiwenwen/hope-agent/discussions) 聊清楚再开 issue |
| 改 Bug / 加功能 | fork → branch → PR（详见下方"提 PR 流程"） |
| 帮做翻译 | 见下方"翻译贡献" |
| 加新 Skill / Provider / Channel | 见下方"插件式贡献" |
| 改文档 / 修 typo | 直接 PR，无需开 issue |

## 提 PR 流程

### 1. fork & 起分支

```bash
# fork 后 clone 你自己的 fork
git clone git@github.com:<你的账号>/hope-agent.git
cd hope-agent
git remote add upstream git@github.com:shiwenwen/hope-agent.git

# 起短期分支（不要在你 fork 的 main 上直接改）
git checkout -b feat/xxx   # 或 fix/xxx, docs/xxx
```

### 2. 装环境 + 跑起来

```bash
pnpm install              # 装前端依赖 + Husky pre-push 钩子
pnpm tauri dev            # 启动桌面开发模式（前端 + Rust 后端 + 热重载）
```

详细命令清单见 [AGENTS.md "开发命令"](AGENTS.md#开发命令)。

### 3. 改代码

注意以下契约（详见 AGENTS.md）：

- 核心业务逻辑必须在 `crates/ha-core/`（**零 Tauri 依赖**），`src-tauri/` 和 `crates/ha-server/` 只做适配薄壳
- 前端用 React 19 + TypeScript + Tailwind v4 + shadcn/ui
- 文件路径别名 `@/` → `src/`
- 禁止 `console.log` / `log::info!` 等原生日志，必须用 [`app_info!`](crates/ha-core/src/logging.rs) 系列宏
- 跨平台分支优先 `#[cfg(unix)]` / `#[cfg(windows)]`，新原语放 [`crates/ha-core/src/platform/`](crates/ha-core/src/platform/)

### 4. 提交前自检（强制）

`git push` 之前，[`.husky/pre-push`](.husky/pre-push) 钩子会自动跑这六条：

```bash
cargo fmt --all --check
cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings
cargo test  -p ha-core -p ha-server --locked
pnpm typecheck
pnpm lint
pnpm test
```

任何一条失败 push 都被拒。**不要用 `--no-verify` 绕过**——CI 会再跑一遍同样的检查并阻塞 PR。

### 5. Commit message 规范

跟着仓库现有风格（[`git log`](https://github.com/shiwenwen/hope-agent/commits/main) 看最近 20 条）：

```
<type>(<scope>): <一句话描述>

<可选的详细说明，每行 < 80 字>
```

- `type`：`feat` / `fix` / `docs` / `ci` / `chore` / `refactor` / `perf` / `test`
- `scope`：子系统名（`chat` / `provider` / `channel` / `mcp` / `skill` / `plan` / `cron` ...）
- 中英文皆可，**优先中文**（仓库主语言）

✅ `feat(provider): 升级内置模型模板的 provider/model 列表`
✅ `ci(release): 修 Linux/Windows release 构建 + 轮换 updater 公钥`
❌ `update code` / `fix bug`

**DCO**：每个 commit 加 `Signed-off-by:`（用 `git commit -s`）。这是项目的[原创性声明要求](https://developercertificate.org/)。

### 6. PR 描述

PR 模板会引导你填写。重点：

- 关联的 issue（`closes #xxx`）
- 改动概要
- 测试方式（手动验证 / 单测覆盖）
- 涉及的子系统是否需要更新 [`docs/architecture/<name>.md`](docs/architecture/)
- CHANGELOG `Unreleased` 段是否更新（详见下方）

### 7. CI 通过 + Review

- CI 必须全绿才能 merge（[`lint.yml`](.github/workflows/lint.yml) + [`rust.yml`](.github/workflows/rust.yml)）
- 关键路径（`.github/`、`tauri.conf.json`、`crates/ha-core/src/security/`、`docs/architecture/`）由 [CODEOWNERS](.github/CODEOWNERS) 强制 maintainer review
- 一般路径单一非 maintainer 也可 review，但 merge 仍由 maintainer 执行

### 8. Squash merge

我们用 squash merge 保持 main 线性。你的多个 commit 会合并成一个。所以**单个 PR 聚焦一件事**，太大的改动拆多个 PR。

## 翻译贡献

Hope Agent 支持 12 种语言（zh、en、ja、ko、de、fr、es、pt-BR、ru、it、tr、vi）。`zh` 和 `en` 是真相源，其他 10 种从这两个补齐。

```bash
node scripts/sync-i18n.mjs --check   # 检查缺失翻译
node scripts/sync-i18n.mjs --apply   # 从模板补齐
```

翻译文件在 [`src/i18n/locales/`](src/i18n/locales/)。提 PR 时请：

- 一个 PR 只覆盖一个或几个相关语言
- key 路径必须和 `zh` 完全一致
- 测试方式：`pnpm tauri dev` 在 UI 切到目标语言验证

## 插件式贡献（Skill / Provider / Channel）

这三类是社区最容易贡献的方向，模板成熟。

### 加新 Skill

[`skills/`](skills/) 下每个目录是一个技能，含 `SKILL.md` + 可选附属脚本/资源。详见 [`skills/skill-creator/SKILL.md`](skills/skill-creator/SKILL.md)。

第三方 skill 的 vendor 流程：在 [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) 登记原始来源 + license 全文。

### 加新 LLM Provider

新 provider 接入 [`crates/ha-core/src/provider/`](crates/ha-core/src/provider/)。当前支持 4 种 ApiType（Anthropic / OpenAIChat / OpenAIResponses / Codex）。如果新 provider 用的是这 4 种之一的协议，多数情况只需在内置模板添加配置，无需 Rust 代码。如果是新协议，参考已有 provider 实现 trait。

### 加新 IM Channel

[`crates/ha-core/src/channel/`](crates/ha-core/src/channel/) 下已有 12 个 channel（Telegram、Slack、飞书、企业微信...）作为参考。每个 channel 实现 `ChannelPlugin` trait + 一组事件回调。最简单的是 webhook-based channel（参考 LINE / Discord）。

## CHANGELOG 维护

每个用户可见的改动（feat / fix / breaking）必须在 [`CHANGELOG.md`](CHANGELOG.md) 的 `## Unreleased` 段加一行。发版时一次性切到 `## vX.Y.Z`。

格式：

```markdown
## Unreleased

### Added
- chat: 新增 XXX 功能（#PR_NUMBER）

### Fixed
- channel: 修复 telegram bot Y 问题（#PR_NUMBER）
```

纯 chore / refactor / 内部 docs 改动可以不进 CHANGELOG。

## 文档维护红线

如果你的 PR 涉及以下情况，**同一个 PR 内必须同步改对应文档**（详见 [AGENTS.md "文档维护"](AGENTS.md#文档维护) 表）：

- 新增 / 删除功能、命令、模块 → `CHANGELOG.md`
- 子系统架构变化 → `docs/architecture/<name>.md`
- 新增架构级能力 → `docs/architecture/` 新建 + `docs/README.md` 索引
- 改 Tauri 命令 / HTTP 路由 → [`docs/architecture/api-reference.md`](docs/architecture/api-reference.md)
- 修 README 任一语言 → 同 PR 同步另一语言（`README.md` ↔ `README.en.md`）
- 修 Release Notes → 同 PR 内中英双份

## 给资深 contributor / AI 助手

如果你打算改的是跨 PR 涉及契约的东西（Provider / Permission / Plan Mode / Channel 流式 / 上下文压缩 / Memory 优先级 ...），**先读 [AGENTS.md](AGENTS.md) 整篇**——它列出了 30+ 个跨 PR 必守的红线。同时读对应的 [`docs/architecture/<name>.md`](docs/architecture/)。

## 反馈与讨论

- Bug / 功能请求：[Issues](https://github.com/shiwenwen/hope-agent/issues)
- 设计讨论 / 用法提问：[Discussions](https://github.com/shiwenwen/hope-agent/discussions)
- 安全漏洞：见 [SECURITY.md](SECURITY.md)（**勿在公开 issue 发**）

感谢你对 Hope Agent 的贡献！🎉
