#!/usr/bin/env node
//
// CI / pre-push sanity check for the updater manifest endpoints, which live
// in THREE places that must agree:
//
//   1. src-tauri/tauri.conf.json#plugins.updater.endpoints  (desktop, read
//      by tauri-plugin-updater)
//   2. crates/ha-updater/src/manifest.rs::UPDATE_MANIFEST_URLS
//      (headless / CLI self-update)
//   3. .github/workflows/mirror-release-r2.yml env.PUBLIC_BASE — the domain
//      the mirror actually publishes `download/latest.json` to
//
// Drift between 1 and 2 means the desktop and headless paths disagree about
// which manifest is authoritative: one gets updates, the other silently
// doesn't. Drift between the R2 entry and 3 means every client asks a
// hostname nothing is published to — a 404 that quietly falls through to
// GitHub, taking away exactly the reachability the mirror exists to provide.
//
// Order matters and is checked: these lists are first-success-wins, so
// swapping them changes which source is preferred.
//
// Exit 0 = agree. Exit 1 = drift (prints all sides).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");

const tauriConfPath = path.join(repoRoot, "src-tauri/tauri.conf.json");
const manifestRsPath = path.join(
  repoRoot,
  "crates/ha-updater/src/manifest.rs",
);
const mirrorWorkflowPath = path.join(
  repoRoot,
  ".github/workflows/mirror-release-r2.yml",
);

function readEndpointsFromTauriConf() {
  const conf = JSON.parse(fs.readFileSync(tauriConfPath, "utf8"));
  const eps = conf?.plugins?.updater?.endpoints;
  if (!Array.isArray(eps) || eps.length === 0) {
    throw new Error(
      `tauri.conf.json#plugins.updater.endpoints is missing or empty (${tauriConfPath})`,
    );
  }
  return eps.map((s) => String(s).trim());
}

function readEndpointsFromManifestRs() {
  const raw = fs.readFileSync(manifestRsPath, "utf8");
  const block = raw.match(
    /pub const UPDATE_MANIFEST_URLS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/,
  );
  if (!block) {
    throw new Error(
      `Could not find UPDATE_MANIFEST_URLS literal in ${manifestRsPath}`,
    );
  }
  const urls = [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1].trim());
  if (urls.length === 0) {
    throw new Error(`UPDATE_MANIFEST_URLS is empty in ${manifestRsPath}`);
  }
  return urls;
}

function readPublicBaseFromMirrorWorkflow() {
  const raw = fs.readFileSync(mirrorWorkflowPath, "utf8");
  // Deliberately a line-anchored match on the `env:` entry rather than a YAML
  // parse: this script runs in pre-push, where pulling in a YAML dependency
  // for one scalar is not worth it.
  const m = raw.match(/^\s{2}PUBLIC_BASE:\s*(\S+)\s*$/m);
  if (!m) {
    throw new Error(
      `Could not find the PUBLIC_BASE env entry in ${mirrorWorkflowPath}`,
    );
  }
  return m[1].replace(/\/+$/, "");
}

function main() {
  const fromConf = readEndpointsFromTauriConf();
  const fromRs = readEndpointsFromManifestRs();
  const publicBase = readPublicBaseFromMirrorWorkflow();

  let ok = true;

  if (JSON.stringify(fromConf) !== JSON.stringify(fromRs)) {
    ok = false;
    console.error(
      "[verify-updater-endpoints] DRIFT — desktop and headless updaters would read different manifests.",
    );
    console.error(`  tauri.conf.json#plugins.updater.endpoints:`);
    for (const e of fromConf) console.error(`    - ${e}`);
    console.error(`  ha-core/updater/manifest.rs UPDATE_MANIFEST_URLS:`);
    for (const e of fromRs) console.error(`    - ${e}`);
    console.error(
      "  Resolve by making both lists identical, in the same order (order = preference).",
    );
  }

  // The mirror entry, if configured, must be the domain the mirror workflow
  // publishes to. Absence is allowed — the mirror is an addition, not a
  // requirement — but a mismatched host is always a bug.
  const mirrorEntries = fromConf.filter((e) => !e.includes("github.com"));
  for (const entry of mirrorEntries) {
    if (!entry.startsWith(`${publicBase}/`)) {
      ok = false;
      console.error(
        "[verify-updater-endpoints] DRIFT — a non-GitHub updater endpoint does not match the mirror's published base.",
      );
      console.error(`  endpoint:                       ${entry}`);
      console.error(`  mirror-release-r2.yml PUBLIC_BASE: ${publicBase}`);
      console.error(
        "  Resolve by pointing both at the same host (and remember README.md / README.en.md / update-linux-repo.yml / linux-repo/rpm/hope-agent.repo carry the same base).",
      );
    }
  }

  if (!ok) return 1;
  console.log(
    `[verify-updater-endpoints] OK — ${fromConf.length} endpoint(s) agree across tauri.conf.json, manifest.rs, and PUBLIC_BASE (${publicBase}).`,
  );
  return 0;
}

process.exit(main());
