# 专项能力评测基础设施

## 目标与当前边界

完整能力评测不属于 PR 单测。默认 `cargo test -p ha-core -p ha-server` 只守快速、局部、确定性的代码契约；Coding、Domain、Dreaming、Memory Retrieval 的整包回放由独立 `hope-agent-eval` 在开发者本机显式执行。

当前阶段不在 GitHub Actions、PR、pre-push 或发布 workflow 中运行专项评测，也不上传、查询或校验评测 evidence。产品内 Dashboard、owner API、Campaign、Sidecar、本地历史和 CLI 保持可用；结果用于本地诊断、回归比较和人工发版判断，不构成自动发布门禁。

## 组成

| 位置 | 职责 |
| --- | --- |
| `crates/ha-eval-spec` | 不依赖 `ha-core` 的 manifest、policy、plan、shard、evidence、waiver 类型，canonical JSON、SHA-256、路径与 JSON Schema 校验 |
| `crates/ha-eval` | `hope-agent-eval` CLI；创建计划、稳定分片、逐 case 子进程隔离、聚合与 evidence 校验 |
| `evals/` | JSON Schema、policy、suite manifest 和 fixture 单一真相源 |
| 桌面 Evaluation Center | 通过随包 Sidecar 显式运行本地真实模型评测，并保存进度、结果、历史、对比和趋势 |

仓库当前不包含 `capability-eval.yml` 或其他远端专项评测编排。`weekly`、`release` tier 与相关 evidence 字段仍作为可复用的本地计划/协议保留，但不会自动定时运行，也不会被 `release.yml` 消费。

## 本地运行

```bash
# 校验 schema、policy、suite 和 fixture，不调用模型
cargo run -p ha-eval --locked -- validate

# 生成不可变计划
cargo run -p ha-eval --locked -- plan \
  --tier weekly --ref <40位commit-sha> --output plan.json

# 按 suite/shard 执行；所有 v1 adapter 都是确定性的，不调用模型 API
cargo run -p ha-eval --locked -- run \
  --plan plan.json --suite <id> --shard 1/2 --output shard.json

# 聚合并校验本地产物
cargo run -p ha-eval --locked -- aggregate \
  --plan plan.json --inputs <dir> \
  --output eval-evidence.v1.json --summary eval-summary.md
```

开发者也可以直接使用已构建的 `hope-agent-eval` 二进制。真实模型 Campaign 使用 `hope-agent-eval model ...`，边界见 [`live-model-evaluation.md`](live-model-evaluation.md)。

## 确定性与安全契约

- v1 只允许 `coding_fixture_patch`、`coding_gold_fixture_patch`、`domain_trace_fixture`、`dreaming_golden`、`memory_retrieval_scale`。
- manifest 不能携带任意 shell 命令；fixture 只能使用 suite 目录内的普通相对路径，canonicalize 后越界或 symlink escape 一律拒绝。
- Runner 在启动 case 前移除 API key/token 环境变量；fixture 出现非空 Provider/model/model id/model chain/API key/endpoint、`agent`、`external_model` 或 `mock_provider` 配置时 fail-fast。
- case 使用稳定 SHA-256 分片，在独立子进程运行；超时、崩溃或无结果为 `infra_error`，只自动重试一次，业务断言失败不重试。
- suite/case/policy 以 canonical JSON 和资产内容生成 digest。`evals/version-lock.json` 已有 `id@version` 不得删除或覆写；内容变化必须提升版本并追加 lock。
- 修改 lock 后在本地运行 `node scripts/verify-eval-version-lock.mjs --base <base-sha>`，并在代码审查中确认 append-only。GitHub CI 当前不执行这项评测专用校验。
- Memory latency 只作提示，质量和召回正确性仍是功能断言。

`ha-eval` 默认启用 `full-runner`，完整确定性 adapter 不链接进普通 `ha-core` / `ha-server` 测试。此前迁出的 `eval-internal-tests` 继续保持 opt-in，不回到默认 Cargo test。

## 本地网络与证据语义

确定性 adapter 本身不调用模型 API。某些 Coding fixture 可以执行经过审阅的本地验证命令；这与模型网络访问无关。

`networkPolicy=deny` 只有在真实 OS sandbox 或 Linux network namespace 中才构成出站隔离。仅设置 `HA_EVAL_NETWORK=deny` 不能证明本机已经断网；需要验证隔离时，应在本机提供真实网络 namespace，并设置 `HA_EVAL_REQUIRE_NETWORK_ISOLATION=1` 让 Runner 校验网卡集合。

本地可以运行 dirty worktree 并生成 JSON/Markdown，用于定位失败或比较相同 commit、policy、suite digest 下的功能结果。当前所有本地产物均为 local diagnostic：

- 不上传 GitHub Actions artifact；
- 不被 `release.yml` 查询、验证或附加到 GitHub Release；
- 不因使用 `tier=release`、clean worktree 或精确 SHA 自动获得发布资格；
- policy 中的 advisory/enforce、waiver 和 release eligibility 字段保留为协议兼容信息，当前不触发自动门禁。

发版前团队可以自行选择在同一 commit 上本地运行确定性评测，但这是人工检查项，不阻断 tag 构建。未来若恢复远端评测，必须通过单独设计和配置 PR 重新建立隔离 Runner、凭据边界、artifact 保留、exact-SHA 校验与发布门禁，不能只把旧 workflow 文件放回。
