# 真实模型与复杂任务评测

> 状态：Hope Core 与桌面 Evaluation Center M0–M4 已实现；当前只支持本地 App / CLI 显式运行。GitHub 自动 Campaign、受保护 Runner、签名发布证据和 release gate 均暂停。
> 确定性评测：[`capability-eval.md`](capability-eval.md)

## 1. 已实现边界

Hope Agent 有两条物理分离的能力评测轨道：

| 轨道 | 命令域 | Evidence | 是否调用模型 | 当前用途 |
| --- | --- | --- | --- | --- |
| 确定性专项评测 | `hope-agent-eval validate/plan/run/...` | `eval-evidence.v1` | 禁止 | 本地代码契约回归 |
| 真实模型 Campaign | `hope-agent-eval model ...` / 桌面 App | `eval-model-campaign.v1` | 显式确认后会调用 | 本地任务完成率、稳定性与效率诊断 |

真实模型轨道已经实现：

- 独立 scenario、suite、policy、plan、trial、shard、evidence 和 waiver 类型及 JSON Schema；
- 受限 YAML、canonical JSON、SHA-256 component digest、稳定 trial seed/ID、append-only version lock；
- `validate`、`plan`、稳定 hash 分片、隔离 trial 子进程、三态聚合、exact-SHA evidence 验证；
- 仅基础设施/模拟器错误自动重试一次，业务失败、策略失败和预算耗尽不重试；
- 混合 `k=3/k=5` 的 `any_pass@k` / `all_pass@k` 分组统计、Hard Success 95% Wilson 区间、成功样本 p50/p95；
- 单 trial 与整 campaign 两级时间、模型调用、Token、费用、工具、Agent 和并发预算；
- Hope Server 黑盒 adapter、确定性终态/文件/Git/trace verifier、28 个 Hope Core 场景资产；
- `EvalRunContext` 对模型、工具、Goal、Loop、Workflow、Checkpoint、异步任务、Subagent 和 Team 的贯穿归因；
- 结构化、无正文的因果事件；模型事件记录 Provider/模型/Token/TTFT，工具事件只记录名称与参数/结果摘要；
- Goal/Workflow/子 Agent 检查点后的真实进程重启、用户事件两阶段前置状态检查、control/faulted 与计算量匹配 solo/team 对照臂；
- 运行期硬预算；重启只继承剩余额度，不能通过重启或 infra retry 重置成本；
- 零费用 fake Provider 黑盒 smoke，真实启动 Hope Server 并覆盖模型、工具、Goal、Loop、Workflow、Async 和 Team 归因；
- 本地 CLI 与 App/Sidecar 两种显式入口，以及零费用 fake Provider 黑盒 smoke；
- Dashboard 独立“能力评测”入口，支持 Quick / Standard / Reliability / Custom、1–4 个已配置模型、预算预览、显式费用确认、实时进度、取消和失败后新建重试；
- 随桌面安装包签名的 `hope-agent-eval` Sidecar，通过版本/资产/二进制 digest 握手后，在独立进程树和隔离 Hope Server 中执行；App 默认并发 4，用户可提高到所选画像的全部 trial 数；
- `~/.hope-agent/evals/evals.db`、内容寻址 artifact、启动中断对账、保留期清理，以及 Hope Core / Coding / Domain 的只读统一历史；
- 逐 trial 因果详情、指标化对比/趋势、annotation、pin、受保护 baseline 与本地诊断导出；
- Ed25519 签名 evidence bundle、离线验签导入、撤销状态刷新和未签名 evidence 的显式不可信导入。

外部 BFCL/AppWorld/Gaia2/Terminal-Bench/τ³/TeamBench/CooperBench/MCPMark/OSWorld 的 adapter 名已注册，未安装 Harness 时会明确产生 `benchmark_defect`，绝不伪造通过。带结构化故障但尚无对应受审故障控制器的场景同样 fail closed。注册不等于实现完成；接入职责、优先级和转正要求见本文第 13 节。

policy 资产可展开跨能力 pilot：nightly 8 case / 20 trial，weekly 25 case / 268 trial，release 22 case / 250 trial，monthly 28 case / 74 trial。它覆盖 Goal/Loop、Workflow、Async Jobs、Subagent/Team 和混合 E2E，并为带故障场景生成 clean/control 与 chaos/faulted 对照，为多 Agent 场景生成 compute-matched solo/team 对照。这些名字目前只是本地可选的计划档位，不对应自动 schedule 或发布门禁；首次用真实 Provider 跑新档位后仍需检查任务可解性、模型噪声和 grader 假阳性，再决定 quarantine 或阈值。

## 2. 入口

```bash
# 只校验资产，不调用模型
cargo run -p ha-eval --locked -- model validate

# 固化精确 SHA 的不可变计划，不调用模型
cargo run -p ha-eval --locked -- model plan \
  --tier nightly --ref <40位commit-sha> --output model-plan.json

# 会调用所选真实模型 API；本地必须显式确认费用
cargo run -p ha-eval --locked -- model run \
  --plan model-plan.json \
  --suite hope-core-orchestration \
  --shard 1/4 \
  --output shard.json \
  --confirm-model-costs

cargo run -p ha-eval --locked -- model aggregate \
  --plan model-plan.json --inputs ./shards \
  --output eval-model-campaign.v1.json \
  --summary model-summary.md
```

`model run` 需要一个单独启动、配置真实 Provider 的 Hope Server：

```text
HA_MODEL_EVAL_MODE=1
HA_MODEL_EVAL_SERVER_URL=http://127.0.0.1:<port>
HA_MODEL_EVAL_SERVER_TOKEN=<专用server-token>
```

隔离 `HA_DATA_DIR/config.json` 只保存无凭据 Provider/模型配置，`apiKey` 必须为空且不得包含 `authProfiles`。模型 Key 由本地 owner/Supervisor 通过 `HA_MODEL_EVAL_PROVIDER_SECRETS_B64` 临时注入 Hope Server：默认内容是 `providerId -> apiKey` 的 base64 JSON 对象，首次加载配置后立即从 Server 环境移除，只保留在进程内存中；评测模式同时禁止配置写回，避免内存中的 Key 落盘。Runner 为每个 trial 创建临时任务目录，并从 Harness 子进程环境删除 Provider Key/token/Cookie 等常见凭据变量。Runner 不直接调用 Provider。

桌面 App 的 `local_app/local_native_diagnostic` 额外支持 Codex OAuth：owner 在 preview/start 前按 Campaign `maxWallSeconds + safety margin` 校验 token 剩余寿命，不足时只在 owner 进程使用本机 refresh token 主动刷新；刷新失败、过期时间不可验证或刷新后仍无法覆盖完整 Campaign 时 fail-fast，并提示缩短时长或重新登录。随后只把当前短期 `access_token + account_id + expires_at_ms` 编码为带 `model-eval-codex-oauth.v1` 类型的 Provider secret，通过现有匿名控制通道交给 Sidecar；隔离 Server 校验过期时间后将 access token/account id 放进进程内 Codex cache。主 HOME、OAuth 文件和 `refresh_token` 永不挂载或传入隔离运行时。该分支必须同时具备 App-control 设置的 `HA_MODEL_EVAL_LOCAL_CODEX_OAUTH=1`；headless CLI 默认不接受这类本机 OAuth secret，Codex 结果固定为 local diagnostic。

