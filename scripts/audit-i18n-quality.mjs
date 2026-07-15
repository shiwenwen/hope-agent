#!/usr/bin/env node
/**
 * i18n quality audit helper.
 *
 * Examples:
 *   node scripts/audit-i18n-quality.mjs --locales zh,zh-TW --source src/components/settings/memory-panel --fail-on-same
 *   node scripts/audit-i18n-quality.mjs --locales zh-TW --limit 200
 */

import { readFileSync, readdirSync, statSync } from "fs"
import { dirname, relative, resolve } from "path"
import { fileURLToPath } from "url"

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, "..")
const LOCALES_DIR = resolve(ROOT, "src/i18n/locales")

const args = process.argv.slice(2)
const argValue = (name, fallback = null) => {
  const index = args.indexOf(name)
  return index >= 0 && index + 1 < args.length ? args[index + 1] : fallback
}

const localeArg = argValue("--locales", "zh,zh-TW")
const sourceArg = argValue("--source", "src")
const limit = Number(argValue("--limit", "120"))
const failOnSame = args.includes("--fail-on-same")

const locales = localeArg
  .split(",")
  .map((item) => item.trim())
  .filter(Boolean)
const sourceRoot = resolve(ROOT, sourceArg)

const ALLOW_SAME_AS_EN = [
  /^Agent$/,
  /^API( Key| key| 密钥| 金鑰)?$/,
  /^Endpoint$/,
  /^Provider$/,
  /^Global$/,
  /^Error$/,
  /^Dreaming$/,
  /^Deep Resolver$/,
  /^OK$/,
  /^Audio$/,
  /^Auto$/,
  /^Manual$/,
  /^Feedback$/,
  /^Model$/,
  /^Model: \{\{name\}\}$/,
  /^Status$/,
  /^Format$/,
  /^Input$/,
  /^Prompt$/,
  /^Unit$/,
  /^Domain$/,
  /^Import$/,
  /^Import JSON$/,
  /^Cache$/,
  /^Cron$/,
  /^Dir$/,
  /^Worktree$/,
  /^Sub-Agent$/,
  /^AI Agent$/,
  /^Color$/,
  /^Observe$/,
  /^Backlinks$/,
  /^Zoom$/,
  /^Latitude$/,
  /^Longitude$/,
  /^bytes$/,
  /^min$/,
  /^sec$/,
  /^Video$/,
  /^Total$/,
  /^Total \(ms\)$/,
  /^multimodal$/,
  /^quick_check \{\{value\}\}$/,
  /^\{\{current\}\}\/\{\{total\}\}$/,
  /^\{\{count\}\} total$/,
  /^\{\{count\}\} http$/,
  /^\{\{n\}\}d$/,
  /^1 min$/,
  /^5 min$/,
  /^PDF$/,
  /^OS$/,
  /^GPU$/,
  /^VRAM$/,
  /^TTFT$/,
  /^JSON$/,
  /^HTTP$/,
  /^WebSocket$/,
  /^SQLite$/,
  /^FTS$/,
  /^RRF$/,
  /^MMR$/,
  /^URL$/,
  /^URI$/,
  /^ID$/,
  /^MCP$/,
  /^YOLO$/,
  /^UTC$/,
  /^Git$/,
  /^GitHub$/,
  /^Hope Agent$/,
  /^Markdown$/,
  /^HTML$/,
  /^DOCX$/,
  /^MOC$/,
  /^KB$/,
  /^LLM$/,
  /^Shell$/,
  /^PAT$/,
  /^Fine-grained PAT$/,
  /^Base URL$/,
  /^Web GUI URL$/,
  /^User Agent$/,
  /^Agent ID$/,
  /^Chat ID$/,
  /^Thread ID$/,
  /^Main Agent$/,
  /^AGENTS\.md$/,
  /^IDENTITY\.md$/,
  /^SOUL\.md$/,
  /^TOOLS\.md$/,
  /^Logo$/,
  /^matcher$/,
  /^Embedding$/,
  /^Bug$/,
  /^Claude$/,
  /^Anthropic Agent Skills marketplace$/,
  /^IRC \(Internet Relay Chat\)$/,
  /^LINE Messaging API$/,
  /^create_entities\nsearch_nodes$/,
  /^raw\/$/,
  /^sources\/$/,
  /^v\{\{version\}\}$/,
  /^release, ci$/,
  /^ · \{\{error\}\}$/,
  /^\/Applications\//,
  /^http:\/\//,
  /^https?:\/\/127\.0\.0\.1/,
  /^(Mem0|Zep|Supermemory|Honcho|Hindsight|OpenViking|OpenAI|Anthropic|Ollama|Gemini|Qwen|DeepSeek|Claude|Chrome|Docker|Discord|Slack|Telegram|WhatsApp|Signal|LINE|iMessage|Google Chat|QQ Bot|IRC)$/,
  /^\{\{[^}]+}}$/,
]

function flatKeys(obj, prefix = "") {
  const out = new Map()
  for (const [key, value] of Object.entries(obj)) {
    const full = prefix ? `${prefix}.${key}` : key
    if (value && typeof value === "object" && !Array.isArray(value)) {
      for (const entry of flatKeys(value, full)) out.set(entry[0], entry[1])
    } else {
      out.set(full, value)
    }
  }
  return out
}

function sourceFiles(dir) {
  const files = []
  for (const name of readdirSync(dir)) {
    const full = resolve(dir, name)
    const stat = statSync(full)
    if (stat.isDirectory()) {
      files.push(...sourceFiles(full))
      continue
    }
    if (!/\.(ts|tsx)$/.test(name)) continue
    if (/\.(test|spec)\.(ts|tsx)$/.test(name)) continue
    files.push(full)
  }
  return files
}

function collectLiteralTranslationRefs() {
  const refs = new Map()
  const pattern = /\bt\s*\(\s*(["'])([^"'\n]+)\1/g
  for (const file of sourceFiles(sourceRoot)) {
    const source = readFileSync(file, "utf8")
    for (const match of source.matchAll(pattern)) {
      const key = match[2]
      if (!key.includes(".")) continue
      const line = source.slice(0, match.index).split("\n").length
      const rel = relative(ROOT, file)
      if (!refs.has(key)) refs.set(key, [])
      refs.get(key).push(`${rel}:${line}`)
    }
  }
  return refs
}

function isAllowedSameAsEnglish(value) {
  return ALLOW_SAME_AS_EN.some((pattern) => pattern.test(value))
}

const en = flatKeys(JSON.parse(readFileSync(resolve(LOCALES_DIR, "en.json"), "utf8")))
const refs = collectLiteralTranslationRefs()
let totalSame = 0

for (const locale of locales) {
  const localeFile = resolve(LOCALES_DIR, `${locale}.json`)
  const data = flatKeys(JSON.parse(readFileSync(localeFile, "utf8")))
  const same = []

  for (const [key, locations] of refs) {
    const enValue = en.get(key)
    const localeValue = data.get(key)
    if (typeof enValue !== "string") continue
    if (!enValue) continue
    if (localeValue !== enValue) continue
    if (isAllowedSameAsEnglish(enValue)) continue
    same.push({ key, value: enValue, locations })
  }

  totalSame += same.length
  console.log(`\n${locale}: ${same.length} source-referenced strings equal en`)
  for (const item of same.slice(0, limit)) {
    console.log(`- ${item.key} = ${JSON.stringify(item.value)}`)
    console.log(`  ${item.locations[0]}`)
  }
  if (same.length > limit) {
    console.log(`... ${same.length - limit} more`)
  }
}

if (failOnSame && totalSame > 0) {
  process.exit(1)
}
