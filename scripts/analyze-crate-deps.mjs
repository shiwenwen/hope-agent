#!/usr/bin/env node
// ha-core 模块依赖分析器 —— crate 拆分方案的数字来源。
//
// 为什么需要它：`docs/plan/crate-split.md` 里的每个数字（强连通环大小、
// 各特征 crate 的 LOC 与"需切回边"数）都会随代码演进而变。把口径固化成脚本，
// 方案文档就只描述方法与结论，具体数字随时重算。
//
// 入口：`pnpm analyze:crate-deps`。crate 拆分（ha-base / ha-config-schema /
// 后续特征 crate）各阶段的 SCC 基线、切边清单都由本脚本复现——拆分决策的
// 数字必须可再生，不能只活在一次性的分析会话里。
// 等 crate 拆分真正开工时再提升为 `scripts/analyze-crate-deps.mjs` + package.json
// 的 `analyze:crate-deps` 入口，与那时补的 `docs/architecture/` 正式文档一起提交。
//
// 口径（与文档一致，改动口径必须同步改文档）：
//   1. 节点 = `crates/ha-core/src/` 下的顶层模块（目录或 .rs 文件）。`tools/`
//      额外按子文件/子目录炸开成 `tools::<x>`，因为 tools 是分发注册表：它的
//      每个 adapter 属于对应特征，整体绑在一起会掩盖真实耦合。
//   2. 边 = 源码里的 `crate::<模块>` 引用。**行注释与文档注释一律剔除** ——
//      intra-doc link（`[crate::x]`）不产生编译依赖，不剔除会把大环虚报一圈。
//   3. `#[cfg(test)]` 块内的引用单独计为 test 边（含**模块级传播**：父文件里
//      `#[cfg(test)] mod tests;` 的外置文件如 workflow/tests.rs 整体按测试计）。
//      测试边同样阻塞 crate 拆分（`cargo test` 要编），但修复成本低得多（挪进
//      `tests/` 集成测试即可），所以必须和真依赖分开看。
//   4. "需切回边" = 目标分组之外的模块指向组内的引用数，**排除 app_init /
//      globals**（目标形态里它们是随门面落在最顶层的装配层，向下依赖合法，
//      单列在"装配"列）。其中来自其它特征分组的是合法的 crate 间依赖（只要
//      分组图无环——脚本对分组图跑完整 SCC，成对回边检查会漏长环）。
//
// 用法：
//   node scripts/analyze-crate-deps.mjs            # 摘要（默认只算非测试边）
//   node scripts/analyze-crate-deps.mjs --tests    # 把 cfg(test) 边一起算进来
//   node scripts/analyze-crate-deps.mjs --edges    # 附每条回边的 file:line
//   node scripts/analyze-crate-deps.mjs --json     # 机器可读

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, "..")
const SRC = path.join(repoRoot, "crates/ha-core/src")
// 已独立成 crate 的模块不再出现在 ha-core/src 下，BASE 列表相应收缩到
// 「仍留在 ha-core、待下沉」的部分。已落地的 ha-base 成员见 crates/ha-base/。

const argv = new Set(process.argv.slice(2))
const SHOW_EDGES = argv.has("--edges")
const AS_JSON = argv.has("--json")
const WITH_TESTS = argv.has("--tests")

// ── 目标分层（方案文档 §4 的分组，改这里必须同步改文档） ──────────────────

/**
 * Layer 0 的**目标**成员。脚本会自动求它的最大"已干净"子集（不断剔除仍指向
 * 组外的成员直到不动点）——干净子集 = 今天就能直接搬走的，差集 = 待解耦清单。
 * 手写清单会腐烂，所以这里只声明目标，干净与否由代码现状决定。
 */
const BASE = [
  // 阶段 2 v2 修正后：**没有**剩余的 ha-base 目标模块——
  //   · config / filesystem / file_upload / url_preview / test_support 满身
  //     cached_config() 读取，归属 kernel 层（schema 在 base 之上，base 永远
  //     见不到 AppConfig）；它们的出环靠切边（已完成），不靠挪 crate
  //   · file_extract 维持 v1 刻意排除（pdfium / calamine 等重依赖不进基础层）
  //   · process_registry 已实际迁入 ha-base
  // 空集时 §Layer 0 输出"无剩余目标"。若未来重新指定目标，成员必须真实存在
  // ——脚本对不存在的声明节点**直接报错**（防清单腐烂静默失效）。
]

/** Layer 2 —— 特征 crate。互相之间只允许单向依赖，成环即需合并或再切。
 * 已迁出成真 crate 的分组（ha-updater / ha-weather / ha-acp / ha-mac）一律删除。 */