Trial evidence 额外记录这个无凭据 `config.json` 的 SHA-256；聚合 evidence 记录 Runner OS/架构。进行本地比较时，所有有效 trial 必须使用同一个运行配置摘要，并同时具备不可变模型与价格快照，防止配置漂移被误认为同一基线。

### 2.1 App 手动运行

桌面端进入“仪表盘 → 能力评测 → 运行”：

1. 选择 Quick / Standard / Reliability / Custom；
2. 选择 1–4 个已配置且支持隔离评测的真实模型；Provider 有多个 Auth Profile 时显式选一个非敏感引用；已登录的 Codex 模型也可选择，但卡片固定标记“仅诊断”；
3. 设置总费用、墙钟和并发预算，先生成不可变预览；
4. 确认模型费用与合成工具执行后启动；“运行”页立即切成当前 experiment 的实时工作台，按 Campaign 和 Trial 展示状态、耗时、模型/工具调用、Token、费用、预算告警，并允许取消；切换其他 Tab 不影响执行；
5. 终态结果原地保留在“运行”页，可展开已落库 Trial 的因果轨迹或开始新评测；历史、对比、趋势和基线页继续负责跨运行查询。本机结果可导出但固定为 unsigned/local diagnostic，不能建立受保护 baseline。

App 只提交 `providerId/modelId/credentialProfileRef`，后端在启动前解析一次实际凭据。凭据不进入前端可见 DTO、计划、数据库、命令行、日志或 artifact。未显式选择 profile 时，后端确定性选取第一个启用且非空的 Auth Profile；这一规则也用于 Coding/Domain 的后端兼容入口，保证重试不依赖前端再次回传完整 Provider 配置。Codex 不接受 Auth Profile：owner 使用 App 已登录 OAuth 身份，preview/start 各自刷新短期 access token；计划只记录不含身份原文的 credential digest。

Coding / Domain 的原始 Campaign 入口与表继续保留；Evaluation Center 通过只读 adapter 统一显示它们，不迁移、不覆写，也不把 legacy 指标假装成 Hope Core evidence。App 与 CLI 本地产物都只能标记为 local source，不能晋升为 release evidence。

## 3. 资产和不可变计划

```text
evals/live/
  schema/          # 两轨不兼容的 JSON Schema
  policy/          # nightly / weekly / release / monthly
  suites/          # 注册 adapter、模型角色、分片和 case
  scenarios/       # 版本化任务、公开 fixture、隐藏 truth、verifier
  version-lock.json
```

Manifest 只能引用注册枚举和相对资产路径，不能携带 shell 命令。路径 canonicalize 后必须仍位于对应 scenario 或 `evals/live` 内，symlink/`..` escape 会失败。Scenario digest 同时固定 manifest、公开资产、隐藏 truth、Prompt、工具 schema、verifier 和环境声明。

`model plan` 会重新读取当前 policy/suite/scenario，展开模型角色、重复次数和 trial seed。之后每次 `run`、`aggregate`、`verify-evidence` 都重建计划并做逐字段比较；同版本资产发生变化必须提升版本并追加 lock。

## 4. Hope 黑盒执行和评分

`hope_core_scenario` 只通过生产 HTTP 入口 `POST /api/chat` 驱动 Hope：

1. 把公开资产复制到 per-trial 临时 workspace，隐藏 truth 不进入被测目录；
2. 传入精确 `providerId::modelId`、working directory、可选初始 Goal 与不可变 `EvalRunContext`；
3. 等真实 Agent loop、工具和控制面完成；
4. 读取 owner API、文件、Git 状态以及只读 eval telemetry；
5. 由注册 verifier 判断终态，不以最终回答中的“我完成了”作为成功；
6. 关闭根 trace 后生成 trial result。

v1 注册 verifier 包括：

- `hope_state_subset`：只允许审核过的 loopback owner API 路径；
- `file_exists`、`file_contains_all`、`file_json_subset`；
- `git_changed_paths`；
- `signal_observed`、`trace_closed`；
- `response_non_empty`、`response_contains_all`、`response_json_subset`。

blocking milestone/invariant 任一失败时，trial 不得为 `passed`。`false completion`、权限/作用域、安全副作用等护栏不得被平均分或 waiver 抵消。

## 5. 归因与隐私

`EvalRunContext` 仅在 `HA_MODEL_EVAL_MODE=1` 的隔离 Server 中接受。上下文至少包含 campaign/case/trial/trace/root span/model role/seed；它随 Session 传播到 Subagent 和后台任务。

Registry 有明确上限：最多保留 256 个 trial、每 trial 4096 个事件。事件只保存稳定 ID、状态、序号、时长和受限标量属性，不保存 Prompt、模型正文、工具参数、工具输出或数据库记录。工具参数与结果只写 SHA-256；模型事件只写 Provider/模型、调用类型、Token、TTFT、成功状态和错误类别。主回复后由产品自动触发的 session title、Memory extraction 等模型调用会继续持有 trial 身份，Token/费用纳入总量，同时以 `model_automation.run` 与任务型 Async Job 分开；预算不足时不允许产生未归因调用。模型 usage metadata 只增加 `metadata.eval` 身份；正常 App/Server/ACP 请求没有 eval context 时不会注册 trial。

Evidence 写出前会扫描常见 Key/token/Cookie、Authorization、私钥片段和个人绝对路径。真实模型评测**会**调用 Provider API、消耗 Token 并按价格快照计费；安全限制的含义是：

- 使用组织单独创建的评测账号/API Key，设置最小权限、速率和费用上限，不复用开发者个人生产账号；
- 任务输入使用仓库内合成 fixture，或已经获得授权并完成脱敏的数据，不复制真实用户会话、文件和隐私；
- 本地运行默认记录 `networkEnforcement=unverified`；如自行启用 OS sandbox/防火墙，只应放行模型 Provider 与 scenario 明确批准的 fixture service，不给工具任意访问公网的能力。

因此，“禁止无限制外网”不等于“禁止联网”，而是把联网范围缩到完成该 trial 所必需的目标。App 现有 Coding/Domain 真实模型入口不受影响，用户显式运行时仍按所选 Provider 正常调用 API。

## 6. 本地运行与发布边界

仓库当前没有 `model-campaign.yml`，也没有其他 GitHub Actions 真实模型 Campaign。普通 PR、pre-push、Rust CI 和 `release.yml` 都不会构建或运行这条评测轨道，不需要 Provider Key：

- 桌面用户通过 Evaluation Center 显式预览、确认费用并启动；
- 开发者可以通过 `hope-agent-eval model` 在本机显式运行，或使用 fake Provider 执行 `model smoke` / `model app-smoke`；
- 本地 App/CLI 产物固定为 local diagnostic，可写入历史、导出、对比和查看趋势；
- GitHub 不上传、检索、校验或签名 model evidence，`release.yml` 不把评测结果写入 Release summary/附件，也不据此阻断构建；
- `nightly`、`weekly`、`release`、`monthly` policy 名称只用于本地选择计划范围，没有自动触发语义；advisory/enforce 与 waiver 字段当前不生效。

签名 bundle、trust registry、protected source 和 release verifier 代码保留为协议兼容及未来恢复能力，但当前仓库不自动产生新的受保护 bundle。未来若恢复远端真实模型评测，必须通过独立设计与配置 PR 重新建立 disposable runner、mount/PID namespace、provider-only egress、匿名凭据注入、完整后代树回收、artifact retention、签名密钥和 exact-SHA 发布校验；不能仅恢复旧 workflow 文件或设置环境变量来宣称隔离成立。

