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

function declaredNpmRange(pkg) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(root, "package.json"), "utf8"),
  );
  const range =
    manifest.dependencies?.[pkg] ?? manifest.devDependencies?.[pkg];
  if (!range) fail(`package.json does not depend on "${pkg}"`);
  return range;
}

const majorMinor = (v) => v.split(".").slice(0, 2).join(".");

const crateVersion = lockedCrateVersion("tauri");
const npmRange = declaredNpmRange("@tauri-apps/api");
// `^2.11.1` / `~2.11.1` / `2.11.1` all carry the floor we care about; the CLI
// compares against whatever is installed, and the floor is what pins it.
const npmFloor = npmRange.replace(/^[\^~>=\s]*/, "");

if (majorMinor(crateVersion) !== majorMinor(npmFloor)) {
  fail(
    `tauri crate (Cargo.lock) is ${crateVersion} but @tauri-apps/api (package.json) is ${npmRange}.\n` +
      `  The Tauri CLI requires the same major.minor, so \`tauri build\` — and therefore the\n` +
      `  entire release workflow — will fail on every platform.\n` +
      `  Fix: set "@tauri-apps/api" to ^${majorMinor(crateVersion)}.x in package.json and run \`pnpm install\`.`,
  );
}

console.log(
  `[verify-tauri-version-sync] OK — tauri ${crateVersion} ↔ @tauri-apps/api ${npmRange} (both ${majorMinor(crateVersion)}.x)`,
);
