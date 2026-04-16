# OpenComputer 演进路线图（2026 Q2）

> 制定时间：2026-04-16 | 基线：[`unified-comparison.md`](./unified-comparison.md) v2.2（含 Hermes Agent 对标）
> 目标：在 Claude Code 保持领先、OpenClaw 快速追赶、**Hermes Agent 在自我学习维度开创性领先**的竞争态势下，通过**架构级追齐** + **记忆语义升级** + **自我学习闭环** + **多 Agent/MCP 生态接入** + **体验生态补足**五个阶段，把 OC 的合计评分从 67/85 推向 75+/85（特别是自我学习维度从 1 推到 4），同时守住 Provider 多样性、深度上下文管理、Plan Mode、Dashboard Insights 这几项既有优势。

## 设计原则

1. **先架构后能力**：Context Engine trait、Memory Backend plugin 接口、Hooks 系统是这一季度的"基础设施投资"，它们让后续能力都变成"往稳定 trait 上插"而不是重复折腾热路径
2. **优先对齐 OW v2.1 的架构突破**：Context Engine 可插拔、Active Memory、Dreaming、Auth Profile 轮换、网关 SSRF 纵深防御——这些是 2026 年 3–4 月 OW 新拉开的差距，不追就要持续掉队
3. **自我学习闭环是 v2.2 新增最高优先项**：Hermes Agent 在自我学习维度评分 5 vs OC 1，这是最大的 gap。但 OC 已有 side_query + SKILL.md + async_jobs + FTS5 四块基础设施，追齐成本远低于从零开始。Phase B' 专门解决
4. **MCP 推迟到 Phase C**：MCP 仍是 P0 战略项，但投入大（Client SDK + OAuth + 资源协议 + Skill 桥接）。推到 Phase C 的判断是：先把 Context Engine、自我学习和网关安全打稳，MCP 作为最后一块"插件系统"引入
5. **不碰领先面**：不动 Provider 模板（28/108）、5 层压缩、Side Query 缓存、Plan Mode 六态、Dashboard Insights——这些是守住的阵地
6. **每个 Phase 独立可发布**：Phase 之间尽量解耦，每个 Phase 结束时 OC 是"可用且更强"的，不要出现"改到一半不可用"的中间状态

## 关键判断

| 判断 | 理由 |
|------|------|
| Context Engine trait 是这一季度的**地基**工程 | A1/A2/B1/B3/B4 全系列能力都依赖这个 trait 存在，越晚做越贵 |
| 网关 SSRF 硬化**不能跳过** | `opencomputer server` 已进入生产运行路径（launchd/systemd），SSRF/realpath/constant-time 是基础要求 |
| Active Memory 的实现成本**低到不可思议** | 已有 `side_query` 缓存机制，pre-reply 注入可以完全复用，近乎零新增 API 成本 |
| MCP 推到 Phase C 是**战略取舍** | 不是降权，是让它在 Phase A 的 trait 体系上构建 |
| 多 Agent 协作推到 Phase C 晚了吗？ | 不晚。`SubagentGroup` 已经在展示层解决了 UX 问题，核心缺的是 worktree 隔离 + 双向消息，这两个都依赖 Hooks/安全体系到位 |
| **自我学习闭环（v2.2 新增）是最大 gap** | HA 评分 5 vs OC 1——但 OC 已有 side_query + SKILL.md + async_jobs + FTS5 四块基础设施。Phase B' 用 4–6 周追到 4 分，ROI 极高 |

---

## Phase A：架构补课 & 网关安全硬化（4–6 周）

**Phase 目标**：把可扩展性和生产级安全这两个地基打稳。完成后 OC 的"上下文管理"和"权限安全"两项从 5/3 推到 5/4，为后续 Phase 腾出空间。

### A1. Context Engine trait 抽象 ⭐ 核心地基

**动机**：OW v2.1 的 `registerContextEngine()` 让第三方能注册完整的 context 装配/压缩策略。OC 当前的 `context_compact` 是固定 5 层管线，所有增强都要改核心代码。

