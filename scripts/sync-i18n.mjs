#!/usr/bin/env node
/**
 * i18n 翻译同步脚本
 *
 * 用法：
 *   node scripts/sync-i18n.mjs --check          # 检查各语言缺失的 key，以及源码 t("key") 是否存在
 *   node scripts/sync-i18n.mjs --apply           # 从 translations 文件补齐缺失翻译
 *   node scripts/sync-i18n.mjs --check --apply   # 检查 + 补齐
 *
 * 以 en.json 为基准，对比其它语言文件，找出缺失/多余的 key 和插值变量漂移；
 * 同时扫描源码中所有以字面量 key 开头的 t("key", ...) 调用，
 * 避免所有语言共同漏同一个基准 key，或靠代码内英文 defaultValue 漏到非英文界面。
 * --apply 会从 scripts/i18n-translations.json 读取翻译并写入。
 */

import { readFileSync, writeFileSync, readdirSync, statSync } from "fs"
import { resolve, dirname, relative } from "path"
import { fileURLToPath } from "url"

const __dirname = dirname(fileURLToPath(import.meta.url))
const LOCALES_DIR = resolve(__dirname, "../src/i18n/locales")
const SRC_DIR = resolve(__dirname, "../src")
const TRANSLATIONS_FILE = resolve(__dirname, "i18n-translations.json")
const BUILTIN_TOOLS_DIR = resolve(__dirname, "../crates/ha-core/src/tools")

// ── helpers ──────────────────────────────────────────────────────────

/** 递归取出所有叶子节点的 key（用 . 连接） */
function flatKeys(obj, prefix = "") {
  const keys = []
  for (const k of Object.keys(obj)) {
    const full = prefix ? `${prefix}.${k}` : k
    if (typeof obj[k] === "object" && obj[k] !== null && !Array.isArray(obj[k])) {
      keys.push(...flatKeys(obj[k], full))
    } else {
      keys.push(full)
    }
  }
  return keys
}

/** 找出无法被 getByPath 读取的字面点号属性（例如 { "a.b": ... }） */
function dottedObjectKeys(obj, prefix = "") {
  const keys = []
  for (const [key, value] of Object.entries(obj ?? {})) {
    const full = prefix ? `${prefix}.${key}` : key
    if (key.includes(".")) keys.push(full)
    if (value && typeof value === "object" && !Array.isArray(value)) {
      keys.push(...dottedObjectKeys(value, full))
    }
  }
  return keys
}

/** 根据 dot-path 取值 */
function getByPath(obj, path) {
  return path.split(".").reduce((o, k) => o?.[k], obj)
}

/** 根据 dot-path 设值（自动创建中间对象） */
function setByPath(obj, path, value) {
  const parts = path.split(".")
  let cur = obj
  for (let i = 0; i < parts.length - 1; i++) {
    if (!(parts[i] in cur) || typeof cur[parts[i]] !== "object") {
      cur[parts[i]] = {}
    }
    cur = cur[parts[i]]
  }
  cur[parts[parts.length - 1]] = value
}

/** 按 en.json 的 key 顺序对 locale 对象排序（递归） */
function sortByReference(ref, target) {
  if (typeof ref !== "object" || ref === null) return target
  const sorted = {}
  for (const k of Object.keys(ref)) {
    if (k in target) {
      sorted[k] =
        typeof ref[k] === "object" && ref[k] !== null
          ? sortByReference(ref[k], target[k] || {})
          : target[k]
    }
  }
  // 保留 target 中有但 ref 中没有的 key（放末尾）
  for (const k of Object.keys(target)) {
    if (!(k in sorted)) sorted[k] = target[k]
  }
  return sorted
}

function sourceFiles(dir) {
  const files = []
  for (const name of readdirSync(dir)) {
    const path = resolve(dir, name)
    const stat = statSync(path)
    if (stat.isDirectory()) {
      files.push(...sourceFiles(path))
      continue
    }
    if (!/\.(ts|tsx)$/.test(name)) continue
    if (/\.(test|spec)\.(ts|tsx)$/.test(name)) continue
    files.push(path)
  }
  return files
}

