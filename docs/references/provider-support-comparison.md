# Provider 支持对比分析：OpenComputer vs Claude Code vs OpenClaw

> 基线对比时间：2026-04-05 | 对应主文档章节：2.7

---

## 一、架构总览

| 维度 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **语言** | Rust (后端) + TypeScript (前端) | TypeScript (Bun) | TypeScript (Node/Bun) |
| **Provider 注册** | 28 个前端 GUI 模板 + 后端 4 种 ApiType 分发 | 单一 Anthropic SDK 客户端 + 环境变量切换 | Extension 插件注册制（92 个扩展目录） |
| **模型配置** | JSON 持久化 (`config.json`) + GUI 编辑 | 环境变量 + `settings.json` + `/model` 命令 | `models.json` 自动生成 + `config.yaml` 声明式 |
| **认证方式** | API Key 明文 / Codex OAuth | API Key / OAuth / AWS IAM / GCP ADC / Azure AD | Auth Profile 系统（API Key / OAuth Token / IAM） |
| **降级策略** | 模型链顺序降级 + 指数退避重试 | 529 连续失败后 Opus→Sonnet 降级 + 重试 | Auth Profile 轮转 + 模型候选列表降级 + 冷却探测 |
| **代理支持** | 三模式（System/None/Custom）+ macOS scutil 检测 | 环境变量 + fetchOptions 注入 | 依赖 Node/Bun 原生代理支持 |

---

## 二、OpenComputer 实现

### 2.1 四种 API 类型

OpenComputer 后端定义了四种 `ApiType`，每种对应独立的 Provider 实现文件：

| ApiType | 协议 | 实现文件 | 端点 |
|---------|------|---------|------|
| `Anthropic` | Anthropic Messages API | `agent/providers/anthropic.rs` | `/v1/messages` |
| `OpenaiChat` | OpenAI Chat Completions | `agent/providers/openai_chat.rs` | `/v1/chat/completions` |
| `OpenaiResponses` | OpenAI Responses API | `agent/providers/openai_responses.rs` | `/v1/responses` |
| `Codex` | ChatGPT Codex OAuth | `agent/providers/codex.rs` | `chatgpt.com/backend-api/codex` |

每种 Provider 实现统一的 Tool Loop 架构：请求 → 解析 tool_call → 并发/串行执行 → 回传 → 继续（最多 10 轮）。所有 Provider 共享：
- `run_compaction()` — Tier 1-3 上下文压缩
- `select_memories_if_needed()` — LLM 记忆语义选择
- `save_cache_safe_params()` — Side Query 缓存参数快照
- `normalize_history_for_*()` — 跨 Provider 历史消息格式归一化（支持 failover 后协议切换）

### 2.2 28 个 Provider 模板

前端提供 4 类共 28 个预置 Provider 模板（108 个预设模型），用户通过 GUI 一键创建：

**国际 Provider（7 个）**：
- Anthropic（Claude Sonnet/Opus/Haiku）
- OpenAI Responses API（GPT-4o/o3/o4-mini）
- OpenAI Chat API（同上，Chat Completions 协议）
- DeepSeek（V3/R1）
- Google Gemini（2.5 Pro/Flash）
- xAI（Grok 4/3 系列，5 个模型）
- Mistral（Large/Codestral/Devstral/Magistral/Medium/Small/Pixtral，7 个模型）

**国内 Provider（10 个）**：
- Moonshot AI / Kimi（K2.5/K2 Thinking 系列）
- 通义千问 Qwen（Max/Plus/Turbo/QwQ，ThinkingStyle=Qwen）
- 火山引擎 / 豆包（Seed 1.8/Kimi/GLM/DeepSeek）
- 智谱 AI / Z.AI（GLM-5/4.7/4.6/4.5 系列，9 个模型，ThinkingStyle=Zai）
- MiniMax（M2.7/VL/M2.5，Anthropic 兼容协议）
- Kimi Coding（Anthropic 兼容协议）
- 小米 MiMo（V2 Pro/Omni/Flash）
- 百度千帆（DeepSeek V3.2/ERNIE 5.0）
- ModelStudio / DashScope（Qwen 3.5/Coder/MiniMax/GLM/Kimi 聚合）