const FEATURES = {
  // ha-updater：阶段 3 首个特征 crate，已实际迁出（crates/ha-updater/），
  // 不再是 ha-core 内的分组。已拆出的 crate 一律从本表删除——留着会以
  // LOC 0 的幽灵行出现在报表里。
  // ha-improve 已实际迁出（crates/ha-improve/，阶段 5 第八刀）——**台账留
  // kernel**：`coding_improvement` / `domain_eval` / `domain_quality` 三个
  // 模块的 `impl SessionDB` 里，124 个直接摸连接的方法与全部 wire 类型 /
  // 行映射原地不动，只有 34 个**一处连接都不碰**的顶层入口（四道闸、提案
  // 流水线、报表与执行器）改写成自由函数上浮。
  //
  // 这条边界回答的正是本表旧注释里的那个悬案（「100 处 `self.conn.lock()`
  // 搬走就得把可写连接开成跨 crate 公开 API」）——不动点算出来的答案是
  // **一处都不用开**：`with_conn_internal` 仍是 `pub(crate)`，特征 crate 只
  // 经类型化仓储方法触达 `sessions.db`，ha-dash 那刀立的契约完好。
  //
  // `review` / `verification` / `domain_workflow` **不随本刀迁出**，归属仍
  // 待定：它们同时是 workflow 的内置步骤与 Goal / Loop 的模板来源
  // （`workflow.review` / `workflow.verify` / `workflow.evidence.record` 三个
  // op 就在 `workflow/runtime.rs` 里），真要上浮得给 runtime 开 6~7 个钩子并
  // 下沉一族 wire 类型；也可能最终判定它们本就是控制面本体、该留 kernel。
  // 它们**留在本表**作为唯一的待定组——本表的用途是追踪「还没拆的」，
  // 组名 `ha-workflow-quality` 是候选名不是结论。
  //
  // `lsp` / `tools::lsp` 已判定**留 kernel**：被 kernel 核心路径消费（agent
  // streaming loop 的诊断 prompt 段 + `apply_patch` / `edit` / `write` 三个
  // Core 工具的 `sync_file_after_tool`），同 `activity` / `local_model_jobs` /
  // `CronDB` 的规则，故从本组移出。
  "ha-workflow-quality": ["domain_workflow", "review", "verification"],
  // ha-knowledge 已实际迁出（crates/ha-knowledge/，阶段 5 第六刀）——**台账、
  // 契约与裁决留 kernel**：`knowledge/registry.rs`（`knowledge_bases` + 访问绑定
  // 的真相源，81 处直接 `session_db.conn.lock()`，撞「sessions.db 写连接不对
  // 特征 crate 开放」红线）、`access.rs`（`effective_kb_access` 是「KB 访问默认
  // deny」的唯一裁决点，不能挪到可选装配的钩子后面）、`types.rs` 与
  // `maintenance_defs.rs`（registry 方法签名用的 wire 类型，后者自
  // `maintenance/types.rs` 下沉）。正因 registry 留下，`KNOWLEDGE_DB` 全局、
  // `get_knowledge_db()` 与 `AppState.knowledge_db` 一处未改。
  // 迁出前报的 44 条需切边里约 36 条是 `KbAccess` / `KbAccessSource` /
  // `ChannelKbContext` / `NoteSearchHit` 的**纯类型引用**——契约留下即消失，
  // 真正开钩子的只有 4 处行为 + 6 处装配（`knowledge_hooks` 十槽）——工具面的
  // 解析链（`access_map_for_tool_ctx` 等）随裁决点留 kernel，刻意不走钩子。
  // ha-channel 已实际迁出（crates/ha-channel/，阶段 5 第五刀）——**台账与契约
  // 留 kernel**：`channel/db.rs`（红线「一 chat ↔ 一 session 双向 1:1、读写一律走
  // db.rs helper」的执行点，且它持 SessionDB 连接）、`cancel.rs`（绑
  // CHANNEL_CANCELS 全局与 AppState 的 ptr_eq_lock 不变量）、`traits.rs` +
  // `registry.rs`（契约与持有者，不是机器——正因它们留下，CHANNEL_REGISTRY
  // 全局与全部壳层调用点一处未改）、`types.rs` / `config.rs`（AppConfig wire
  // 类型）。35 个 `feishu_*` 名字常量下沉 `tool_defs::feishu_names`，兑现了
  // AGENTS 里那条「留给 ha-channel 那刀连同 channel/ 一起破」的欠账。
  // `tools::send_attachment` / `tools::notification` **留 kernel**：它们不是
  // IM 渠道专属（桌面通知 / 通用附件发送），归进本组只是主题相似。
  // `wakeup` **不在本组**（阶段 5 第三刀重新分组）：它对 cron / loop_control
  // 零引用、反向也零——两者只是「都跟排程有关」的主题相似，AGENTS 本身就写着
  // 「`schedule_wakeup` ≠ cron、不复用入口」。它的消费者全在 kernel（goal 的
  // 目标唤醒排程 / agent_lifecycle / session::cleanup_watcher），归 ha-cron 会
  // 凭空给这一刀记上 11 条并不存在的切边。`tools::schedule_wakeup` 同理随之留下。
  // ha-cron 已实际迁出（crates/ha-cron/，阶段 5 第三刀）——但**只迁走了机器**：
  // 调度器 / 执行器 / 投递 / 时间线 + `manage_cron` adapter。台账 `cron/db.rs`
  // 与排程算术 `cron/schedule.rs`、内存取消注册表 `cron/cancel.rs`、wire 类型
  // `cron_defs` 全部**留 kernel**——`loop_control` 的托管 `/loop` 全程持
  // `&CronDB`（20+ 处签名）、`agent_lifecycle` 改名时重写 payload，台账不是
  // cron 调度专属。同破环那刀对 `local_model_jobs` 的分法。
  // `loop_control` 与 `tools::loop_tool` 也留 kernel：前者有一个 58 方法、
  // 2673 行的 `impl SessionDB` 块，固有 impl 只能待在定义 `SessionDB` 的
  // crate 里，改扩展 trait 会让 kernel 的 15+ 处调用点反向 use ha-cron。
  // slash_commands 已移出：它是**装配层**（handler 逐个调 skills / channel /
  // cron / dash / improve，出度 30 模块、入度 2），与 app_init / globals 同型，
  // 见下方 ASSEMBLY。契约物在 kernel 的 slash_defs，分发经 slash_hooks 三槽。
  // ha-skills 已实际迁出（crates/ha-skills/，阶段 5 第七刀）——**契约、台账与
  // 纯谓词留 kernel**：`skills/types.rs`（`SkillEntry` 等 wire 契约 +
  // `skill_cache_version` / `bump_skill_version` 目录版本计数器）、
  // `skills/activation.rs`（`session_skill_activation` 表 + 进程内热缓存的真相
  // 源，三个 kernel 调用点读写：`tools::execution` 写 / `system_prompt` 读 /
  // `session::cleanup_watcher` 清）、`skills/{requirements,prompt,slash}.rs`
  // （对契约类型的纯谓词与纯渲染，不碰文件系统 / LLM / 网络）。
  // 迁出前报 19 条需切 + 16 条装配，契约留下后真正开钩子的只有 8 处行为 +
  // 1 处装配（`skills_hooks` 九槽）——`tools::execution` 的条件激活块与
  // `cleanup_watcher` 的清理**一行未改**。
  // ha-dash 已实际迁出（crates/ha-dash/，阶段 5 第二刀）。`activity` **留在
  // kernel**：它是 `impl SessionDB` 的扩展方法，唯一 kernel 消费者是 Core
  // 工具 tools::goal（无条件注册，放钩子后面会静默缺数据）。
  // **代价**：activity.rs 的 `use crate::loop_control::…` 因此从
  // `ha-dash → ha-cron` 兄弟边变成 kernel→ha-cron，现计入 ha-cron 的需切列，
  // 且它调的两个 `impl SessionDB` 方法就写在 loop_control.rs 里——ha-cron 那
  // 刀必须先解这条。**这与 ha-vcs 的 worktree / project_bootstrap 不是一回事**
  // （那两个没造出反向边），别拿来当先例。
  // ha-local-llm 已实际迁出（crates/ha-local-llm/，阶段 5 首刀），同
  // ha-updater / ha-acp / ha-weather 从本表删除。留在 kernel 的
  // `local_model_jobs` 是**通用后台任务台账**（DB / 快照类型 / spawn /
  // finish / 进度写入）：memory::reembed_job 与知识库 reembed 都靠它记账，
  // 合进 local-llm 会让 knowledge 为了记账而依赖它（7-环最后一条边）。
  // ha-acp 已实际迁出（crates/ha-acp/），同 ha-updater / ha-weather 从本表删除。
  // weather_location_macos 已随阶段 2 v1 迁入 ha-base（平台原语）；
  // ha-weather 本体已实际迁出（crates/ha-weather/），同 ha-updater 从本表删除。
}

