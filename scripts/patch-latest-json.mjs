#!/usr/bin/env node
//
// Patch `latest.json` (the Tauri updater manifest) with a `bare_binary`
// section that maps each platform to a tar.gz / zip archive + its
// Minisign signature. The headless self-update path in
// `ha_core::updater::self_contained` consumes that section.
//
// Usage:
//   node scripts/patch-latest-json.mjs <latest.json> <artifacts-dir> <version>
//
// Layout expected under `<artifacts-dir>`:
//   bare-binary-macos-arm64/hope-agent-<v>-darwin-aarch64.tar.gz
//   bare-binary-macos-arm64/hope-agent-<v>-darwin-aarch64.tar.gz.sig
//   bare-binary-macos-x64/hope-agent-<v>-darwin-x86_64.tar.gz
//   ...
//
// Output is written back to `<latest.json>` in place.

import fs from "node:fs";
import path from "node:path";

const PLATFORM_MAP = {
  "macos-arm64": { key: "darwin-aarch64", archive: "tar_gz", binary: "hope-agent" },
  "macos-x64": { key: "darwin-x86_64", archive: "tar_gz", binary: "hope-agent" },
  "linux-x64": { key: "linux-x86_64", archive: "tar_gz", binary: "hope-agent" },
  "linux-arm64": { key: "linux-aarch64", archive: "tar_gz", binary: "hope-agent" },
  "windows-x64": { key: "windows-x86_64", archive: "zip", binary: "hope-agent.exe" },
};

function usage() {
  console.error(
    "Usage: node scripts/patch-latest-json.mjs <latest.json> <artifacts-dir> <version>",
  );
  process.exit(2);
}

const [, , manifestPath, artifactsDir, versionRaw] = process.argv;
if (!manifestPath || !artifactsDir || !versionRaw) usage();
const version = versionRaw.replace(/^v/, "");

const repoUrl = `https://github.com/shiwenwen/hope-agent/releases/download/v${version}`;

const manifestRaw = fs.readFileSync(manifestPath, "utf8");
const manifest = JSON.parse(manifestRaw);

const bareBinaryPlatforms = {};

for (const [platformDir, meta] of Object.entries(PLATFORM_MAP)) {
  const dir = path.join(artifactsDir, `bare-binary-${platformDir}`);
  if (!fs.existsSync(dir)) {
    console.warn(`[patch-latest-json] skip ${platformDir}: artifact dir not found (${dir})`);
    continue;
  }
  const ext = meta.archive === "tar_gz" ? ".tar.gz" : ".zip";
  const archiveFile = `hope-agent-${version}-${meta.key}${ext}`;
  const archivePath = path.join(dir, archiveFile);
  const sigPath = `${archivePath}.sig`;
  if (!fs.existsSync(archivePath)) {
    console.warn(`[patch-latest-json] skip ${meta.key}: archive missing (${archivePath})`);
    continue;
  }
  if (!fs.existsSync(sigPath)) {
    console.warn(`[patch-latest-json] skip ${meta.key}: signature missing (${sigPath})`);
    continue;
  }
  const signature = fs.readFileSync(sigPath, "utf8").trim();
  bareBinaryPlatforms[meta.key] = {
    url: `${repoUrl}/${archiveFile}`,
    signature,
    archive: meta.archive,
    binary_path: meta.binary,
  };
  console.log(`[patch-latest-json] added bare_binary entry for ${meta.key}`);
}

manifest.bare_binary = { platforms: bareBinaryPlatforms };
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
console.log(
  `[patch-latest-json] wrote ${Object.keys(bareBinaryPlatforms).length} bare_binary entries to ${manifestPath}`,
);