**基础设施 / 聚合 Provider（8 个）**：
- OpenRouter（多模型聚合，7 个预设）
- Groq（LPU 推理）
- NVIDIA AI Endpoints
- Together AI（8 个模型）
- Hugging Face Inference API
- BytePlus 海外火山
- Chutes TEE
- Cloudflare AI Gateway

**本地 / 自托管 Provider（4 个）**：
- LiteLLM（`127.0.0.1:4000`）
- Ollama（`127.0.0.1:11434`，含云端模型）
- vLLM（`127.0.0.1:8000`）
- LM Studio（`127.0.0.1:1234`）

每个模板包含完整的模型元数据：`id`、`name`、`inputTypes`（text/image/video）、`contextWindow`、`maxTokens`、`reasoning`、`costInput`/`costOutput`（$/M tokens）。

### 2.3 模型链降级策略

`failover.rs` 实现了基于错误分类的降级系统：

**错误分类（FailoverReason）**：

| 分类 | 触发条件 | 行为 |
|------|---------|------|
| `RateLimit` | 429/rate_limit/throttle | **可重试** — 同模型指数退避 |
| `Overloaded` | 503/502/521/522/524 | **可重试** — 同模型指数退避 |
| `Timeout` | timeout/econnreset/econnrefused | **可重试** — 同模型指数退避 |
| `Auth` | 401/403/unauthorized | **跳下一模型** |
| `Billing` | 402/quota/insufficient | **跳下一模型** |
| `ModelNotFound` | 404/model_not_found | **跳下一模型** |
| `ContextOverflow` | context_length_exceeded | **触发压缩** — 非终端，执行 compaction 后重试 |
| `Unknown` | 其他 | **跳下一模型** |

**重试策略**：指数退避 `base_ms * 2^attempt`，上限 `max_ms`，加 ±10% 随机抖动。

**模型链解析**（`resolve_model_chain`）：Agent 级配置优先于全局配置。全局 `fallback_models` 列表按序尝试。

### 2.4 Prompt Cache

通过 Side Query 机制实现缓存复用：
- `save_cache_safe_params()` 在每轮主请求 compaction 后快照 system_prompt + tool_schemas + conversation_history
- 侧查询（Tier 3 摘要、记忆提取）构建字节一致的前缀请求
- Anthropic 显式 prompt caching / OpenAI 自动前缀缓存
- 成本降低约 90%

### 2.5 Extended Thinking

通过 `ThinkingStyle` 枚举支持 5 种 Thinking 参数格式：

| ThinkingStyle | 格式 | 适用 Provider |
|---------------|------|--------------|
| `Openai` | `reasoning_effort: "low"/"medium"/"high"` | OpenAI 系（默认） |
| `Anthropic` | `thinking: { type: "enabled", budget_tokens: N }` | Anthropic / MiniMax / Kimi Coding |
| `Zai` | 同 Anthropic（预留） | 智谱 Z.AI |
| `Qwen` | `enable_thinking: true` | 通义千问 / ModelStudio |
| `None` | 不发送任何参数 | 不支持的 Provider |

### 2.6 温度三层覆盖

```
会话级 temperatureOverride > Agent 级 agent.json → model.temperature > 全局 config.json → temperature
```

范围 0.0-2.0，`None` 表示使用 API 默认值。在四种 Provider 的 API 请求中统一注入。

---

## 三、Claude Code 实现

### 3.1 API 客户端架构

Claude Code 采用单一 Anthropic SDK 客户端架构（`@anthropic-ai/sdk`），通过环境变量和运行时配置切换部署方式：

**核心文件**：
- `services/api/client.ts` — `getAnthropicClient()` 工厂函数，根据环境变量创建不同后端的 Anthropic 客户端
- `services/api/claude.ts` — 主请求逻辑（Beta API 消息流式处理）
- `services/api/withRetry.ts` — 重试包装器
- `services/api/errors.ts` — 错误分类与用户消息

**SDK 客户端创建流程**：
1. 构建 `defaultHeaders`（`x-app: cli`、`User-Agent`、Session ID 等）
2. OAuth Token 自动检查刷新（`checkAndRefreshOAuthTokenIfNeeded`）
3. 非订阅用户追加 API Key 相关 header
4. 注入代理设置（`getProxyFetchOptions`）
5. 根据 `CLAUDE_CODE_USE_BEDROCK` / `CLAUDE_CODE_USE_VERTEX` / `CLAUDE_CODE_USE_FOUNDRY` 环境变量切换后端

