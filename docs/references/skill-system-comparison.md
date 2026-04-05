# Skill 系统对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.3

## 一、架构总览

三个项目均采用 **SKILL.md + YAML Frontmatter** 作为 Skill 的核心定义格式，但在发现机制、执行模型、扩展能力上存在显著差异。

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| 语言 | Rust (后端解析) | TypeScript (Node/Bun) | TypeScript (Node) |
| Skill 格式 | `SKILL.md` + YAML frontmatter | `SKILL.md` + YAML frontmatter + 编程式注册 | `SKILL.md` + YAML frontmatter (JSON metadata) |
| 发现来源数 | 3 (extra/managed/project) | 6+ (managed/user/project/additional/legacy commands/bundled/MCP/plugin) | 7 (extra/bundled/managed/agents-personal/agents-project/workspace/plugin) |
| 执行模型 | 懒加载 read → LLM 执行 | SkillTool 工具调用 → fork/inline | read 工具加载 → LLM 执行 |
| 内置 Skill 数 | 0 (纯文件驱动) | ~15 (编程式注册) | 50+ (文件目录形式) |
| 远程 Skill | 无 | 实验性 Skill Search | ClawHub 注册中心 |
| 工具隔离 | `allowed-tools` frontmatter | `allowedTools` frontmatter/API | 无专用字段 |
| Fork 模式 | `context: fork` → 子 Agent | `context: fork` → 子 Agent | 无 |
| 安装系统 | `install:` frontmatter (brew/node/go/uv/download) | 无 | `install:` metadata (brew/node/go/uv/download) |
| Prompt 预算 | 三级渐进降级 (full/compact/truncated) | 按上下文窗口百分比 (1%) 动态预算 | 三级渐进降级 (full/compact/truncated) |

## 二、OpenComputer 实现

### 2.1 Skill 发现与加载

**源码位置**：`src-tauri/src/skills/discovery.rs`

OpenComputer 的 Skill 发现由 Rust 后端同步执行，采用三层优先级覆盖：

```
优先级（低 → 高）：
1. Extra directories — 用户通过 UI 导入的外部目录
2. Managed skills — ~/.opencomputer/skills/
3. Project skills — .opencomputer/skills/（相对于 cwd）
```

发现逻辑：
- 扫描每个源目录的直接子目录，检查是否包含 `SKILL.md`
- 支持嵌套 `skills/` 子目录的递归扫描
- 同名 Skill 高优先级覆盖低优先级（`all.retain(|e| e.name != entry.name)`）
- 文件大小上限 256KB，目录候选数上限 300，防止 DoS
- 30 秒 TTL 缓存 + 全局版本号失效机制（`AtomicU64` + `bump_skill_version()`）

### 2.2 Frontmatter 规范

**源码位置**：`src-tauri/src/skills/frontmatter.rs`

OpenComputer 使用自研轻量 YAML 解析器（非完整 YAML 库），支持以下字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string (必填) | Skill 标识符 |
| `description` | string | 人类可读描述 |
| `skillKey` | string | 自定义配置查找键 |
| `user-invocable` | bool | 是否允许用户 `/command` 调用（默认 true） |
| `disable-model-invocation` | bool | 是否从模型 prompt 目录中隐藏 |
| `command-dispatch` | string | 命令分发方式（"tool" / "prompt"） |
| `command-tool` | string | dispatch=tool 时绑定的工具名 |
| `command-arg-mode` | string | 参数传递模式（"raw"） |
| `command-arg-placeholder` | string | UI 显示的参数占位符 |
| `command-arg-options` | string[] | 固定参数选项列表 |
| `command-prompt-template` | string | 支持 `$ARGUMENTS` 展开的模板 |
| `allowed-tools` | string[] | 工具隔离白名单 |
| `context` | string | 执行模式（"fork" / "inline"） |
| `requires:` | block | 环境需求（bins/anyBins/env/os/config） |
| `install:` | block[] | 依赖安装规格 |
| `always` | bool | 跳过所有需求检查 |
| `primaryEnv` | string | 可由 apiKey 替代的主环境变量 |

