#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, realpathSync } from "node:fs";
import path from "node:path";

const LOCK_PATH = "evals/version-lock.json";
const LIVE_LOCK_PATH = "evals/live/version-lock.json";

function fail(message) {
  console.error(`eval version lock: ${message}`);
  process.exit(1);
}

function parseLock(raw, label) {
  let lock;
  try {
    lock = JSON.parse(raw);
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
  if (lock?.schemaVersion !== "eval-version-lock.v1") {
    fail(`${label} has an unsupported schemaVersion`);
  }
  for (const section of ["suites", "policies"]) {
    if (!lock[section] || Array.isArray(lock[section]) || typeof lock[section] !== "object") {
      fail(`${label} is missing object section ${section}`);
    }
    for (const [key, digest] of Object.entries(lock[section])) {
      if (typeof digest !== "string" || !/^[0-9a-f]{64}$/i.test(digest)) {
        fail(`${label} entry ${section}.${key} is not a SHA-256 digest`);
      }
    }
  }
  return lock;
}

function parseLiveLock(raw, label, allowLegacyMissing = false) {
  let lock;
  try {
    lock = JSON.parse(raw);
  } catch (error) {
    fail(`${label} is not valid JSON: ${error.message}`);
  }
  if (lock?.schemaVersion !== "model-campaign-version-lock.v1") {
    fail(`${label} has an unsupported schemaVersion`);
  }
  // These sections were added after the original live lock. Treat their
  // absence in an older base as empty while requiring them in the worktree.
  if (!lock.appProfiles && allowLegacyMissing) lock.appProfiles = {};
  if (!lock.trustRegistries && allowLegacyMissing) lock.trustRegistries = {};
  for (const section of ["suites", "policies", "scenarios", "appProfiles", "trustRegistries"]) {
    if (!lock[section] || Array.isArray(lock[section]) || typeof lock[section] !== "object") {
      fail(`${label} is missing object section ${section}`);
    }
    for (const [key, digest] of Object.entries(lock[section])) {
      if (typeof digest !== "string" || !/^[0-9a-f]{64}$/i.test(digest)) {
        fail(`${label} entry ${section}.${key} is not a SHA-256 digest`);
      }
    }
  }
  return lock;
}

function verifyAppendOnly(previous, current, sections, label) {
  for (const section of sections) {
    for (const [key, digest] of Object.entries(previous[section])) {
      if (!(key in current[section])) {
        fail(`append-only violation in ${label}: removed ${section}.${key}`);
      }
      if (current[section][key] !== digest) {
        fail(`immutable-version violation in ${label}: changed ${section}.${key}; increment the version and append a new entry instead`);
      }
    }
  }
}

function canonicalValue(value) {
  if (Array.isArray(value)) return value.map(canonicalValue);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([key, child]) => [key, canonicalValue(child)]),
    );
  }
  return value;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function digestValue(value) {
  return sha256(JSON.stringify(canonicalValue(value)));
}

function normalizedCase(source) {
  const normalized = { id: source.id };
  if (source.path !== undefined) normalized.path = source.path;
  if (source.timeoutSeconds !== undefined) normalized.timeoutSeconds = source.timeoutSeconds;
  normalized.tags = source.tags ?? [];
  return normalized;
}

function suiteDigest(suitePath) {
  const suiteDir = path.dirname(suitePath);
  const canonicalSuiteDir = realpathSync(suiteDir);
  const source = JSON.parse(readFileSync(suitePath, "utf8"));
  const cases = source.cases.map(normalizedCase);
  const manifest = {
    schemaVersion: source.schemaVersion,
    id: source.id,
    version: source.version,
    capability: source.capability,
    adapter: source.adapter,
    tiers: source.tiers,
    runnerClass: source.runnerClass,
    networkPolicy: source.networkPolicy,
    shards: source.shards ?? 1,
    timeoutSeconds: source.timeoutSeconds ?? 180,
    thresholds: source.thresholds ?? {},
    cases,
  };
  const caseDigests = {};
  for (const evalCase of cases) {
    const value = structuredClone(evalCase);
    if (evalCase.path !== undefined) {
      if (path.isAbsolute(evalCase.path) || evalCase.path.split(/[\\/]/).some((part) => part === ".." || part === "." || part === "")) {
        fail(`${suitePath} contains unsafe asset path ${evalCase.path}`);
      }
      const assetPath = realpathSync(path.join(suiteDir, evalCase.path));
      if (assetPath !== canonicalSuiteDir && !assetPath.startsWith(`${canonicalSuiteDir}${path.sep}`)) {
        fail(`${suitePath} asset escapes its suite directory: ${evalCase.path}`);
      }
      value.assetSha256 = sha256(readFileSync(assetPath));
    }
    caseDigests[evalCase.id] = digestValue(value);
  }
  manifest.caseDigests = caseDigests;
  return { key: `${source.id}@${source.version}`, digest: digestValue(manifest) };
}