### 3.2 模型配置与验证

**模型解析优先级**（`getUserSpecifiedModelSetting`）：
1. `/model` 命令运行时覆盖（最高优先级）
2. `--model` 启动参数
3. `ANTHROPIC_MODEL` 环境变量
4. `settings.json` 中保存的 model 设置
5. 内置默认值

**模型别名系统**：
- `sonnet` → Claude Sonnet 4.6（1P）/ Sonnet 4.5（3P）
- `opus` → Claude Opus 4.6
- `haiku` → Claude Haiku 4.5
- `sonnet[1m]` / `opus[1m]` — 1M 上下文版本
- `opusplan` — Plan Mode 用 Opus，其他用 Sonnet

**分层定价显示**：
- Max/Team Premium 用户：Opus 默认，Sonnet 备选
- Pro/Team Standard/Enterprise：Sonnet 默认，Opus 额外计费
- PAYG 1P：显示 $/M tokens 价格
- PAYG 3P：支持自定义模型字符串覆盖

**模型能力缓存**（`modelCapabilities.ts`）：
- 仅 Ant 用户 + firstParty 适用
- 调用 `anthropic.models.list()` 获取 `max_input_tokens` / `max_tokens`
- 本地缓存在 `~/.claude/cache/model-capabilities.json`

### 3.3 重试与降级（withRetry）

`withRetry` 是一个 AsyncGenerator，支持在重试等待期间 yield `SystemAPIErrorMessage` 给前端展示：

**重试参数**：
- `DEFAULT_MAX_RETRIES = 10`（可通过 `CLAUDE_CODE_MAX_RETRIES` 环境变量覆盖）
- `BASE_DELAY_MS = 500`，指数退避 + 25% 随机抖动，上限 32s
- 支持 `retry-after` 响应头

**529 过载错误特殊处理**：
- `MAX_529_RETRIES = 3` — 连续 3 次 529 后触发 Opus→Sonnet 降级
- `FOREGROUND_529_RETRY_SOURCES` 白名单 — 仅前台查询重试，后台查询直接失败
- `FallbackTriggeredError` 通知调用方切换模型

**Fast Mode 降级**：
- 短 `retry-after`（<20s）：保持 Fast Mode 重试（保留 prompt cache）
- 长 `retry-after`：进入冷却期（10-30 分钟），切换到标准速度
- API 明确拒绝 Fast Mode（400）：永久禁用

**持久重试模式**（`CLAUDE_CODE_UNATTENDED_RETRY`）：
- 429/529 无限重试，退避上限 5 分钟
- 每 30 秒 yield 心跳防止 session 空闲超时
- 6 小时绝对上限

**认证错误恢复**：
- 401 → 刷新 OAuth Token 后重试
- 403 "token revoked" → 同上
- Bedrock 403 / CredentialsProviderError → 清除 AWS 缓存后重试
- Vertex 401 / google-auth-library 错误 → 清除 GCP 缓存后重试
- ECONNRESET/EPIPE → 禁用 keep-alive 后重试

**上下文溢出处理**：
- 解析 400 错误中的 `input_tokens + max_tokens > context_limit`
- 动态调整 `maxTokensOverride`，保证至少 3000 输出 token

### 3.4 Bedrock/Vertex/Foundry 支持

Claude Code 通过环境变量无缝切换 4 种部署方式：

| 部署方式 | 环境变量 | 认证 | 区域 |
|---------|---------|------|------|
| **First Party** | 默认 | `ANTHROPIC_API_KEY` 或 OAuth | N/A |
| **AWS Bedrock** | `CLAUDE_CODE_USE_BEDROCK=1` | AWS IAM（aws-sdk 默认链） | `AWS_REGION`，模型级覆盖 |
| **Google Vertex** | `CLAUDE_CODE_USE_VERTEX=1` | GCP ADC（google-auth-library） | `CLOUD_ML_REGION`，模型级 `VERTEX_REGION_*` |
| **Azure Foundry** | `CLAUDE_CODE_USE_FOUNDRY=1` | `ANTHROPIC_FOUNDRY_API_KEY` 或 Azure AD | 资源名嵌入 URL |