// 分组必须互斥：同一模块出现在两组会让 featureOwner 静默用后者覆盖前者，
// LOC 又在两组各算一遍——残留 core 被少算、结论自相矛盾。启动即断言。
{
  const owner = new Map()
  for (const [g, ms] of [["ha-base", BASE], ...Object.entries(FEATURES)])
    for (const m of ms) {
      if (owner.has(m))
        throw new Error(`模块 ${m} 同时属于 ${owner.get(m)} 与 ${g} —— 分组必须互斥`)
      owner.set(m, g)
    }
}

// ── 扫描 ─────────────────────────────────────────────────────────────────

function walkRs(dir) {
  const out = []
  for (const e of readdirSync(dir)) {
    const p = path.join(dir, e)
    if (statSync(p).isDirectory()) out.push(...walkRs(p))
    else if (e.endsWith(".rs")) out.push(p)
  }
  return out
}

/** 节点 → 文件列表。`tools/` 按子项炸开。 */
function collectNodes() {
  const nodes = new Map()
  for (const e of readdirSync(SRC).sort()) {
    const p = path.join(SRC, e)
    const isDir = statSync(p).isDirectory()
    if (e === "tools" && isDir) {
      for (const sub of readdirSync(p).sort()) {
        const q = path.join(p, sub)
        if (statSync(q).isDirectory()) nodes.set(`tools::${sub}`, walkRs(q))
        else if (sub.endsWith(".rs")) nodes.set(`tools::${sub.slice(0, -3)}`, [q])
      }
    } else if (isDir) nodes.set(e, walkRs(p))
    else if (e.endsWith(".rs") && e !== "lib.rs") nodes.set(e.slice(0, -3), [p])
  }
  return nodes
}

