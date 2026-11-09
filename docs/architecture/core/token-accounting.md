# Token Accounting

本文记录模型请求的 token 预测、Provider 真实用量归一化和误差校正契约。实现入口是 `crates/ha-core/src/token_accounting/`；容量决策不得自行复制字符比率。

## 两个平面

请求前的 prediction 用于上下文容量、压缩和工具输出预算；请求后的 accounting 用于消息、Dashboard、成本和校正。二者不能互相冒充：Provider preflight count 仍是预测，只有 sampling 响应的 usage 是实际用量。

`TokenCount` 同时携带 `lower_bound <= estimated <= upper_bound`、source、confidence、tokenizer id/revision、request shape、分类 breakdown 和 unknown modality。容量放行、压缩与输出预算只使用 `upper_bound`；`lower_bound` 用于识别可能过早压缩的歧义区间。

## 解析与降级顺序

1. 请求完成后，以 Provider sampling usage 为实际值。
2. 请求前处于临界区间时，优先使用 Provider count endpoint。
3. 已登记 OpenAI/Codex 模型使用内置离线 `o200k_base` 或 `cl100k_base`。
4. 未知模型使用 Unicode、JSON、代码标点和 modality 感知 heuristic。
5. 兼容包装器只保留 model-neutral conservative fallback，不再把 `String::len()/4` 当作主计数器。

Tokenizer registry 是版本化代码表：exact/prefix 规则要求模型边界，未知模型不继承相似名称。词表随二进制发布，运行时不下载 tokenizer 文件、不执行远程代码。v1 没有用户设置项；如将来暴露策略，必须同步 GUI、`ha-settings` 和风险登记。

## Provider preflight

主请求总是先做本地计数。只有阈值落入 `[lower_bound, upper_bound]` 或包含未知 modality 时，adapter 才尝试远程计数：

| Adapter | 端点 |
|---|---|
| Anthropic Messages | `POST /v1/messages/count_tokens` |
| OpenAI Responses | `POST /v1/responses/input_tokens` |
| OpenAI Chat / Codex | v1 不调用远程 count |

Count body 从同一个 round request builder 派生，只移除 sampling/stream 字段。调用受 800ms 上限、cancel、64KiB 响应上限和统一 SSRF 检查约束。404/405/501 可缓存为 endpoint 不支持；401/403 只短时抑制当前凭据指纹，不能污染 Provider 能力。并发 probe 由 endpoint singleflight 闸去重。失败或超时回退本地预测，不阻断 sampling；确认溢出返回 `PreflightOverflow`，由既有 `ContextOverflow` 压缩路径处理，不进入 Provider cooldown。

## Usage 与 coverage

Provider 未返回 usage 和真实返回 `0` 是不同状态。`ChatUsage` 分别维护 `input_coverage` / `output_coverage`：`complete`、`partial`、`missing`。缺失字段不会写入 usage stream event，也不会以 `0` 落入 `model_usage_events`。

归一化口径保持不变：

- Anthropic：`context_input = input + cache_creation + cache_read`；
- OpenAI：`context_input = input`，cache read 是 input 子集；
- `fresh_input = context_input - cache_read`。

工具循环的累计值与最近一轮值仍分开。Goal、Loop、ACP 和现有成本口径不得因 tokenizer 迁移而改成另一种累计定义。

缓存诊断另保留 `ChatUsage.cache_receipt` 的可选实际值，避免兼容数值计数器把缺失的缓存字段当作实际零。Anthropic 只有完整输入与缓存桶才给出精确 context；OpenAI 的完整 input 可直接给出 context，但缺少缓存读取回执时 fresh 保持未知。主调用的评测事件，以及摘要/侧查询的用量台账记录，使用这些实际值生成脱敏 `model.call`；逐调用 TTFT 和摘要请求模式供[缓存基准](../agent/cache-benchmark.md)使用，不以标准输入价预算估值冒充缓存账单。