3P Provider 的模型默认值有意保守（如 Sonnet 默认 4.5 而非 4.6），因为 3P 可用性滞后于 firstParty。

### 3.5 OAuth 认证

- Claude.ai 订阅用户通过 OAuth 登录
- `isClaudeAISubscriber()` 区分订阅用户与 PAYG API 用户
- 订阅层级：Free / Pro / Max / Team Standard / Team Premium / Enterprise
- Max/Team Premium 用户 429 不重试（非 PAYG 额度限制）
- Enterprise 用户 429 可重试（通常使用 PAYG）

---

## 四、OpenClaw 实现

### 4.1 Extension 插件化 Provider

OpenClaw 采用插件架构，92 个 `extensions/` 目录中包含大量 Provider 和功能扩展：

**LLM Provider 扩展**（按类别）：

| 类别 | 扩展名 | 注册方式 |
|------|--------|---------|
| 核心 | `anthropic`, `openai`, `google` | `definePluginEntry()` + `api.registerProvider()` |
| 云平台 | `amazon-bedrock`, `anthropic-vertex`, `microsoft-foundry` | 同上 |
| 第三方 | `deepseek`, `groq`, `mistral`, `xai`, `zai`, `nvidia`, `together`, `venice`, `perplexity`, `huggingface`, `ollama`, `vllm`, `sglang` | 同上 |
| 国内 | `moonshot`, `volcengine`, `byteplus`, `qianfan`, `xiaomi`, `minimax`, `kilocode`, `kimi-coding`, `stepfun`, `modelstudio` | 同上 |
| 聚合网关 | `openrouter`, `litellm`, `cloudflare-ai-gateway`, `vercel-ai-gateway`, `copilot-proxy` | 同上 |
| 代码专用 | `opencode`, `opencode-go`, `openshell`, `github-copilot` | 同上 |
| 特殊 | `chutes`(TEE), `lobster`, `synthetic` | 同上 |

每个 Provider 扩展通过 `definePluginEntry` 注册，支持：
- `api.registerProvider(buildXXXProvider())` — 注册 LLM Provider
- `api.registerCliBackend(buildXXXCliBackend())` — 注册 CLI 后端
- `api.registerSpeechProvider()` — 语音
- `api.registerMediaUnderstandingProvider()` — 多模态理解
- `api.registerImageGenerationProvider()` — 图片生成
- `api.registerRealtimeVoiceProvider()` — 实时语音
- `api.on("before_prompt_build", ...)` — Prompt 钩子

**示例 — OpenAI 扩展注册**：
```typescript
api.registerProvider(buildOpenAIProvider());
api.registerProvider(buildOpenAICodexProviderPlugin());
api.registerCliBackend(buildOpenAICodexCliBackend());
api.registerSpeechProvider(buildOpenAISpeechProvider());
api.registerImageGenerationProvider(buildOpenAIImageGenerationProvider());
// + 2 个 MediaUnderstanding + 1 个 RealtimeTranscription + 1 个 RealtimeVoice
```

**示例 — Google Gemini 扩展**：
- 主 Provider + CLI Provider（`google-gemini-cli` — Gemini OAuth）
- 图片生成 Provider
- 多模态理解 Provider（图片/音频/视频）
- Web Search Provider
- 模型 ID 规范化（`normalizeGoogleModelId`）
- 重放策略钩子（`buildGoogleGeminiProviderHooks`）

### 4.2 Auth Profile 系统

OpenClaw 实现了精细的多凭证管理系统：

**AuthProfileCredential 类型**：
- `api_key` — 传统 API Key（自动 `normalizeSecretInput` 去除空白）
- `token` — OAuth Token
- 其他类型保留

**核心功能**：
- `upsertAuthProfile()` — 创建/更新凭证 profile
- `setAuthProfileOrder()` — 设置 Provider 级 profile 优先级
- `listProfilesForProvider()` — 列出某 Provider 下所有 profile
- `markAuthProfileGood()` — 记录最近成功的 profile（`lastGood`）
- `updateAuthProfileStoreWithLock()` — 文件锁保护的原子更新

**Profile 轮转机制**（`order.ts`）：
- Provider 配置指定 profile 优先级列表
- 失败时按 `cooldown` 冷却 + 轮转到下一个 profile
- `lastGood` 记录用于快速恢复

### 4.3 模型配置

