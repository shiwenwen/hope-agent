#!/usr/bin/env node
// Architecture guard for docs/architecture/core/chat-engine.md and
// docs/architecture/system/backend-separation.md.
//
// Only the feature-owned engine may destructure ChatEngineParams and call the
// concrete AssistantAgent. Shells and sibling features must enter through the
// sealed TurnKernel admission API; no compatibility adapter is permitted.

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const asJson = process.argv.includes("--json")

const legacyEntrypoints = new Map()

const directAgentChat = new Map()
const streamingLoopAdapters = new Map([
  ["crates/ha-agent-runtime/src/chat_dispatch.rs", 4],
])
const turnProducerFiles = new Set([
  "src-tauri/src/commands/chat.rs",
  "crates/ha-server/src/routes/chat.rs",
  "crates/ha-acp/src/acp/agent.rs",
  "crates/ha-channel/src/channel/worker/dispatcher.rs",
  "crates/ha-cron/src/cron/executor.rs",
  "crates/ha-core/src/subagent/spawn.rs",
  "crates/ha-core/src/subagent/injection.rs",
  "crates/ha-core/src/tools/sessions.rs",
])
const evaluationModelChainFiles = new Set([
  "crates/ha-eval-runtime/src/coding_eval.rs",
  "crates/ha-improve/src/domain_eval.rs",
  "crates/ha-core/src/chat_engine/turn_kernel.rs",
])

function walkRust(directory) {
  if (!existsSync(directory)) return []
  const files = []
  for (const entry of readdirSync(directory)) {
    const full = path.join(directory, entry)
    if (statSync(full).isDirectory()) files.push(...walkRust(full))
    else if (entry.endsWith(".rs")) files.push(full)
  }
  return files
}