**实施**：
- 在 [crates/oc-core/src/context_compact/](../../crates/oc-core/src/context_compact/) 定义 `ContextEngine` trait，暴露生命周期钩子：
  - `ingest(&mut self, turn: &Turn)` — 单轮摄入
  - `assemble(&self, budget: TokenBudget) -> Vec<Message>` — 请求前装配
  - `compact(&mut self, pressure: Pressure) -> CompactResult` — 主动压缩
  - `after_turn(&mut self, usage: &TokenUsage)` — 回合结束 hook（预警/idle maintenance）
  - `system_prompt_addition(&self) -> Option<String>` — 动态 system 补丁（给 Active Memory 用）
- 把现有 5 层压缩实现封装为默认后端 `DefaultContextEngine`，保持行为字节一致
- `CoreState` 持有 `Arc<dyn ContextEngine>`，通过 `AppConfig.contextEngine` 选择
- 前端 Agent 设置面板增加"上下文引擎"选择器（默认隐藏，只在有多个 engine 时显示）

**验收**：
- [ ] 默认 engine 与旧实现行为一致（快照对比 10 条真实会话）
- [ ] Side Query 缓存继续命中（cache snapshot 不变）
- [ ] Cache-TTL 节流继续生效
- [ ] 新增一个 `NoopContextEngine` 作为测试用例，确认 trait 边界正确

**复杂度**：中高。不涉及 API 行为变化，但是热路径重构。

### A2. 可插拔 Compaction Provider

**动机**：摘要模型当前硬绑定主对话 Provider，无法用更便宜/更快的专用模型（例如用 Haiku 做摘要而主对话跑 Opus）。

**实施**：
- 定义 `CompactionProvider` trait：`async fn summarize(&self, messages: &[Message], target_tokens: usize) -> Result<Summary>`
- 内置两个实现：
  - `ReuseModelProvider`（当前行为：复用主对话模型 + side_query 缓存）
  - `DedicatedModelProvider`（配置独立 Provider/model，不共享缓存，牺牲 10% 成本换灵活性）
- 失败时自动 fallback 到 `ReuseModelProvider`
- `AppConfig.compact.summaryProvider` 配置项

**验收**：默认行为不变；配置 dedicated 后能独立调用且失败优雅回退。

**复杂度**：低（依赖 A1 trait 体系）。

### A3. Auth Profile 轮换 failover

**动机**：OC 的 failover 只在模型级工作。同 provider 多 key 场景下（例如两个 Anthropic organization），rate limit 时无法自动切换。

**实施**：
- `ProviderConfig` 增加 `authProfiles: Vec<AuthProfile>`，每个 profile 持有独立 API key + base_url
- `AssistantAgent` 调用路径在现有 5 类错误分类之前插入一级"profile 迭代"：
  - RateLimit / Overloaded → 先试下一个同 provider 的 profile → 都失败再跳模型
  - Auth / Billing → 跳 profile 而不是跳模型
- 每 session 记录"当前活跃 profile"实现 cache-friendly stickiness
- Per-profile cooldown（rate limit 后 N 秒不再尝试）
- 前端 Provider 设置面板支持添加多个 API key profile

**验收**：单 provider 多 key 配置下，一个 key rate limit 时能自动切到下一个 key，不中断对话。

**复杂度**：中。

### A4. 网关 SSRF 纵深防御

**动机**：OC `opencomputer server` 已在生产运行路径，但 `browser` / `web_fetch` / `image_generate`（URL 输入）工具缺少统一的 SSRF 策略。

**实施**：
- 新模块 [crates/oc-core/src/security/ssrf.rs](../../crates/oc-core/src/security/ssrf.rs)，定义：
  - `SsrfPolicy { strict / default / allow_private }`
  - `check_url(url, policy) -> Result<()>` — 统一检查点
  - `check_hostname(host) -> HostKind { public / loopback / private / link_local / metadata }`
  - 阻止 169.254.169.254（云 metadata）、RFC1918、loopback（strict 模式下）
