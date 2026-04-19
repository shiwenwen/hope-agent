# 项目重命名清单：OpenComputer → Hope Agent

> **Status: DONE（2026-04-19，随 0.1.0 首发一把落地）**。本清单作为改名当时的执行基线保留原文 —— 里面的 file:line 引用和旧名字是改名前的现状描述，不再对齐当前代码。
>
> 目标：把项目品牌从 `OpenComputer` 全量替换为 `Hope Agent`。应用尚未发布，**不考虑向后兼容**（老 `~/.opencomputer/` 数据目录直接抛弃、launchd/systemd 老 label 不保留、所有配置默认值直接换新名）。
>
> 本文件只列清单，不含代码改动。落地时按「执行顺序」分 3 个 commit 做。
>
> 扫描基线（2026-04-18，分支 `main`，不含本清单自身的自引用）：
>
> - `OpenComputer` / `opencomputer` 字面量：**525 次 / 130 文件**
> - `oc-core` / `oc-server` / `oc_core` / `oc_server`：**1032 次 / 105 文件**（其中 `Cargo.lock` 5 处，随 `cargo build` 自动重写）
> - `~/.opencomputer` 数据目录路径：**257 次 / 74 文件**（其中 [`crates/oc-core/src/paths.rs`](../../crates/oc-core/src/paths.rs) 单文件 44 处）
> - `OPENCOMPUTER_*` 环境变量：**4 个 env 名，15 处引用**
> - `com.opencomputer.*` bundle id / service label：**4 处**（tauri.conf.json × 1、service_install.rs × 1、docs/architecture/backend-separation.md × 2）

---

## 0. 命名映射（改动前先在此锁定）

| 维度 | 旧 | 新 |
|---|---|---|
| 品牌名 | `OpenComputer` | `Hope Agent` |
| 工程名 / CLI | `opencomputer` | `hope-agent` |
| 简写（保留） | `oc` | `ha` / `HA` |
| Cargo crate | `oc-core` / `oc-server` | `ha-core` / `ha-server` |
| Rust 模块路径（`-` → `_`） | `oc_core` / `oc_server` | `ha_core` / `ha_server` |
| src-tauri crate name | `open-computer` | `hope-agent` |
| 二进制产物 | `opencomputer` | `hope-agent`（主）+ `hope`（symlink 短别名） |
| 数据目录 | `~/.opencomputer/` | `~/.hope-agent/` |
| 环境变量前缀 | `OPENCOMPUTER_` | `HOPE_AGENT_` |
| Bundle identifier | `com.opencomputer.app` | `com.hopeagent.app` |
| Service label | `com.opencomputer.server` | `com.hopeagent.server` |
| Skills 目录 | `skills/oc-settings` `skills/oc-skill-creator` | `skills/ha-settings` `skills/ha-skill-creator` |

> **决定记录**：
>
> 1. CLI 主二进制名采用 `hope-agent`（带连字符，符合 Unix 工具惯例），同时打 `hope` symlink 作为短别名（Homebrew 安装时由 formula `bin.install_symlink` 创建，`.dmg` / `.pkg` 里由 postinstall 脚本创建，Linux 包同理）。两种入口指向同一个二进制，内部通过 `argv[0]` 无需区分。`ha` 只作为 Rust crate 名前缀和文档简写，不做 CLI 入口。
> 2. Rust crate 名采用 `ha-core` / `ha-server`，Cargo 允许且不与 workspace 内路径依赖冲突。crate 不发布 crates.io，无名字冲突风险。
> 3. macOS bundle id 采用 `com.hopeagent.app`（单词合并），与现有 `com.opencomputer.app` 模式一致；不考虑 `com.hope-agent.app`，bundle id 不允许连字符。

---

## 1. Rust Workspace 重命名（🔴 必须第一步，影响所有后续编译）

### 1.1 目录 & crate 名

