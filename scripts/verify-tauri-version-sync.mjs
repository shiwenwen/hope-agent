#!/usr/bin/env node
//
// Assert the Rust `tauri` crate and the npm `@tauri-apps/api` package agree on
// major.minor.
//
// Why this exists: the Tauri CLI refuses to build when they disagree —
//
//   Found version mismatched Tauri packages. Make sure the NPM package and
//   Rust crate versions are on the same major/minor releases:
//   tauri (v2.11.5) : @tauri-apps/api (v2.10.1)
//
// — and nothing else in CI catches it, because `cargo clippy` / `cargo test` /
// `vitest` never invoke `tauri build`. So the mismatch is invisible until a tag
// is pushed and the real release workflow runs, at which point every platform
// lane fails within minutes and the tag has to be deleted and re-cut.
//
// That is exactly what happened to v0.30.0: #621 bumped the locked `tauri`
// crate 2.10.3 → 2.11.5 as an incidental dependency update, `@tauri-apps/api`
// stayed pinned at 2.10.1, four PRs merged green on top of it, and the break
// only surfaced during the release build.
//
// Patch versions are deliberately NOT compared — the CLI only requires
// major/minor agreement, and demanding exact equality would fail on every
// routine `cargo update`.

import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");

function fail(msg) {
  console.error(`[verify-tauri-version-sync] ${msg}`);
  process.exit(1);
}

// Cargo.lock stores packages as `[[package]]` blocks; find the one whose name
// is exactly `tauri` (not `tauri-build`, `tauri-utils`, …).
function lockedCrateVersion(crate) {
  const lock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
  const re = new RegExp(
    `\\[\\[package\\]\\]\\s*\\nname = "${crate}"\\s*\\nversion = "([^"]+)"`,
  );
  const m = lock.match(re);
  if (!m) fail(`Cargo.lock has no [[package]] entry for "${crate}"`);
  return m[1];
}

// Read the RESOLVED version from pnpm-lock.yaml, not the range in
// package.json. The declared range is not what gets installed: `^2.11.1`
// happily admits 2.12.x, so a routine `pnpm update` could resolve the lockfile
// onto a version the crate disagrees with while the floor still reads 2.11 —
// and a floor-based check would wave it through, which is precisely the
// release-breaking case this guard exists to catch. `--frozen-lockfile` CI
// installs whatever the lockfile pins, so the lockfile is the truth.
//
// The importers block records both, e.g.
//       '@tauri-apps/api':
//         specifier: ^2.11.1
//         version: 2.11.1
function resolvedNpmVersion(pkg) {
  const lock = fs.readFileSync(path.join(root, "pnpm-lock.yaml"), "utf8");
  const re = new RegExp(
    `'${pkg}':\\s*\\n\\s*specifier:\\s*(\\S+)\\s*\\n\\s*version:\\s*(\\S+)`,
  );
  const m = lock.match(re);
  if (!m) {
    fail(
      `pnpm-lock.yaml has no resolved importer entry for "${pkg}" — run \`pnpm install\` to refresh the lockfile`,
    );
  }
  // Peer-resolved entries carry a suffix like `2.11.1(typescript@5.9.2)`.
  return { specifier: m[1], version: m[2].replace(/\(.*$/, "") };
}

const majorMinor = (v) => v.split(".").slice(0, 2).join(".");

const crateVersion = lockedCrateVersion("tauri");
const npm = resolvedNpmVersion("@tauri-apps/api");

if (majorMinor(crateVersion) !== majorMinor(npm.version)) {
  fail(
    `tauri crate (Cargo.lock) is ${crateVersion} but @tauri-apps/api resolves to ${npm.version} (declared ${npm.specifier}).\n` +
      `  The Tauri CLI requires the same major.minor, so \`tauri build\` — and therefore the\n` +
      `  entire release workflow — will fail on every platform.\n` +
      `  Fix: set "@tauri-apps/api" to ^${majorMinor(crateVersion)}.x in package.json and run \`pnpm install\`.`,
  );
}

console.log(
  `[verify-tauri-version-sync] OK — tauri ${crateVersion} ↔ @tauri-apps/api ${npm.version} (declared ${npm.specifier}; both ${majorMinor(crateVersion)}.x)`,
);
