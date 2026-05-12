#!/usr/bin/env node
//
// CI / pre-push sanity check: the Minisign pubkey embedded in
// `crates/ha-core/src/updater/keys.rs::MINISIGN_PUBKEY_BASE64` must match
// `src-tauri/tauri.conf.json#plugins.updater.pubkey`.
//
// Drift means the desktop bundle (`tauri-plugin-updater`) and the
// headless self-update (`ha_core::updater`) verify against different
// keys — one of the two paths silently breaks. We refuse to ship a
// release with that mismatch.
//
// Exit 0 = match. Exit 1 = drift (prints both values).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");

const tauriConfPath = path.join(repoRoot, "src-tauri/tauri.conf.json");
const keysRsPath = path.join(repoRoot, "crates/ha-core/src/updater/keys.rs");

function readPubkeyFromTauriConf() {
  const raw = fs.readFileSync(tauriConfPath, "utf8");
  const conf = JSON.parse(raw);
  const pk = conf?.plugins?.updater?.pubkey;
  if (typeof pk !== "string" || pk.length === 0) {
    throw new Error(
      `tauri.conf.json#plugins.updater.pubkey is missing or empty (${tauriConfPath})`,
    );
  }
  return pk.trim();
}

function readPubkeyFromKeysRs() {
  const raw = fs.readFileSync(keysRsPath, "utf8");
  const match = raw.match(
    /pub const MINISIGN_PUBKEY_BASE64:\s*&str\s*=\s*"([^"]+)"\s*;/,
  );
  if (!match) {
    throw new Error(
      `Could not find MINISIGN_PUBKEY_BASE64 literal in ${keysRsPath}`,
    );
  }
  return match[1].trim();
}

function main() {
  const fromConf = readPubkeyFromTauriConf();
  const fromRs = readPubkeyFromKeysRs();
  if (fromConf === fromRs) {
    console.log(
      "[verify-updater-pubkey] OK — tauri.conf.json and ha-core/updater/keys.rs agree.",
    );
    return 0;
  }
  console.error(
    "[verify-updater-pubkey] DRIFT — desktop and headless updaters will verify against different Minisign keys.",
  );
  console.error(`  tauri.conf.json#plugins.updater.pubkey: ${fromConf}`);
  console.error(`  ha-core/updater/keys.rs MINISIGN_PUBKEY_BASE64: ${fromRs}`);
  console.error(
    "  Resolve by syncing both literals to the same value (and regenerate latest.json with the matching private key if the key changed).",
  );
  return 1;
}

process.exit(main());