## 7. 维护契约

- 普通 PR、pre-push、默认 Cargo test 不运行真实模型 Campaign，也不需要 Provider Key。
- 修改 schema/policy/suite/scenario/verifier/Prompt/tool schema 时必须提升对应版本并更新 `evals/live/version-lock.json`。
- 新 adapter/verifier/fault 只能在 Rust/Python Harness 的注册代码中实现，Manifest 禁止命令字符串。
- 新的异步或多 Agent 执行边界必须传播 `EvalRunContext`，终态必须关闭对应 guard；trial attribution 不是 `complete` 时本地产物同样无效。
- 工具调用少、Token 少、速度快都不能单独覆盖任务失败；效率只在成功样本上比较，并同时展示全量失败率。
- 外部 benchmark 版本、镜像 digest、许可证和 grader 必须先审计，再加入 allowlist 与 version lock。

## 8. 目标、非目标与最终设计决策

真实模型轨道要回答的是“Hope 通过真实产品入口执行任务时，是否稳定完成目标、遵守控制面与安全契约，并以多少时间、Token、工具和费用完成”，而不是只衡量模型生成文本的观感。它必须：

- 覆盖 Goal、Loop、Workflow、Async Job、Subagent、Team 及组合路径；
- 区分模型能力、Hope scaffold、任务/评分器缺陷和运行基础设施故障；
- 支持相同 case 在 commit、模型、Prompt、工具 schema、权限和编排策略之间做成对比较；
- 以环境终态和过程不变量判定完成，以完整因果归因解释成本、重试、并发和失败；
- 保留本地显式真实模型入口、完整历史和可比较 evidence；当前不建立自动发布证据链。

明确不做的事情：

- 不把真实模型、容器、浏览器、外部账号或随机 Judge 放入普通单测、PR required check 或 pre-push；
- 不用一个综合分替代任务成功、安全、可靠性和效率向量；
- 不以公开榜单替 Hope 定义成功，也不让外部 Harness 重写 Hope 的 Goal/Workflow/Team loop；
- 不承诺浮动模型别名、实时网页或外部 SaaS 在不同时刻完全可复现；
- 不让本地 App/CLI 结果或另一条确定性 evidence 冒充 GitHub 发布门禁；当前发布流程不消费任何评测 evidence。

最终决策如下：

| 编号 | 决策 |
| --- | --- |
| D1 | deterministic 与 model campaign 使用不同命令域、资产根、adapter allowlist、schema 和 evidence verifier；二者不能互相转换，未来远端 bundle 也只能只读引用两份证据 |
| D2 | Hope Core Scenarios 是产品正确性与人工发版判断的核心信号；外部 benchmark 只补任务分布、成熟环境和横向参照 |
| D3 | 程序化终态和 blocking invariant 优先于 LLM Judge；Judge 永远不能把硬失败、安全失败或越权改判为通过 |
| D4 | Hope 自身是被测编排器；Harness 只负责环境、输入、故障、预算、采集和评分 |
| D5 | JSON/JSONL evidence 是本地运行与比较的真相源；OpenTelemetry 只作为未来可选的 trace 交换和查看接口，不能成为唯一证据存储 |
| D6 | 对可靠性场景做独立重复，稳定性主看 `all_pass@k`，不把“至少偶然成功一次”的 `any_pass@k` 当稳定性 |
| D7 | 多 Agent 必须与同模型、同任务、同权限、同总预算的单 Agent 配对，并提供 Planner/Verifier/串行等可解释消融 |
| D8 | 工具数、Token、耗时和费用首期 advisory，只在成功样本上比较；critical false completion 和安全不变量可以独立成为硬门禁 |
| D9 | 当前只运行本地 App/CLI；结果用于诊断、回归和人工判断，不参与 GitHub 发布门禁 |
| D10 | 外部 benchmark 固定代码、数据、任务列表、镜像、grader 和 adapter 版本；禁止跟随 `latest` 建立趋势 |
| D11 | Manifest 只引用注册 adapter/verifier/fault/user-simulator 与受限资产路径，永不执行任意 shell 字符串 |
| D12 | 本地真实模型需要访问所选 Provider；无法证明 provider-only egress 时必须标记为 network unverified，未来远端 Runner 仍需外部防火墙实施最小出站范围 |
| D13 | App 控制面留在 `ha-core::evaluation`，重 Runner 保持独立 Sidecar；普通 `ha-core/ha-server` 单测不链接完整评测包 |
| D14 | 统一历史只是只读索引；Hope Core、Coding、Domain、本地和导入数据的来源、完整性与评分语义不得合并 |
| D15 | 总预算按模型数切分且绝不向上扩张；无法为每个 child campaign 分到至少一个整数额度时直接拒绝计划 |
| D16 | App `maxConcurrency` 表示可并行的 trial/shard 数，不替代 suite/scenario 内部 Agent、model、tool 与 span 预算 |
| D17 | 跨 commit 的功能比较按 case/version/arm/model/config/资产环境身份连接；由 commit 派生的 trial seed 不能阻断逻辑配对，只有要求 seed 一致的指标再单独校验 |
| D18 | 功能成功率分母是 valid trials；`end_to_end_yield` 才以全部 scheduled trials 为分母，infra 单列；不同来源不计算一个混合“全局成功率” |

## 9. 执行架构与被测边界

```text
versioned scenario / suite / policy
                 │
                 ▼
      hope-agent-eval model control plane
 validate → immutable plan → shard/run → aggregate/verify
                 │
                 ▼
       registered environment adapter
                 │
                 ▼
      real hope-agent server / ACP / desktop
                 │
 Goal / Loop / Workflow / Async / Subagent / Team
                 │
                 ▼
 normalized causal trace + observable final state
                 │
 deterministic verifier / milestone / invariant
                 │
                 ▼
 eval-model-campaign.v1 + redacted artifacts
```

### 9.1 产品路径原则

当前 `hope_core_scenario` 只通过真实 Hope Server 的生产 HTTP 入口驱动任务。Provider 解析、模型链、Prompt、工具 schema、权限、failover、Goal/Workflow/Team 状态机和后台 automation 都由 Hope 自己执行；Runner 不直接调用 Provider，也不实现第二套 Agent loop。ACP 适合未来的 Coding/Terminal Harness，桌面入口只留给 Browser、Office、Design 等必须观察 GUI 的扩展场景。

执行模式必须分开形成基线：

| 模式 | 模型请求由谁执行 | 用途 | 当前地位 |
| --- | --- | --- | --- |
| `native_provider` | Hope 自身 | 覆盖真实产品 Provider、failover、usage 和 automation 路径 | 当前唯一启用的本地模式 |
| `bridged_provider` | Inspect/受控模型代理 | 统一不同 Agent 的模型后端和生成参数，适合横向研究 | 后续研究模式，不能混入 native 基线 |

### 9.2 环境与网络类别

当前本地 Campaign 使用 `local_native_diagnostic + native_provider`，网络约束标记为 unverified。未来若恢复远端 adapter，只能从受审类别中选择：