**默认值**（`defaults.ts`）：
```typescript
DEFAULT_PROVIDER = "openai";
DEFAULT_MODEL = "gpt-5.4";
DEFAULT_CONTEXT_TOKENS = 200_000;
```

**模型选择系统**（`model-selection.ts`）：
- `ModelRef = { provider, model }` — 规范化的模型引用
- `ThinkLevel` = `"off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "adaptive"`
- `ModelAliasIndex` — 别名 → ModelRef 映射
- `resolveConfiguredModelRef()` — 从配置解析模型引用
- `buildConfiguredAllowlistKeys()` — 白名单过滤

**models.json 自动生成**（`models-config.ts`）：
- 根据 `config.yaml` + `auth-profiles.json` + 环境变量自动构建
- 指纹校验避免重复写入
- 文件锁（`withModelsJsonWriteLock`）防并发
- 原子写入（先写临时文件再 rename）
- 权限 0o600

**模型目录缓存**（`model-catalog.ts`）：
- 类似 Claude Code 的 `modelCapabilities.ts`
- 从 API 或本地缓存获取模型能力信息

### 4.4 降级系统

**FailoverReason 类型**（10 种）：

| 分类 | 处理策略 |
|------|---------|
| `auth` | 跳过当前 profile |
| `auth_permanent` | 永久禁用 profile |
| `format` | 跳过当前模型（400 请求格式错误） |
| `rate_limit` | 冷却 + 轮转 profile/模型 |
| `overloaded` | 冷却 + 轮转 |
| `billing` | 跳过当前 profile |
| `timeout` | 轮转（也可冷却探测） |
| `model_not_found` | 跳过当前模型 |
| `session_expired` | 会话过期，跳过 |
| `unknown` | 冷却 + 轮转 |

**冷却探测策略**（`failover-policy.ts`）：
- `shouldAllowCooldownProbeForReason` — rate_limit/overloaded/billing/unknown 允许冷却后探测
- `shouldUseTransientCooldownProbeSlot` — rate_limit/overloaded/unknown 使用瞬态探测槽
- `shouldPreserveTransientCooldownProbeSlot` — model_not_found/format/auth/session_expired 保留探测槽

**模型降级流程**（`model-fallback.ts`）：
1. 收集候选模型列表（primary + fallbacks + allowlist 过滤）
2. 按序尝试每个候选
3. 用户中断（AbortError 非 timeout）直接抛出
4. FailoverError 记录后继续下一候选
5. 所有候选耗尽 → 抛出 `FallbackSummaryError`（含每次尝试详情 + 最早冷却到期时间）

---

## 五、逐项功能对比

| 功能 | OpenComputer | Claude Code | OpenClaw |
|------|-------------|-------------|----------|
| **支持的 LLM Provider 数** | 28 模板（4 种协议） | 1 个（Anthropic SDK，4 种部署） | 40+ Provider 扩展 |
| **预设模型数** | 108 | ~10（Opus/Sonnet/Haiku 各版本） | 动态（按扩展 + 配置） |
| **自定义 Provider** | GUI 创建任意 OpenAI-compatible | 仅 `ANTHROPIC_BASE_URL` 自定义 | `config.yaml` 任意声明 |
| **非 Anthropic 模型** | 原生支持（OpenAI/Gemini/国产 20+ 家） | 不支持 | 原生支持（40+ 家） |
| **Anthropic 原生协议** | 支持 | 支持（唯一协议） | 支持 |
| **OpenAI Chat 协议** | 支持 | 不支持 | 支持（含 Codex） |
| **OpenAI Responses 协议** | 支持 | 不支持 | 支持 |
| **AWS Bedrock** | 不支持（可通过自定义 URL） | 原生支持 | 原生支持（扩展） |
| **Google Vertex** | 不支持 | 原生支持 | 原生支持（扩展） |
| **Azure Foundry** | 不支持 | 原生支持 | 原生支持（扩展） |
| **Codex OAuth** | 原生支持（ChatGPT 订阅） | 不支持 | 原生支持（扩展） |
| **本地模型** | 4 个模板（Ollama/vLLM/LM Studio/LiteLLM） | 不支持 | 原生支持（Ollama/vLLM/sglang） |
| **国产模型** | 10 个专属模板 | 不支持 | 10+ 扩展 |
| **模型降级链** | 有序列表 + 7 种错误分类 | 仅 Opus→Sonnet（529 触发） | 有序列表 + 10 种错误分类 + Auth Profile 轮转 |
| **重试策略** | 指数退避（2 次），±10% 抖动 | 指数退避（10 次），25% 抖动 + retry-after | 冷却探测 + Profile 轮转 |
| **上下文溢出处理** | 触发 5 层渐进压缩 | 动态调整 max_tokens + microcompact | 检测后报告 |
| **Prompt Cache** | Side Query 缓存共享 | Beta header + cache scope | Provider 级钩子 |
| **Thinking 参数** | 5 种 ThinkingStyle | Anthropic 原生 | 7 级 ThinkLevel |
| **温度控制** | 三层覆盖（会话>Agent>全局） | 环境变量 + 设置 | 配置文件 |
| **多凭证管理** | 每 Provider 单 Key | 环境变量 + OAuth | Auth Profile 系统（多 Key 轮转） |
| **代理支持** | System/None/Custom + macOS scutil | 环境变量 + fetchOptions | Node/Bun 原生 |
| **API Key 安全** | 前端脱敏（`****`）+ 禁止日志 | 不存储（环境变量） | 文件权限 0o600 + normalizeSecretInput |
| **GUI 配置** | 完整 GUI（模板选择/模型管理/连通性测试） | CLI 交互（`/model` 命令） | CLI + 配置文件 |