- `crates/oc-core/` → `crates/ha-core/`
- `crates/oc-server/` → `crates/ha-server/`
- 根 [`Cargo.toml`](../../Cargo.toml) workspace `members` 路径同步
- [`crates/oc-core/Cargo.toml`](../../crates/oc-core/Cargo.toml) `[package] name = "oc-core"` → `"ha-core"`
- [`crates/oc-server/Cargo.toml`](../../crates/oc-server/Cargo.toml) 同上 + 对 `oc-core` 的 path 依赖改为 `ha-core`
- [`src-tauri/Cargo.toml`](../../src-tauri/Cargo.toml):
  - `[package] name = "open-computer"` → `"hope-agent"`
  - `description`、`repository` 字段同步
  - `[dependencies]` 里 `oc-core = { path = "../crates/oc-core" }` / `oc-server = ...` 改名 + 改路径

### 1.2 全仓导入替换

- `use oc_core::` → `use ha_core::`（Rust 模块路径把 `-` 映射为 `_`）
- `use oc_server::` → `use ha_server::`
- 宏里引用（`crate::` 路径在 crate 内部不变，跨 crate 的 `oc_core::xxx` 路径要改）

### 1.3 验证

- `cargo check --workspace`
- `cargo build --workspace`
- `Cargo.lock` 自动重生成，不手改

---

## 2. CLI / 二进制 / Tauri Bundle

### 2.1 CLI usage 文案

[`src-tauri/src/main.rs`](../../src-tauri/src/main.rs) 12 处硬编码：

- `opencomputer acp`（L14 / L135 / L141 / L159）
- `opencomputer server`（L20 / L233 / L235 / L252 / L359 / L372 / L375 / L377）
- `"OpenComputer ACP Server"` / `"OpenComputer HTTP/WebSocket Server"` 启动横幅
- `"opencomputer-acp"` / `"opencomputer-server"` version 输出

### 2.2 Tauri 配置

[`src-tauri/tauri.conf.json`](../../src-tauri/tauri.conf.json):

- `"productName": "OpenComputer"` → `"Hope Agent"`
- `"identifier": "com.opencomputer.app"` → `"com.hopeagent.app"`
- 检查是否需要 `mainBinaryName` 字段（当前缺，默认取 Cargo package name）

[`src-tauri/Info.plist`](../../src-tauri/Info.plist):

- L6 / L8 `NSLocationUsageDescription` 里 `"OpenComputer needs your location..."`

[`src-tauri/capabilities/`](../../src-tauri/capabilities/) 下的 JSON：2026-04-18 main 扫描无命中，跳过（未来如新增 capability 文件再核查）

### 2.3 图标（可选）

`src-tauri/icons/` 目录只要文件名不变可以保留，品牌视觉资产是否重做由设计决定，不阻塞本次改名。

---

## 3. 数据路径 & 环境变量

### 3.1 数据目录

[`crates/oc-core/src/paths.rs`](../../crates/oc-core/src/paths.rs) 44 处 `.opencomputer` 字面量，全部统一改 `.hope-agent`。这是**所有数据目录的单一真源**，改完后：

```bash
~/.opencomputer/ → ~/.hope-agent/
├── config.json
├── user.json
├── session.db
├── memory.db
├── async_jobs.db
├── recap/recap.db
├── credentials/auth.json
├── channels/
├── agents/
├── projects/
├── attachments/
├── tool_results/
├── async_jobs/
├── backups/
├── logs/
└── skills/（用户级）
```

全仓其他 75 个文件里命中 `.opencomputer` 的，多数是**注释或日志文本**里引用路径，需要 grep 后逐个确认是硬编码路径还是文案说明。优先级：

- **硬编码路径（🔴）**：test/fixture/migration 里拼接路径字符串
- **日志文案（🟡）**：`app_info!("opened ~/.opencomputer/...")` 之类，路径已由 `paths.rs` 统一返回，日志里的字符串要同步改
- **文档/注释（ℹ️）**：放到 §7 文档步骤

### 3.2 环境变量（4 个）