const nodes = collectNodes()

// 声明分组里的顶层模块必须真实存在于当前代码——静默忽略会让清单腐烂后
// 输出貌似合理的错误结论（曾把已迁走的 process_registry 继续算作 ha-base
// 目标）。`tools::x` 形式的 adapter 子路径不在节点表里，另行跳过。
for (const [g, ms] of [["ha-base", BASE], ...Object.entries(FEATURES)]) {
  for (const m of ms) {
    if (!m.includes("::") && !nodes.has(m)) {
      console.error(
        `✗ 分组 ${g} 声明的模块 \`${m}\` 不存在于 ha-core/src——清单已过期，请同步方案文档后修正`,
      )
      process.exit(1)
    }
  }
}
const REF = /\bcrate::([a-z_][a-z0-9_]*)(?:::([a-z_][a-z0-9_]*))?/g
const COMMENT = /^\s*(\/\/\/|\/\/!|\/\/)/

/** `crate::a::b` → 节点名。tools 的引用尽量落到具体 adapter。 */
function resolveRef(a, b) {
  if (a === "tools") {
    const specific = b ? `tools::${b}` : null
    if (specific && nodes.has(specific)) return specific
    return nodes.has("tools::mod") ? "tools::mod" : null
  }
  return nodes.has(a) ? a : null
}

const CFG_TEST = /#\[cfg\((any\()?test\b/

// ── cfg(test) 的模块级传播 ───────────────────────────────────────────────
// `#[cfg(test)] mod tests;`（分号形式）把整个外置文件变成测试代码，但目标文件
// 内部没有任何 cfg 标记——只看单文件会把 workflow/tests.rs 等的几十条 crate::
// 引用误判成生产依赖。预扫一遍父文件，把这些外置模块（含目录）整体标记为测试。
const testFiles = new Set()
{
  const MOD_DECL = /^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+([a-z_0-9]+)\s*;/
  for (const files of nodes.values()) {
    for (const f of files) {
      let txt
      try {
        txt = readFileSync(f, "utf8")
      } catch {
        continue
      }
      const lines = txt.split("\n")
      lines.forEach((line, i) => {
        if (!CFG_TEST.test(line)) return
        // cfg 属性与 mod 声明之间只允许夹杂其它属性/注释/空行；一旦撞上任何
        // 别的 item 就停——cfg 作用于那个 item，后续 `mod x;` 是无关的生产模块，
        // 继续扫会把它整文件误标成测试。
        for (let j = i; j <= i + 3 && j < lines.length; j++) {
          const probe = j === i ? line.replace(CFG_TEST, "") : lines[j]
          const m = MOD_DECL.exec(probe)
          if (!m) {
            if (j === i) continue // cfg 属性本行（replace 后只剩 `)]` 残留），不算 item
            const rest = probe.trim()
            if (rest === "" || rest.startsWith("#[") || rest.startsWith("//")) continue
            break // 撞上非属性 item，cfg 已被它消费
          }
          const dir =
            path.basename(f) === "mod.rs" || path.basename(f) === "lib.rs"
              ? path.dirname(f)
              : f.slice(0, -3) // foo.rs 的子模块在 foo/ 下
          for (const cand of [path.join(dir, `${m[1]}.rs`), path.join(dir, m[1], "mod.rs")])
            if (existsSync(cand)) {
              testFiles.add(cand)
              const subdir = path.join(dir, m[1])
              if (existsSync(subdir) && statSync(subdir).isDirectory())
                for (const sub of walkRs(subdir)) testFiles.add(sub)
            }
          break
        }
      })
    }
  }
}

const deps = new Map() // node -> Map(target -> count)   非测试边
const testDeps = new Map() // node -> Map(target -> count)   cfg(test) 边
const loc = new Map()
const edgeSites = new Map() // `${from} ${to}` -> ["file:line: src"]

