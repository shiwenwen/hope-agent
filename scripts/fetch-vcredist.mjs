#!/usr/bin/env node
// Fetches the Visual C++ 2015-2022 Redistributable (x64) installer for the
// Windows NSIS bundle. Wired into bundle.resources via tauri.windows.conf.json
// and executed by src-tauri/windows/installer-hooks.nsh during install.

import { createWriteStream, existsSync, statSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname } from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const URL = "https://aka.ms/vs/17/release/vc_redist.x64.exe";
const DEST = "src-tauri/resources/vc_redist.x64.exe";
const MIN_BYTES = 10_000_000;

if (existsSync(DEST) && statSync(DEST).size >= MIN_BYTES) {
  console.log(`[fetch-vcredist] already present at ${DEST}, skipping`);
  process.exit(0);
}

await mkdir(dirname(DEST), { recursive: true });
console.log(`[fetch-vcredist] downloading ${URL} → ${DEST}`);

const res = await fetch(URL, { redirect: "follow" });
if (!res.ok || !res.body) {
  console.error(`[fetch-vcredist] HTTP ${res.status} ${res.statusText}`);
  process.exit(1);
}

await pipeline(Readable.fromWeb(res.body), createWriteStream(DEST));
const size = statSync(DEST).size;
if (size < MIN_BYTES) {
  console.error(`[fetch-vcredist] downloaded file too small (${size} bytes)`);
  process.exit(1);
}
console.log(`[fetch-vcredist] done (${(size / 1024 / 1024).toFixed(1)} MB)`);