// Remove comments and literal contents while retaining identifiers and line
// breaks. This avoids false bypasses from architecture comments such as
// `agent.chat()` and from string fixtures.
function executableSurface(source) {
  let output = ""
  let index = 0
  let state = "code"
  let blockDepth = 0
  let rawTerminator = ""
  while (index < source.length) {
    const char = source[index]
    const next = source[index + 1]
    if (state === "code") {
      const rawStart = source.slice(index).match(/^(?:br|r)(#{0,255})"/)
      if (rawStart) {
        state = "raw-string"
        rawTerminator = `"${rawStart[1]}`
        output += " ".repeat(rawStart[0].length)
        index += rawStart[0].length
      } else if (char === "/" && next === "/") {
        state = "line-comment"
        output += "  "
        index += 2
      } else if (char === "/" && next === "*") {
        state = "block-comment"
        blockDepth = 1
        output += "  "
        index += 2
      } else if (char === '"') {
        state = "string"
        output += " "
        index += 1
      } else if (char === "'") {
        // Lifetimes (`'a`) are identifiers, not char literals. Only enter the
        // char state when a closing quote appears within the next few bytes.
        const tail = source.slice(index + 1, index + 6)
        if (/^(?:\\.|[^\\'])'/.test(tail)) {
          state = "char"
          output += " "
        } else {
          output += char
        }
        index += 1
      } else {
        output += char
        index += 1
      }
    } else if (state === "line-comment") {
      if (char === "\n") {
        state = "code"
        output += "\n"
      } else output += " "
      index += 1
    } else if (state === "block-comment") {
      if (char === "/" && next === "*") {
        blockDepth += 1
        output += "  "
        index += 2
      } else if (char === "*" && next === "/") {
        blockDepth -= 1
        if (blockDepth === 0) state = "code"
        output += "  "
        index += 2
      } else {
        output += char === "\n" ? "\n" : " "
        index += 1
      }
    } else if (state === "raw-string") {
      if (source.startsWith(rawTerminator, index)) {
        state = "code"
        output += " ".repeat(rawTerminator.length)
        index += rawTerminator.length
      } else {
        output += char === "\n" ? "\n" : " "
        index += 1
      }
    } else {
      // string / char
      if (char === "\\") {
        output += "  "
        index += 2
      } else if (
        (state === "string" && char === '"') ||
        (state === "char" && char === "'")
      ) {
        state = "code"
        output += " "
        index += 1
      } else {
        output += char === "\n" ? "\n" : " "
        index += 1
      }
    }
  }
  return output
}

function countMatches(text, pattern) {
  return [...text.matchAll(pattern)].length
}

const lexerProbe = executableSurface(`
fn probe<'a>(value: &'a str) {
  let _raw = r###"agent.chat(); ChatEngineParams { fake: true }"###;
  let _byte_raw = br#"run_chat_engine(fake)"#;
  /* outer /* nested agent.chat() */ still comment */
  agent.chat();
}
`)
if (countMatches(lexerProbe, /\bagent\s*\.\s*chat\s*\(/g) !== 1) {
  throw new Error("Rust surface lexer self-test failed")
}

const rustFiles = [
  ...walkRust(path.join(repoRoot, "crates")),
  ...walkRust(path.join(repoRoot, "src-tauri")),
]

const inventory = {
  chatEngineParamConstructors: {},
  runChatEngineCalls: {},
  directAgentChatCalls: {},
  streamingLoopCalls: {},
  runtimeCompatCalls: {},
  turnRequestLiterals: {},
  producerModelResolutionCalls: {},
  evaluationModelChainCalls: {},
}
const violations = []

for (const file of rustFiles) {
  const relative = path.relative(repoRoot, file).split(path.sep).join("/")
  const code = executableSurface(readFileSync(file, "utf8"))
  const isEngineInternals =
    relative.startsWith("crates/ha-core/src/chat_engine/") ||
    relative === "crates/ha-agent-runtime/src/engine.rs"

  if (!isEngineInternals) {
    const constructors = countMatches(code, /\bChatEngineParams\s*\{/g)
    const calls = countMatches(code, /\brun_chat_engine(?:_classified)?\s*\(/g)
    if (constructors) inventory.chatEngineParamConstructors[relative] = constructors
    if (calls) inventory.runChatEngineCalls[relative] = calls
    const allowed = legacyEntrypoints.get(relative) ?? 0
    if (constructors > allowed) {
      violations.push(`${relative}: ${constructors} ChatEngineParams constructor(s), allowed ${allowed}`)
    }
    if (calls > allowed) {
      violations.push(`${relative}: ${calls} run_chat_engine call(s), allowed ${allowed}`)
    }
  }

  if (relative !== "crates/ha-core/src/chat_engine/turn_kernel.rs") {
    const literals = countMatches(code, /\bTurnRequest\s*\{/g)
    if (literals) {
      inventory.turnRequestLiterals[relative] = literals
      violations.push(`${relative}: ${literals} external TurnRequest struct literal(s); use the sealed builder`)
    }
  }

  if (turnProducerFiles.has(relative)) {
    const resolutions = countMatches(
      code,
      /\bresolve_model_chain(?:_with_preferred)?\s*\(/g,
    )
    if (resolutions) {
      inventory.producerModelResolutionCalls[relative] = resolutions
      violations.push(
        `${relative}: ${resolutions} caller-owned model-chain resolution(s); pass routing intent to TurnKernel`,
      )
    }
  }

  const evaluationChains = countMatches(code, /\.\s*with_evaluation_model_chain\s*\(/g)
  if (evaluationChains) {
    inventory.evaluationModelChainCalls[relative] = evaluationChains
    if (!evaluationModelChainFiles.has(relative)) {
      violations.push(
        `${relative}: isolated evaluation model chain used outside an evaluation runtime`,
      )
    }
  }

  if (relative !== "crates/ha-core/src/chat_engine/engine.rs") {
    const direct = countMatches(code, /\bagent\s*\.\s*chat\s*\(/g)
    if (direct) inventory.directAgentChatCalls[relative] = direct
    const allowed = directAgentChat.get(relative) ?? 0
    if (direct > allowed) {
      violations.push(`${relative}: ${direct} direct agent.chat call(s), allowed ${allowed}`)
    }
  }

  const streamingCalls = countMatches(code, /\.\s*run_streaming_chat\s*\(/g)
  if (streamingCalls) inventory.streamingLoopCalls[relative] = streamingCalls
  const allowedStreamingCalls = streamingLoopAdapters.get(relative) ?? 0
  if (streamingCalls > allowedStreamingCalls) {
    violations.push(
      `${relative}: ${streamingCalls} run_streaming_chat call(s), allowed ${allowedStreamingCalls}`,
    )
  }

  if (relative !== "crates/ha-core/src/chat_engine/turn_kernel.rs") {
    const compatCalls = countMatches(code, /\bexecute_compat\s*\(/g)
    if (compatCalls) inventory.runtimeCompatCalls[relative] = compatCalls
    const allowed = 0
    if (compatCalls > allowed) {
      violations.push(`${relative}: ${compatCalls} runtime compatibility call(s), allowed ${allowed}`)
    }
  }
}

const loopManifest = path.join(repoRoot, "crates/ha-agent-loop/Cargo.toml")
if (existsSync(loopManifest)) {
  const manifest = readFileSync(loopManifest, "utf8")
  for (const forbidden of ["ha-core", "ha-base", "ha-config-schema", "reqwest", "rusqlite", "rmcp"]) {
    const dependency = new RegExp(`^${forbidden.replaceAll("-", "\\-")}\\s*=`, "m")
    if (dependency.test(manifest)) {
      violations.push(`crates/ha-agent-loop/Cargo.toml: forbidden dependency ${forbidden}`)
    }
  }
}

const coreManifest = readFileSync(path.join(repoRoot, "crates/ha-core/Cargo.toml"), "utf8")
for (const feature of ["ha-agent-runtime", "ha-memory", "ha-goal", "ha-workflow"]) {
  const dependency = new RegExp(`^${feature}\\s*=`, "m")
  if (dependency.test(coreManifest)) {
    violations.push(`crates/ha-core/Cargo.toml: ha-core must not depend on ${feature}`)
  }
}

// QuickJS belongs to the Workflow execution feature. Core may compile the
// exact feature source under cfg(test), but its production dependency section
// must stay free of the VM and serde bridge.
const coreProductionManifest = coreManifest.split(/^\[dev-dependencies\]\s*$/m, 1)[0]
for (const dependencyName of ["rquickjs", "rquickjs-serde"]) {
  const dependency = new RegExp(`^${dependencyName}\s*=`, "m")
  if (dependency.test(coreProductionManifest)) {
    violations.push(
      `crates/ha-core/Cargo.toml: ${dependencyName} is test-only in core; production ownership belongs to ha-workflow`,
    )
  }
}

for (const adapter of [
  "anthropic_adapter.rs",
  "openai_chat_adapter.rs",
  "openai_responses_adapter.rs",
  "codex_adapter.rs",
  "cancel.rs",
]) {
  const coreAdapter = path.join(repoRoot, "crates/ha-core/src/agent/providers", adapter)
  const runtimeAdapter = path.join(
    repoRoot,
    "crates/ha-agent-runtime/src/provider_adapters",
    adapter,
  )
  if (existsSync(coreAdapter)) {
    violations.push(`crates/ha-core/src/agent/providers/${adapter}: Provider runtime must live in ha-agent-runtime`)
  }
  if (!existsSync(runtimeAdapter)) {
    violations.push(`crates/ha-agent-runtime/src/provider_adapters/${adapter}: required Provider runtime is missing`)
  }
}

const retiredImplementationPaths = [
  "crates/ha-core/src/agent/streaming_loop.rs",
  "crates/ha-core/src/agent/vision_bridge.rs",
  "crates/ha-core/src/tools/goal.rs",
  "crates/ha-core/src/tools/workflow_tool.rs",
  "crates/ha-core/src/memory_extract.rs",
  "crates/ha-core/src/memory/embedding/api_provider.rs",
  "crates/ha-core/src/memory/external_provider/mem0.rs",
  "crates/ha-core/src/memory/external_provider/zep.rs",
  "crates/ha-core/src/memory/external_provider/supermemory.rs",
  "crates/ha-core/src/memory/external_provider/honcho.rs",
  "crates/ha-core/src/memory/external_provider/hindsight.rs",
  "crates/ha-core/src/memory/external_provider/open_viking.rs",
  "crates/ha-core/src/memory/external_provider/custom.rs",
  "crates/ha-core/src/memory/external_provider/http.rs",
]
for (const relative of retiredImplementationPaths) {
  if (existsSync(path.join(repoRoot, relative))) {
    violations.push(`${relative}: retired execution implementation must not return to ha-core`)
  }
}

const requiredFeaturePaths = [
  "crates/ha-agent-runtime/src/engine.rs",
  "crates/ha-agent-runtime/src/chat_dispatch.rs",
  "crates/ha-agent-runtime/src/one_shot.rs",
  "crates/ha-agent-runtime/src/streaming_loop.rs",
  "crates/ha-agent-runtime/src/vision_bridge.rs",
  "crates/ha-goal/src/runner.rs",
  "crates/ha-goal/src/policy.rs",
  "crates/ha-goal/src/tools/goal.rs",
  "crates/ha-workflow/src/preview.rs",
  "crates/ha-workflow/src/runtime_machine.rs",
  "crates/ha-workflow/src/typed_result.rs",
  "crates/ha-workflow/src/tools/workflow.rs",
  "crates/ha-memory/src/extract.rs",
  "crates/ha-memory/src/recall_planner.rs",
  "crates/ha-memory/src/reembed.rs",
  "crates/ha-memory/src/embedding/api_provider.rs",
  "crates/ha-memory/src/external_provider.rs",
  "crates/ha-memory/src/dreaming_pipeline.rs",
  "crates/ha-memory/src/dreaming_resolver.rs",
  "crates/ha-memory/src/dreaming_triggers.rs",
]
for (const relative of requiredFeaturePaths) {
  if (!existsSync(path.join(repoRoot, relative))) {
    violations.push(`${relative}: required feature-owned execution implementation is missing`)
  }
}

if (existsSync(path.join(repoRoot, "crates/ha-core/src/agent/llm_adapter.rs"))) {
  const coreOneShot = executableSurface(
    readFileSync(path.join(repoRoot, "crates/ha-core/src/agent/llm_adapter.rs"), "utf8"),
  )
  if (/\.\s*(?:post|send|json)\s*\(/.test(coreOneShot)) {
    violations.push("crates/ha-core/src/agent/llm_adapter.rs: one-shot network implementation leaked back into core")
  }
}

const memoryExtractFeature = path.join(repoRoot, "crates/ha-memory/src/extract.rs")
if (existsSync(memoryExtractFeature)) {
  const code = executableSurface(readFileSync(memoryExtractFeature, "utf8"))
  if (
    /\bha_core\s*::\s*automation\b/.test(code) ||
    /\bautomation\s*::\s*/.test(code)
  ) {
    violations.push(
      "crates/ha-memory/src/extract.rs: Memory Extract must use its captured single-model port, not the automation runtime",
    )
  }
}

const acpAgentPath = path.join(repoRoot, "crates/ha-acp/src/acp/agent.rs")
if (existsSync(acpAgentPath)) {
  const code = executableSurface(readFileSync(acpAgentPath, "utf8"))
  const runAgentChat = code.match(
    /\bfn\s+run_agent_chat\s*\([\s\S]*?\n\s*fn\s+build_modes\s*\(/,
  )
  if (!runAgentChat) {
    violations.push(
      "crates/ha-acp/src/acp/agent.rs: unable to locate run_agent_chat runtime boundary",
    )
  } else if (/\bruntime\s*::\s*Builder\b/.test(runAgentChat[0])) {
    violations.push(
      "crates/ha-acp/src/acp/agent.rs: ACP turns must use the injected process runtime; function-local runtimes cancel post-turn jobs",
    )
  }
}

for (const feature of ["ha-agent-runtime", "ha-memory", "ha-goal", "ha-workflow"]) {
  const root = path.join(repoRoot, "crates", feature)
  for (const file of walkRust(root)) {
    const relative = path.relative(repoRoot, file).split(path.sep).join("/")
    const source = readFileSync(file, "utf8")
    const code = executableSurface(source)
    if (/\brusqlite\s*::\s*Connection\b/.test(code) || /\.\s*with_conn(?:_internal|_for_test)?\s*\(/.test(code)) {
      violations.push(`${relative}: feature runtime must not acquire a raw SessionDB connection`)
    }
    if (/\b(?:INSERT\s+INTO|UPDATE|DELETE\s+FROM)\s+(?:sessions|messages)\b/i.test(source)) {
      violations.push(`${relative}: feature runtime must not write sessions/messages with direct SQL`)
    }
  }
}

if (asJson) {
  process.stdout.write(`${JSON.stringify({ inventory, violations }, null, 2)}\n`)
} else if (violations.length === 0) {
  console.log("agent-kernel migration boundaries: OK")
  console.log(
    `  legacy constructors=${Object.values(inventory.chatEngineParamConstructors).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  legacy engine calls=${Object.values(inventory.runChatEngineCalls).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  direct agent.chat bypasses=${Object.values(inventory.directAgentChatCalls).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  provider streaming adapters=${Object.values(inventory.streamingLoopCalls).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  runtime compatibility adapters=${Object.values(inventory.runtimeCompatCalls).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  external TurnRequest literals=${Object.values(inventory.turnRequestLiterals).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  producer model-chain resolutions=${Object.values(inventory.producerModelResolutionCalls).reduce((a, b) => a + b, 0)}`,
  )
  console.log(
    `  isolated evaluation chains=${Object.values(inventory.evaluationModelChainCalls).reduce((a, b) => a + b, 0)}`,
  )
} else {
  console.error("agent-kernel migration boundary violations:")
  for (const violation of violations) console.error(`  - ${violation}`)
}

if (violations.length > 0) process.exit(1)