| Runner 类别 | 适用场景 | 网络边界 |
| --- | --- | --- |
| `hosted_linux` | 资产校验、fake smoke、轻量无凭据协议任务 | 无 Provider Key；默认无外部任务网络 |
| `docker_linux` | 文件、数据库、MCP、终端、仓库和 AppWorld 类任务 | Provider + 场景私网/固定 allowlist |
| `dedicated_linux` | 高并发、长任务、真实 Provider、weekly/release | 外部防火墙强制 provider-only 或受审 allowlist |
| `desktop_vm` | Browser、Office、OSWorld、跨应用桌面任务 | 可销毁 VM、专用账号、逐 suite allowlist |
| `isolated_external_service` | GitHub、Notion 等专用测试租户 | 最小权限 token，只允许指定租户和 API |

环境变量 `HA_MODEL_EVAL_NETWORK_ENFORCED=1` 只是部署证明，不能代替网络 namespace、防火墙或 egress proxy。动态重定向、私网地址、云 metadata 和未知目标仍服从 Hope 的 SSRF/权限策略。

### 9.3 已实现与后续扩展的边界

已实现的是 Rust/Sidecar 控制面、Hope Server 黑盒 adapter、28 个 Hope Core 资产、JSONL 因果事件、预算、本地 evidence，以及 App 运行、独立历史、对比、趋势、签名导入和基线管理。以下内容尚未实现、暂停或尚未完成外部运维配置，不能因枚举、UI 或文档存在而宣称已有能力：

- Inspect AI/Harbor 与外部 benchmark 的实际 Harness、镜像和 grader；
- 通用 OTLP exporter、Langfuse/warehouse 趋势后端；
- Browser/Office/OSWorld、外部 SaaS 账号池和专用桌面 VM；
- GitHub 自动 Campaign、组织专用 Provider 账号、受保护 Runner egress/secrets、签名 key registry，以及持续的远端真实 Provider 基线；
- BFCL V4 非 live pilot。它仍以“未安装 Harness → `benchmark_defect`”fail closed。

## 10. Scenario、评分与故障契约

### 10.1 Scenario 组成与版本

每个 Scenario 固定以下组件，任何影响任务含义或判分的内容变化都必须升版本：

```text
identity          id/version/tags/digest
instruction       用户任务、公开验收条件、可选多轮脚本
environment       image/snapshot/services/network/files/db/time
hope_config       model/features/permissions/budgets
initial_state     Goal/Workflow/Loop/session/project/fixtures
fault_schedule    timeout/429/error/restart/event/race/schema drift
oracle            hidden truth + final verifier + milestone DAG + invariants
limits            wall/model/tool/token/cost/turn/agent/job/concurrency
artifacts         allowlist/redaction/retention
comparison        control/faulted、solo/team、baseline/candidate 配对
```

Prompt、公开 fixture、隐藏 truth、verifier、fault、用户脚本、环境、Hope 配置、工具 schema 和 adapter 都进入 digest。隐藏 truth 与 verifier 细节不得复制进被测 workspace 或模型上下文。路径允许相对引用，但 canonicalize 后必须留在 scenario/`evals/live` 根内；symlink、`..` 和同版本覆写均 fail closed。

### 10.2 判分顺序与结果分类

判分顺序固定为：

1. 程序化环境终态；
2. 安全、权限、顺序、幂等、取消和残留资源等 blocking invariant；
3. milestone DAG 与部分进展；
4. 只有无法程序化表达的语义/审美质量才进入版本化 Judge rubric。

单 trial 保留细粒度 outcome，顶层再映射为兼容的 `passed | failed | infra_error`：

| Outcome | 含义 | Runner 自动重试 |
| --- | --- | --- |
| `passed` | 所有 blocking verifier/invariant 通过 | 否 |
| `task_failed` | 环境有效，但终态或必要里程碑未满足 | 否 |
| `policy_failed` | 越权、泄漏、禁止副作用、脱敏失败等安全失败 | 否 |
| `budget_exhausted` | 被测 Agent 用尽 trial 预算；属于能力结果 | 否 |
| `infra_error` | Runner、Provider 接入、环境或 scorer 无法形成有效试验 | 最多一次 |
| `benchmark_defect` | 任务、truth 或 grader 经审计确认有缺陷 | 否，进入 quarantine |
| `simulator_error` | 用户模拟器偏离契约，trial 无效 | policy 可允许最多一次 |
| `cancelled` | 外部取消或预期取消路径 | 否 |

`task_failed | policy_failed | budget_exhausted` 聚合为业务 `failed`；invalid/cancelled 保留独立计数并由 policy 决定 campaign 是否成为 infra-error。必须同时报告 `passed/valid_trials`、`passed/scheduled_trials` 和 `infra_error/scheduled_trials`，禁止通过排除大量无效 trial 美化成功率。

### 10.3 Milestone 与过程不变量

Milestone 必须形成 DAG，允许 `requires`、`anyOf`、blocking、weight、deadline、public/hidden 和 evidence 引用。评分者接受多条正确路径，不要求模型复刻固定工具序列。过程 DSL 只锁不可妥协的语义：`never`、`before/after`、`at_most_once/exactly_once`、`eventually/eventually_within`、`max_concurrent`、`no_overlap` 和 `parent_child_closed`。

任何 blocking invariant 失败都是 hard fail，不能被部分分、其他 case 高分、Judge 或性能优势抵消。`false_completion` 指 Agent/Goal 宣告完成但 hard verifier 失败，必须单独统计。

### 10.4 故障、用户事件与对照臂

- 故障由注册控制器按 seed 和结构化触发点注入，必须产生 `fault_activated/released` 证据；禁止用随机 `sleep` 假造竞态。
- 每个故障场景同时保留 clean/control 与 chaos/faulted arm，避免把本来就失败误判为恢复失败。
- Model gateway、tool、scheduler、process、storage、user 和 environment fault 分开归因；重启从 durable row 恢复 trial 身份和剩余预算。
- blocking 场景优先使用 `scripted_fsm`，允许受审 `replay`；LLM User 只用于探索，固定模型/Prompt/预算并与 Agent 成本分开，不得决定本地 required case 的 pass/fail。
- 用户改需求、拒绝审批、取消等事件执行前后都检查持久状态，确保事件确实命中预定阶段而不是只在日志中出现。
- 业务失败不重试；infra retry 保留原 attempt、累计用量和独立 trace，不能重置成本或覆盖失败证据。

### 10.5 Hope Core 28 场景覆盖

| 分组 | 场景 ID | 核心契约 |
| --- | --- | --- |
| Goal / Loop | `HA-GL-001..006` | 验收证据、假完成恢复、不可满足目标收敛、预算停止、需求修订、checkpoint/restart |
| Workflow | `HA-WF-001..006` | fan-out/join、可重试与非幂等写、restart、补偿、pause/resume/cancel、拒绝审批 |
| Async Jobs | `HA-AJ-001..006` | 乱序汇总、前台 busy 延迟注入、cancel/complete 竞态、重试分类、incognito purge、公平调度 |
| Subagent / Team | `HA-ST-001..006` | 冲突资料研究、Planner/Executor/Verifier、worktree 合并、成员崩溃重分派、取消子树、origin/权限/KB/incognito |
| 多模块 E2E | `HA-E2E-001..004` | Coding 发布修复、冻结语料 Research、Knowledge/File stale-write、Browser/Terminal incident |