for (const [n, files] of nodes) {
  const d = new Map()
  const td = new Map()
  let lines = 0
  for (const f of files) {
    let txt
    try {
      txt = readFileSync(f, "utf8")
    } catch {
      continue
    }
    const rel = path.relative(SRC, f)
    const arr = txt.split("\n")
    lines += arr.length
    const fileIsTest = testFiles.has(f)
    // 括号配平地跟踪 `#[cfg(test)]` 作用域：属性出现时记下当时的深度，深度回落
    // 到该值即离开这个块。不复位会把文件后半段全部误判成测试代码。
    let depth = 0
    let testDepth = null
    let pendingTest = false
    arr.forEach((line, i) => {
      if (COMMENT.test(line)) return
      if (CFG_TEST.test(line)) pendingTest = true
      // pendingTest 期间（属性行到其作用的 item 之间）的引用同样是测试门控的，
      // 覆盖 `#[cfg(test)] use crate::x;` 这类单行形式。
      const inTest = fileIsTest || testDepth !== null || pendingTest
      REF.lastIndex = 0
      let m
      while ((m = REF.exec(line))) {
        const t = resolveRef(m[1], m[2])
        if (!t || t === n) continue
        const bucket = inTest ? td : d
        bucket.set(t, (bucket.get(t) ?? 0) + 1)
        const k = `${n} ${t}`
        if (!edgeSites.has(k)) edgeSites.set(k, [])
        edgeSites
          .get(k)
          .push((inTest ? "[test] " : "") + rel + ":" + (i + 1) + ": " + line.trim().slice(0, 120))
      }

      const opens = (line.match(/\{/g) ?? []).length
      const closes = (line.match(/\}/g) ?? []).length
      if (pendingTest) {
        if (opens > 0) {
          // 块形式（`mod tests {`）：进入括号配平的测试作用域
          if (testDepth === null) testDepth = depth
          pendingTest = false
        } else if (/;\s*(\/\/.*)?$/.test(line)) {
          // 分号形式（`mod tests;` / `use …;`）：单 item 结束即离开，
          // 不清除会把父文件后续第一个带花括号的声明整个误判成测试
          pendingTest = false
        }
      }
      depth += opens - closes
      if (testDepth !== null && depth <= testDepth) testDepth = null
    })
  }
  deps.set(n, d)
  testDeps.set(n, td)
  loc.set(n, lines)
}

// `--tests` 把 cfg(test) 边并回主图：拆 crate 后 `cargo test` 同样要能编过，
// 但这批边通常靠"测试挪进 tests/ 集成测试"就能化解，成本远低于真依赖。
if (WITH_TESTS) {
  for (const [n, td] of testDeps) {
    const d = deps.get(n)
    for (const [t, c] of td) d.set(t, (d.get(t) ?? 0) + c)
  }
}

// ── 红线守卫：tool_defs（工具契约层）单向依赖 tools（分发注册表）──────────
//
// 阶段 4「类型归位」的全部价值在这条方向上：契约物下沉后 kernel 不再经
// `tools/` 中转，`tools/` 才能随特征 crate 上浮。**同 crate 内加一条回边照样
// 编译通过**，所以只靠 review 守不住——这里断言到底，回归即非零退出。
// 契约见 AGENTS.md「工具契约层 tool_defs 与分发层 tools 单向」与
// docs/architecture/backend-separation.md 的 tool_defs 小节。
//
// 默认（生产边）：零容忍。`--tests`：只放行文档登记的 3 条遗留测试边
// （types.rs / metadata.rs 的全表断言），多一条就报错——新增测试边请直接
// 写进 `tests/` 集成测试，别在这里加白名单。
const TOOL_DEFS_TEST_EDGE_BUDGET = { "tools::dispatch": 2, "tools::mod": 1 }

{
  const offenders = [...(deps.get("tool_defs") ?? new Map())].filter(([t]) =>
    t.startsWith("tools::"),
  )
  const budget = WITH_TESTS ? TOOL_DEFS_TEST_EDGE_BUDGET : {}
  const violations = offenders.filter(([t, c]) => c > (budget[t] ?? 0))
  if (violations.length) {
    console.error(
      `✗ 红线违规：tool_defs（契约层）依赖 tools（分发层）——` +
        violations.map(([t, c]) => `${t}(${c}/${budget[t] ?? 0})`).join(" "),
    )
    console.error(
      WITH_TESTS
        ? "  超出已登记的 3 条遗留测试边。新测试请放 tests/ 集成测试，不要扩白名单。"
        : "  契约层不得反向依赖分发层：需要分发行为的方法改 extension trait 挂 tools/ 侧" +
            "（现有 dispatch::ToolDefinitionApiExt）。",
    )
    process.exit(1)
  }
}