function policyDigest(policyPath) {
  const source = JSON.parse(readFileSync(policyPath, "utf8"));
  const policy = {
    schemaVersion: source.schemaVersion,
    id: source.id,
    version: source.version,
    tier: source.tier,
    mode: source.mode,
    allowedAdapters: source.allowedAdapters,
    suites: source.suites.map((suite) => ({
      id: suite.id,
      minPassRate: suite.minPassRate ?? 1,
    })),
    performanceBlocking: source.performanceBlocking ?? false,
    maxDurationSeconds: source.maxDurationSeconds ?? 1800,
  };
  return { key: `${source.id}@${source.version}`, digest: digestValue(policy) };
}

function verifyCurrentDigests(lock) {
  const current = {
    suites: readdirSync("evals/suites", { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => suiteDigest(path.join("evals/suites", entry.name, "suite.json"))),
    policies: readdirSync("evals/policy")
      .filter((name) => name.endsWith(".json"))
      .map((name) => policyDigest(path.join("evals/policy", name))),
  };
  for (const section of ["suites", "policies"]) {
    for (const { key, digest } of current[section]) {
      if (lock[section][key] !== digest) {
        fail(`${section}.${key} does not match current canonical content; increment the version and append digest ${digest}`);
      }
    }
  }
}

const baseIndex = process.argv.indexOf("--base");
let base = baseIndex >= 0 ? process.argv[baseIndex + 1] : undefined;
if (!base) {
  fail("usage: verify-eval-version-lock.mjs --base <git-ref>");
}
if (/^0+$/.test(base)) {
  const githubSha = process.env.GITHUB_SHA;
  if (!githubSha) {
    fail("a zero push base requires GITHUB_SHA so the parent commit can be checked");
  }
  base = `${githubSha}^`;
}

const current = parseLock(readFileSync(LOCK_PATH, "utf8"), "working-tree lock");
verifyCurrentDigests(current);
try {
  execFileSync("git", ["cat-file", "-e", `${base}^{commit}`], {
    stdio: "ignore",
  });
} catch {
  fail(`base ref ${base} is not available; fetch history before validating the lock`);
}
let previousRaw;
try {
  previousRaw = execFileSync("git", ["show", `${base}:${LOCK_PATH}`], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
} catch {
  console.log(`eval version lock: ${LOCK_PATH} does not exist at ${base}; treating this as initial introduction`);
  process.exit(0);
}

const previous = parseLock(previousRaw, `lock at ${base}`);
verifyAppendOnly(previous, current, ["suites", "policies"], LOCK_PATH);

const liveCurrent = parseLiveLock(readFileSync(LIVE_LOCK_PATH, "utf8"), "working-tree live lock");
let livePreviousRaw;
try {
  livePreviousRaw = execFileSync("git", ["show", `${base}:${LIVE_LOCK_PATH}`], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
} catch {
  console.log(`eval version lock: ${LIVE_LOCK_PATH} does not exist at ${base}; treating this as initial introduction`);
  console.log(`eval version lock: existing entries from ${base} are unchanged; new entries are append-only`);
  process.exit(0);
}
const livePrevious = parseLiveLock(livePreviousRaw, `live lock at ${base}`, true);
verifyAppendOnly(livePrevious, liveCurrent, ["suites", "policies", "scenarios", "appProfiles", "trustRegistries"], LIVE_LOCK_PATH);

console.log(`eval version lock: existing entries from ${base} are unchanged; new entries are append-only`);
