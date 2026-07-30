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
  "ha-improve": [
    "coding_improvement",
    "coding_eval",
    "domain_eval",
    "domain_quality",
    "domain_workflow",
    "context_retrieval",
    "review",
    "verification",
    "lsp",
    "evaluation",
    "tools::lsp",
  ],
  "ha-knowledge": ["knowledge", "tools::note"],
  "ha-channel": ["channel", "tools::feishu", "tools::send_attachment", "tools::notification"],
  "ha-cron": [
    "cron",
    "loop_control",
    "wakeup",
    "tools::cron",
    "tools::loop_tool",
    "tools::schedule_wakeup",
  ],
  "ha-skills": ["skills", "slash_commands", "tools::skill"],
  "ha-dash": ["dashboard", "recap", "activity"],
  "ha-local-llm": ["local_llm", "local_model_jobs", "local_embedding"],
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

// 装配层（composition root）：目标形态里 app_init / globals 随门面 ha-core 落在
// 最顶层，向下依赖任何特征都合法——它们指向特征的边不是切割成本，单独归类。
const ASSEMBLY = new Set(["app_init", "globals"])

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
if (!report.featureScc.length) console.log(`  ✅ 无环 —— 全部特征可按任意顺序拆出`)
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
