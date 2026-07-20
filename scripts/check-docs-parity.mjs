#!/usr/bin/env node
// Bilingual parity guard for the built-in user manual (docs/user-guide).
//
// The manual is compiled into the app (crates/ha-core/src/manual) and served
// to users and the ha-manual skill; the zh tree and the en/ tree must stay
// chapter-for-chapter aligned or one language silently loses content:
//   1. Chapter-number sets must match 1:1 (01..NN + README on both sides).
//   2. H2/H3 heading counts must match per chapter — a section added to one
//      language only is the most common drift.
//   3. Chapter links inside each README must reference existing chapters.
//
// Runs in PR CI on any change to docs/user-guide/** or this script.
// Exits non-zero on any failure.

import { readFileSync, readdirSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, "..")
const guideDir = path.join(repoRoot, "docs", "user-guide")

const errors = []

function chapterNumber(name) {
  if (name.toLowerCase() === "readme.md") return 0
  const m = /^(\d{2})-.+\.md$/.exec(name)
  return m ? Number(m[1]) : null
}

function listChapters(dir) {
  const map = new Map()
  for (const name of readdirSync(dir, { withFileTypes: true })) {
    if (!name.isFile() || !name.name.endsWith(".md")) continue
    const num = chapterNumber(name.name)
    if (num === null) {
      errors.push(`${path.relative(repoRoot, dir)}/${name.name}: unrecognized manual filename`)
      continue
    }
    map.set(num, name.name)
  }
  return map
}

// Fence-aware heading counter (same semantics as the Rust parser).
function countHeadings(file, levels) {
  const text = readFileSync(file, "utf8")
  let inFence = false
  let count = 0
  for (const raw of text.split("\n")) {
    const line = raw.replace(/\r$/, "")
    if (/^ {0,3}(`{3,}|~{3,})/.test(line)) {
      inFence = !inFence
      continue
    }
    if (inFence) continue
    const m = /^ {0,3}(#{1,6})[ \t]/.exec(line)
    if (m && levels.includes(m[1].length)) count += 1
  }
  return count
}

const zh = listChapters(guideDir)
const en = listChapters(path.join(guideDir, "en"))

// ─── Check 1: chapter sets match ────────────────────────────────────
for (const num of zh.keys()) {
  if (!en.has(num)) errors.push(`chapter ${num} exists in zh but not in en/`)
}
for (const num of en.keys()) {
  if (!zh.has(num)) errors.push(`chapter ${num} exists in en/ but not in zh`)
}

// ─── Check 2: per-chapter H2/H3 counts match ────────────────────────
for (const [num, zhName] of zh) {
  const enName = en.get(num)
  if (!enName) continue
  for (const levels of [[2], [3]]) {
    const zhCount = countHeadings(path.join(guideDir, zhName), levels)
    const enCount = countHeadings(path.join(guideDir, "en", enName), levels)
    if (zhCount !== enCount) {
      errors.push(
        `chapter ${String(num).padStart(2, "0")}: H${levels[0]} count differs — ` +
          `zh(${zhName})=${zhCount} vs en(${enName})=${enCount}`,
      )
    }
  }
}

// ─── Check 3: README chapter links resolve ──────────────────────────
function checkReadmeLinks(dir, chapters, label) {
  const readme = readFileSync(path.join(dir, "README.md"), "utf8")
  for (const m of readme.matchAll(/\]\((?:\.\/)?(\d{2})-[^)#]*?\.md(?:#[^)]*)?\)/g)) {
    const num = Number(m[1])
    if (!chapters.has(num)) {
      errors.push(`${label} README links to missing chapter ${m[1]} (${m[0]})`)
    }
  }
}
checkReadmeLinks(guideDir, zh, "zh")
checkReadmeLinks(path.join(guideDir, "en"), en, "en")

if (errors.length > 0) {
  console.error("docs/user-guide parity check FAILED:\n")
  for (const e of errors) console.error(`  ✗ ${e}`)
  console.error(
    "\nThe user manual ships inside the app in both languages — " +
      "update zh and en/ together (AGENTS.md 文档维护).",
  )
  process.exit(1)
}

console.log(`docs parity OK — ${zh.size} zh + ${en.size} en chapters aligned`)