特色设计：
- **`command-dispatch: tool`**：Skill 可以直接绑定到内置工具，实现 `/command → tool_call` 的确定性分发，无需 LLM 推理
- **`command-prompt-template`**：支持 `$ARGUMENTS` 变量替换，dispatch=prompt 时 body 自动作为模板

### 2.3 工具隔离（allowed-tools）

**架构约定**（来自 AGENTS.md）：

SKILL.md frontmatter 的 `allowed-tools:` 字段实现双重防护：

1. **Schema 级过滤**：Skill 激活时，Agent 的 `skill_allowed_tools` 字段在 Provider 层过滤工具 schema，LLM 只能看到白名单中的工具
2. **执行层白名单**：`ToolExecContext.plan_mode_allowed_tools` 在执行层检查，即使 LLM 幻觉调用未暴露的工具也会被拦截

空列表 = 全部工具可用（向后兼容）。

### 2.4 Fork 模式

**架构约定**（来自 AGENTS.md）：

`context: fork` 指定 Skill 在子 Agent 中执行：
- tool_call 不污染主对话上下文
- 子 Agent 继承 `allowed_tools` 限制
- 执行结果通过注入系统自动推送回主对话
- 与 `subagent(action="spawn_and_wait")` 机制集成

### 2.5 安装规范与环境检测

**源码位置**：`src-tauri/src/skills/requirements.rs`

环境检测逻辑：
- **bins（AND）**：所有二进制必须存在于 PATH
- **anyBins（OR）**：至少一个二进制存在
- **env**：环境变量必须非空，优先检查用户 UI 配置的值（`configured_env`）
- **primaryEnv + apiKey**：若 `primaryEnv` 匹配且 `__apiKey__` 已配置，视为满足
- **os**：支持 `darwin`/`mac`/`linux`/`windows` 等标识
- **config**：配置路径必须为真值
- **always: true**：跳过所有检查

安装方式（`install:` block）：

| kind | 参数 | 说明 |
|------|------|------|
| `brew` | formula | Homebrew 安装 |
| `node` | package | npm 全局安装 |
| `go` | module | go install |
| `uv` | package | Python uv 安装 |
| `download` | — | 直接下载 |

### 2.6 预算控制

**源码位置**：`src-tauri/src/skills/prompt.rs`

三级渐进降级策略：

1. **Full format**：`- name: description (read: ~/path/SKILL.md)` — 完整描述 + 路径
2. **Compact format**：`- name (read: ~/path/SKILL.md)` — 仅名称 + 路径，省略描述
3. **Truncated**：二分搜索最大 compact 前缀，附加截断警告

默认预算：
- `max_count`: 150 skills
- `max_chars`: 30,000 字符
- `max_file_bytes`: 256 KB
- `max_candidates_per_root`: 300

路径压缩：home 目录前缀替换为 `~`，每个 Skill 节省 ~5-6 tokens。

## 三、Claude Code 实现

### 3.1 Skill 发现来源（6+ 种）

**源码位置**：`src/skills/loadSkillsDir.ts`

Claude Code 拥有最复杂的多源 Skill 发现系统：

| 来源 | 路径 | 优先级 |
|------|------|--------|
| Policy (managed) | `{managedPath}/.claude/skills/` | 最低 |
| User | `~/.claude/skills/` | |
| Project | `.claude/skills/`（cwd 向上遍历至 home） | |
| Additional | `--add-dir` 指定目录 | |
| Legacy commands | `.claude/commands/`（已废弃，向后兼容） | |
| Bundled | 编程式注册 | 最高 |
| MCP | MCP 服务器提供 | 独立 |
| Plugin | 插件提供 | 独立 |

