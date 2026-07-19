#!/usr/bin/env node
//
// Patch `latest.json` (the Tauri updater manifest) with a `bare_binary`
// section that maps each platform to a tar.gz / zip archive + its
// Minisign signature. The headless self-update path in
// `ha_core::updater::self_contained` consumes that section.
//
// Usage:
//   node scripts/patch-latest-json.mjs <latest.json> <artifacts-dir> <version>
//                                      [--require=<platform,...>]
//
// `--require` names the platforms that MUST yield a bare_binary entry; a
// missing one is a hard error instead of a warning. Without it the script
// stays best-effort, which is what a single-platform backfill wants.
//
// Why this exists: every platform used to be skipped with a console.warn, so
// a lost or half-uploaded artifact produced a manifest missing that
// platform's bare_binary entry — CI green, and headless self-update for that
// platform silently dead until someone noticed. release.yml now passes its
// full build matrix, so the release fails loudly instead.
//
// It cannot simply require every platform in PLATFORM_MAP: `macos-x64` has
// been a disabled lane since v0.2.0 (see release.yml matrix), so its artifact
// dir is legitimately absent on every run.
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

// `extras`: additional executables shipped in the same archive (the
// native-messaging browser host); `self_contained::install` swaps them next
// to the main binary so bare-binary upgrades keep the host current too.
const PLATFORM_MAP = {
  "macos-arm64": { key: "darwin-aarch64", archive: "tar_gz", binary: "hope-agent", extras: ["ha-browser-host"] },
  "macos-x64": { key: "darwin-x86_64", archive: "tar_gz", binary: "hope-agent", extras: ["ha-browser-host"] },
  "linux-x64": { key: "linux-x86_64", archive: "tar_gz", binary: "hope-agent", extras: ["ha-browser-host"] },
  "linux-arm64": { key: "linux-aarch64", archive: "tar_gz", binary: "hope-agent", extras: ["ha-browser-host"] },
  "windows-x64": { key: "windows-x86_64", archive: "zip", binary: "hope-agent.exe", extras: ["ha-browser-host.exe"] },
};

function usage() {
  console.error(
    "Usage: node scripts/patch-latest-json.mjs <latest.json> <artifacts-dir> <version> [--require=<platform,...>]",
  );
  process.exit(2);
}

const argv = process.argv.slice(2);
const requireFlag = argv.find((a) => a.startsWith("--require="));
const [manifestPath, artifactsDir, versionRaw] = argv.filter(
  (a) => !a.startsWith("--"),
);
if (!manifestPath || !artifactsDir || !versionRaw) usage();
const version = versionRaw.replace(/^v/, "");

const required = new Set(
  (requireFlag ? requireFlag.slice("--require=".length) : "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean),
);
// A typo here would silently require nothing, which is exactly the
// fail-open behaviour this flag exists to remove.
for (const p of required) {
  if (!PLATFORM_MAP[p]) {
    console.error(
      `[patch-latest-json] --require names unknown platform "${p}" (known: ${Object.keys(PLATFORM_MAP).join(", ")})`,
    );
    process.exit(1);
  }
}

const failures = [];
function miss(platform, message) {
  if (required.has(platform)) failures.push(`${platform}: ${message}`);
  else console.warn(`[patch-latest-json] skip ${platform}: ${message}`);
}

const repoUrl = `https://github.com/shiwenwen/hope-agent/releases/download/v${version}`;

const manifestRaw = fs.readFileSync(manifestPath, "utf8");
const manifest = JSON.parse(manifestRaw);

const bareBinaryPlatforms = {};

for (const [platformDir, meta] of Object.entries(PLATFORM_MAP)) {
  const dir = path.join(artifactsDir, `bare-binary-${platformDir}`);
  if (!fs.existsSync(dir)) {
    miss(platformDir, `artifact dir not found (${dir})`);
    continue;
  }
  const ext = meta.archive === "tar_gz" ? ".tar.gz" : ".zip";
  const archiveFile = `hope-agent-${version}-${meta.key}${ext}`;
  const archivePath = path.join(dir, archiveFile);
  const sigPath = `${archivePath}.sig`;
  // A present dir with a missing archive / signature means the upload was
  // truncated rather than the lane being absent — always fatal, even when
  // the platform was not explicitly required.
  if (!fs.existsSync(archivePath)) {
    failures.push(`${platformDir}: artifact dir exists but archive missing (${archivePath})`);
    continue;
  }
  if (!fs.existsSync(sigPath)) {
    failures.push(`${platformDir}: artifact dir exists but signature missing (${sigPath})`);
    continue;
  }
  const signature = fs.readFileSync(sigPath, "utf8").trim();
  bareBinaryPlatforms[meta.key] = {
    url: `${repoUrl}/${archiveFile}`,
    signature,
    archive: meta.archive,
    binary_path: meta.binary,
    extra_binaries: meta.extras,
  };
  console.log(`[patch-latest-json] added bare_binary entry for ${meta.key}`);
}

// Merge mode: keep entries that already exist on the manifest (e.g.
// from a previous release.yml patch run), and add/override only the
// platforms we found artifacts for in this run. This lets independent
// best-effort workflows (e.g. build-macos-x64.yml) backfill a single
// platform's entry without wiping the other four.
if (failures.length) {
  console.error("[patch-latest-json] refusing to write an incomplete manifest:");
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}

const existing = manifest.bare_binary?.platforms || {};
manifest.bare_binary = { platforms: { ...existing, ...bareBinaryPlatforms } };
fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
console.log(
  `[patch-latest-json] merged ${Object.keys(bareBinaryPlatforms).length} new bare_binary entries into ${manifestPath} (manifest now has ${Object.keys(manifest.bare_binary.platforms).length} platforms total)`,
);