| 旧 | 新 | 引用点 |
|---|---|---|
| `OPENCOMPUTER_CHILD` | `HOPE_AGENT_CHILD` | [src-tauri/src/main.rs:27](../../src-tauri/src/main.rs#L27) [src-tauri/src/main.rs:66](../../src-tauri/src/main.rs#L66) |
| `OPENCOMPUTER_RECOVERED` | `HOPE_AGENT_RECOVERED` | [crates/oc-core/src/guardian.rs:139](../../crates/oc-core/src/guardian.rs#L139) [src-tauri/src/commands/crash.rs:7](../../src-tauri/src/commands/crash.rs#L7) [crates/oc-server/src/routes/crash.rs:10](../../crates/oc-server/src/routes/crash.rs#L10) |
| `OPENCOMPUTER_CRASH_COUNT` | `HOPE_AGENT_CRASH_COUNT` | [crates/oc-core/src/guardian.rs:140](../../crates/oc-core/src/guardian.rs#L140) [src-tauri/src/commands/crash.rs:8](../../src-tauri/src/commands/crash.rs#L8) [crates/oc-server/src/routes/crash.rs:11](../../crates/oc-server/src/routes/crash.rs#L11) |
| `OPENCOMPUTER_BUNDLED_SKILLS_DIR` | `HOPE_AGENT_BUNDLED_SKILLS_DIR` | [crates/oc-core/src/skills/discovery.rs:22](../../crates/oc-core/src/skills/discovery.rs#L22) |

### 3.3 系统服务 label

[`crates/oc-core/src/service_install.rs:4`](../../crates/oc-core/src/service_install.rs#L4):

- `SERVICE_LABEL: &str = "com.opencomputer.server"` → `"com.hopeagent.server"`
- launchd plist 文件名 `~/Library/LaunchAgents/com.opencomputer.server.plist` 随常量自动变化
- systemd unit 文件名 `~/.config/systemd/user/opencomputer-server.service`（如存在，检查 `service_install.rs` 里的 Linux 分支常量）

---

## 4. Skills 目录 & 工具名

### 4.1 目录改名

- `skills/oc-settings/` → `skills/ha-settings/`
- `skills/oc-skill-creator/` → `skills/ha-skill-creator/`

### 4.2 SKILL.md frontmatter

- [`skills/oc-settings/SKILL.md:2`](../../skills/oc-settings/SKILL.md#L2) `name: oc-settings` → `name: ha-settings`
- [`skills/oc-skill-creator/SKILL.md:2`](../../skills/oc-skill-creator/SKILL.md#L2) 同上
- SKILL.md 正文里 `OpenComputer` 文案 → `Hope Agent`（含 `oc-skill-creator` 自己的使用示例）

### 4.3 代码里的 skill 名字引用

搜 `"oc-settings"` / `"oc-skill-creator"` 作为字符串 literal 的引用：

- [`src/hooks/useTheme.ts:57`](../../src/hooks/useTheme.ts#L57)（注释）
- [`src/components/common/StarrySky.tsx:109`](../../src/components/common/StarrySky.tsx#L109)（注释）
- [`docs/architecture/skill-system.md`](../../docs/architecture/skill-system.md)
- [`skills/oc-skill-creator/scripts/init_skill.py:9`](../../skills/oc-skill-creator/scripts/init_skill.py#L9) 和 test 脚本
- [`docs/architecture/tool-system.md:261`](../../docs/architecture/tool-system.md#L261)

> **注意**：`system_prompt` 装配链在运行时从 SKILL.md frontmatter 读取 `name:` 字段，只要 frontmatter 改了，LLM 看到的就是新名字；不存在"老 agent.json 引用旧 skill 名"的迁移问题（未发布）。

---

## 5. 前端 / UI

### 5.1 根配置

- [`package.json:2`](../../package.json#L2) `"name": "opencomputer"` → `"hope-agent"`
- [`index.html`](../../index.html) `<title>` 和 meta
- [`package-lock.json`](../../package-lock.json) `npm install` 自动重生成

### 5.2 i18n 文件（12 语言）

`src/i18n/locales/` 下 12 个 JSON（ar / en / es / ja / ko / ms / pt / ru / tr / vi / zh / zh-TW），每个 5 处左右 `OpenComputer` 出现。

- **所有语言同步改**（CLAUDE.md 规定，修改已存在 key 时不能只改 zh/en 等 sync 补齐）
- 有 `OpenComputer` 作为**品牌名**出现的（如 "关于 OpenComputer"）→ `Hope Agent`
- 有 `opencomputer` 作为**路径**出现的（如数据目录说明）→ `hope-agent`

### 5.3 硬编码 UI 文案

- [`src/components/chat/ChatTitleBar.tsx:288`](../../src/components/chat/ChatTitleBar.tsx#L288) `🖥️ OpenComputer`
- [`src/components/settings/AboutPanel.tsx`](../../src/components/settings/AboutPanel.tsx)（1 处，About 面板）
- [`src/components/settings/skills-panel/SkillListView.tsx`](../../src/components/settings/skills-panel/SkillListView.tsx)（2 处）
- [`src/components/settings/provider-setup/TemplateGrid.tsx`](../../src/components/settings/provider-setup/TemplateGrid.tsx)（1 处）
- [`src/types/project.ts`](../../src/types/project.ts)（注释）
- [`src/components/chat/hooks/useNotificationListeners.ts`](../../src/components/chat/hooks/useNotificationListeners.ts)

### 5.4 托盘

[`src-tauri/src/tray.rs`](../../src-tauri/src/tray.rs) 13 处 `OpenComputer`：12 语言 "退出 OpenComputer" + L209 tooltip。

---

## 6. 后端杂项文案

跟 §3 数据目录耦合较浅、主要是文案的点：

- [`crates/oc-core/src/app_init.rs`](../../crates/oc-core/src/app_init.rs)
- [`crates/oc-core/src/self_diagnosis.rs`](../../crates/oc-core/src/self_diagnosis.rs)
- [`crates/oc-core/src/weather.rs`](../../crates/oc-core/src/weather.rs) `User-Agent` 或类似字段
- [`crates/oc-core/src/sandbox.rs`](../../crates/oc-core/src/sandbox.rs)
- [`crates/oc-core/src/oauth.rs`](../../crates/oc-core/src/oauth.rs) OAuth callback 里如含 `OpenComputer` 品牌名
- [`crates/oc-core/src/acp/*`](../../crates/oc-core/src/acp) ACP protocol 对外声明的 client/server name
- [`crates/oc-core/src/docker/*`](../../crates/oc-core/src/docker) Docker 容器命名前缀
- [`crates/oc-core/src/tools/browser/advanced.rs`](../../crates/oc-core/src/tools/browser/advanced.rs) 浏览器 UA
- [`crates/oc-core/src/tools/notification.rs`](../../crates/oc-core/src/tools/notification.rs) 系统通知 app name
- [`crates/oc-core/src/system_prompt/build.rs`](../../crates/oc-core/src/system_prompt/build.rs) 身份行 3 处 `"running in OpenComputer on {os} {arch}"`（openclaw / custom / structured 三种模式各一处；注意 `personality.mode == SoulMd` 装配路径也复用 structured 分支的身份行，改名后一并生效）
- [`crates/oc-core/src/system_prompt/constants.rs`](../../crates/oc-core/src/system_prompt/constants.rs) `APP_INTRO` 常量——系统提示词里注入的产品简介：`"OpenComputer is a local, open-source AI assistant with configurable model providers, tools, skills, and persistent memory."`，改名后同步替换品牌名
- [`crates/oc-core/src/system_prompt/sections.rs`](../../crates/oc-core/src/system_prompt/sections.rs)
- [`crates/oc-core/templates/agent.{lang}.md`](../../crates/oc-core/templates/) 12 个 agent 模板里的品牌名

⚠️ **特别检查**：`system_prompt` 和 `agent.*.md` 模板里如果写了"你是 OpenComputer 的助手"这种**模型侧身份**，改名后需要考虑是否让模型知道产品改名了，或者改成通用身份。

---

## 7. 文档

### 7.1 根级文档

- [`README.md`](../../README.md)（如存在，grep）
- [`CLAUDE.md`](../../CLAUDE.md)（实际内容在 AGENTS.md，通过 `@AGENTS.md` 包含）
- [`AGENTS.md`](../../AGENTS.md) 21 处 `OpenComputer` + 32 处 `oc-core`/`oc-server` + 12 处 `.opencomputer`
- [`CHANGELOG.md`](../../CHANGELOG.md) 45 处品牌名 + 51 处 crate 名 + 34 处路径。**历史条目保留原始文本**（record-of-truth），在顶部新加一条 `### Changed` 说明改名
- [`.gitignore`](../../.gitignore)

### 7.2 架构文档

[`docs/architecture/`](../../docs/architecture/) 全部 20+ 个 md 文件 grep 替换。高密度文件：
`backend-separation.md` / `skill-system.md` / `memory.md` / `plan-mode.md` / `provider-system.md` / `acp.md` / `tool-system.md`。

### 7.3 审计文档

- [`docs/audit/2026-04-17-codebase-audit.md`](2026-04-17-codebase-audit.md)：历史审计报告，里面的 `file:line` 引用会因 crate 目录改名而失效。**选项**：
  - (a) 保留原文 + 在顶部加一条 note 说明"路径中 `crates/oc-core/` 对应当前 `crates/ha-core/`"（推荐）
  - (b) 逐条改路径（工作量大且违反 record-of-truth 原则）
- 本文件 [`2026-04-18-rename-to-hope-agent.md`](2026-04-18-rename-to-hope-agent.md)：改名完成后在顶部加 `Status: DONE`

### 7.4 docs/README.md

[`docs/README.md`](../README.md) 文档索引，2 处品牌名 + 1 处路径。

---

## 8. 执行顺序（3 个 commit）

### Commit 1：Rust workspace 重命名

范围：§1 全部 + §2.1 CLI usage 文案。

- 目录 `git mv crates/oc-core → crates/ha-core`、`crates/oc-server → crates/ha-server`
- `Cargo.toml` × 4 改 name / dependency
- 全仓 `use oc_core::` → `use ha_core::`（`rg` + `sd` 批量，然后 `cargo check` 验证）
- 全仓 `use oc_server::` → `use ha_server::`
- `src-tauri/Cargo.toml` package name 改 + main.rs CLI 文案改
- **验证**：`cargo check --workspace` + `cargo build --workspace` 通过

### Commit 2：路径 / env / bundle / service / skills

范围：§2.2 §2.3 + §3 + §4 + §6。

- [`crates/ha-core/src/paths.rs`](../../crates/oc-core/src/paths.rs) 44 处 `.opencomputer` → `.hope-agent`
- 4 个环境变量全仓替换
- [`crates/ha-core/src/service_install.rs`](../../crates/oc-core/src/service_install.rs) service label 改
- [`tauri.conf.json`](../../src-tauri/tauri.conf.json) productName / identifier 改
- [`Info.plist`](../../src-tauri/Info.plist) 文案改
- `skills/oc-*` 目录改名 + SKILL.md frontmatter 改 + 引用字符串替换
- 后端杂项文案（§6）
- **验证**：`cargo check` + `npm run tauri dev` 启动一次，确认托盘显示、数据目录 `~/.hope-agent/` 创建、HTTP server 启动 label 正确、`opencomputer server install` → `status` → `uninstall` 走一遍（注意：此时命令名应该已经改成 `hope-agent server`）

### Commit 3：前端文案 + 文档

范围：§5 + §7。

- `package.json` name 改 + `npm install` 重跑
- `index.html` title
- 12 语言 i18n 文件同步改
- 所有前端组件硬编码文案
- `src-tauri/src/tray.rs` 12 语言翻译
- 所有文档（根级 md + `docs/architecture/` + `docs/README.md`）
- CHANGELOG 加一条改名记录
- 审计文档加 preamble note
- **验证**：`npx tsc --noEmit` + `npm run lint` + `node scripts/sync-i18n.mjs --check` + `npm run tauri dev` 全量 smoke test（About 面板 / 设置 / 托盘菜单各一遍）

---

## 9. 验证清单（总）

改完后全部通过才算完成：

- [ ] `cargo check --workspace` 无 warning 级增量
- [ ] `cargo build --workspace` 通过
- [ ] `npx tsc --noEmit` 通过
- [ ] `npm run lint` 通过
- [ ] `node scripts/sync-i18n.mjs --check` 12 语言齐全
- [ ] `npm run tauri dev` 正常启动，数据目录落在 `~/.hope-agent/`
- [ ] 托盘 tooltip / 退出菜单文案显示 `Hope Agent`
- [ ] `hope-agent server start` CLI 正常工作
- [ ] `hope-agent server install` 注册的 launchd label 为 `com.hopeagent.server`
- [ ] 桌面应用 bundle id `com.hopeagent.app`（macOS: `mdls -name kMDItemCFBundleIdentifier *.app`）
- [ ] 应用启动后 About 面板显示 `Hope Agent`
- [ ] 全仓 grep `OpenComputer|opencomputer|oc-core|oc-server|oc_core|oc_server|\.opencomputer|OPENCOMPUTER_|com\.opencomputer` 只剩历史 CHANGELOG / 审计文档里作为"历史记录"存在的条目

---

## 10. 代码外工作（Commit 3 之外）

改名不只是代码问题，以下是**不在 git 提交里**但同样影响改名完成度的工作，按「必须 / 建议 / 未来」三档排序。

### 10.1 必须做（会影响开发和发布）

#### GitHub 仓库

- [ ] **仓库重命名**：GitHub 仓库 settings → rename `OpenComputer` → `hope-agent`
  - GitHub 会自动保留旧名字的 HTTP redirect（`git@github.com:shiwenwen/OpenComputer.git` 仍可用）
  - 但**建议本地 remote URL 同步更新**：`git remote set-url origin git@github.com:shiwenwen/hope-agent.git`
  - 其他 contributor 也要同步改本地 remote
- [ ] **仓库 description**：GitHub 页面上那句介绍（目前可能是 `Personal AI Assistant...` 之类），改成新描述
- [ ] **仓库 topics**：GitHub 页面的标签（如有 `opencomputer` 这种 topic 就删掉）
- [ ] **仓库 Homepage URL**：如果填了 opencomputer.* 之类的域名要改
- [ ] **GitHub repo 的 About / Social Preview 图**：如果上传过自定义社交卡片图
- [ ] **Default branch protection rules**：规则本身不受影响，但如果规则里写了 `opencomputer` 相关的 required check 名字要同步
- [ ] **Open Issues / Open PRs**：标题或正文如有品牌名引用不用逐条改，保留历史

#### 本地工作树

- [ ] **本地目录**：`/Users/shiwenwen/Codes/OpenComputer/` 建议改为 `/Users/shiwenwen/Codes/hope-agent/`。注意：所有 IDE 工作区文件（`.vscode/settings.json` 如有）、Claude Code 的工作目录引用、AGENTS.md/CLAUDE.md 全局配置里的路径、`~/.claude/projects/` 下的会话记录路径都会失效。推荐做法：改完名后 `cd` 到新路径重开 IDE
- [ ] **全局 git config**：一般无关，除非你手动写过 `insteadOf` rewrite

#### Tauri macOS 签名 / 公证（如已配置）

- [ ] 当前 [`tauri.conf.json`](../../src-tauri/tauri.conf.json) 没有 `signingIdentity` 字段（未签名），**如果未来发布 macOS 应用**，需要：
  - Apple Developer Team 里新 Bundle ID `com.hopeagent.app` 注册一次
  - 公证（notarytool）时 app name 显示为 `Hope Agent`
  - 如果有 Provisioning Profile 绑定旧 bundle id，要换

#### 第三方 OAuth originator（特殊点，⚠️ 易忽略）

- [ ] [`crates/oc-core/src/oauth.rs:140`](../../crates/oc-core/src/oauth.rs#L140) URL query 里的 `originator=opencomputer`。这是传给 OAuth provider（Codex/ChatGPT）的**应用标识**，对方服务端可能：
  - (a) 允许任意 originator（只做 logging，改 `originator=hope-agent` 无副作用）
  - (b) 白名单校验（改了会导致授权失败）

  **上线前必须验证**：先在开发环境把 `originator` 改成 `hope-agent` 跑一次完整 OAuth 流程，确认能拿到 token 再合入。如果对方白名单了 `opencomputer`，要么保留旧值（作为字段而不是品牌名），要么联系对方白名单新值。

### 10.2 建议做（视项目阶段决定）

#### CI / 自动化

- [ ] 本仓当前**没有** `.github/workflows/`（未配置 CI），改名前后不涉及
- [ ] 未来加 CI 时，workflow 里注意别硬编码 `OpenComputer` 字符串

#### 包管理器 / 分发渠道（未发布，暂不适用）

- [ ] npm：`package.json` 是 `"private": true`，未来发布前再定 npm 包名
- [ ] crates.io：crate 目前不发布
- [ ] Homebrew Cask：若未来有 tap，formula 名字同步
- [ ] Scoop / Chocolatey / winget：Windows 分发渠道
- [ ] AUR / .deb / .rpm：Linux 分发渠道
- [ ] Mac App Store / Microsoft Store：商店应用名

#### 文档站 / 官网

- [ ] 当前仓库无独立 docs site（文档在 `docs/` 仓内），无需处理
- [ ] 如果未来起官网，域名选择与品牌对齐（例：`hopeagent.app` / `hope-agent.dev`）

#### 社交 / 社区

- [ ] Twitter / X、Discord、小红书、微信公众号等（如已注册）
- [ ] GitHub sponsors / Open Collective 页面（如已注册）

### 10.3 未来事项（发布时再定）

- [ ] **商标检索**：`Hope Agent` 在目标市场（中国 / 美国 / 日本）是否已有注册商标，发布前做一次 TM search，避免法律风险
- [ ] **域名**：`hopeagent.com` / `hopeagent.app` / `hope-agent.dev` 等是否可注册
- [ ] **品牌视觉**：现有 `src-tauri/icons/` 图标是否需要重设计（本次改名不重做视觉资产，仅文本层面）
- [ ] **应用内更新服务器**：当前 [`tauri.conf.json`](../../src-tauri/tauri.conf.json) 无 updater 配置，未来配 updater 时 endpoint URL、pubkey 都用新品牌
- [ ] **遥测 / 错误上报**：当前无外部遥测（仅本地 `logging.rs` + `crash_journal.rs`），未来接 Sentry / PostHog 时 project name 用新品牌
- [ ] **代码签名证书续期 / 重购**：仅限发布前

### 10.4 保留不改的项

- [ ] `CHANGELOG.md` 历史条目里的 `OpenComputer` / `oc-core` / `.opencomputer` **保留原文**（作为 record of truth），只在顶部加一条改名记录
- [ ] `docs/audit/2026-04-17-codebase-audit.md` 的历史 `file:line` 引用保留旧 crate 路径，顶部加 preamble 说明映射
- [ ] Git commit history 里的旧消息不 rewrite
- [ ] GitHub Issues / PRs 里的旧引用不改

---

## 11. 已知风险 & 后续事项（不阻塞本次改名）

- **开发机老数据**：本地 `~/.opencomputer/` 目录手动删（用户已说不考虑兼容）
- **CI / GitHub repo 名**：`github.com/shiwenwen/OpenComputer` 是否跟随改名到 `hope-agent`？改名会 break 已有 clone 但 GitHub 会做 redirect。**建议本次不改**，等发版后再定。`src-tauri/Cargo.toml` 的 `repository` 字段暂保持原值。
- **`hope` 短别名 CLI（方向已定，落地在打包阶段）**：Tauri 默认只产一个二进制 `hope-agent`，`hope` 作为 symlink 提供短别名。落地位置因渠道而异：Homebrew formula 里 `bin.install_symlink "hope-agent" => "hope"`；macOS `.dmg` / `.pkg` 在 postinstall 脚本里 `ln -s hope-agent hope`；Linux `.deb` / `.rpm` 在 postinst 里同理；Windows 用 `.cmd` shim 文件（`hope.cmd` 内部 `@"%~dp0hope-agent.exe" %*`）。本次改名（代码层）不涉及，只在打包阶段加，不需要改 Rust 源码。开发期可手动 `ln -s target/debug/hope-agent target/debug/hope` 验证。
- **crates.io 注册**：crate 不发布时不存在名字冲突风险。未来若发布，需检查 `ha-core` / `ha-server` 是否被占用，占用时回退到 `hope-agent-core` / `hope-agent-server`。
- **模型侧身份**：`system_prompt` / `agent.*.md` 里如果写了"你是 OpenComputer 助手"，改名后要决定：(a) 同步改 "Hope Agent"；(b) 去掉产品名用通用身份。这是**产品设计决定**，本清单按 (a) 直改处理，如需 (b) 另议。
