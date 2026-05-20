#!/usr/bin/env node
// Static validator for release-related GitHub Actions workflows.
// Catches the classes of bugs that took down the v0.2.0 release three
// times in a row:
//   1. cargo target_dir written as `src-tauri/target/...` while Hope
//      Agent is a Cargo workspace (real target lives in `./target/`).
//   2. Swatinem/rust-cache `workspaces:` pointing at `./src-tauri ->
//      target` for the same reason — the cache never hits.
//   3. update-*.yml referencing release artifact filename patterns that
//      release.yml no longer produces (e.g. `x64.dmg` after the
//      macos-x64 lane was temporarily removed in v0.2.0).
//
// Runs in PR CI on any change to .github/workflows/*.yml or this
// script. Exits non-zero on any check failure.

import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(__dirname, "..")

const errors = []
const warnings = []

function readWorkflow(name) {
  return readFileSync(path.join(repoRoot, ".github", "workflows", name), "utf8")
}

function readJson(name) {
  return JSON.parse(readFileSync(path.join(repoRoot, name), "utf8"))
}

// ─── Check 1 ─────────────────────────────────────────────────────────
// release.yml bare-binary step: every `target_dir=...` must start with
// `target/`, never `src-tauri/target/`. Hope Agent is a Cargo workspace
// — cargo writes binaries to the workspace root target/, not to the
// src-tauri member subdirectory.
{
  const release = readWorkflow("release.yml")
  const targetDirMatches = [...release.matchAll(/target_dir=([^\s;)]+)/g)]
  if (targetDirMatches.length === 0) {
    errors.push("release.yml: expected at least one target_dir= assignment in bare-binary step, found none")
  }
  for (const m of targetDirMatches) {
    const value = m[1]
    if (value.startsWith("src-tauri/")) {
      errors.push(
        `release.yml: target_dir="${value}" starts with src-tauri/ — Hope Agent is a Cargo workspace, cargo writes to repo-root ./target/, not src-tauri/target/. Drop the src-tauri/ prefix.`,
      )
    } else if (!value.startsWith("target/") && value !== "target") {
      errors.push(
        `release.yml: target_dir="${value}" does not start with target/ — bare-binary step expects cargo output paths.`,
      )
    }
  }
}

// ─── Check 2 ─────────────────────────────────────────────────────────
// Swatinem/rust-cache `workspaces:` value. Should point at workspace
// root (".") or a sibling crate path that contains a Cargo.toml with
// `[workspace]`, never the src-tauri member by itself.
{
  const release = readWorkflow("release.yml")
  const wsMatch = release.match(/Swatinem\/rust-cache[\s\S]*?workspaces:\s*"([^"]+)"/)
  if (!wsMatch) {
    warnings.push("release.yml: no Swatinem/rust-cache workspaces: value found (or format changed) — skip rust-cache check")
  } else {
    const value = wsMatch[1]
    // Format: "<dir1> -> target<sep><dir2> -> target..." but value
    // must reference the workspace root, not the src-tauri member.
    if (/^\.?\/?src-tauri\b/.test(value)) {
      errors.push(
        `release.yml: rust-cache workspaces="${value}" points at src-tauri/ — Hope Agent is a Cargo workspace, cache target lives at repo-root ./target/. Use ". -> target".`,
      )
    }
  }
}

// ─── Check 3 ─────────────────────────────────────────────────────────
// Cross-workflow artifact-name consistency. update-homebrew-tap.yml /
// update-aur.yml / update-scoop-bucket.yml / update-linux-repo.yml all
// download specific filename patterns from the GitHub Release. If
// release.yml stops producing a pattern (e.g. macos-x64 lane removed),
// the downstream workflow will fail silently — leaving package
// managers stuck on the prior version.
//
// We can't tell from release.yml *exactly* what tauri-action will name
// the bundle, but we can detect "downstream wants pattern P, but
// release.yml's matrix has no platform that produces P".

const matrixPlatforms = (() => {
  const release = readWorkflow("release.yml")
  const matrixSection = release.match(/strategy:\s*\n([\s\S]*?)runs-on:/)
  if (!matrixSection) return new Set()
  return new Set(
    [...matrixSection[1].matchAll(/-\s+platform:\s*(\S+)/g)].map((m) => m[1]),
  )
})()