// ── 红线守卫：slash 三层单向（契约层 → 装配层 方向禁止）─────────────────
//
// 阶段 4 破环把 slash 拆成 slash_defs（契约层，kernel）/ slash_hooks（回调
// 面）/ slash_commands（装配层）。装配层进了下面的 `ASSEMBLY` 名单，而名单
// **只对「以它为源的边」豁免**——指向它的边不进任何分组指标（它不属任何
// FEATURES 组），所以将来某个特征模块加一条 `use crate::slash_commands::…`，
// 四个头条指标会纹丝不动。那正是名单开始说谎的时刻，只能靠断言兜住。
//
// 两条：① 除装配层自身与 app_init（钩子注册点）外，任何模块不得依赖
// slash_commands；② slash_defs 不得反向依赖 slash_commands。
// 契约见 AGENTS.md 与 backend-separation.md 的装配层小节。
{
  const ALLOWED_INBOUND = new Set(["app_init"])
  const inbound = [...deps]
    .filter(([m]) => m !== "slash_commands" && !ALLOWED_INBOUND.has(m))
    .map(([m, d]) => [m, d.get("slash_commands") ?? 0])
    .filter(([, c]) => c > 0)
  if (inbound.length) {
    console.error(
      `✗ 红线违规：模块直接依赖装配层 slash_commands——` +
        inbound.map(([m, c]) => `${m}(${c})`).join(" "),
    )
    console.error(
      "  装配层入向必须为零或走钩子：契约物请用 crate::slash_defs::…，" +
        "真分发请走 crate::slash_hooks 三槽（dispatch / menu_entries / skill_command_help）。",
    )
    process.exit(1)
  }
  const backEdge = (deps.get("slash_defs") ?? new Map()).get("slash_commands") ?? 0
  if (backEdge > 0) {
    console.error(
      `✗ 红线违规：slash_defs（契约层）依赖 slash_commands（装配层）——${backEdge} 处`,
    )
    process.exit(1)
  }
}

const rev = new Map([...nodes.keys()].map((n) => [n, new Map()]))
for (const [m, d] of deps) for (const [t, c] of d) rev.get(t).set(m, c)

// ── Tarjan SCC ───────────────────────────────────────────────────────────

function tarjan(allNodes, succOf) {
  const index = new Map(),
    low = new Map(),
    onStack = new Set(),
    stack = [],
    comps = []
  let counter = 0
  for (const root of allNodes) {
    if (index.has(root)) continue
    const work = [[root, succOf(root), 0]]
    index.set(root, counter)
    low.set(root, counter++)
    stack.push(root)
    onStack.add(root)
    while (work.length) {
      const frame = work[work.length - 1]
      const [v, succ] = frame
      let advanced = false
      while (frame[2] < succ.length) {
        const w = succ[frame[2]++]
        if (!index.has(w)) {
          index.set(w, counter)
          low.set(w, counter++)
          stack.push(w)
          onStack.add(w)
          work.push([w, succOf(w), 0])
          advanced = true
          break
        } else if (onStack.has(w)) low.set(v, Math.min(low.get(v), index.get(w)))
      }
      if (advanced) continue
      work.pop()
      if (work.length) {
        const p = work[work.length - 1][0]
        low.set(p, Math.min(low.get(p), low.get(v)))
      }
      if (low.get(v) === index.get(v)) {
        const comp = []
        for (;;) {
          const w = stack.pop()
          onStack.delete(w)
          comp.push(w)
          if (w === v) break
        }
        comps.push(comp)
      }
    }
  }
  return comps
}

const sumLoc = (ms) => ms.reduce((s, m) => s + (loc.get(m) ?? 0), 0)
const comps = tarjan(nodes.keys(), (n) => [...(deps.get(n)?.keys() ?? [])]).sort(
  (a, b) => sumLoc(b) - sumLoc(a),
)
const cycle = comps[0].length > 1 ? new Set(comps[0]) : new Set()

// ── 分组切割成本 ─────────────────────────────────────────────────────────

// 装配层（composition root）：目标形态里 app_init / globals / slash_commands 随
// 门面 ha-core 落在最顶层，向下依赖任何特征都合法——它们指向特征的边不是切割
// 成本，单独归类。
//
// slash_commands 入列的判据（阶段 4 破环）：出度 30 个模块、入度只有 2 个，
// 且入向已全部改走 kernel——契约物（命令表 / wire 类型 / parser / fuzzy /
// 转录落库 / 选择器渲染）归位 `slash_defs`，真正的分发经 `slash_hooks` 三槽
// 回调。**这两条是它算装配层的前提**：若哪天又出现「特征模块直接
// `use crate::slash_commands::…`」，本表就在说谎，应当先把那条边改成钩子。
const ASSEMBLY = new Set(["app_init", "globals", "slash_commands"])

function cutCost(members) {
  const S = new Set(members.filter((m) => nodes.has(m)))
  const incoming = new Map()
  const assembly = new Map()
  for (const m of S)
    for (const [src, c] of rev.get(m) ?? new Map()) {
      if (S.has(src)) continue
      const bucket = ASSEMBLY.has(src) ? assembly : incoming
      bucket.set(src, (bucket.get(src) ?? 0) + c)
    }
  return {
    S,
    incoming,
    assembly,
    total: [...incoming.values()].reduce((a, b) => a + b, 0),
    assemblyTotal: [...assembly.values()].reduce((a, b) => a + b, 0),
    loc: sumLoc([...S]),
  }
}

