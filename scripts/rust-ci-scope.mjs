#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";

const SAFE_EXACT_PATHS = new Set([
  ".github/CODEOWNERS",
  ".npmrc",
  ".prettierignore",
  ".prettierrc",
  "LICENSE",
  "eslint.config.js",
  "index.html",
  "pnpm-lock.yaml",
  "tsconfig.app.json",
  "tsconfig.json",
  "tsconfig.node.json",
  "vite.config.ts",
  "vitest.setup.ts",
]);

function normalizePath(filePath) {
  return filePath.replaceAll("\\", "/").replace(/^\.\//, "");
}

export function requiresRustCi(filePath) {
  const path = normalizePath(filePath);
  if (!path) return false;

  // These trees are consumed by ha-core at compile time. Keep all frontend
  // assets fail-closed too: ha-core already embeds src/assets/logo.png and a
  // future asset can become embedded without requiring a classifier change.
  if (
    path.startsWith("src/assets/") ||
    path.startsWith("docs/user-guide/") ||
    path.startsWith("extensions/chrome/") ||
    path.startsWith("skills/")
  ) {
    return true;
  }

  if (
    SAFE_EXACT_PATHS.has(path) ||
    /^[^/]+\.md$/.test(path) ||
    path.startsWith(".claude/") ||
    path.startsWith(".github/ISSUE_TEMPLATE/") ||
    path.startsWith("docs/") ||
    path.startsWith("public/") ||
    /^src\/.+\.(?:css|js|json|md|ts|tsx)$/.test(path)
  ) {
    return false;
  }

  // Unknown paths fail closed. This includes Rust sources/manifests, build and
  // release scripts, workflows, eval fixtures, Tauri config, and package.json
  // (the application's canonical version source).
  return true;
}

function scopeForPaths(paths) {
  return paths.some(requiresRustCi);
}

function gitOutput(args) {
  return execFileSync("git", args, {
    encoding: "buffer",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function nullSeparatedPaths(buffer) {
  return buffer
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

function gitDiffPaths(base, head) {
  return nullSeparatedPaths(
    gitOutput(["diff", "--name-only", "-z", base, head]),
  );
}

function mergeBase(localSha) {
  for (const upstream of ["origin/main", "main"]) {
    try {
      return gitOutput(["merge-base", localSha, upstream])
        .toString("utf8")
        .trim();
    } catch {
      // Try the next locally available main ref.
    }
  }
  throw new Error("cannot determine merge base for a new remote branch");
}

function pathsFromPrePushRefs(refsFile) {
  const zeroSha = /^0+$/;
  const sha = /^[0-9a-f]{40,64}$/i;
  const paths = new Set();
  let sawRef = false;

  for (const line of fs.readFileSync(refsFile, "utf8").split(/\r?\n/)) {
    if (!line.trim()) continue;
    sawRef = true;
    const [localRef, localSha, remoteRef, remoteSha] = line.trim().split(/\s+/);
    if (!localRef || !remoteRef || !sha.test(localSha) || !sha.test(remoteSha)) {
      throw new Error(`malformed pre-push ref line: ${line}`);
    }
    if (zeroSha.test(localSha)) continue; // Remote ref deletion.

    const base = zeroSha.test(remoteSha) ? mergeBase(localSha) : remoteSha;
    for (const path of gitDiffPaths(base, localSha)) paths.add(path);
  }

  if (!sawRef) throw new Error("pre-push ref list is empty");
  return [...paths];
}

function selfTest() {
  for (const path of [
    "src/components/ChatInput.tsx",
    "public/favicon.png",
    "docs/architecture/session.md",
    "README.md",
    ".github/CODEOWNERS",
    "tsconfig.json",
  ]) {
    assert.equal(requiresRustCi(path), false, `${path} should skip Rust CI`);
  }

  for (const path of [
    "src/assets/logo.png",
    "src/assets/hero.png",
    "docs/user-guide/en/index.md",
    "extensions/chrome/manifest.json",
    "skills/ha-manual/SKILL.md",
    "crates/ha-core/src/lib.rs",
    "Cargo.toml",
    ".github/workflows/rust.yml",
    "scripts/rust-ci-scope.mjs",
    "package.json",
  ]) {
    assert.equal(requiresRustCi(path), true, `${path} should run Rust CI`);
  }
}

function printScope(paths) {
  process.stdout.write(scopeForPaths(paths) ? "true\n" : "false\n");
}

const [mode, ...args] = process.argv.slice(2);

try {
  switch (mode) {
    case "--self-test":
      selfTest();
      break;
    case "--git-diff":
      if (args.length !== 2) throw new Error("--git-diff requires BASE HEAD");
      printScope(gitDiffPaths(args[0], args[1]));
      break;
    case "--pre-push":
      if (args.length !== 1) throw new Error("--pre-push requires a refs file");
      printScope(pathsFromPrePushRefs(args[0]));
      break;
    default:
      throw new Error(
        "usage: rust-ci-scope.mjs --self-test | --git-diff BASE HEAD | --pre-push REFS_FILE",
      );
  }
} catch (error) {
  if (mode === "--self-test") {
    console.error(`[rust-ci-scope] self-test failed: ${error.message}`);
    process.exitCode = 1;
  } else {
    // Scope detection is an optimization, never a reason to omit the gate. Any
    // malformed input, missing ref, or Git error therefore requires full Rust CI.
    console.error(`[rust-ci-scope] ${error.message}; running full Rust CI`);
    process.stdout.write("true\n");
  }
}