关键设计：
- **symlink 去重**：通过 `realpath()` 解析真实路径，防止同一文件通过不同路径重复加载
- **条件 Skill（paths frontmatter）**：带 `paths:` 字段的 Skill 默认不加载，只在用户触及匹配文件时激活
- **`--bare` 模式**：跳过自动发现，仅加载 `--add-dir` 显式指定的 Skill
- **Policy 锁定**：`isRestrictedToPluginOnly('skills')` 可限制只允许 Plugin 来源

### 3.2 MCP Skills

**源码位置**：`src/skills/mcpSkillBuilders.ts`

MCP Skill 通过写后注册模式解决循环依赖：

```typescript
// mcpSkillBuilders.ts 是依赖图叶节点
registerMCPSkillBuilders({ createSkillCommand, parseSkillFrontmatterFields })
// MCP 服务器连接后调用 getMCPSkillBuilders() 构建 Skill
```

MCP Skill 有特殊安全限制：
- 不执行 markdown 中的内联 shell 命令（`!`...`` / ````! ... ````）
- `${CLAUDE_SKILL_DIR}` 对 MCP Skill 无意义

### 3.3 Effort 级别

**源码位置**：`src/skills/loadSkillsDir.ts` (parseSkillFrontmatterFields)

Claude Code 独有的 `effort` frontmatter 字段，控制 Skill 执行时 LLM 的推理深度。在 fork 模式下通过 `agentDefinition.effort` 传递给子 Agent。

### 3.4 内置 Skills

**源码位置**：`src/skills/bundled/index.ts`

Claude Code 通过编程式 `registerBundledSkill()` 注册内置 Skill，而非文件系统发现。当前内置 Skill 列表：

| Skill | 说明 | 特殊能力 |
|-------|------|----------|
| `update-config` | 配置管理 | |
| `keybindings` | 快捷键配置 | |
| `verify` | 验证工作完成度 | |
| `debug` | 会话调试 | 按需启用调试日志 |
| `lorem-ipsum` | 填充文本 | |
| `skillify` | Skill 创建向导 | |
| `remember` | 记忆管理 | |
| `simplify` | 代码简化 | |
| `batch` | 批量操作 | |
| `stuck` | 解决卡顿问题 | |
| `loop` | 定时循环执行 | 需 KAIROS feature flag |
| `schedule` | 远程定时任务 | 需 AGENT_TRIGGERS_REMOTE |
| `claude-api` | Claude API 文档 | 247KB 延迟加载内容 |
| `claude-in-chrome` | Chrome 集成 | 条件自动启用 |

特色设计：
- **`files` 字段**：内置 Skill 可声明虚拟文件，首次调用时解压到磁盘（`~/.claude/bundled-skills/{name}/`），模型可通过 Read/Grep 工具访问
- **`isEnabled` 回调**：运行时动态判断可用性（如 `isKairosCronEnabled()`）
- **Feature flag 门控**：部分 Skill 通过 Bun 的 `feature()` 条件编译
- **`whenToUse` 字段**：比 description 更详细的触发条件描述，用于模型判断

### 3.5 SkillTool 实现

**源码位置**：`src/tools/SkillTool/SkillTool.ts`

Claude Code 的 Skill 执行通过专用 `Skill` 工具：

```
模型识别任务 → 调用 Skill tool(skill, args) → 查找 Command → 执行
```

**Fork 执行流程**：
1. `prepareForkedCommandContext()` 构建隔离上下文
2. `runAgent()` 在子 Agent 中流式执行
3. 收集 agentMessages，提取结果文本
4. 进度通过 `onProgress` 回调实时报告
5. 完成后 `clearInvokedSkillsForAgent()` 释放状态

**Prompt 预算**（`src/tools/SkillTool/prompt.ts`）：
- 默认占上下文窗口的 1%（`SKILL_BUDGET_CONTEXT_PERCENT = 0.01`）
- 200K 上下文 → 8,000 字符预算
- 每条描述硬限 250 字符
- Bundled Skill 描述永不截断，非 bundled 优先截断
- 极端情况下非 bundled 退化为仅名称