const featureOwner = new Map()
for (const [g, ms] of Object.entries(FEATURES)) for (const m of ms) featureOwner.set(m, g)

const report = {
  totalLoc: sumLoc([...nodes.keys()]),
  nodeCount: nodes.size,
  cycle: {
    size: cycle.size,
    loc: sumLoc([...cycle]),
    members: [...cycle].sort((a, b) => loc.get(b) - loc.get(a)),
  },
  // `tools/` 是分发注册表：core 侧对它的引用量决定了它能否随特征上浮（见方案 §3.2）。
  // 阶段 4「类型归位」后共享契约物已迁 `tool_defs/`（独立节点、不计入本指标），
  // 所以这里剩下的才是真正指向分发层/adapter 的引用。
  toolsPressure: cutCost([...nodes.keys()].filter((n) => n.startsWith("tools::"))),
  deps: Object.fromEntries([...deps].map(([m, d]) => [m, Object.fromEntries(d)])),
  base: cutCost(BASE),
  baseClean: null,
  baseBlocked: [],
  features: {},
  siblingEdges: [],
}

// 最大干净底层子集：反复剔除仍指向组外的成员，直到不动点。
{
  let clean = new Set(BASE.filter((m) => nodes.has(m)))
  for (;;) {
    const drop = [...clean].filter((m) =>
      [...(deps.get(m) ?? new Map()).keys()].some((t) => !clean.has(t)),
    )
    if (!drop.length) break
    for (const m of drop) clean.delete(m)
  }
  report.baseClean = {
    loc: sumLoc([...clean]),
    members: [...clean].sort((a, b) => loc.get(b) - loc.get(a)),
  }
  // 区分两类：真正越界（指向 BASE 目标集之外）才是工作项；只因依赖了某个越界
  // 成员而被级联剔除的，随根因一起解决。不区分会把"一处引用"放大成一屏清单。
  const target = new Set(BASE.filter((m) => nodes.has(m)))
  for (const m of BASE.filter((m) => nodes.has(m) && !clean.has(m))) {
    const outs = [...(deps.get(m) ?? new Map())].filter(([t]) => !target.has(t))
    report.baseBlocked.push({
      member: m,
      loc: loc.get(m) ?? 0,
      refs: outs.reduce((s, [, c]) => s + c, 0),
      direct: outs.length > 0,
      targets: outs.sort((a, b) => b[1] - a[1]).map(([t, c]) => `${t}(${c})`),
    })
  }
  report.baseBlocked.sort((a, b) => b.refs - a.refs)
}
for (const [g, ms] of Object.entries(FEATURES)) report.features[g] = cutCost(ms)

for (const [m, d] of deps) {
  const a = featureOwner.get(m)
  if (!a) continue
  for (const [t, c] of d) {
    const b = featureOwner.get(t)
    if (b && b !== a) {
      const hit = report.siblingEdges.find((e) => e.from === a && e.to === b)
      if (hit) hit.refs += c
      else report.siblingEdges.push({ from: a, to: b, refs: c })
    }
  }
}
report.siblingEdges.sort((x, y) => y.refs - x.refs)

// 特征分组图的完整 SCC：只查 A↔B 成对回边会漏掉三节点以上的环（如
// channel→skills→cron→channel），而环里的每个成员都不能先于破环单独拆出。
{
  const succ = new Map()
  for (const e of report.siblingEdges) {
    if (!succ.has(e.from)) succ.set(e.from, [])
    succ.get(e.from).push(e.to)
  }
  report.featureScc = tarjan(Object.keys(FEATURES), (g) => succ.get(g) ?? []).filter(
    (c) => c.length > 1,
  )
}

// ── 红线守卫：特征分组图必须保持无环 ──────────────────────────────────────
//
// 阶段 4 破环（A slash 三分 / B learning 事件下沉 / C KB 闸门归位 /
// D 任务台账留 kernel）把 7-环打成 DAG。**这个状态不能回退**：环内任何成员
// 都不能先于破环单独成 crate——已拆出的会依赖仍在残留 blob 里的环友，blob
// 又依赖已拆出者，Cargo 直接拒绝。一条随手加的特征间反向边就能把后续所有
// 拆分卡死，而它在编译期完全无感。
//
// 兄弟间**单向**边仍然合法（只约束拆分顺序），成环才拒。
if (report.featureScc.length) {
  console.error("✗ 红线违规：特征分组图重新成环——阶段 4 的破环成果被回退")
  for (const c of report.featureScc) {
    console.error(`  环: ${c.join(" ↔ ")}`)
    for (const e of report.siblingEdges) {
      if (c.includes(e.from) && c.includes(e.to)) {
        console.error(`    ${e.from} -> ${e.to}  ${e.refs}`)
      }
    }
  }
  console.error(
    "  兄弟间单向边合法、成环不合法：把回边下沉 kernel typed port / 注册钩子，" +
      "或重新分组。详见 docs/architecture/backend-separation.md「破环完成」小节。",
  )
  process.exit(1)
}