- `browser` 工具默认 strict 模式（snapshot/screenshot/navigate/eval 全路径）
- `web_fetch` 默认 default 模式（允许 hostname 导航但禁止 private network）
- Per-provider `allowPrivateNetwork` allowlist（自托管 Ollama/LM Studio 场景）
- `~/.opencomputer/` 下落盘的 config 提供"受信主机名" allowlist

**验收**：browser 工具默认无法访问 `http://127.0.0.1:22` / `http://169.254.169.254/` / `http://192.168.x.x`，但通过 allowlist 可放行特定主机。

**复杂度**：中。

### A5. workspace fs-safe 符号链接防护

**动机**：OW v2.1 修复了 `open` 与 `realpath` 之间的 symlink swap TOCTOU 攻击。OC 的 `read/write/edit` 工具在 sandbox root 外可被同样攻击。

**实施**：
- 新增 helper `open_within_root(root: &Path, rel: &Path) -> Result<File>`：
  - 打开文件后从 FD 读取真实路径（`/proc/self/fd/N` on Linux、`fcntl F_GETPATH` on macOS）
  - 对比 root 前缀，不匹配则拒绝
- `read` / `write` / `edit` 工具统一走这个 helper
- 符号链接别名拒绝：如果目标文件是指向 root 外的 symlink，直接拒绝

**验收**：构造 symlink 从 sandbox 指向 `/etc/passwd`，`read` 工具拒绝。

**复杂度**：中（需要 platform-specific FD→path 实现）。

### A6. constant-time secret 比较

**动机**：API Key 鉴权中间件 [crates/oc-server/src/middleware.rs](../../crates/oc-server/src/middleware.rs) 当前用普通字符串比较，理论上有侧信道风险。

**实施**：
- 新增 helper `safe_eq_secret(a: &[u8], b: &[u8]) -> bool`（使用 `subtle::ConstantTimeEq` crate）
- 所有 auth 门统一切换：server middleware、future OAuth、future device auth

**验收**：中间件用 constant-time 比较，单元测试覆盖。

**复杂度**：低。

**Phase A 总预估**：4–6 周。里程碑：Context Engine trait 合并 + 网关安全硬化发布。

---

## Phase B：记忆语义升级（6–8 周）

**Phase 目标**：让 OC 的记忆系统从"被动召回 + 自动提取"升级到"主动查询 + 离线固化 + 人格分离"。完成后 OC 记忆系统再次甩开 OW（预期保持 5 分，但质的提升）。

### B1. Active Memory pre-reply 阻塞注入 ⭐ 高 ROI

**动机**：OW v2.1 的 Active Memory 在主回复前跑一轮阻塞式记忆子 Agent，把相关记忆以 system prompt 补丁注入。OC 当前只做被动召回（提示词注入固定 N 条）+ 自动提取，缺乏"根据当前对话主题主动查记忆"的语义。

**实施**：
- 在 [crates/oc-core/src/agent/](../../crates/oc-core/src/agent/) 新增 `active_memory.rs`
- 触发时机：每轮用户消息进入 → `active_memory.recall(query=lastMsg, budget=2K tokens) -> Vec<Memory>`
- 使用 `side_query()` 复用 prompt cache，额外成本几乎为零
- 结果通过 Context Engine trait 的 `system_prompt_addition()` 注入（依赖 A1）
- 配置：`AgentConfig.memory.activeMemory { enabled, maxRecalled: 5, timeoutMs: 3000 }`
- 超时降级：超时后跳过，走被动召回

**验收**：
- [ ] 开启后能在系统提示注入"根据当前对话召回的记忆"段落
- [ ] Side Query 缓存命中率 ≥ 90%
- [ ] 延迟增加 ≤ 500ms（缓存命中场景）