// Map of (filename-pattern fragment) → required platform.
// Only catches gross mismatches; signatures-of-truth in tauri.conf.json
// are not parsed here.
const downstreamArtifactDeps = [
  {
    workflow: "update-homebrew-tap.yml",
    fragment: "x64.dmg",
    requires: "macos-x64",
    severity: "warn",
  },
  {
    workflow: "update-homebrew-tap.yml",
    fragment: "aarch64.dmg",
    requires: "macos-arm64",
    severity: "error",
  },
  {
    workflow: "update-aur.yml",
    fragment: "amd64.deb",
    requires: "linux-x64",
    severity: "error",
  },
  {
    workflow: "update-scoop-bucket.yml",
    fragment: "x64-setup.exe",
    requires: "windows-x64",
    severity: "error",
  },
  {
    workflow: "update-linux-repo.yml",
    fragment: "amd64.deb",
    requires: "linux-x64",
    severity: "error",
  },
  {
    workflow: "update-linux-repo.yml",
    fragment: "x86_64.rpm",
    requires: "linux-x64",
    severity: "error",
  },
]

for (const dep of downstreamArtifactDeps) {
  let yml
  try {
    yml = readWorkflow(dep.workflow)
  } catch {
    warnings.push(`Cross-check skipped: ${dep.workflow} not found.`)
    continue
  }
  if (!yml.includes(dep.fragment)) continue
  if (matrixPlatforms.has(dep.requires)) continue
  const message = `${dep.workflow} references artifact pattern "${dep.fragment}" but release.yml matrix has no platform "${dep.requires}" — downstream workflow will fail on release publish.`
  if (dep.severity === "error") errors.push(message)
  else warnings.push(message + " (warn-only: downstream workflow is expected to handle the missing artifact gracefully.)")
}

// ─── Check 4 ─────────────────────────────────────────────────────────
// NSIS language names must be the canonical NSIS identifiers used by
// `!insertmacro MUI_LANGUAGE`, not locale aliases. `SimplifiedChinese`
// looks natural but makes makensis look for `SimplifiedChinese.nlf`;
// upstream NSIS ships that language as `SimpChinese.nlf`.
{
  const conf = readJson("src-tauri/tauri.conf.json")
  const nsisLanguages = conf.bundle?.windows?.nsis?.languages ?? []
  const wixLanguage = conf.bundle?.windows?.wix?.language
  const wixLanguages = Array.isArray(wixLanguage) ? wixLanguage : [wixLanguage].filter(Boolean)

  if (nsisLanguages.includes("SimplifiedChinese")) {
    errors.push(
      'src-tauri/tauri.conf.json: bundle.windows.nsis.languages uses "SimplifiedChinese"; NSIS expects "SimpChinese".',
    )
  }
  if (wixLanguages.includes("zh-CN") && !nsisLanguages.includes("SimpChinese")) {
    errors.push(
      'src-tauri/tauri.conf.json: WiX includes "zh-CN" but NSIS languages do not include "SimpChinese"; Windows installers should expose matching Chinese UI coverage.',
    )
  }
}

// ─── Check 5 ─────────────────────────────────────────────────────────
// release.yml must include at least the 4 platforms required for the
// updater manifest's bare_binary entries to be useful: macos-arm64,
// linux-x64, linux-arm64, windows-x64. macos-x64 is currently optional
// (temporarily removed in v0.2.0; recovery tracked as F-088).
const requiredPlatforms = ["macos-arm64", "linux-x64", "linux-arm64", "windows-x64"]
for (const p of requiredPlatforms) {
  if (!matrixPlatforms.has(p)) {
    errors.push(`release.yml matrix is missing required platform "${p}" — releases without it ship broken bare-binary manifests for that OS/arch.`)
  }
}

// ─── Report ──────────────────────────────────────────────────────────
if (warnings.length > 0) {
  console.warn("[check-release-paths] warnings:")
  for (const w of warnings) console.warn(`  - ${w}`)
}
if (errors.length > 0) {
  console.error("[check-release-paths] errors:")
  for (const e of errors) console.error(`  - ${e}`)
  process.exit(1)
}

console.log(`[check-release-paths] OK — ${matrixPlatforms.size} platforms (${[...matrixPlatforms].join(", ")}), ${warnings.length} warning(s).`)