## 校正与持久化

校正 key 包含 Provider family、model、request shape、tokenizer id、registry version 和 modality。中心值使用 EMA（alpha 0.2），每 key 最多保留 64 个近期 ratio，并以 p05/p95 形成上下界；ratio 钳在 `[0.5, 4.0]`。

每个 sampling round 生成一个不含正文的 `TokenAccountingObservation`：operation key、预测区间、raw estimate、tokenizer revision、input/output coverage、actual input 和该轮 output reservation。Partial / Missing 预算只对缺失轮次补 prediction upper / output reservation，不重复累计已有 actual。一个 turn 的 observations 放进最终 chat usage 行的 `metadata.tokenAccounting.observations`，不会额外插入计费行，因此 Dashboard 不会重复累计调用或 token。进程首次非无痕请求通过 `SessionDB::run` 有界加载最近 256 行，并按时间正序重放 EMA；无痕会话既不持久化，也不更新进程级校正桶。

## 性能与隐私

- tiktoken 使用进程单例；不可变 text/JSON part 通过 2048 项 LRU 缓存，key 只有 tokenizer revision、长度和内存内 digest。
- `count_local()` 只做 CPU/内存工作，不读取 SQLite、不联网。
- 日志只记录计数、source、revision、unknown 数量和短指纹，不记录 prompt、tool arguments、API Key 或 count response body。
- 图片、文件、音频不把 Base64/原始字节交给 text tokenizer；使用 modality 上界并登记 unknown。

## 关键文件

| 文件 | 职责 |
|---|---|
| `token_accounting/types.rs` | 预测区间、coverage、observation wire 类型 |
| `token_accounting/resolver.rs` | 版本化 model → tokenizer registry |
| `token_accounting/tokenizer.rs` | 离线 tiktoken backend 与 panic 隔离 |
| `token_accounting/heuristic.rs` | Unicode/JSON/modality fallback |
| `token_accounting/calibration.rs` | EMA 与 p05/p95 上下界 |
| `token_accounting/capability_cache.rs` | endpoint capability、singleflight、profile suppression |
| `token_accounting/part_cache.rs` | content-free bounded part cache |
| `token_accounting/service.rs` | 唯一门面、预热与 compaction counter |
| `agent/token_manifest.rs` | Provider request 分类诊断与 `/context` 快照 |

### Kimi Basic 搜索按次入账

`web_search.kimi.basic` 是独立搜索 REST 请求，`model_id` 和 Token 字段保持缺失。非空 HTTP 200 的官方计费资格、空结果、非200及未知响应分别记录，不以有效链接数量或字符数估算实际用量/费用。取消保持未知，沿用会话/子代理归属与无痕拒写。详见[工具合同](tool-system.md)。

`web_search.perplexity` 在现有 Sonar 请求的发送、限量读取、解析和取消期间统一收尾。协议错误标记失败；若已收到合法 JSON 中的实际用量，仍保留 Token 字段。无响应或用量缺失保持未知，实际零保留为零；响应来源数量不推算费用。会话/子代理归属与无痕拒写继续由统一台账裁决。

评测内的单次侧查询在请求接纳后通过 `eval_context::retain_model_call` 保留会话身份，直至成功或失败用量写入完成。该保留不占用新的 Agent 并发槽；即使调用方未保留后台生命周期，评测根关闭也必须等实际调用结束。注册与关闭的竞争在锁内再次校验，身份缺失时请求前拒绝；普通非评测调用保持原有计量行为。

单次模型请求在进入适配器前启用 `ModelUsageAttemptGuard`。正常结束只写一次实际覆盖的用量；已启用请求被取消或超时时写入用量未知及 callerCancelled 标记，未进入请求的准备阶段不虚构调用。真实会话已删除且无隐私真相源时不恢复其取消用量持久化；合法的后台合成键仍按既有聚合规则记账。
