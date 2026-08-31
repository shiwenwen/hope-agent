import { readFileSync, readdirSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const credentialSteps = new Map([
  ["mirror-release-r2.yml", new Set([
    "Upload assets (immutable)",
    "Upload latest/ aliases (mutable, short TTL)",
    "Publish download/latest.json (short TTL)",
  ])],
  ["update-linux-repo.yml", new Set([
    "Pull existing repo state from R2", "Publish to R2",
  ])],
])

// These workflows intentionally use block-style YAML. Refuse opaque uses:
// expressions/anchors rather than trying to resolve them into trusted code.
export function inspectWorkflow(name, source) {
  const errors = []
  let step = null
  for (const [index, line] of source.split("\n").entries()) {
    if (/^\s*(#|$)/.test(line)) continue
    if (/^\S|^ {1,4}\S/.test(line)) step = null
    const stepMatch = line.match(/^ {6}- name: (.+)$/)
    if (stepMatch) step = stepMatch[1].trim()
    else if (/^ {6}- /.test(line)) step = null
    const uses = line.match(/^\s*(?:-\s*)?uses:\s*(.*?)\s*(?:#.*)?$/)
    if (uses) {
      const ref = uses[1].replace(/^(["'])(.*)\1$/, "$2")
      if (!/^\.\/[A-Za-z0-9_./-]+$/.test(ref)
          && !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_./-]+@[a-f0-9]{40}$/.test(ref)) {
        errors.push(`${name}:${index + 1}: action must use a full commit SHA or local path`)
      }
    }
    const presenceOnly = /^ {10}R2_(ACCESS_KEY_PRESENT|SECRET_KEY_PRESENT): \$\{\{ secrets\.R2_(ACCESS_KEY_ID|SECRET_ACCESS_KEY) != '' \}\}$/.test(line)
      || /^ {10}R2_ACCOUNT_IS_ACCESS_KEY: \$\{\{ secrets\.R2_ACCOUNT_ID == secrets\.R2_ACCESS_KEY_ID \}\}$/.test(line)
    if (!presenceOnly && /secrets\.R2_(ACCESS_KEY_ID|SECRET_ACCESS_KEY)/.test(line)
        && (!credentialSteps.get(name)?.has(step)
          || !/^ {10}RCLONE_CONFIG_R2_(ACCESS_KEY_ID|SECRET_ACCESS_KEY):/.test(line))) {
      errors.push(`${name}:${index + 1}: R2 credentials must be scoped to an approved I/O step env`)
    }
  }
  if (name === "mirror-release-r2.yml") {
    const start = source.indexOf("      - name: Install rclone (pinned) + jq")
    const end = source.indexOf("\n      - ", start + 1)
    const installer = source.slice(start, end < 0 ? undefined : end)
    const verify = installer.indexOf('printf \'%s  %s\\n\' "$R2_RCLONE_SHA256" "$tmp/rclone.zip" | sha256sum --check --status')
    const unpack = installer.indexOf('unzip -q "$tmp/rclone.zip"')
    if (start < 0 || !/R2_RCLONE_SHA256: [a-f0-9]{64}\s/.test(installer)
        || !/R2_RCLONE_PIN: v\d+\.\d+\.\d+\s/.test(installer)
        || verify < 0 || unpack < 0 || verify > unpack) {
      errors.push(`${name}: pinned rclone SHA-256 must be verified before unpacking/execution`)
    }
  }
  return errors
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const directory = path.join(repoRoot, ".github/workflows")
  const errors = readdirSync(directory).filter((name) => /\.ya?ml$/.test(name))
    .flatMap((name) => inspectWorkflow(name, readFileSync(path.join(directory, name), "utf8")))
  if (errors.length) {
    console.error(errors.join("\n"))
    process.exitCode = 1
  } else {
    console.log("Workflow supply-chain guards passed")
  }
}