function rustSourceFiles(dir) {
  const files = []
  for (const name of readdirSync(dir)) {
    const path = resolve(dir, name)
    const stat = statSync(path)
    if (stat.isDirectory()) {
      files.push(...rustSourceFiles(path))
      continue
    }
    if (name.endsWith(".rs")) files.push(path)
  }
  return files
}

function findLiteralSourceTranslationKeys() {
  const refs = new Map()
  // Use separate alternatives so a double-quoted default value may contain an
  // apostrophe (and vice versa). The former shared character class silently
  // missed calls such as t("key", "Couldn't ...").
  const callPattern = /\bt\s*\(\s*(?:"([^"\n]+)"|'([^'\n]+)')/g
  // Lookup tables commonly pass these fields to t(...) later. Scanning the
  // literal field values closes the gap left by call-only matching.
  const indirectKeyPattern =
    /\b(?:labelKey|titleKey|descriptionKey|descKey|errorKey|hintKey|nameKey|promptKey|reasonKey|subjectScopeKey)\s*:\s*(?:"([^"\n]*\.[^"\n]*)"|'([^'\n]*\.[^'\n]*)')/g
  const translationLikeLiteralPattern =
    /(["'])([a-z][A-Za-z0-9_-]*(?:\.[a-z][A-Za-z0-9_-]*)+)\1/g
  const enRoots = new Set(Object.keys(en))
  const enPrefixes = new Set()
  for (const key of enKeys) {
    const parts = key.split(".")
    for (let index = 1; index < parts.length; index += 1) {
      enPrefixes.add(parts.slice(0, index).join("."))
    }
  }
  const technicalLiteral =
    /\.(?:md|json|ya?ml|toml|tsx?|jsx?|rs|html|css|csv|xlsx?|docx|pptx|db|sqlite|zip|tar|gz|png|jpe?g|gif|svg|webp|bin|m4a|mp3|ogg|wav|webm|mp4)$/i

  for (const file of sourceFiles(SRC_DIR)) {
    const source = readFileSync(file, "utf8")
    const recordRef = (key, index) => {
      const before = source.slice(0, index)
      const line = before.split("\n").length
      const rel = relative(resolve(__dirname, ".."), file)
      if (!refs.has(key)) refs.set(key, [])
      refs.get(key).push(`${rel}:${line}`)
    }
    for (const pattern of [callPattern, indirectKeyPattern]) {
      for (const match of source.matchAll(pattern)) {
        const key = match[1] ?? match[2]
        if (!key.includes(".")) continue
        recordRef(key, match.index)
      }
    }
    // A final conservative pass catches translation-key lookup maps whose
    // property names are generic (for example PHASE_KEY). Restrict candidates
    // to known locale roots, lowercase key segments, non-file literals, and
    // non-prefixes so model IDs, filenames and dynamic namespace bases stay out.
    for (const match of source.matchAll(translationLikeLiteralPattern)) {
      const key = match[2]
      if (!enRoots.has(key.split(".")[0])) continue
      if (technicalLiteral.test(key)) continue
      if (enPrefixes.has(key)) continue
      recordRef(key, match.index)
    }
  }

  // Built-in tool labels are looked up dynamically from canonical Rust ids,
  // so the TypeScript literal-key scan cannot see them. Scan the complete tool
  // source tree (including integration submodules) and accept whitespace/newline
  // around declarations. Restrict values to canonical tool ids so diagnostic
  // constants such as TOOL_ERROR_PREFIX are not mistaken for dispatchable tools.
  for (const file of rustSourceFiles(BUILTIN_TOOLS_DIR)) {
    const source = readFileSync(file, "utf8")
    const rel = relative(resolve(__dirname, ".."), file)
    for (const match of source.matchAll(
      /pub\s+const\s+TOOL_[A-Z0-9_]+\s*:\s*&str\s*=\s*"([a-z0-9_]+)"\s*;/g,
    )) {
      const name = match[1]
      const suffix = name
        .split("_")
        .filter(Boolean)
        .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
        .join("")
      const line = source.slice(0, match.index).split("\n").length
      refs.set(`tools.${name}`, [`${rel}:${line}`])
      refs.set(`settings.tool${suffix}Desc`, [`${rel}:${line}`])
    }
  }

  return refs
}

function interpolationKeys(value) {
  if (typeof value !== "string") return []
  return [...value.matchAll(/{{\s*([^},\s]+)[^}]*}}/g)]
    .map((match) => match[1])
    .sort()
}

