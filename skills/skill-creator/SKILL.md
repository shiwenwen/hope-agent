---
name: skill-creator
description: "创建、编辑、改进或审计 OpenComputer 技能。当用户想要：(1) 从头创建一个新技能，(2) 编辑或改进现有技能，(3) 审查或清理 SKILL.md 文件，(4) 运行评估测试技能效果，(5) 优化技能描述以提升触发准确度时使用。触发短语示例：'创建一个技能'、'做一个 skill'、'改进这个 skill'、'审查技能'、'create a skill'、'improve this skill'。"
always: true
---

# Skill Creator

创建新技能和迭代改进现有技能的工具。

## 技能系统概述

OpenComputer 的技能是模块化、自包含的包，通过提供专业知识、工作流和工具来扩展 AI 助手的能力。技能将通用 AI 转变为特定领域的专家。

### 技能加载机制（三级渐进式披露）

1. **元数据**（name + description）— 始终在上下文中（~100 词）
2. **SKILL.md 正文** — 技能触发时加载（理想 <500 行）
3. **捆绑资源** — 按需加载（脚本可直接执行，无需读入上下文）

### 技能目录结构

```
skill-name/
├── SKILL.md          （必需：frontmatter + Markdown 指令）
├── scripts/          （可选：可执行脚本，Python/Bash 等）
├── references/       （可选：按需加载的参考文档）
└── assets/           （可选：模板、图标等输出素材）
```

### 技能来源（优先级从低到高）

1. **内置技能** — 随 OpenComputer 发行，`skills/` 目录
2. **外部目录** — 用户导入，`config.json` 的 `extraSkillsDirs`
3. **托管技能** — `~/.opencomputer/skills/`
4. **项目技能** — `.opencomputer/skills/`（相对于 cwd，最高优先级）

---

## SKILL.md 格式规范

### Frontmatter（YAML）

```yaml
---
# ── 必填 ──
name: my-skill                          # 技能标识符（小写 + 连字符）
description: "技能描述..."               # 主触发机制，写清做什么+何时用

# ── 可选：运行前提 ──
requires:
  bins: [git, gh]                       # PATH 中必须全部存在（AND）
  anyBins: [rg, grep]                   # 至少一个存在（OR）
  env: [GITHUB_TOKEN]                   # 需要的环境变量
  os: [darwin, linux]                   # 支持的操作系统
  config: [webSearch.provider]          # 配置路径须为 truthy
always: false                           # true = 跳过所有前提检查
primaryEnv: MY_API_KEY                  # 主环境变量（可通过 apiKey 满足）

# ── 可选：调用控制 ──
user-invocable: true                    # 注册为 /command 斜杠命令
disable-model-invocation: false         # true = 从模型提示目录隐藏
skillKey: custom-key                    # 自定义配置查找键

# ── 可选：命令分发 ──
command-dispatch: tool                  # "tool" 或 "prompt"
command-tool: exec                      # dispatch=tool 时绑定的工具名
command-arg-mode: raw                   # 参数传递模式
command-arg-placeholder: "<query>"      # UI 占位提示
command-arg-options: [on, off]          # 固定参数选项
command-prompt-template: "..."          # 支持 $ARGUMENTS 展开的模板

# ── 可选：执行模式 ──
context: inline                         # "fork" = 子 Agent 执行，"inline" = 主对话
allowed-tools: [exec, read, write]      # 执行期间的工具白名单

# ── 可选：依赖安装 ──
install:
  - kind: brew
    formula: gh
    bins: [gh]
    label: "Install GitHub CLI (brew)"
    os: [darwin]
  - kind: node
    package: "@anthropic-ai/sdk"
    bins: [anthropic]
  - kind: go
    module: github.com/user/tool@latest
  - kind: uv
    package: my-python-tool
---
```

### 正文（Markdown）

技能触发后模型读取的指令。编写原则：

- **使用祈使句**：直接告诉模型做什么
- **解释 why 而非堆 MUST**：模型很聪明，理解原因比死记规则更有效
- **简洁优先**：上下文窗口是公共资源，只写模型不知道的信息
- **示例胜于说教**：一个好的示例比三段解释更高效

### description 编写要点

description 是技能的**主触发机制**——模型根据它决定是否使用该技能。

- 写清楚技能**做什么**和**什么时候用**
- 所有"何时使用"信息放在 description 里，不要放在正文中（正文在触发后才加载）
- 适当"主动"一些，避免 under-trigger。例如：

  差：`"GitHub 操作工具"`
  好：`"通过 gh CLI 进行 GitHub 操作：issues、PRs、CI 查看、代码审查。当用户提到 PR 状态、CI 检查、创建 issue、合并请求时使用，即使没有明确说'GitHub'。"`

---

## 创建技能的流程

### 第 1 步：理解意图

从当前对话中提取信息，或通过提问了解：

1. 这个技能要让 AI 做什么？
2. 什么时候应该触发？（用户会说什么）
3. 期望的输出格式是什么？
4. 是否需要测试用例验证？

