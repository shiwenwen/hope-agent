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