---

## 六、差距分析与建议

### 6.1 OpenComputer 优势

1. **GUI 用户体验**：28 个预置模板 + 可视化配置，面向非技术用户的入门门槛最低
2. **中国市场覆盖**：10 个国内 Provider 模板是独特优势，Claude Code 和 OpenClaw 均不覆盖百度千帆、小米 MiMo 等
3. **协议多样性**：4 种 API 协议原生支持，一个客户端兼容绝大多数 LLM API
4. **ThinkingStyle 适配**：5 种 Thinking 参数格式覆盖所有主流 Provider
5. **上下文溢出处理**：ContextOverflow → 自动压缩 → 重试，而非直接失败

### 6.2 OpenComputer 差距

1. **企业云部署**：缺少 Bedrock/Vertex/Foundry 原生支持。建议：
   - 添加 AWS IAM 签名认证（参考 Claude Code 的 `refreshAndGetAwsCredentials`）
   - 添加 GCP ADC 认证（参考 `google-auth-library`）
   - 低优先级：可通过自定义 base_url 部分替代

2. **多凭证轮转**：当前每个 Provider 仅支持单个 API Key。建议：
   - 参考 OpenClaw 的 Auth Profile 系统，支持同 Provider 多 Key
   - 实现 Key 级冷却和轮转策略
   - 对于高频使用场景（IM Channel）尤为重要

3. **重试次数偏保守**：当前仅重试 2 次（vs Claude Code 10 次）。建议：
   - 可重试错误（429/503/timeout）增加重试上限
   - 添加 `retry-after` 响应头解析
   - 参考 Claude Code 的无人值守持久重试模式

4. **动态模型发现**：所有模型为静态预置。建议：
   - 参考 Claude Code 的 `modelCapabilities.ts`，从 API 动态获取可用模型列表
   - 参考 Ollama 的 `/api/tags` 自动发现本地模型

5. **错误分类精度**：当前基于字符串子串匹配。建议：
   - 优先匹配 HTTP 状态码（数字精确匹配而非字符串包含 "429"）
   - 添加 Provider 特有错误模式（参考 OpenClaw 的 `provider-error-patterns.ts`）
   - 区分 `auth` 和 `auth_permanent`（临时 vs 永久认证失败）

### 6.3 建议优先级

| 优先级 | 改进项 | 预估工作量 | 影响面 |
|--------|--------|-----------|--------|
| P1 | retry-after 头解析 + 重试次数可配置 | 小 | 可靠性提升 |
| P1 | HTTP 状态码精确匹配 | 小 | 降级准确性 |
| P2 | 同 Provider 多 Key 支持 | 中 | IM Channel 高频场景 |
| P2 | Ollama/vLLM 模型自动发现 | 中 | 本地模型用户体验 |
| P3 | Bedrock/Vertex 原生认证 | 大 | 企业用户 |
| P3 | 动态模型能力查询 | 中 | 模型参数准确性 |
