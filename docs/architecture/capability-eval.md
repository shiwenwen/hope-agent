# 专项能力评测基础设施

## 目标与边界

完整能力评测不属于 PR 单测。默认 `cargo test -p ha-core -p ha-server` 只守快速、局部、确定性的代码契约；Coding、Domain、Dreaming、Memory Retrieval 的整包回放由独立 `hope-agent-eval` 执行，每周运行一次，并在发版前针对准备打 tag 的精确 commit SHA 运行。

产品内现有 Dashboard、owner API、campaign 与 `sessions.db` 历史保持不变，仍可由用户显式选择 Provider 运行真实模型。GitHub 发布评测是另一条文件证据链，不导入 App，也不把 App 的本地历史当作发布证据。

## 组成

| 位置 | 职责 |
|---|---|
| `crates/ha-eval-spec` | 不依赖 `ha-core` 的 manifest、policy、plan、shard、evidence、waiver 类型，canonical JSON、SHA-256、路径与 JSON Schema 校验 |
| `crates/ha-eval` | `hope-agent-eval` CLI；创建计划、稳定分片、逐 case 子进程隔离、聚合与发布证据校验 |
| `evals/` | JSON Schema、weekly/release policy、suite manifest 和 fixture 单一真相源 |
| `.github/workflows/capability-eval.yml` | weekly schedule 与手动 release-tier 编排；运行公开 suite 及 `eval-internal-tests` 遗留契约，不监听 PR/push，不是 branch protection required check |

CLI：

```bash
hope-agent-eval validate
hope-agent-eval plan --tier weekly|release --ref <sha> --output plan.json
hope-agent-eval run --plan plan.json --suite <id> --shard 1/2 --output shard.json
hope-agent-eval aggregate --plan plan.json --inputs <dir> --output eval-evidence.v1.json --summary eval-summary.md
hope-agent-eval verify-evidence --evidence eval-evidence.v1.json --ref <sha> --tier release --tag <tag>
```

## 确定性与安全契约

- v1 只允许 `coding_fixture_patch`、`coding_gold_fixture_patch`、`domain_trace_fixture`、`dreaming_golden`、`memory_retrieval_scale`。
- suite 必须声明 `runnerClass=hosted_linux`、`networkPolicy=deny`；case 超时范围为 1–900 秒。
- Runner 在启动 case 前移除 API key/token 环境变量，设置 `HA_EVAL_NETWORK=deny`；fixture 出现非空 Provider/model/model id/model chain/API key/endpoint、`agent`、`external_model` 或 `mock_provider` 配置时 fail-fast。
- GitHub case job 通过 Linux network namespace 运行，namespace 中只允许 loopback；`HA_EVAL_REQUIRE_NETWORK_ISOLATION=1` 会让 Runner 检查真实网卡集合，环境变量本身不能冒充隔离。首期发布证据因此具有实际出站网络边界，而不只是“没有配置 API Key”。
- manifest 不能携带任意 shell 命令；fixture 路径只能使用 suite 目录内的普通相对路径，canonicalize 后越界或 symlink escape 一律拒绝。
- case 使用稳定 SHA-256 分片，在独立子进程运行；超时/崩溃/无结果为 `infra_error`，只自动重试一次。业务断言失败不重试。
- suite/case/policy 以 canonical JSON 和资产内容生成 digest；`evals/version-lock.json` 使用 `id@version` 追加式锁定内容。修改 suite 或 fixture 时必须提升 suite `version` 并追加 lock；修改阈值/套件集合时必须提升 policy `version` 并追加 lock，原版本 digest 不得改写。PR 的 Rust `fmt` check 会运行 `scripts/verify-eval-version-lock.mjs` 对比 base commit，拒绝删除或覆写已有 key。

`ha-eval` 默认启用 `full-runner`，供本地、weekly 和 release 执行完整确定性 adapter。PR 的 fake-Provider smoke 使用 `--no-default-features` 构建同一控制面，只复用普通 feature 集的 `ha-core`，不再次 codegen/link 确定性 adapter。此前从默认测试移出的内部评测契约通过 `ha-core/eval-internal-tests` 在 `Capability Evals` workflow 单独执行；它们不回到普通 PR 或 pre-push。

所有 v1 adapter 都不调用模型 API。某些 Coding fixture 可执行其已审阅的本地验证命令；这与模型网络访问无关。Memory latency 只进入 advisory check，质量/召回正确性才是 blocking check。

## 本地与 GitHub 证据

本地和 GitHub 使用同一 binary、policy、suite、case、scorer 和 digest，因此功能结果应一致。操作系统、CPU 与磁盘差异允许影响 latency；性能首期不阻断。本地调试仍会移除模型凭据并拒绝模型配置，但默认不创建跨平台网络 sandbox；需要验证网络隔离时应在 Linux 中设置 `HA_EVAL_REQUIRE_NETWORK_ISOLATION=1` 并像 GitHub workflow 一样置于无网络 namespace。

本地可以运行 dirty worktree，结果只用于调试。`eval-evidence.v1.json` 只有同时满足以下条件才可用于发版：

- `source=github_actions`、`dirty=false`；
- commit SHA 与 tag 指向的 SHA 完全一致；
- runner、policy、suite、case digest 与该 SHA 仓库内容一致；
- 所有计划 case 恰好出现一次；
- enforce 模式下全部 policy 阈值通过，或存在覆盖全部失败 suite 的一次性受审计 waiver。

evidence 同时记录应用版本、三态汇总、逐 case check、时长、重试和 shard artifact hash。总时长使用最早 shard `startedAt` 到最晚 shard `completedAt` 的墙钟区间，不累加并行 case；artifact path 保留 suite/shard 目录以保持唯一。weekly artifact 保存 30 天，release artifact 保存 90 天。

## 发布与策略升级

发版 PR 合并后、打 tag 前，在 `Capability Evals` workflow 手动选择 `tier=release` 和目标分支/精确 SHA。完成后只给同一 SHA 打 tag。`release.yml` 会查找并验证该 SHA 的 release evidence，并把 JSON/Markdown 附到 draft Release。

初始 `release.json` 为 `advisory`：缺失或失败会显著写入 workflow summary/draft Release，但不阻止构建。连续 3 次 weekly 与 1 次 release candidate 全绿、无 infra error、每次不超过 30 分钟后，通过配置 PR 将 `mode` 改为 `enforce` 并提升 policy version。

紧急豁免只能从 `Capability Evals` 手动输入原因、suite 和单一 tag，经过 `release-eval-waiver` protected environment 审批。waiver 绑定 SHA、tag、workflow run、审批人和时间，不可复用于其他发布。
