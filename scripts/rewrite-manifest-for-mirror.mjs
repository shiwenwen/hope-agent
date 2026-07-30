#!/usr/bin/env node
//
// Derive the R2-hosted copy of `latest.json` from the authoritative
// GitHub-hosted one: repoint every download URL at the mirror, and repoint
// the two GitHub doc links inside `notes` at the mirrored copies.
//
// Usage:
//   node scripts/rewrite-manifest-for-mirror.mjs <latest.json> <tag> <public-base> [--out=<path>]
//
// Example:
//   node scripts/rewrite-manifest-for-mirror.mjs manifest/latest.json v0.26.0 \
//     https://repo.hopeagent.ai --out=upload/latest.json
//
// WHY this exists: a real population of users cannot reach github.com at
// all. For them the manifest is not a latency question — it is whether
// auto-update works, because a manifest that never loads means the
// installer URLs inside it are never read. Mirroring the manifest without
// rewriting its URLs would be pointless: they'd resolve the manifest and
// then fail on the 100–190 MB download.
//
// The GitHub-hosted manifest stays AUTHORITATIVE and untouched. This
// output is a derived artifact. The two deliberately diverge in exactly
// two ways — download URLs and the `notes` doc links — and nothing else.
// Every URL must be recognized and rewritten: an unrecognized one, or a
// surviving `releases/download/` URL, fails the run rather than shipping a
// manifest that sends some users back to github.com for bytes.
//
// SECURITY: the per-platform `signature` values are copied verbatim and
// never recomputed. Verification happens against the Minisign public key
// compiled into the binary (`ha_core::updater::keys::MINISIGN_PUBKEY_BASE64`),
// so a tampered mirror cannot cause a malicious install — the worst it can
// do is deny service or advertise a stale version. Rewriting a signature
// here, or deriving one from mirror bytes, would break that property.

import fs from "node:fs";

function usage() {
  console.error(
    "Usage: node scripts/rewrite-manifest-for-mirror.mjs <latest.json> <tag> <public-base> [--out=<path>]",
  );
  process.exit(2);
}

const argv = process.argv.slice(2);
const outFlag = argv.find((a) => a.startsWith("--out="));
const docsOutFlag = argv.find((a) => a.startsWith("--docs-out="));
const [manifestPath, tagRaw, publicBaseRaw] = argv.filter(
  (a) => !a.startsWith("--"),
);
if (!manifestPath || !tagRaw || !publicBaseRaw) usage();

const tag = tagRaw.startsWith("v") ? tagRaw : `v${tagRaw}`;
const version = tag.slice(1);
const publicBase = publicBaseRaw.replace(/\/+$/, "");
const outPath = outFlag ? outFlag.slice("--out=".length) : manifestPath;

// Mirror layout, mirroring GitHub's own: immutable per-version prefix. The
// only mutable object is `download/latest.json`, written by the workflow
// (not here) with a short Cache-Control so the edge cannot pin a stale
// manifest — see mirror-release-r2.yml.
const mirrorBase = `${publicBase}/download/${tag}`;

// Any release asset URL on the GitHub release for THIS tag. Matching on the
// tag (not a bare hostname) means a URL pointing at some other release —
// which would be a bug upstream — fails the assertion below instead of
// being silently rewritten to a file we never mirrored.
const RELEASE_ASSET_RE = new RegExp(
  `^https://github\\.com/shiwenwen/hope-agent/releases/download/${tag.replace(/\./g, "\\.")}/(.+)$`,
);

const problems = [];
const rewrites = [];
// Repo-relative doc paths the `notes` rewrite now points at. Emitted via
// --docs-out so the workflow uploads exactly the set that got linked,
// instead of carrying a hardcoded list that silently drifts when a release
// note gains or loses a cross-file link.
const mirroredDocs = new Set();

function rewriteAssetUrl(where, url) {
  const m = RELEASE_ASSET_RE.exec(url);
  if (!m) {
    problems.push(`${where}: not a v${version} release asset URL — ${url}`);
    return url;
  }
  const next = `${mirrorBase}/${m[1]}`;
  rewrites.push(`${where}: ${m[1]}`);
  return next;
}