**Inline Shell 执行**：
- Skill markdown 中的 `!`command`` 和 ``` ```! command ``` ``` 会被执行
- MCP Skill 禁止此功能（安全考虑）
- 支持 `${CLAUDE_SKILL_DIR}` 和 `${CLAUDE_SESSION_ID}` 变量替换

**实验性远程 Skill 搜索**：
- `feature('EXPERIMENTAL_SKILL_SEARCH')` 门控
- 通过 `remoteSkillLoader` 动态发现和安装 Skill

## 四、OpenClaw 实现

### 4.1 Skill 架构体系

**源码位置**：`src/agents/skills/` 目录

OpenClaw 的 Skill 系统建立在 `@mariozechner/pi-coding-agent` 之上，Skill 类型继承自上游 `Canonical Skill`：

```typescript
type Skill = CanonicalSkill & { source?: string; }
```

### 4.2 发现与加载

**源码位置**：`src/agents/skills/workspace.ts`

7 层优先级覆盖（低 → 高）：

```
1. Extra directories（config.skills.load.extraDirs + plugin skill dirs）
2. Bundled skills（内置打包）
3. Managed skills（~/.openclaw/skills/ 等效路径）
4. Agents skills - personal（~/.agents/skills/）
5. Agents skills - project（{workspace}/.agents/skills/）
6. Workspace skills（{workspace}/skills/）
```

加载安全机制：
- **路径逃逸检测**：`resolveContainedSkillPath()` + `isPathInside()` 确保 symlink 不逃逸出配置根目录
- **symlink 验证**：`openVerifiedFileSync({ rejectPathSymlink: true })` 拒绝指向外部的 SKILL.md 符号链接
- **嵌套 skills/ 启发式**：自动检测 `dir/skills/*/SKILL.md` 结构并切换扫描根目录

默认限制（与 OpenComputer 高度一致）：
- `maxCandidatesPerRoot`: 300
- `maxSkillsLoadedPerSource`: 200
- `maxSkillsInPrompt`: 150
- `maxSkillsPromptChars`: 30,000
- `maxSkillFileBytes`: 256,000

### 4.3 Frontmatter 与元数据

**源码位置**：`src/agents/skills/frontmatter.ts`

OpenClaw 使用 JSON 嵌套的 `metadata.openclaw` 块扩展标准 frontmatter：

```yaml
---
name: github
description: "GitHub operations via gh CLI"
metadata:
  openclaw:
    emoji: "..."
    requires:
      bins: [gh]
    install:
      - kind: brew
        formula: gh
        bins: [gh]
---
```

**SkillInvocationPolicy**：
- `user-invocable`（默认 true）
- `disable-model-invocation`（默认 false）

### 4.4 Plugin Skill 集成

**源码位置**：`src/agents/skills/plugin-skills.ts`

OpenClaw 独有的插件系统集成：
- 通过 `PluginManifestRegistry` 发现插件中声明的 Skill 目录
- 插件激活状态（`resolveEffectivePluginActivationState()`）决定是否加载
- 支持 legacy plugin ID 别名映射
- Memory slot 决策影响 Skill 可见性

### 4.5 ClawHub 远程 Skill 注册中心

**源码位置**：`src/agents/skills-clawhub.ts`

OpenClaw 独有的 Skill 分发平台：
- `searchClawHubSkills()` — 搜索远程 Skill
- `fetchClawHubSkillDetail()` — 获取详情
- `downloadClawHubSkillArchive()` — 下载归档
- 安装后写入 `.clawhub/origin.json` 追踪来源
- 支持版本锁文件（`ClawHubSkillsLockfile`）
- SHA 校验安装完整性

### 4.6 安装系统

**源码位置**：`src/agents/skills-install.ts`

与 OpenComputer 类似但更丰富的安装规格：