**复杂度**：中。依赖 A1 Context Engine trait。

### B2. SOUL.md 人格文件

**动机**：OC 的 Agent 人格和操作指令混在系统提示里，难以独立管理。OW 的 SOUL.md 模型很优雅。

**实施**：
- `AgentConfig` 增加 `soulFile: Option<String>` 路径（默认 `~/.opencomputer/agents/{agent_id}/SOUL.md`）
- 系统提示拼装时：`SOUL.md 内容 → AGENTS.md 操作指令 → 工具 schema → 记忆 → 对话历史`
- 前端 Agent 编辑面板增加 SOUL.md 独立 tab（与 system prompt 分开）
- 导出/导入 Agent 时 SOUL.md 一并打包

**复杂度**：低。

### B3. Dreaming 离线记忆固化（light 阶段先行）

**动机**：OC 的自动提取是"在线 inline"，没有离线深加工路径，候选晋升不分冷热。

**实施**：
- 新模块 [crates/oc-core/src/memory/dreaming/](../../crates/oc-core/src/memory/dreaming/)
- Light 阶段先上（最简单且最有价值）：
  - 后台 idle 任务（应用空闲 30 分钟触发，可配置）
  - 扫描最近 24h 的会话，用 `side_query` 跑"信号打分"提示词
  - 高置信度（score > 0.8）候选晋升到 `core_memory`
  - 生成人类可读 Dream Diary 写入 `~/.opencomputer/memory/dreams/{date}.md`
- Deep/REM 阶段延后到 Phase B 末尾（可选）
- Dashboard 增加 Dream Diary 查看 tab

**验收**：连续使用一周后能在 Dream Diary 看到自动生成的记忆晋升记录。

**复杂度**：中。

### B4. Memory Backend plugin 接口

**动机**：为将来接入 Honcho / QMD / 用户自定义后端预留抽象，避免到时候再次重构 memory 模块。

**实施**：
- 定义 `MemoryBackend` trait，接口与现有 SQLite 实现对齐
- 把现有 SQLite 实现封装为 `SqliteMemoryBackend`
- `CoreState` 持有 `Arc<dyn MemoryBackend>`
- Phase B 不实现新后端，但把接口对齐到未来接 Honcho 的形状

**复杂度**：中。不急但值得顺手做。

### B5. Reactive Compact

**动机**：Context Engine trait 到位后顺手实现。Token 使用率接近阈值时主动触发压缩，而不是等到溢出。

**实施**：Context Engine 的 `after_turn()` hook 里检查 `usage.ratio > 0.75` 则主动调 `compact(Pressure::Proactive)`。

**复杂度**：低。

**Phase B 总预估**：6–8 周。里程碑：Active Memory + SOUL.md + Dreaming light 阶段发布。

---

## Phase B'：自我学习闭环（4–6 周，与 Phase B 末段并行）

> **v2.2 新增**。对标 Hermes Agent 的核心差异化：闭环自我学习。目标是把 OC 的"自我学习"评分从 1 推到 4。

**Phase 目标**：OC 从"用户创建 Skill + 被动记忆提取"升级到"Agent 自主创建/修补 Skill + 反省式学习 + 跨会话知识整合"。

### B'1. 自主 Skill 创建 ⭐ 对标 HA 核心

**动机**：HA 的 `_spawn_background_review()` 让 Agent 在对话结束后自动分析是否有可复用模式，创建或修补 Skill。OC 的 SKILL.md 系统只能人工创建。

**实施**：
- 新模块 [crates/oc-core/src/skills/auto_review.rs](../../crates/oc-core/src/skills/auto_review.rs)
- 触发时机：对话结束 + 每 N 轮（默认 10，可配 `skills.autoReview.nudgeInterval`）
- 执行方式：**复用 `async_jobs` 后台执行**，不阻塞对话
- Skill Review 提示词（参考 HA）：
  > "分析这段对话：是否使用了非平凡的方法？是否经过试错或经验修正？如果有可复用模式，创建一个新 Skill（YAML frontmatter + Markdown body）或修补已有 Skill。"