// ── 输出 ─────────────────────────────────────────────────────────────────

if (AS_JSON) {
  console.log(
    JSON.stringify(
      report,
      (_k, v) => (v instanceof Set ? [...v] : v instanceof Map ? Object.fromEntries(v) : v),
      2,
    ),
  )
  process.exit(0)
}

const pad = (s, n) => String(s).padEnd(n)
const num = (s, n) => String(s).padStart(n)

console.log(`ha-core: ${report.nodeCount} 个分析节点, ${report.totalLoc} 行\n`)
console.log(`最大强连通环: ${report.cycle.size} 个模块, ${report.cycle.loc} 行`)
console.log(
  `  ${report.cycle.members.slice(0, 12).join(", ")}${report.cycle.size > 12 ? " …" : ""}\n`,
)

console.log(
  `tools/ 注册表压力: 非 tools 模块对 tools::* 共 ${report.toolsPressure.total} 处引用` +
    `  —— ${[...report.toolsPressure.incoming.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6)
      .map(([s, c]) => `${s}(${c})`)
      .join(" ")}\n`,
)

console.log(`Layer 0  ha-base 目标: ${report.base.loc} 行`)
console.log(
  `  ✅ 已干净、可直接搬: ${report.baseClean.loc} 行 —— ${report.baseClean.members.join(", ")}`,
)
const directBlockers = report.baseBlocked.filter((b) => b.direct)
const cascaded = report.baseBlocked.filter((b) => !b.direct)
if (directBlockers.length) {
  console.log(`  ⚠️  真正越界的根因（这些是工作项）:`)
  for (const b of directBlockers)
    console.log(
      `     ${pad(b.member, 20)}${num(b.loc, 7)}  ${num(b.refs, 4)} 处 -> ${b.targets.slice(0, 6).join(", ")}${b.targets.length > 6 ? " …" : ""}`,
    )
}
if (cascaded.length)
  console.log(
    `  ↳ 仅被级联带下水（根因修完即自动干净）: ${cascaded.map((b) => b.member).join(", ")}`,
  )

console.log(
  `\nLayer 2  特征 crate（"需切"已排除 app_init/globals 装配边；仍含来自兄弟特征的合法依赖，须逐条区分）:`,
)
console.log(`  ${pad("crate", 16)}${num("LOC", 8)}${num("需切", 7)}${num("装配", 6)}   主要来源`)
const feats = Object.entries(report.features).sort((a, b) => a[1].total - b[1].total)
for (const [g, r] of feats) {
  const top = [...r.incoming.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, 5)
    .map(([s, c]) => `${s}(${c})`)
    .join(" ")
  console.log(
    `  ${pad(g, 16)}${num(r.loc, 8)}${num(r.total, 7)}${num(r.assemblyTotal, 6)}   ${top}`,
  )
}
const featLoc = Object.values(report.features).reduce((s, r) => s + r.loc, 0)
console.log(`\n  特征层合计 ${featLoc} 行 (${((featLoc / report.totalLoc) * 100).toFixed(1)}%)`)
console.log(`  残留 core   ${report.totalLoc - featLoc - report.base.loc} 行`)

console.log(`\n特征分组图的强连通分量（环内成员必须先破环才能各自成 crate）:`)
if (!report.featureScc.length)
  console.log(`  ✅ 无环 —— 存在可行的拓扑拆分顺序（下方单向边即该顺序的约束）`)
const inScc = new Set(report.featureScc.flat())
for (const c of report.featureScc) {
  console.log(`  ⚠️  ${c.length}-crate 环: ${c.join(" ↔ ")}`)
  for (const e of report.siblingEdges)
    if (c.includes(e.from) && c.includes(e.to))
      console.log(`       ${pad(e.from, 14)} -> ${pad(e.to, 14)} ${num(e.refs, 4)}`)
}

console.log(`\n兄弟特征之间的单向边（合法，只约束拆分顺序）:`)
const sccOf = new Map()
report.featureScc.forEach((c, i) => c.forEach((g) => sccOf.set(g, i)))
for (const e of report.siblingEdges)
  if (!(sccOf.has(e.from) && sccOf.get(e.from) === sccOf.get(e.to)))
    console.log(`  ${pad(e.from, 14)} -> ${pad(e.to, 14)} ${num(e.refs, 4)}`)

if (SHOW_EDGES) {
  console.log(`\n──── 逐条回边（file:line） ────`)
  for (const [g, r] of feats) {
    console.log(`\n########## ${g} ##########`)
    for (const [src] of [...r.incoming.entries()].sort((a, b) => b[1] - a[1])) {
      for (const m of r.S) {
        const sites = edgeSites.get(`${src} ${m}`)
        if (sites) for (const s of sites) console.log(`  ${s}`)
      }
    }
  }
}
