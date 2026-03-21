#!/usr/bin/env node
/**
 * i18n 翻译同步脚本
 *
 * 用法：
 *   node scripts/sync-i18n.mjs --check          # 检查各语言缺失的 key
 *   node scripts/sync-i18n.mjs --apply           # 从 translations 文件补齐缺失翻译
 *   node scripts/sync-i18n.mjs --check --apply   # 检查 + 补齐
 *
 * 以 en.json 为基准，对比其它语言文件，找出缺失的 key，
 * 然后从 scripts/i18n-translations.json 读取翻译并写入。
 */

import { readFileSync, writeFileSync, readdirSync } from "fs"
import { resolve, dirname } from "path"
import { fileURLToPath } from "url"

const __dirname = dirname(fileURLToPath(import.meta.url))
const LOCALES_DIR = resolve(__dirname, "../src/i18n/locales")
const TRANSLATIONS_FILE = resolve(__dirname, "i18n-translations.json")

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

// 读取翻译数据（如果需要 apply）
let translations = {}
if (doApply) {
  try {
    translations = JSON.parse(readFileSync(TRANSLATIONS_FILE, "utf8"))
  } catch {
    console.error(`❌ 找不到翻译文件: ${TRANSLATIONS_FILE}`)
    console.error("   请先准备好翻译数据文件")
    process.exit(1)
  }
}

// 获取所有 locale 文件（排除 en.json 和 zh.json）
const localeFiles = readdirSync(LOCALES_DIR)
  .filter((f) => f.endsWith(".json") && f !== "en.json" && f !== "zh.json")

let totalMissing = 0
let totalApplied = 0

for (const file of localeFiles) {
  const lang = file.replace(".json", "")
  const filePath = resolve(LOCALES_DIR, file)
  const locale = JSON.parse(readFileSync(filePath, "utf8"))
  const localeKeySet = new Set(flatKeys(locale))

  const missing = enKeys.filter((k) => !localeKeySet.has(k))
  const extra = flatKeys(locale).filter((k) => !new Set(enKeys).has(k))

  if (doCheck) {
    if (missing.length === 0 && extra.length === 0) {
      console.log(`✅ ${lang}: 完整 (${localeKeySet.size} keys)`)
    } else {
      console.log(`\n⚠️  ${lang}: ${localeKeySet.size} keys, 缺失 ${missing.length}, 多余 ${extra.length}`)
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
    }
    totalMissing += missing.length
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
}

console.log("\n────────────────────────────────")
if (doCheck) console.log(`总计缺失: ${totalMissing} 条`)
if (doApply) console.log(`总计写入: ${totalApplied} 条`)