- 创建路径：`side_query()` 分析 → 判断 create/patch/skip → 调用现有 `skills::create_skill()` / `skills::update_skill()`
- **模糊匹配修补**：新增 `patch_skill_fuzzy(skill_id, old_text_approx, new_text)` — 使用编辑距离找到最接近的片段替换，而不是要求精确匹配
- **安全扫描**：创建/修补前扫描 prompt injection 模式、不可见 unicode、凭证泄漏
- **人工审核缓冲期**：新创建的 Skill 标记为 `status: draft`，在 GUI Skill 面板顶部显示待审核列表，用户确认后切为 `status: active`。可配 `skills.autoReview.autoActivate: true` 跳过审核

**验收**：
- [ ] 使用 10 轮编码对话后，后台自动生成至少一个 draft Skill
- [ ] Skill 内容合理（非 trivial，有复用价值）
- [ ] 安全扫描拦截包含 `curl | bash` 的 Skill
- [ ] 模糊修补能在文本略有差异时成功匹配

**复杂度**：中。核心工作是提示词工程 + 模糊匹配 + 安全扫描，基础设施全部复用。

### B'2. 记忆 nudging（反省式学习）

**动机**：HA 的 `_MEMORY_REVIEW_PROMPT` 专门问"用户有什么偏好/期望/工作习惯？"。OC 的自动提取是"提取事实"，不是"反省学到了什么"。

**实施**：
- 在现有自动记忆提取 [`crates/oc-core/src/memory_extract.rs`](../../crates/oc-core/src/memory_extract.rs) 的 `side_query` 路径里**新增一个反省提示词**：
  > "回顾这段对话。关于用户，你学到了什么？他们的偏好、沟通风格、期望、工作习惯？把新发现保存为记忆（type: preferences / type: user_profile）。"
- 新增记忆类型 `user_profile`，与现有 `facts/preferences/instructions/context` 并列
- 新增 `USER_PROFILE.md` 或在现有记忆中标记 `scope: user`（参考 HA 的 `USER.md`）
- 触发：复用现有阈值触发机制（冷却 5min + 8K token / 10 条消息），额外新增一个"反省"提示词轮次

**复杂度**：低。在现有管线里加一个提示词 + 新记忆类型。

### B'3. 跨会话知识召回 + LLM 摘要

**动机**：OC 已有 FTS5 搜索但结果是原始 snippet。HA 的做法是：搜索命中 → 截取 ~100K chars 上下文 → 廉价模型摘要 → 返回结构化总结。

**实施**：
- 在 `recall_memory` / `session_search` 工具路径里新增可选的 LLM 摘要层
- 搜索结果 > 阈值（默认 3 条命中）时，自动触发 `side_query()` 摘要：
  > "以下是过去对话中与当前主题相关的片段。请整合为一个简洁摘要，提取关键经验教训和可操作信息。"
- 摘要结果作为 tool_result 返回，替代原始 snippet 列表
- 可选配置 `memory.recallSummary.enabled`（默认关闭，opt-in）

**复杂度**：低。在现有搜索结果上叠加 side_query 调用。

### B'4. 学习效果追踪 Dashboard

**动机**：自我学习的效果需要可观测——创建了多少 Skill？用了几次？记忆命中率？nudging 提取了什么？

**实施**：
- Dashboard 新增 "Learning" Tab：
  - Auto-created Skills 时间线（创建/修补/使用/淘汰）
  - Memory nudging 提取统计（每日新增 facts/preferences/user_profile）
  - 跨会话召回命中率
  - Skill 使用频率 Top N
- 后端在 skills/memory 操作时新增计数统计到 `dashboard` 模块

**复杂度**：中。

**Phase B' 总预估**：4–6 周。里程碑：自主 Skill 创建 + 记忆 nudging + 跨会话摘要发布。自我学习评分 1 → 4。