如果对话中已有工作流（用户说"把这个做成一个 skill"），从对话历史中提取步骤、用到的工具、用户的纠正等信息。

### 第 2 步：访谈与调研

- 询问边界情况、输入输出格式、成功标准
- 确认前置依赖（需要哪些 CLI 工具、环境变量）
- 确定技能保存位置：
  - **项目级**（`.opencomputer/skills/<name>/`）— 仅限此项目的工作流
  - **用户级**（`~/.opencomputer/skills/<name>/`）— 跨项目通用

### 第 3 步：编写 SKILL.md

#### 3.1 先写 frontmatter

确定 name、description 和所有需要的字段。description 要足够详细以确保正确触发。

#### 3.2 规划捆绑资源

分析每个使用场景：
- 是否有需要重复编写的代码？→ 放到 `scripts/`
- 是否有模型需要参考的文档？→ 放到 `references/`
- 是否有需要在输出中使用的模板？→ 放到 `assets/`

#### 3.3 编写正文

遵循渐进式披露：
- SKILL.md 保持 <500 行
- 大文件拆到 `references/` 并在正文中指明何时读取
- 参考文件保持一层深度，从 SKILL.md 直接引用

#### 3.4 编写风格

**设定适当的自由度**：

- **高自由度**（文本指令）：多种方式都可行时
- **中自由度**（伪代码/参数化脚本）：有首选模式但允许变通时
- **低自由度**（具体脚本/步骤）：操作脆弱、一致性关键时

### 第 4 步：确认与保存

在写入前，将完整 SKILL.md 内容以 yaml 代码块形式展示给用户审查。确认后写入文件，告诉用户：
- 保存位置
- 调用方式：`/<skill-name> [参数]`
- 可以直接编辑 SKILL.md 来调整

---

## 测试与评估

### 编写测试用例

创建 2-3 个真实的测试 prompt，保存到 `evals/evals.json`：

```json
{
  "skill_name": "my-skill",
  "evals": [
    {
      "id": 1,
      "prompt": "用户的任务描述",
      "expected_output": "期望结果描述",
      "files": [],
      "expectations": [
        "输出包含 X",
        "使用了脚本 Y"
      ]
    }
  ]
}
```

完整 schema 见 `references/schemas.md`。

### 运行测试

在 `<skill-name>-workspace/iteration-<N>/` 目录中组织结果。

对每个测试用例，使用 `subagent` 启动两个并行运行：
1. **有技能**：读取 SKILL.md 后执行任务
2. **基线**：不使用技能直接执行

### 评估结果

1. **评分**：使用 `agents/grader.md` 的指令评估每个 assertion
2. **聚合**：运行 `python scripts/aggregate_benchmark.py <workspace>/iteration-N --skill-name <name>`
3. **分析**：使用 `agents/analyzer.md` 的指令发现聚合统计隐藏的模式
4. **可视化**：运行 `python eval-viewer/generate_review.py <workspace>/iteration-N --skill-name "my-skill"` 启动浏览器查看器

### 迭代改进

根据用户反馈改进技能时：

1. **从反馈中归纳**：技能会被使用无数次，不要为几个测试用例过拟合
2. **保持精简**：删除不起作用的内容
3. **解释 why**：理解用户反馈背后的原因，传递理解而非死板规则
4. **提取共性**：如果多个测试用例都重复写类似脚本，应该把脚本预置到 `scripts/`

---

## 高级：盲测对比

使用 `agents/comparator.md` 做 A/B 盲测：将两个输出标记为 A 和 B，不告诉评判者哪个来自哪个技能版本。然后用 `agents/analyzer.md` 分析赢家为什么赢。

这是可选的，大多数情况下人工审查循环就够了。

---

## Description 优化

技能完成后，可以优化 description 以提升触发准确度：

1. **生成触发评估集**：创建 20 个查询（should-trigger + should-not-trigger 各约 10 个）
   - should-trigger：不同措辞的同一意图，包括不明确提到技能名的情况
   - should-not-trigger：近似但实际需要不同工具的查询（难度越高越有价值）
   - 查询要具体真实，包含文件路径、个人背景等细节

2. **用户审查**：展示评估集让用户确认或修改

3. **迭代优化**：基于触发测试结果改进 description，直到 should-trigger 和 should-not-trigger 的准确率都令人满意

---

## 参考文件

- `agents/grader.md` — 评估 assertion 的评分指令
- `agents/comparator.md` — 盲测 A/B 对比指令
- `agents/analyzer.md` — 分析赢家原因和改进建议的指令
- `references/schemas.md` — evals.json、grading.json、benchmark.json 等 JSON 结构定义

---

## 不应包含在技能中的内容

技能只应包含 AI agent 完成任务所需的文件。不要创建：
- README.md、INSTALLATION_GUIDE.md、CHANGELOG.md
- 关于创建过程的文档
- 用户面向的安装指南
- 测试程序文档

这些只会增加混乱。技能是给 AI 用的，不是给人读的手册。