const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

if (manifest.version !== version) {
  console.error(
    `[rewrite-manifest] manifest.version is "${manifest.version}" but tag says "${version}" — refusing to rewrite a manifest for a different release.`,
  );
  process.exit(1);
}

// ── platforms[] (tauri-plugin-updater) ─────────────────────────────────
const platforms = manifest.platforms ?? {};
if (Object.keys(platforms).length === 0) {
  console.error("[rewrite-manifest] manifest has no `platforms` — refusing.");
  process.exit(1);
}
for (const [key, entry] of Object.entries(platforms)) {
  entry.url = rewriteAssetUrl(`platforms.${key}`, entry.url);
}

// ── bare_binary.platforms[] (headless self-update) ─────────────────────
// Absent on manifests predating the extension; tolerated (the headless
// path falls back to the package manager), but if present it must be
// complete — a half-rewritten section would send headless installs to
// GitHub for some platforms and R2 for others.
const bare = manifest.bare_binary?.platforms ?? {};
for (const [key, entry] of Object.entries(bare)) {
  entry.url = rewriteAssetUrl(`bare_binary.platforms.${key}`, entry.url);
}

// ── notes: the two tag-pinned GitHub doc links ─────────────────────────
// `notes` is the release-notes body, rendered as clickable markdown in the
// in-app "update available" dialog (AboutPanel -> MarkdownRenderer). Its
// language-switch and CHANGELOG links point at github.com/blob/<tag>/…,
// which are dead ends for exactly the users the mirror exists for.
//
// The repo's source files keep their GitHub URLs — AGENTS.md's "release
// notes links must be absolute tag-pinned GitHub URLs" rule is unchanged.
// Only this derived copy is rewritten.
let notesRewrites = 0;
if (typeof manifest.notes === "string" && manifest.notes.length > 0) {
  const BLOB_RE = new RegExp(
    `https://github\\.com/shiwenwen/hope-agent/blob/${tag.replace(/\./g, "\\.")}/([^)\\s]+)`,
    "g",
  );
  manifest.notes = manifest.notes.replace(BLOB_RE, (_full, docPath) => {
    notesRewrites += 1;
    // Strip any #anchor: the mirror serves raw markdown as text/plain, where
    // a fragment has nothing to jump to. Dropping it lands the reader at the
    // top of the file — and the section they want is the newest release,
    // which is at the top of CHANGELOG.md anyway.
    //
    // The path is kept repo-relative under the version prefix (so
    // `docs/release-notes/x.md` and `CHANGELOG.md` land where their repo
    // paths say) — the workflow then uploads those same paths verbatim and
    // there is no mapping table to drift.
    const [cleanPath] = docPath.split("#");
    mirroredDocs.add(cleanPath);
    return `${mirrorBase}/${cleanPath}`;
  });
}

// ── assertions: exactly two kinds of divergence, nothing else ──────────
if (problems.length) {
  console.error("[rewrite-manifest] refusing to write a partly-rewritten manifest:");
  for (const p of problems) console.error(`  - ${p}`);
  process.exit(1);
}

const remaining = JSON.stringify(manifest).match(
  /https:\/\/github\.com\/shiwenwen\/hope-agent\/releases\/download\//g,
);
if (remaining) {
  console.error(
    `[rewrite-manifest] ${remaining.length} GitHub release-download URL(s) survived the rewrite — the mirror manifest must not send anyone back to github.com for bytes.`,
  );
  process.exit(1);
}

fs.writeFileSync(outPath, JSON.stringify(manifest, null, 2) + "\n");
if (docsOutFlag) {
  const docsOutPath = docsOutFlag.slice("--docs-out=".length);
  fs.writeFileSync(
    docsOutPath,
    [...mirroredDocs].sort().map((p) => `${p}\n`).join(""),
  );
}
console.log(
  `[rewrite-manifest] ${rewrites.length} download URL(s) + ${notesRewrites} notes link(s) repointed at ${mirrorBase}`,
);
for (const r of rewrites) console.log(`  ${r}`);
for (const d of [...mirroredDocs].sort()) console.log(`  notes doc: ${d}`);
