# 提示词缓存基准

这是本地显式运行的真实模型专项，入口是 Evaluation Center 的 **缓存成本与延迟专项**（`prompt-cache`）。它复用 `hope_core_scenario`、生产 `POST /api/chat`、真实工具和会话压缩接口；Harness 不调用 Provider、不实现第二套 Agent loop。它不加入 nightly 默认策略、普通 Cargo test、PR、pre-push 或 GitHub CI。

## 场景与判定

`HA-CACHE-001@1.0.0` 使用人工构造的背景、路径、状态与文件，按顺序观测九个阶段：

| 阶段 | 行为 | 内容与过程校验 |
| --- | --- | --- |
| `cold_history` | 首次发送带独立命名空间的历史 | 初始事实 JSON 精确匹配 |
| `warm_1` | 继续对话，加入可压缩旧背景 | 新增早期 `archiveKey`，精确匹配事实 |
| `warm_2` | 再次复用历史 | 保留早期事实 |
| `tail_update` | 尾部状态从 R1 更新为 R2 | 早期事实不变，状态必须更新 |
| `tool_rounds` | 先读取两个文件，再按结果读取第三个 | 至少三个工具调用、三个模型调用，三个标记正确 |
| `pre_summary` | 连续三轮复述事实 | 保留早期事实、最新状态、工具标记 |
| `summary` | 通过生产接口执行第 3 层摘要 | 必须实际应用第 3 层且减少上下文 |
| `post_summary` | 摘要后首次继续对话 | 完整恢复事实，不重新读取文件 |
| `post_summary_warm` | 摘要后再次复用 | 再次精确匹配事实 |

每个 attempt 在首条用户消息前加入随机合成命名空间，同一 attempt 的后续轮次保持它不变。这个操作只隔离历史前缀，**不能清空云端的系统提示词或工具缓存**；`cold_history` 表示首次发送该历史，是否命中始终取实际回执。重复运行也不假定缓存已过期。

尾部更新覆盖普通用户信息变化；系统注入数据随生产构建器运行，不伪造其缓存命中。手动摘要沿用生产策略；`summaryRequestModes` 显示实际使用 `native_prefix_fork` 还是 `independent`。例如未捕获可用前缀、容量不足或配置了摘要专用模型时，独立摘要仍是合法结果。该小场景不保证填满所有大窗口，也不等同于容量边界评测；边界与早期事实安全仍由 `context-compaction-safety` / `context-compaction` 覆盖。

### 自动摘要对照

`prompt-cache-auto` 画像与 suite 使用 `HA-CACHE-002@1.0.0`。它保留相同的合成事实与文件工具，通过连续对话触发生产高水位策略，完全不调用手动压缩接口。最终检查点的 `minTotalSummaryCalls=1` 要求已出现成功的摘要调用；未触发摘要不能算通过。事实和工具校验仍为阻断条件，摘要是否实际安装及其请求模式需结合生产遥测核对。

缩小模型在隔离配置中的 `context_window` 可以低成本覆盖自动摘要路径；必须记录相同的窗口、阈值、摘要模型和推理配置，不能将缩放窗口结果当作服务商实际满窗口压力测试。两版使用同一 Runner、资产、价格与凭据隔离修复；旧版保留真实回执采集，但恢复优化前的产品请求构建与摘要逻辑。不要在产品增加运行时回退开关。每版至少串行重复三次，可交错为旧、新、新、旧、旧、新；分别保留独立历史命名空间、产品二进制摘要、源码差异与所有失败。

画像不会自动缩放所选模型窗口。对照运行须由可信 owner 在凭据剥离后的隔离启动快照中显式设置窗口，保留原 owner 配置；若直接使用大窗口而未触发摘要，属于本场景条件未满足，不能据此认定产品摘要失败或声称优化已被覆盖。

## 运行

1. 在评测中心选择“缓存成本与延迟专项”、支持缓存的模型与预算，预览后显式开始。默认串行重复三次；不同 Provider、模型、上下文和推理配置应分别运行。预览与真实调用的凭据、临时数据目录、取消和预算边界沿用[真实模型评测](live-model-evaluation.md)。
2. 完成后在本地评测历史点击导出，得到 `.hope-eval.zip`。解压后按 `manifest.json` 选择 `campaigns/0000/evidence.json` 等模型 Campaign 文件（`eval-model-campaign.v1`）。`cache-report` 接受解压后的 trial、shard 或 campaign JSON；多模型可传多个文件，不直接读取 ZIP。不要同时输入 shard 与其聚合结果，以免重复计数。
3. 离线生成逐阶段报告：

```bash
cargo run -p ha-eval --locked -- model cache-report \
  --inputs /tmp/cache-campaign.json \
  --output /tmp/cache-report.json --summary /tmp/cache-report.md
```

费用需要显式传入 `--pricing /tmp/cache-pricing.json`。价格表格式为：

```json
{
  "schemaVersion": "cache-benchmark-pricing.v1",
  "models": [{
    "providerId": "your-provider-id",
    "modelId": "your-model-id",
    "basis": "核验日期、价格来源、币种 USD、缓存写入 TTL 和适用计价档位",
    "inputUsdPerMillion": 1.0,
    "outputUsdPerMillion": 2.0,
    "cacheReadUsdPerMillion": 0.1,
    "cacheWriteUsdPerMillion": 1.25
  }]
}
```