---

## Phase C：多 Agent 协作 & MCP 生态（8–12 周）

**Phase 目标**：补齐 MCP 生态接入和多 Agent 协作这两块 CC 领先的能力。完成后 OC 的"协议支持"从 2 推到 4，"Agent 协作"从 3 推到 4。

### C1. MCP Client SDK

**实施**：
- 新 crate `crates/oc-mcp/`（或作为 oc-core 子模块）
- 传输层：stdio + SSE + HTTP 三种
- JSON-RPC 2.0 实现
- 工具代理：MCP 工具在 `get_available_tools()` 返回时打上 `mcp:{server_name}` 前缀，执行时路由到对应 MCP server
- 资源访问：`mcp_list_resources` / `mcp_read_resource` 两个内置工具
- Prompt 模板桥接
- OAuth 认证流（复用现有 OAuth 模块）
- 前端设置面板新增 MCP Server 管理（URL / stdio 命令 / 环境变量 / OAuth）
- **官方注册表接入**：列出可用 MCP servers 供一键安装

**复杂度**：高。这是最大的一块工作。

### C2. MCP Skills 桥接

**实施**：MCP server 的 prompts 可以被解析为 Skill 格式并注入 skill 发现流程。依赖 C1 完成。

**复杂度**：中。

### C3. Hooks 系统

**动机**：EventBus 已经提供了基础设施，缺的是声明式配置和执行管线。

**实施**：
- 定义 Hook 事件：`PreToolUse` / `PostToolUse` / `pre-compact` / `post-compact` / `session-start` / `session-end` / `user-message` / `assistant-message`
- `settings.json` 新增 `hooks` 字段：
  ```json
  {
    "hooks": {
      "PreToolUse": [
        { "matcher": "tool.name == 'exec'", "command": "shellcheck {{args.cmd}}", "onFail": "deny" }
      ]
    }
  }
  ```
- 执行管线：EventBus 订阅 → 匹配 matcher → spawn shell → 根据 exit code + stdout 决定 allow/deny/ask/stop
- Hook 可写 `$HOOK_RESULT` JSON 控制下一步

**复杂度**：中。

### C4. Git Worktree 隔离（子 Agent 独立分支）

**实施**：
- `subagent` 工具新增 `worktree: true` 参数
- 调用 `git worktree add ~/.opencomputer/worktrees/{subagent_id}` 创建隔离分支
- 子 Agent 的 `cwd` 切到 worktree 路径
- 子 Agent 完成后：(1) 自动 merge / (2) 等待用户手动决定
- `git worktree remove` 在子 Agent 退出时触发

**复杂度**：中。

### C5. Agent 间双向实时消息

**实施**：`sessions_send` 扩展为双向，复用 mailbox 机制。源 Agent 可以阻塞等待对端回复（有超时）。

**复杂度**：中。

**Phase C 总预估**：8–12 周。MCP 是最大的一块，其他项共享基础设施。

---

## Phase D：体验与生态补足（持续，8+ 周）

**Phase 目标**：把 P3 清单里的小功能逐步补齐。每一项都是独立的小改动，可以穿插在 Phase A/B/C 之间做。

### 分组交付

**工具执行增强**
- D1. 流式工具执行 + 投机分类器（CC `StreamingToolExecutor`）
- D2. `read` context window 自适应截断

**权限模型扩展**
- D3. 多权限模式（acceptEdits / bypass / dontAsk）
- D4. 7 层规则来源（policy > flag > project > local > user > session > cliArg）
- D5. 拒绝追踪降级
- D6. 中断行为控制（cancel/block per tool）

**IDE 集成深化**
- D7. LSP 工具（goToDefinition/findReferences/hover）
- D8. REPL Bridge

**Provider 生态**
- D9. 补齐 LM Studio / Arcee / GitHub Copilot embedding
- D10. localModelLean 模式

**记忆细节**
- D11. Pin 置顶记忆
- D12. 记忆老化 + freshness notes