function allowedPlaceholderDifference(lang, key, expected, actual) {
  // zh/zh-TW deliberately use action-specific tool status sentences instead
  // of interpolating the generic tool name. Keep this legacy exception narrow.
  return (
    (lang === "zh" || lang === "zh-TW") &&
    /^executionStatus\.tool\.single\.[^.]+\.(completed|failed|running)$/.test(key) &&
    expected.length === 1 &&
    expected[0] === "name" &&
    actual.length === 0
  )
}

// ── main ─────────────────────────────────────────────────────────────

const args = process.argv.slice(2)
const doCheck = args.includes("--check")
const doApply = args.includes("--apply")

if (!doCheck && !doApply) {
  console.log("用法：node scripts/sync-i18n.mjs --check | --apply | --check --apply")
  process.exit(0)
}

// 读取基准文件
const en = JSON.parse(readFileSync(resolve(LOCALES_DIR, "en.json"), "utf8"))
const enKeys = flatKeys(en)
const enKeySet = new Set(enKeys)

// 读取翻译数据。check 同时校验种子，避免 --apply 将旧译文或点号扁平键写回 locale。
let translations = {}
if (doApply || doCheck) {
  try {
    translations = JSON.parse(readFileSync(TRANSLATIONS_FILE, "utf8"))
  } catch {
    console.error(`❌ 找不到翻译文件: ${TRANSLATIONS_FILE}`)
    console.error("   请先准备好翻译数据文件")
    process.exit(1)
  }
}

// 获取所有 locale 文件（仅排除作为结构基准的 en.json）
const localeFiles = readdirSync(LOCALES_DIR)
  .filter((f) => f.endsWith(".json") && f !== "en.json")

let totalMissing = 0
let totalExtra = 0
let totalPlaceholderMismatches = 0
let totalApplied = 0
let totalSeedDottedKeys = 0
let totalSeedUnknownKeys = 0
let totalSeedMismatches = 0

for (const file of localeFiles) {
  const lang = file.replace(".json", "")
  const filePath = resolve(LOCALES_DIR, file)
  const locale = JSON.parse(readFileSync(filePath, "utf8"))
  const localeKeySet = new Set(flatKeys(locale))

  const missing = enKeys.filter((k) => !localeKeySet.has(k))
  const extra = flatKeys(locale).filter((k) => !enKeySet.has(k))
  const placeholderMismatches = enKeys.filter((key) => {
    if (!localeKeySet.has(key)) return false
    const expected = interpolationKeys(getByPath(en, key))
    const actual = interpolationKeys(getByPath(locale, key))
    return (
      expected.join("\u0000") !== actual.join("\u0000") &&
      !allowedPlaceholderDifference(lang, key, expected, actual)
    )
  })

  if (doCheck) {
    if (missing.length === 0 && extra.length === 0 && placeholderMismatches.length === 0) {
      console.log(`✅ ${lang}: 完整 (${localeKeySet.size} keys)`)
    } else {
      console.log(
        `\n⚠️  ${lang}: ${localeKeySet.size} keys, 缺失 ${missing.length}, 多余 ${extra.length}, 插值不一致 ${placeholderMismatches.length}`,
      )
      if (missing.length > 0) {
        console.log("   缺失的 key：")
        for (const k of missing) {
          const enVal = getByPath(en, k)
          console.log(`     - ${k} = "${enVal}"`)
        }
      }
      if (extra.length > 0) {
        console.log("   多余的 key：")
        for (const k of extra) console.log(`     + ${k}`)
      }
      if (placeholderMismatches.length > 0) {
        console.log("   插值变量不一致：")
        for (const key of placeholderMismatches) {
          console.log(
            `     - ${key}: en=[${interpolationKeys(getByPath(en, key)).join(", ")}] ${lang}=[${interpolationKeys(getByPath(locale, key)).join(", ")}]`,
          )
        }
      }
    }
    totalMissing += missing.length
    totalExtra += extra.length
    totalPlaceholderMismatches += placeholderMismatches.length
  }

  if (doApply && missing.length > 0) {
    const langTranslations = translations[lang]
    if (!langTranslations) {
      console.log(`⏭️  ${lang}: 翻译文件中无此语言数据，跳过`)
      continue
    }

    let applied = 0
    let notFound = []
    for (const key of missing) {
      const val = getByPath(langTranslations, key)
      if (val !== undefined) {
        setByPath(locale, key, val)
        applied++
      } else {
        notFound.push(key)
      }
    }

    // 按 en.json 的顺序排序后写入
    const sorted = sortByReference(en, locale)
    writeFileSync(filePath, JSON.stringify(sorted, null, 2) + "\n", "utf8")

    console.log(`✏️  ${lang}: 写入 ${applied} 条翻译`)
    if (notFound.length > 0) {
      console.log(`   ⚠️  ${notFound.length} 条未找到翻译：`)
      for (const k of notFound) console.log(`     - ${k}`)
    }
    totalApplied += applied
  }

  if (doCheck && translations[lang]) {
    const seed = translations[lang]
    const dotted = dottedObjectKeys(seed)
    const seedKeys = flatKeys(seed)
    const unknown = seedKeys.filter((key) => !enKeySet.has(key))
    const mismatches = seedKeys.filter(
      (key) => enKeySet.has(key) && getByPath(seed, key) !== getByPath(locale, key),
    )
    if (dotted.length > 0 || unknown.length > 0 || mismatches.length > 0) {
      console.log(
        `\n⚠️  ${lang} 翻译种子: 点号属性 ${dotted.length}, 失效 key ${unknown.length}, 与 locale 不一致 ${mismatches.length}`,
      )
      for (const key of dotted) console.log(`     · 无效点号属性 ${key}`)
      for (const key of unknown) console.log(`     - 失效 ${key}`)
      for (const key of mismatches) console.log(`     ≠ ${key}`)
    }
    totalSeedDottedKeys += dotted.length
    totalSeedUnknownKeys += unknown.length
    totalSeedMismatches += mismatches.length
  }
}