上面的数字只展示格式。应填入该模型、窗口、缓存 TTL 与计费档位的已核验单价；阶梯计价应按档位拆开运行/报告。仓库中的 `synthetic-pricing.json` 只给 fake smoke 验算使用，不能套在真实模型上。报告保存输入与价格表 SHA-256、价格说明和代码/模型/场景/运行配置身份。

### 零费用采集冒烟

构建并指定本次源码的两个二进制，避免误用旧 sidecar/server：

```bash
cargo build -p ha-eval -p ha-server --locked \
  --bin hope-agent-eval --bin hope-agent-server
target/debug/hope-agent-eval model cache-smoke \
  --server-bin target/debug/hope-agent-server --output /tmp/cache-smoke.json
target/debug/hope-agent-eval model cache-report \
  --inputs /tmp/cache-smoke.json \
  --pricing evals/live/suites/prompt-cache/synthetic-pricing.json \
  --output /tmp/cache-smoke-report.json --summary /tmp/cache-smoke-report.md
```

自动摘要采集冒烟使用同一假服务商：`model cache-smoke --suite prompt-cache-auto --context-window 48000 --server-bin <本次构建的 Server> --output /tmp/cache-auto-smoke.json`。窗口仅修改临时配置，命中回执仍是合成数据；正式对照窗口以运行计划和实测报告为准。

`cache-smoke` 用临时 `HA_DATA_DIR`、假凭据、本地 loopback Provider 与真实 Hope Server 执行同一个场景，包括真实文件工具与摘要。**缓存 Token 回执是固定合成 fixture**，报告标记为 `synthetic_fixture`；通过只证明采集、判分、聚合与费用算术通畅，不证明服务商缓存兼容、价格、延迟或优化收益。

## 指标口径

- 每个 `cache_checkpoint` 读取当前脱敏 telemetry，记录前后原始事件序号范围。最后追加 `benchmark.cache_phase` 闭合叶事件；它不改变产品关键路径。可选 `minTotalSummaryCalls` 只统计当前 attempt 的成功摘要回执，缺失或失败不计入。原始模型调用保留，缺失阶段之外的调用计入 `unassignedModelCalls`。
- `model.call` 的 `contextInputTokens` 是实际归一化上下文输入：Anthropic 为原始 input + cache read + cache write；OpenAI 为原始 input。不能用 Anthropic fresh-only 的 input 直接作命中率分母。`freshInputTokens`、cache read/write、output 都只取实际回执，缺失保持 `null`，实际零保留零。
- 缓存读取比例为 `cacheReadTokens / contextInputTokens`。费用为未缓存输入、缓存读取、缓存写入与输出各自乘对应单价；“无缓存反事实”把相同上下文输入全部按普通输入价计算。两者差值是**基于价格假设的建模收益**，可以为负，并非账单或一次真实无缓存对照调用。普通 Campaign 的标准输入/输出预算估值不冒充缓存费用。
- DeepSeek Chat 的 `prompt_cache_hit_tokens` 同样作为实际缓存读取回执。未报告的物理写入数量保持 `null`；若已核验写入与普通输入同价，未知写入数量在费用公式中相消，仍可计算费用，不把写入数量伪造成零。写入存在溢价且数量未知时，费用继续保持未知。
- 阶段墙钟包含该阶段对话/工具/摘要与 checkpoint 读取开销；每次模型调用另保留耗时、TTFT 样本。阶段 `operations` 列出调用类型，例如摘要前的记忆提取也计入 `summary`，不只计摘要请求。TTFT 从逐调用事件计算，不能对整场最小 TTFT 作差。摘要非流式调用可能没有 TTFT，样本数明确列出。
- P50/P95 使用 nearest-rank，分别保留实际样本数。默认三次只是诊断小样本；不同阶段输入与输出工作量不同，不直接宣称耗时差就是缓存加速。需要更多串行重复和相同设备、模型快照、上下文、工具、推理档位、网络及 TTL 才能评估趋势。
- 失败质量、失败调用、不完整归因及此前重试 attempt 全部保留。比较分布只纳入通过全部内容/过程校验、完整归因且当前阶段无失败调用的 attempt。原始失败、未覆盖阶段调用和缺失价格不会被抹去。
- 不同 commit、model digest、scenario digest、runtime config digest、实验臂和合成/真实模式分别分组。旧版/新版各导出一次证据，再把两份文件传给同一次 `cache-report --inputs ... ...`；按同阶段核对身份和工作量，不在产品里增加“恢复旧实现”开关，也不自动宣告显著收益。

## 维护

入口为 `crates/ha-eval/src/model_cache.rs`（离线报告）、`model_adapter.rs`（`scripted-user-flow.v3` 注册 checkpoint）和 `model_fake.rs`（零费用链路）。v1/v2 用户脚本保持原义；v2 不接受新动作。v3 的阶段名唯一且有界，真值只留在 Harness，禁止空阶段与任意命令。

suite/profile/scenario/fixture 变更按既有语义版本和 append-only `evals/live/version-lock.json` 管理。App 专项引用的 suite 与默认 policy suite 一样接受 schema、脚本和 digest 校验，无须加入默认 nightly 计划。检查与真实模型结果分开报告。