| kind | 额外参数 | 说明 |
|------|---------|------|
| `brew` | formula, cask | 支持 cask 别名 |
| `node` | package | npm 规格验证 |
| `go` | module | Go module 路径验证 |
| `uv` | package | Python uv 包验证 |
| `download` | url, archive, extract, stripComponents, targetDir | 完整下载解压流程 |

安全扫描：`scanSkillInstallSource()` 在安装前验证安装规格安全性。

### 4.7 Prompt 生成

OpenClaw 采用 XML 格式输出 Skill 目录（区别于 OpenComputer/Claude Code 的纯文本）：

```xml
<available_skills>
  <skill>
    <name>github</name>
    <description>GitHub operations via gh CLI</description>
    <location>~/.openclaw/skills/github/SKILL.md</location>
  </skill>
</available_skills>
```

同样支持三级降级：full → compact（省略 description）→ 二分搜索截断。

### 4.8 50+ 内置 Skill 目录

OpenClaw 附带大量开箱即用的 Skill 目录（按类别）：

| 类别 | Skills |
|------|--------|
| 开发工具 | github, gh-issues, coding-agent, tmux |
| 消息/社交 | slack, discord, imsg, bluebubbles |
| 笔记/知识 | notion, obsidian, bear-notes, apple-notes, apple-reminders |
| 媒体 | spotify-player, sonoscli, songsee, camsnap, video-frames, gifgrep |
| AI/模型 | gemini, openai-whisper, openai-whisper-api, sherpa-onnx-tts |
| 任务管理 | things-mac, trello, taskflow, taskflow-inbox-triage |
| 网络/浏览 | xurl, blogwatcher |
| 系统/工具 | weather, healthcheck, openhue, goplaces, peekaboo, oracle |

## 五、逐项功能对比

| 功能维度 | OpenComputer | Claude Code | OpenClaw |
|---------|-------------|-------------|----------|
| **Skill 格式** | SKILL.md + YAML | SKILL.md + YAML + 编程式 | SKILL.md + YAML (JSON metadata) |
| **自研 Frontmatter 解析** | 是（Rust 手写） | 否（通用库） | 否（通用库） |
| **发现层数** | 3 | 6+ | 7 |
| **优先级覆盖** | 名称去重��盖 | realpath 去重 | 名称去重覆盖 |
| **缓存机制** | 30s TTL + 版本号 | memoize（进程级） | 无显式缓存（同步加载） |
| **条件激活（paths）** | 无 | 有（文件路径匹配） | 无 |
| **工具隔离** | Schema + 执行层双重防护 | allowedTools 字段 | 无专用机制 |
| **Fork 执行** | context: fork → 子 Agent | context: fork → runAgent | 无 |
| **Effort 控制** | 无 | effort frontmatter | 无 |
| **Inline Shell** | 无 | `!`cmd`` 语法 | 无 |
| **变量替换** | `$ARGUMENTS` | `${CLAUDE_SKILL_DIR}`, `${CLAUDE_SESSION_ID}`, `$ARGUMENTS` | 无 |
| **Hooks** | 无 | hooks frontmatter (HooksSchema) | 内置 hooks 系统（独立） |
| **MCP 集成** | 无 | MCP Skill builders | 无 |
| **远程 Skill** | 无 | 实验性 Skill Search | ClawHub 注册中心 |
| **Plugin 集成** | 无 | Plugin 来源 | PluginManifestRegistry |
| **安装系统** | brew/node/go/uv/download | 无 | brew/node/go/uv/download + 安全扫描 |
| **安装安全** | 基础验证 | N/A | npm/brew/go/uv 规格验证 + 安全扫描 |
| **环境检测** | bins/anyBins/env/os/config + always | 无（Skill 自行检查） | bins/anyBins/env/config + always |
| **primaryEnv + apiKey** | 有 | 无 | 有 |
| **Prompt 格式** | 纯文本列表 | 纯文本列表 | XML 标签 |
| **Prompt 预算** | 固定 30K chars | 上下文窗口 1% 动态 | 固定 30K chars |
| **降级策略** | full → compact → truncated | full → truncated desc → names-only | full → compact → truncated |
| **路径压缩** | `~` 替换 home | 无（由 model 处理） | `~` 替换 home |
| **Slash 命令** | 自动注册 /command | Skill tool 分发 | skill-commands 系统 |
| **command-dispatch: tool** | 有（确定性分发） | 无 | 有（dispatch.kind: tool） |
| **内置 Skill 数** | 0 | ~15 | 50+ |
| **Skill 健康检查** | `check_all_skills_status()` | 无专用 API | `SkillStatusReport` |
| **Emoji 标记** | 无 | 无 | metadata.openclaw.emoji |
| **版本字段** | 无 | version frontmatter | 无 |
| **Model 覆盖** | 无 | model frontmatter | 无 |
| **Feature Flag 门控** | 无 | Bun feature() | 无 |
| **Agent 指定** | 无 | agent frontmatter | 无 |