当前 suite manifest 是 case、版本、标签、arm、重复次数和 tier 的单一真相源。业务域扩展沿用 Coding、Research、Knowledge、File、Browser、Terminal 六类终态契约；Pre-release 档位的 Research 使用冻结语料，实时 Web 必须单列 exploratory 基线并记录 URL、抓取时间和内容 hash。

## 11. 指标、成本与统计口径

### 11.1 主 KPI

```text
hard_task_success = blocking verifier 全过且 blocking invariant 为 0
valid_task_success = passed / valid_trials
end_to_end_yield = passed / scheduled_trials
infra_error_rate = infra_error / scheduled_trials
reliable_success_all_k = 同 case 的 k 次独立 trial 全部通过
successful_efficiency = 成功 trial 的 wall/token/cost p50 与 p95 向量
```

`any_pass@k` 用于观察至少一次成功，`all_pass@k` 才表示可靠重复；二者必须同时标明含义，不能用含混的 `pass@k`。当前聚合已实现分组 any/all、Hard Success 的 95% Wilson 区间和成功样本 p50/p95。

### 11.2 工具、时间、Token 与成本

- Hope 接受结构化调用时计 `tool_calls_attempted`；解析失败另记 parse error。执行终态必须互斥落入 succeeded/failed/cancelled，attempted 与三者总和一致。
- Runner retry 保留 logical call id 和递增 attempt；总 attempt 与逻辑调用数同时报告。duplicate/invalid/unused/effective call 是诊断指标，不是让 Agent 跳过必要验证的目标。
- `wall_ms` 只覆盖 trial；provision/cleanup 单列。model/tool active 为叶子 span 时长和，可能因并发超过 wall；critical path 按因果 DAG 计算，不能按日志顺序估算。
- queue、approval、environment wait 和 Provider TTFT 分开；成功与失败样本分组展示，防止快速失败被误判高效。
- Provider usage 是 Token 真相源，按 input/output/cache read/write/reasoning 拆分；缺失字段为 `null`，估算值标记 `usageSource=estimated`。
- Agent、Subagent、automation、User Simulator、Judge、retry 和 failover 的实际模型调用都计费；父级只汇总叶子调用，禁止把子级汇总再次相加。
- USD 成本使用 evidence 内固定的 Provider/model/生效时间价格快照；未知价格为 `null + warning`，本地 required trial 会明确标记价格数据不完整，历史成本不按最新价格回算。

### 11.3 多 Agent 净收益

所有声称并发或团队收益的场景都要和 `single_agent_compute_matched` 配对：同一模型、任务、权限、环境和 trial seed，单 Agent 获得与 Team **总量相同**的 Token、工具、费用和 wall budget。至少报告：

```text
team_uplift_pp        = team_hard_success - solo_hard_success
wall_speedup          = solo_wall_time / team_wall_time
token_amplification   = team_total_tokens / solo_total_tokens
cost_amplification    = team_total_cost / solo_total_cost
parallel_efficiency   = sum(child_active_ms) / (wall_ms * concurrency_cap)
coordination_overhead = coordination_tokens / team_total_tokens
```

full team 之外逐步加入 no-planner、no-verifier、serialized 和 restricted-communication 消融。若成功率没有提升或 wall 没有下降，不能因 spawn 数、消息数或调用量增加宣称能力增强；成功率提升但资源显著放大时明确标记 trade-off。

### 11.4 重复、比较和基线断点

- baseline/candidate 使用同 case/version、环境、模型配置和配对 trial index；业务失败不选择性补跑。
- trial seed 含 commit reference，用于保证某一 immutable plan 内稳定且不同 commit 独立；跨 commit 趋势先按不含 seed 的逻辑 trial identity 连接，再由具体指标判断是否要求 seed 相同。否则每次提交都会被误判为 trial-set mismatch。
- 当前 evidence 保存原始计数、Wilson 区间和逐 case 结果；样本不足标为不足/不可下结论，不能补跑到刚好通过。
- 转 enforce 前，成功率/成本差异应补齐 case-paired bootstrap；二元成对结果可用 McNemar。统计方法、样本集和回归边界必须先写入 policy，不能看完结果再改。
- 模型 snapshot/reasoning/temperature/failover、system Prompt、工具 schema、关键工具语义、Memory/context 策略、scenario/grader/rubric、数据集或环境镜像变化都建立新基线，不覆写旧基线。
- Provider 只有漂移 alias 时标记 `modelReproducibility=best_effort`，报告中显示不可直接纵向比较的断点。

## 12. 本地运行节奏与 Quarantine

### 12.1 运行层级

| Tier | 当前触发 | 内容 | 模型重复 | 当前用途 |
| --- | --- | --- | --- | --- |
| PR | 不运行 | 无评测任务 | 0 个真实模型调用 | 普通契约测试保持快速 |
| Nightly | 本地手动 | 8 个 Hope Core case、20 trial | 多数 `k=1`，critical 可更高 | 快速诊断 |
| Weekly | 本地手动 | 25 个 case、268 trial | 默认 `k=3`，含对照臂 | 可靠性和模型趋势 |
| Pre-release | 本地手动，可选精确 SHA | 22 个 case、250 trial | 默认 `k=3`，critical `k=5` | 人工发版判断，不阻断 release workflow |
| Monthly | 本地手动 | 28 个 case、74 trial，重型/chaos 扩展 | `k=1`，选中 case 多 seed | 能力发现和长周期趋势 |

这些数量来自当前 `1.8.0` suite/policy 展开的不可变计划；资产版本、标签或 policy 改变后必须重新以 `model plan` 为准，文档数字不是执行器输入。当前没有自动矩阵；用户在 App/CLI 中显式选择模型。Product Default、Challenger、Economical 和锁定权重的 Local 模型可作为本地比较角色，必须记录精确版本且避免相同模型重复花费。普通模型横比默认关闭 failover，只有 failover 专项显式开启并逐跳归因。

### 12.2 Policy 语义

本地聚合仍按 policy 展示能力、infra、护栏、预算和效率结果，并对来源、schema、hash、secret scan、计划和 digest 完整性 fail closed。`mode=advisory|enforce` 当前只影响本地报告的判定，不会改变 PR、tag 或 GitHub Release 状态。

若未来恢复远端 enforce，必须通过独立配置 PR 重新评审样本量、重复次数、grader 质量、预算、隔离、签名和回滚条件；历史“4 weekly + 2 release”等门槛不能在没有持续远端基线的情况下直接沿用。

### 12.3 Quarantine、Waiver 与基线治理

- 只有已证实的 grader、任务数据、上游或 infra 缺陷能进入 quarantine；Agent 业务失败不得借此移除。记录 owner、原因、证据、进入版本和恢复条件，case 仍显示在汇总中。
- 外部 benchmark 升级先在同一 Hope commit/model/config 上做 old/new bridge run，然后建立新基线；不把不同版本分数连成一条无断点趋势。
- 本地 waiver 不能豁免 secret、digest、source 或证据结构不合法；它只能作为诊断注释，不改变 GitHub 发布状态。
- 当前不存在远端模型 gate；未来启用时必须重新定义受审计 waiver 和保护环境。

## 13. 外部 Harness、Benchmark 与治理

### 13.1 职责分工

