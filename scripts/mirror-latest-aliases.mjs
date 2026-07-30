#!/usr/bin/env node
//
// Build the version-less `download/latest/` alias set from a release's
// assets, so README can carry download links that never go stale.
//
// Usage:
//   node scripts/mirror-latest-aliases.mjs <assets-dir> <version> <out-dir>
//
// WHY version-less aliases exist: the mirror stores each release under an
// immutable `download/<tag>/` prefix, which is right for permanence but
// unusable in documentation — README cannot hardcode a version that changes
// every release. Users who cannot reach github.com need ONE stable URL per
// platform, so this produces `Hope.Agent_aarch64.dmg` next to the pinned
// `download/v0.26.0/Hope.Agent_0.26.0_aarch64.dmg`.
//
// The rename is a RULE (strip the version token, tidy separators), not a
// hand-maintained table, so a new artifact kind is covered automatically.
// But rules mis-fire silently, so `REQUIRED_ALIASES` pins the names README
// actually links: if the rule ever stops producing one of those, the mirror
// fails instead of publishing a README full of 404s.

import fs from "node:fs";
import path from "node:path";

// Exactly the filenames README.md / README.en.md link under
// https://repo.hopeagent.ai/download/latest/. Adding a link there? Add it
// here too, or it is not covered by the mirror's verification gate.
//
// Note the arch labels are NOT uniform upstream — AppImage says `aarch64`
// where deb says `arm64`, and the rpm drops the `_` separator entirely.
// That inconsistency is exactly why this list is pinned rather than assumed.
const REQUIRED_ALIASES = [
  "Hope.Agent_aarch64.dmg",
  "Hope.Agent_x64-setup.exe",
  "Hope.Agent_amd64.AppImage",
  "Hope.Agent_aarch64.AppImage",
  "Hope.Agent_amd64.deb",
  "Hope.Agent_arm64.deb",
  "Hope.Agent.x86_64.rpm",
  "Hope.Agent.aarch64.rpm",
];

/**
 * Strip the version out of a release asset filename.
 *
 * The four shapes tauri-action + release.yml produce:
 *   Hope.Agent_0.26.0_aarch64.dmg        -> Hope.Agent_aarch64.dmg
 *   Hope.Agent-0.26.0-1.x86_64.rpm       -> Hope.Agent.x86_64.rpm
 *   hope-agent-0.26.0-linux-x86_64.tar.gz-> hope-agent-linux-x86_64.tar.gz
 *   Hope.Agent_aarch64.app.tar.gz        -> unchanged (already version-less)
 */
export function aliasFor(name, version) {
  if (!name.includes(version)) return name;
  let out = name;
  // The rpm carries a release number after the version (`-<v>-1.`); drop both
  // so the alias does not pin a build revision either.
  out = out.replace(`-${version}-1.`, ".");
  out = out.replace(`_${version}_`, "_");
  out = out.replace(`-${version}-`, "-");
  out = out.replace(`_${version}`, "");
  out = out.replace(`-${version}`, "");
  out = out.replace(version, "");
  // Collapse separators a removal may have doubled up, and never emit a name
  // that starts or ends on one.
  out = out.replace(/([._-])\1+/g, "$1").replace(/^[._-]+|[._-]+$/g, "");
  return out;
}

function main(argv) {
  const [assetsDir, version, outDir] = argv;
  if (!assetsDir || !version || !outDir) {
    console.error(
      "Usage: node scripts/mirror-latest-aliases.mjs <assets-dir> <version> <out-dir>",
    );
    return 2;
  }

  fs.mkdirSync(outDir, { recursive: true });
  const produced = new Map(); // alias -> source name

  for (const name of fs.readdirSync(assetsDir).sort()) {
    const src = path.join(assetsDir, name);
    if (!fs.statSync(src).isFile()) continue;
    const alias = aliasFor(name, version);
    const prior = produced.get(alias);
    if (prior && prior !== name) {
      // Two different assets collapsing onto one alias would make the alias
      // ambiguous — whichever copied last would win, silently.
      console.error(
        `[mirror-latest-aliases] alias collision: "${prior}" and "${name}" both map to "${alias}"`,
      );
      return 1;
    }
    fs.copyFileSync(src, path.join(outDir, alias));
    produced.set(alias, name);
  }

  const missing = REQUIRED_ALIASES.filter((a) => !produced.has(a));
  if (missing.length) {
    console.error(
      "[mirror-latest-aliases] refusing to publish: README links these aliases but the release produced no asset for them:",
    );
    for (const m of missing) console.error(`  - ${m}`);
    console.error(
      "  Either the release is incomplete, or an artifact filename changed and REQUIRED_ALIASES / README need updating together.",
    );
    return 1;
  }

  console.log(
    `[mirror-latest-aliases] ${produced.size} alias(es) staged in ${outDir} (all ${REQUIRED_ALIASES.length} README-linked names present)`,
  );
  for (const [alias, from] of [...produced].sort()) {
    console.log(`  ${alias}${alias === from ? "  (unchanged)" : `  <- ${from}`}`);
  }
  return 0;
}

// Importable for tests; only runs the CLI when invoked directly.
if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  process.exit(main(process.argv.slice(2)));
}