## 六、差距分析与建议

### 6.1 OpenComputer 的优势

1. **工具隔离双重防护**：Schema 级 + 执行层白名单是三个项目中最严格的安全设计
2. **command-dispatch: tool**：确定性工具分发避免 LLM 幻觉，减少延迟
3. **Rust 后端解析**：性能优势明显，30s TTL 缓存 + 原子版本号机制高效
4. **安装系统完整**：GUI 一键安装 + 环境检测 + primaryEnv 替代机制，用户体验好
5. **command-prompt-template**：body 自动作为模板的设计简洁实用

### 6.2 建议补齐的功能

| 优先级 | 功能 | 参考来源 | 说明 |
|--------|------|---------|------|
| P0 | 内置 Skill | Claude Code | 开箱即用体验，至少提供 debug/verify/simplify 等通用 Skill |
| P1 | 条件 Skill（paths） | Claude Code | 大型项目中避免无关 Skill 占据 prompt 预算 |
| P1 | Effort 控制 | Claude Code | 不同 Skill 需要不同推理深度，降低简单 Skill 的成本 |
| P1 | 远程 Skill 注册中心 | OpenClaw (ClawHub) | 社区 Skill 生态分发，可先用简单 URL 导入 |
| P2 | MCP Skill 集成 | Claude Code | 扩展 Skill 来源到 MCP 服务器 |
| P2 | Inline Shell 执行 | Claude Code | Skill 中嵌入可执行命令，增强能力 |
| P2 | 变量替换扩展 | Claude Code | 支持 `${SKILL_DIR}`, `${SESSION_ID}` 等 |
| P2 | Plugin 集成 | OpenClaw | 第三方插件声明 Skill 的标准接口 |
| P3 | 动态预算 | Claude Code | 根据模型上下文窗口自适应预算 |
| P3 | Hooks 集成 | Claude Code | Skill 级别的 pre/post 钩子 |
| P3 | Version 字段 | Claude Code | Skill 版本追踪，支持升级通知 |
| P3 | Agent 指定 | Claude Code | 不同 Skill 使用不同 Agent 类型 |

### 6.3 安全差距

| 方面 | 当前状态 | 建议 |
|------|---------|------|
| 安装规格验证 | 基础验证 | 参考 OpenClaw 的 npm/brew/go 规格正则验证 + `scanSkillInstallSource()` 安全扫描 |
| 路径逃逸检测 | 无显式检查 | 参考 OpenClaw 的 `isPathInside()` + symlink realpath 验证 |
| SKILL.md 内容注入 | 信任文件内容 | 参考 Claude Code 对 MCP Skill 禁用 inline shell 的策略 |

### 6.4 Prompt 优化

OpenComputer 当前的纯文本格式在 Skill 数量多时信息密度低。可考虑：
- 参考 OpenClaw 的 XML 格式，结构化信息对 LLM 解析更友好
- 参考 Claude Code 的动态预算（上下文窗口百分比），避免小窗口模型浪费过多 budget
- 当前的 `description` 无长度限制，应参考 Claude Code 的 250 字符硬限