| 组件 | 角色 | 不承担的责任 |
| --- | --- | --- |
| `ha-eval model` | 顶层计划、digest、预算、分片、聚合和 evidence 验证 | 不实现 benchmark 私有环境语义 |
| Hope Core adapter | 测 Hope 独有 Goal/Workflow/Async/Team、安全和恢复契约 | 不提供公开榜单横比 |
| [Inspect AI](https://inspect.aisi.org.uk/) | 未来通用 sandbox、dataset、并发、预算和自定义 scorer Harness | 不替代 Hope loop，也不是 evidence 真相源 |
| [Harbor](https://www.harborframework.com/docs) | 未来 Terminal-Bench/Coding 容器任务专用 Runner | 不评分 Hope durable control plane |
| Benchmark 原生 Runner | 保留 τ³、TeamBench、CooperBench、MCPMark、OSWorld 的原生环境与 grader | 不为统一框架而改写原任务语义 |
| OpenTelemetry GenAI | 未来 model/tool trace 的交换字段，Hope 扩展使用版本化 `hope.*` | 不定义 task、scorer、gate 或证据完整性 |
| Langfuse/仓库 UI | 可选趋势、实验和人工分析界面 | 不成为唯一存储或 release 判定源 |

外部 Runner 的统一 adapter 生命周期为 `validate → provision → execute → collect → grade → cleanup`。Rust 只按注册枚举调用受审代码，并通过 JSON/JSONL 交换数据；上游原始分数必须保留，再映射为 Hope outcome。未安装、版本不匹配、grader 不可用或 cleanup 失败都不能伪绿。

### 13.2 接入顺序

| 优先级 | Benchmark | 主要信号 | 运行建议 |
| --- | --- | --- | --- |
| P1 | BFCL V4 非 live 子集 | 工具选择、参数、并行和多轮协议 | 低成本 nightly/weekly；分类型报告 |
| P1 | AppWorld / AppWorld-UL | 有状态 App、跨工具、副作用、澄清与确认 | 先基础集；UL 用户模拟长期 advisory |
| P1 | Gaia2 / ARE | 异步事件、时间约束、环境变化和 A2A | weekly/soak，版本升级做 bridge run |
| P1 | Terminal-Bench 2.1 | 真实终端、Coding、运维和长任务 | Harbor + 人工审计精选子集 |
| P1 | τ³-bench text | 多轮用户、业务 policy、工具和可靠性 | 分离 Agent/User Simulator 成本 |
| P2 | TeamBench / CooperBench | 角色隔离、协作、冲突和多 Agent 消融 | 双周/月度，必须有 compute-matched solo |
| P2 | MCPMark | Filesystem/Postgres/Playwright/GitHub/Notion MCP | 先无凭据本地域，外部域用专用账号 |
| P3 | OSWorld-Verified/2.0 | Browser、Office、桌面和跨应用长任务 | 专用可销毁 VM，月度 advisory |
| 观察 | ToolSandbox / GAIA / AgentBench | 状态工具、通用研究和历史环境广度 | 补充信号，不作首期门禁 |
| 谨慎 | SWE-bench 系列 | 真实 Coding issue 形式 | 仅内部复核子集，不以公开总分作 release gate |

先完成 Hope Core 和统一证据，再依次接 BFCL、AppWorld、Gaia2/ARE、Terminal-Bench、τ³；随后才做 TeamBench/CooperBench、MCPMark 和 OSWorld。外部 adapter 当前均未安装，实际接入必须逐项 PR，不能把注册枚举视为完成。

### 13.3 质量、许可和数据预检

每个进入趋势或门禁的外部任务至少通过：环境空载启动、gold solution、null solution、幂等 reset、并发 trial 隔离、grader 重打分一致、人工任务清晰度审查、合理替代实现检查、无模型 infra baseline 和 quarantine 演练。

每个 adapter 维护机器可读清单：

```text
component / source_url / code_license / dataset_license
third_party_assets / allowed_internal_use / redistribution_allowed
attribution_required / credential_required
pinned_revision / task_selection_digest / image_digest / scorer_digest
reviewed_at / owner
```

代码开源不等于任务数据、附件、镜像、被测仓库、网站内容或模型输出可以再分发。AppWorld protected bundle、GAIA gated split、受版权保护页面和专用 SaaS 数据按各自许可保存；受限内容只保留 hash/声明，不能进入公开 artifact。Promptfoo/自定义 hook 等能执行本机代码的配置一律视为代码审查对象，而不是数据资产。

### 13.4 数据等级与隔离

| 等级 | 内容 | 当前本地处理 |
| --- | --- | --- |
| `synthetic` | 人工 fixture、生成账号、虚构文档 | 可进入本地诊断 artifact |
| `licensed_fixture` | 明确允许评测的外部数据 | 遵守访问、保留和再分发限制 |
| `sanitized_replay` | 经授权、去标识化的真实轨迹 | 默认不运行；显式启用时只保存在本机且不得导出原文 |
| `restricted` | 仍可能包含敏感业务/用户信息 | 禁止运行 |

每 trial 使用独立 temp home、data dir、session DB、KB、workspace、端口、浏览器 profile 和容器/VM 网络；结束时验证无进程、挂载、账号、副作用、spool、worktree lock 或数据库句柄残留。Incognito 场景还要用 synthetic canary 扫描 sessions DB、旁路 DB、tool/job spool、Memory/Dreaming/Awareness、KB index、FTS/Dashboard 统计和共享模型/视觉缓存，然后销毁整个环境。

## 14. 桌面 Evaluation Center 架构

### 14.1 分层与控制协议

```text
EvaluationTab
    │ typed Tauri owner commands + evaluation:changed
    ▼
ha-core::evaluation::EvalOrchestrator
    │ EvalWorkerRuntime（不依赖 ha-eval）
    ▼
签名 hope-agent-eval Sidecar / eval-app-control.v1 JSONL
    │ 固定 product binary + 匿名 stdio secret
    ▼
隔离 Hope Server + 并行 trial worker
    │
    ▼
eval-model-campaign.v1 → 内容寻址 artifact → evals.db 索引
```

协议、App profile/request/plan、runtime/provenance/compatibility、bundle/trust 类型在轻量 `ha-eval-spec`；编排、存储、历史、查询和 Provider 解析在 `ha-core::evaluation`；Tauri 只发现安装包内 product/Sidecar/assets、实现进程 runtime 和 owner command；重执行器继续留在 `ha-eval`。这保证普通 `ha-core` / `ha-server` 测试不会链接 Runner、scenario pack 或真实模型代码。

Sidecar 第一个事件必须是 hello，包含 `eval-app-control.v1` 协议、产品版本、runner digest、asset root/version-lock digest、OS/arch 和 adapter 能力。App 重新计算 Sidecar 二进制 hash，并以自己的产品版本和资源 digest 回执；任一不匹配都拒绝执行。stdout 只允许有序 JSONL 控制事件，日志走 stderr；seq 重复或倒退、未知事件、握手超时和 event stream 意外关闭均 fail closed。

### 14.2 App profile、计划与预算

版本化 profile 位于 `evals/live/app-profiles/`：

| Profile | 本地选择 | 默认用途 |
| --- | --- | --- |
| Quick | critical/smoke 的 control 子集，`k=1` | 配置与主路径快速确认 |
| Standard | weekly 覆盖的本地兼容 control 子集 | 日常版本预检 |
| Reliability | 已注册对照 arm 与 suite 重复 | 恢复、稳定性和多 Agent 对照 |
| Custom | 只能从 Reliability allowlist 继续缩小 case/arm，`k=1..5` | 定位单一问题 |

App request 不能提供 tier/source/runner/network/digest/任意 adapter，也不能扩大 profile 的场景/arm/重复/模型/trial 范围或 suite/scenario 的单 trial 预算。Sidecar 重新从打包资产解析不可变 `eval-app-plan.v1`，对每个模型生成独立 child campaign；多模型只属于同一 experiment 的比较组，不合并成一份可晋升 evidence。最多 4 个模型、500 trial；同一 App 只允许一个 active experiment。费用、experiment 总墙钟和 trial 调度并发由用户显式填写，不再使用 Quick/Standard 等画像设置低额产品上限；协议仅以 500 个总 trial 作为并发的自然上限，并保留 `1,000,000 USD` 的异常输入防护值。

`campaignBudget.maxConcurrency` 在 App 控制面表示最多同时运行多少 trial/shard。suite/scenario 的 model/tool/token/agent/内部并发预算仍逐 trial 生效，不能用 App 并发覆盖或放大。模型总预算采用保守整数切分；若一个非零整数上限小于模型数，计划直接拒绝，而不是用 `max(1)` 将总预算偷偷放大。App 新建评测默认费用上限 100 USD、experiment 墙钟 480 分钟、并发 4；三项都可在预览前直接调整，其中墙钟输入不设前端最大值，并发最多等于画像可能生成的 trial 数。

App profile 可用 `maxTrialSeconds` 进一步收紧单 trial 墙钟，但内置 Quick `1.2.0` 起不再附加画像级时限；不可变计划仍会把 experiment 总墙钟的 90% 按全部 trial 保守均分，取可选 profile、均分值与 suite/scenario 注册上限中的最小值，剩余 10% 专用于启动、落证据、取消和 Supervisor 回收。Harness HTTP deadline 必须早于 trial 子进程 deadline，为 telemetry/Session cleanup 留出固定窗口。触及该墙钟属于 `budget_exhausted/trial_wall_timeout`，不是 infra-error，因此不得触发 infra 自动重试。

运行期间 Sidecar 每秒通过隔离 Server 的 owner-token telemetry endpoint 拉取一次当前 trial 的脱敏快照，并发送 `trial_progress`：只包含墙钟、模型/工具调用、Token、费用、Loop、Agent、Async、活跃子任务、归因状态和最后一个无 payload 事件类型。Prompt、模型正文、工具参数/输出不进入控制协议。UI 用该事件更新场景行并允许打开实时详情；最终 `trial_completed` 与 evidence 仍是权威结果。只有 telemetry endpoint 已实际注册的 trial 才进入“运行中”，同一 shard 中尚未开始的 case 必须保持“排队中”。

预览与启动绑定同一个 `planDigest`；用户修改选择、资产变化或预览过期时必须重新预览。中断/失败后的“重试”新建 experiment/campaign 并保存 parent id，不覆盖第一次已经产生的调用、费用或结果。

### 14.3 凭据、进程与取消

UI 只看见脱敏模型与 credential profile label/ref。owner 后端从可信 App 配置解析单个有效 Provider，把 credential-free config 与一次性 `providerId → secret` 映射通过匿名 stdio 首条 start 消息传给 Sidecar；Sidecar 不记录控制消息正文，再只对子 Hope Server 的初始化环境注入。普通 Provider secret 是 API Key；本地 App Codex secret 只能是当前 access token 与 account id，不得含 refresh token。Server 读取后立即移除环境变量并禁止评测配置写回。任何 Key/token 不得进入 argv、计划、SQLite、artifact、stdout/stderr 或导出包。

Desktop supervisor 只接受当前安装包中、版本/digest/平台身份匹配的 Hope product binary，并使用固定 `server start` 参数；Manifest、前端和 request 都不能传任意 executable/argv。每个 campaign 创建独立 HOME、`HA_DATA_DIR`、workspace、token、端口和进程组；Unix 回收 process group，Windows 用 Job Object。用户取消先发协议 cancel，超时后终止完整进程树；App 退出或 Sidecar 失联时非终态记录变为 `interrupted`，不自动续跑。

v1 不提供伪“暂停”：冻结本地进程不能冻结已经发往 Provider 的 HTTP 请求、计费、OAuth 有效期或外部副作用，直接 `SIGSTOP/SIGCONT` 会产生不可审计的预算与结果。`cancelled` / `interrupted` 也不能原地恢复；当前“重试”始终基于原 request 新建 experiment，并以 parent id 保留关系。未来若增加暂停，只能实现为**调度边界暂停**（不再启动新 trial、允许在途 trial 正常收敛）并持久化剩余 shard/预算/checkpoint；进程失联后的恢复还必须重新校验 plan、二进制、凭据寿命和合成环境，不能复用不完整 evidence。

Experiment 总墙钟到期时禁止直接 drop `run_experiment` future；Sidecar 必须先复用协议 cancel，等待 trial/shard 与 Supervisor 正常清理，再发终态。隔离 Hope Server 同时校验 Supervisor PID 并运行失联 watchdog：Supervisor 异常退出时 Server 必须杀死自己的完整进程组，防止 macOS 上因独立 process group 产生孤儿 Server。每个 `trial_completed` 事件在最终 evidence 生成前先以部分记录写入 Evaluation DB；失败、取消或中断会把未完成 Campaign 一并置为对应终态，刷新页面后仍以 DB 恢复已完成 trial，不得继续显示 `running`。最终 evidence 到达时再以校验后的完整 trial 记录原子替换部分记录。

本地真实模型允许用户显式使用自己的 Provider 配置和已登录 Codex OAuth 身份，但只运行合成 fixture，并记录 `source=local_app`、`executionProfile=local_native_diagnostic` 与真实 OS/arch；无法证明 provider-only egress 时写 `networkEnforcement=unverified`。这些字段从结构上阻止它冒充 dedicated protected evidence。Codex token 只存在于 owner、Sidecar supervisor 与隔离 Server 内存；试验结束/取消/进程树回收后即失效，不同步到历史。

### 14.4 存储、历史与 Owner API

```text
~/.hope-agent/evals/
  evals.db
  artifacts/<sha256-prefix>/<sha256>
  runs/<experiment-id>/...
  imports/<bundle-sha256>/...
```

SQLite 只保存 experiment/campaign/trial/import/baseline/annotation/artifact 的可查询标量、digest 和内容寻址引用；完整 evidence/trace 保存在原子写入的 artifact 中。导入和本地产物路径拒绝绝对路径、`..`、symlink、重复文件、未声明文件、archive traversal、超限大小/数量与 hash 不符。启动时把遗留非终态实验对账为 interrupted，并清理未 pin、未受保护且过期的 artifact；受保护导入不会被普通 retention 清理。

统一 History 同时读取新 `evals.db` 与现有 `sessions.db` 中的 Coding/Domain Campaign，但 legacy source 始终标记为 `LegacyLocal`，详情由各自 adapter 解析。Hope Core 的 trial 查询除脱敏 `ModelTrialResult` 外，还从已验证 evidence 中返回该 trial 不可变计划实际采用的 `budget` 与 `timeoutSeconds`，供 Owner UI 展示“实际值 / 上限”和精确触发维度；这些字段仍不包含 Prompt、模型正文或工具 payload。没有完整 evidence 的异常旧记录只返回已落库标量摘要，禁止把缺失轨迹伪造成 0 或完整详情。Overview 只汇总总 trial、infra、已完成 campaign 和已知费用，不跨异构评分器计算一个“全局任务成功率”。

Tauri owner 平面提供 catalog/readiness/model options、preview/start/cancel/retry、history/detail/trial、compare/trends、pin/annotation、baseline、signed/unverified import 与 local export。HTTP/WS 平面只开放脱敏只读 history/query；真实模型 preview/run/cancel/retry/import/export 默认固定拒绝，避免远程 API 绕过桌面 Sidecar 和本机文件选择。`evaluation:changed` 只作进度/失效提示，刷新后仍以 DB 为真相源。

正式桌面包通过 Tauri Resource resolver 取得 `evals/live`，只执行产品二进制同目录的随包 Sidecar；任一资源或 Sidecar 缺失时在发送 Provider 凭据前 fail closed。只有 `debug_assertions` 开发构建允许从当前 checkout 回退到 `evals/live` 和 `target/debug/hope-agent-eval`。因此签名 evidence 的 trust registry、version lock 和执行程序不能被 App 启动时的工作目录替换。Headless Server 不扫描 exe ancestor/cwd；若要让 HTTP 只读查询刷新签名状态，管理员必须显式设置绝对、已 canonicalize 且不含 symlink 的 `HA_EVAL_TRUST_REGISTRY_PATH`，未配置或无效时统一 fail closed 为 key missing。

## 15. 对比、趋势、签名与基线

### 15.1 可比性

查询对每个指标返回 `exact | functional | diagnostic_only | incompatible`，而不是使用一个万能 compatibility key：

- 功能：suite/case/scenario/verifier/prompt/tool schema、模型 snapshot/推理配置、执行 arm、runtime config 和环境族；
- Token：再要求相同 tokenizer/usage source；
- Wall：再要求相同硬件/OS/arch/并发负载类；
- Tool：再要求相同工具 schema/关键工具语义；
- USD：再要求相同 price snapshot；
- 多 Agent：再要求相同 seed、权限、环境和 compute-matched 总预算。

commit SHA 是比较轴，不进入功能 compatibility key。trial seed 因包含 commit reference 只用于 immutable plan 内稳定；跨 commit 先按不含 seed 的逻辑 identity 连接。受保护 enforced 与本地 unverified 默认只能 `diagnostic_only` 并排，不显示回归结论。

趋势同时保存 `valid_task_success`、`end_to_end_yield`、any/all-pass@k、infra/policy/budget/false-completion、成功样本 wall/tool/token/cost 与 multi-agent uplift。`valid_task_success` 使用 valid trial 分母；`end_to_end_yield` 使用 scheduled trial 分母，避免过滤 infra 后虚高。效率优势永远不能覆盖业务或安全失败。

### 15.2 Signed bundle 与信任刷新

`eval-evidence-bundle.v1.zip` 含 canonical manifest、Ed25519 signature、evidence 与 manifest allowlist 中的 artifact。App 验证顺序固定为 archive 安全 → manifest/schema → key/时效/状态 → 签名 → evidence/artifact SHA-256 → model evidence source/SHA/digest/secret scan。验证通过且当前资产已知时标 `ProtectedVerified`；签名有效但资产版本未知时标 `ProtectedUnknownAssets`，只能保存查看；裸 JSON 明确标 `UnverifiedImport`。

信任注册表仅包含公钥和状态。受保护 bundle 导入时同时固化 `key_id + SHA-256(public key bytes)`；每次查询/导入刷新当前 key 状态时必须同时匹配二者，只有同 ID 且同公钥才能保持可信。旧记录缺少指纹、key 被替换、retired/revoked/missing 都不改写“导入时曾验签成功”的审计事实，但会取消其继续批准或作为 baseline 的资格；旧记录须重新导入并重新验签后才能补齐指纹。bundle hash 去重，重复导入幂等；annotation 和 baseline 只引用原 evidence，不改 outcome，也不删除原 artifact。

只有 `ProtectedVerified + completed + 当前签名仍可信 + tier 匹配` 的 experiment 可以建立 protected baseline。本地导出固定 unsigned、`releaseEligible=false`；用户即使修改 JSON 中的 source 也无法获得盾牌或发布资格。

## 16. 当前完成度与未来远端恢复条件

当前仓库已完成 M0–M4 的本地闭环：App/CLI 入口、Sidecar 隔离执行、真实 Provider、不可变计划、预算、因果归因、历史、详情、对比、趋势和本地导出都可以使用。仓库不配置 GitHub Provider secret、`model-eval` protected environment、self-hosted Runner、自动 schedule、evidence signing job 或 release preflight。

远端 M5 明确暂停，不属于当前发版前置条件。以后若确有需要恢复，必须新建专项设计/配置 PR，至少重新确认：

1. 组织专用 Provider 评测项目、账单硬上限和告警；
2. 固定 anchor model snapshot、reasoning、max output、endpoint 与 price snapshot；
3. credential-free config、Provider secret、彼此不同的 server/supervisor token 及 protected environment；
4. disposable self-hosted runner、Bubblewrap user/mount/PID namespace、provider-only egress、磁盘/进程清理和并发限额；
5. 独立 Ed25519 key pair、公钥 registry/version lock、签名环境与密钥轮换；
6. fake smoke、连续 advisory 基线、exact-SHA 验证、失败归因审核和门禁回滚演练。

在恢复 PR 合并前，只能宣称“本地真实模型评测可用”，不能宣称已有 GitHub 自动 Campaign、受保护 Provider 稳定基线或评测发布门禁。

外部 BFCL pilot 是后续条件式 M6。当前没有 Harness，不运行，也不影响 Evaluation Center 本地核心闭环；接入时先完成许可、任务质量、环境隔离和成本审计，不再依赖尚未启用的 protected bundle 作为先决条件。

## 17. Evaluation Center 验证契约

- `cargo run -p ha-eval --locked -- model app-smoke --sidecar <hope-agent-eval> --server-bin <hope-agent-server>` 使用 fake Provider 走完整 App control/Sidecar/Supervisor/Hope Server 路径，必须零外部费用；它运行两个相同 case 的独立 trial 并验证时间区间真实重叠，防止“配置写了并发但实际串行”。
- smoke 写入 synthetic secret canary，并扫描 request/plan/DB/log/evidence/artifact/临时目录；任一泄漏或残留进程/端口都失败。
- 同一 request/资产/runtime/model 生成稳定 plan/digest；本地 App evidence 必须保持 local source，不能通过保留的 release verifier。
- 比较测试必须覆盖“不同 commit → seed 不同但逻辑 trial identity 相同”，预算测试必须覆盖小于模型数时拒绝且任何切分不增加总量。
- `cargo test -p ha-core -p ha-server --locked` 仍不链接/运行完整 Runner；普通 PR 和 GitHub Actions 不运行 fake smoke，也不配置 Provider Key。需要时在本地显式运行 smoke。
- 三平台发布前测量 Sidecar 压缩增量；目标不超过 35 MB。打包脚本使用独立 `eval-sidecar` profile（size opt、fat LTO、单 codegen unit、strip、panic abort）；panic 只终止隔离 Sidecar/worker，不改变产品 Agent 的 unwind 策略。超出时继续做依赖裁剪，不能降级为不校验的在线下载。