const sourceKeyRefs = findLiteralSourceTranslationKeys()
const missingSourceKeys = [...sourceKeyRefs.keys()]
  .filter((key) => !enKeySet.has(key))
  .sort()

if (doCheck && missingSourceKeys.length > 0) {
  console.log(`\n⚠️  源码字面量 t(...) 缺失 ${missingSourceKeys.length} 个 en.json 基准 key`)
  for (const key of missingSourceKeys) {
    console.log(`   - ${key}`)
    for (const ref of sourceKeyRefs.get(key)) {
      console.log(`     ${ref}`)
    }
  }
}

console.log("\n────────────────────────────────")
if (doCheck) {
  console.log(`总计缺失: ${totalMissing} 条`)
  console.log(`总计多余: ${totalExtra} 条`)
  console.log(`总计插值不一致: ${totalPlaceholderMismatches} 条`)
  console.log(`翻译种子点号属性: ${totalSeedDottedKeys} 条`)
  console.log(`翻译种子失效 key: ${totalSeedUnknownKeys} 条`)
  console.log(`翻译种子与 locale 不一致: ${totalSeedMismatches} 条`)
}
if (doApply) console.log(`总计写入: ${totalApplied} 条`)

// CI gate: --check 发现缺 key 时退出码 1，让 GitHub Actions / pre-commit
// 能拦截忘记跑 sync-i18n 的 PR。--apply 不影响退出码。
if (
  doCheck &&
  (totalMissing > 0 ||
    totalExtra > 0 ||
    totalPlaceholderMismatches > 0 ||
    totalSeedDottedKeys > 0 ||
    totalSeedUnknownKeys > 0 ||
    totalSeedMismatches > 0 ||
    missingSourceKeys.length > 0)
) {
  console.error(
    `\n❌ 检测到 ${totalMissing} 个 locale 缺失 key、${totalExtra} 个多余 key、${totalPlaceholderMismatches} 个插值不一致、${missingSourceKeys.length} 个源码 key 未进入 en.json 基准，以及 ${totalSeedDottedKeys + totalSeedUnknownKeys + totalSeedMismatches} 个翻译种子问题。请补齐后重新提交。`,
  )
  process.exit(1)
}