**多模态**
- D13. 语音输入（STT: Deepgram / Whisper）
- D14. 语音输出（TTS: ElevenLabs / Edge TTS）

**运维**
- D15. 自动更新
- D16. 热配置重载
- D17. OpenAI 兼容 HTTP API 端点（复用 oc-server）

---

## 时间线概览

```
周次    1  2  3  4  5  6  7  8  9  10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30
──────────────────────────────────────────────────────────────────────────────────────────────────────
Phase A  [===A1 Context Engine===][A3 profile][A4 SSRF][A5 fs-safe][A6 constant-time]
               [A2 Compact Provider]
Phase B               [====B1 Active Memory====][B2 SOUL][====B3 Dreaming====][B4 Memory trait][B5]
Phase B'                                  [=B'1 自主 Skill 创建=][B'2 nudging][B'3 召回摘要][B'4 Dashboard]
Phase C                                                    [======C1 MCP Client======][C2][C3 Hooks][C4 Worktree][C5]
Phase D   D1..D17 穿插在各 Phase 之间按需交付
```

**Phase A**：第 1–6 周
**Phase B**：第 5–12 周（与 A 末段重叠，B1 依赖 A1 完成）
**Phase B'**：第 7–12 周（与 B 并行，B'1 复用 async_jobs + SKILL.md，不依赖 A1）
**Phase C**：第 12–24 周
**Phase D**：全程穿插

## 评分预测

| 能力维度 | 当前 | Phase A 后 | Phase B 后 | Phase B' 后 | Phase C 后 |
|----------|:----:|:----------:|:----------:|:-----------:|:----------:|
| 上下文管理 | 5 | 5 | 5 | 5 | 5 |
| 权限安全 | 3 | **4** | 4 | 4 | **5** |
| 记忆系统 | 5 | 5 | 5（保持，甩开 OW/HA） | 5 | 5 |
| **自我学习** | **1** | 1 | 2 | **4** ⭐ | 4 |
| Agent 协作 | 3 | 3 | 3 | 3 | **5** |
| 协议支持 | 2 | 2 | 2 | 2 | **4** |
| **合计** | **67/85** | **68** | **69** | **72** ⭐ | **76** |

目标 **75–76/85**，继续保持领先 CC（63）、OW（53）和 HA（52）。**Phase B' 是 ROI 最高的阶段**——仅 4–6 周把最大 gap（自我学习 1→4）基本补上。

---

## 非目标（明确不做）

- **Companion App（iOS/Android）**：OW 有但 OC 定位是桌面应用，不追
- **IM 渠道继续扩展**：12 个已够用，不追 OW 的 25+
- **Honcho 托管服务对接**：Honcho 是 OW 选的服务，OC 不绑特定厂商，但保留 Memory Backend plugin 接口让用户自接
- **OpenAI 兼容 HTTP API 作为核心特性**：可以实现（D17），但不作为 OC 的定位之一

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| Context Engine trait 重构引入回归 | 充分快照测试 + 默认 engine 行为字节一致 + 渐进 rollout（配置切换） |
| MCP 实现成本估计不足 | C1 内部再拆子阶段（stdio 先上，SSE/HTTP/OAuth 分批）+ 不追 119K 行 CC 的完整度 |
| Active Memory 额外延迟影响体验 | 严格超时 + fallback 被动召回 + benchmark 监控 |
| 自主 Skill 创建质量不可控 | 安全扫描（注入/凭证泄漏）+ 人工审核缓冲期 + 质量评分自动淘汰 |
| Phase B/C 并行导致 trait 频繁变化 | A1 完成后冻结 trait 接口，后续只能加方法不能改签名 |

## 变更记录

- **2026-04-16** — v2.2：新增 Phase B'（自我学习闭环，对标 Hermes Agent），更新评分预测和时间线
- **2026-04-15** — 初始版本（配合 unified-comparison.md v2.1 发布）
