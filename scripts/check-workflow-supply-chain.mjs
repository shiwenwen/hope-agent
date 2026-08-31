import { readFileSync, readdirSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { parseDocument } from "yaml"

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

const presenceExpressions = {
  R2_ACCESS_KEY_PRESENT: "${{ secrets.R2_ACCESS_KEY_ID != '' }}",
  R2_SECRET_KEY_PRESENT: "${{ secrets.R2_SECRET_ACCESS_KEY != '' }}",
  R2_ACCOUNT_IS_ACCESS_KEY: "${{ secrets.R2_ACCOUNT_ID == secrets.R2_ACCESS_KEY_ID }}",
}
const credentialExpressions = {
  RCLONE_CONFIG_R2_ACCESS_KEY_ID: "${{ secrets.R2_ACCESS_KEY_ID }}",
  RCLONE_CONFIG_R2_SECRET_ACCESS_KEY: "${{ secrets.R2_SECRET_ACCESS_KEY }}",
}
const credentialReference = /secrets\s*(?:\.\s*R2_(?:ACCESS_KEY_ID|SECRET_ACCESS_KEY)\b|\[\s*['"]R2_(?:ACCESS_KEY_ID|SECRET_ACCESS_KEY)['"]\s*\])/i

// Parse the structure so flow mappings, quoted keys and scalar styles cannot
// hide uses/env fields. Aliases and merge keys are deliberately unsupported.
export function inspectWorkflow(name, source) {
  const errors = []
  let workflow
  try {
    const document = parseDocument(source, { version: "1.2", uniqueKeys: true, stringKeys: true, prettyErrors: false })
    if (document.errors.length || document.warnings.length) throw new Error("Invalid YAML")
    workflow = document.toJS({ maxAliasCount: 0 })
    if (!workflow || typeof workflow !== "object" || Array.isArray(workflow)) throw new Error("Invalid workflow")
  } catch {
    // Never echo source snippets, which may contain credentials.
    return [`${name}: invalid YAML or unsupported alias/tag`]
  }

  const steps = []
  function inspectUses(owner, location) {
    if (!owner || !Object.hasOwn(owner, "uses")) return
    const ref = owner.uses
    if (typeof ref !== "string" || (!/^\.\/[A-Za-z0-9_./-]+$/.test(ref)
        && !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_./-]+@[a-f0-9]{40}$/.test(ref))) {
      errors.push(`${name}:${location}: action must use a full commit SHA or local path`)
    }
  }
  for (const [id, job] of Object.entries(workflow.jobs ?? {})) {
    inspectUses(job, `jobs.${id}`)
    if (!Array.isArray(job?.steps)) continue
    job.steps.forEach((step, index) => {
      inspectUses(step, `jobs.${id}.steps.${index}`)
      if (step && typeof step === "object") steps.push(step)
    })
  }

  function inspectValues(value, location = []) {
    if (!value || typeof value !== "object") return
    for (const [key, child] of Object.entries(value)) {
      const next = [...location, key]
      if (key === "<<") errors.push(`${name}:${next.join(".")}: YAML merge keys are unsupported`)
      if (typeof child === "string" && credentialReference.test(child)) {
        const isStepEnv = location.length === 5 && location[0] === "jobs"
          && location[2] === "steps" && location[4] === "env"
        const step = isStepEnv ? workflow.jobs?.[location[1]]?.steps?.[location[3]] : null
        const presenceOnly = isStepEnv && Object.hasOwn(presenceExpressions, key) && child === presenceExpressions[key]
        const approved = isStepEnv && credentialSteps.get(name)?.has(step?.name)
          && Object.hasOwn(credentialExpressions, key) && child === credentialExpressions[key]
        if (!presenceOnly && !approved) {
          errors.push(`${name}:${next.join(".")}: R2 credentials must be scoped to an approved I/O step env`)
        }
      }
      inspectValues(child, next)
    }
  }
  inspectValues(workflow)

  if (name === "mirror-release-r2.yml") {
    const installer = steps.find((step) => step.name === "Install rclone (pinned) + jq")
    const run = typeof installer?.run === "string" ? installer.run : ""
    const verify = run.indexOf('printf \'%s  %s\\n\' "$R2_RCLONE_SHA256" "$tmp/rclone.zip" | sha256sum --check --status')
    const unpack = run.indexOf('unzip -q "$tmp/rclone.zip"')
    if (!/^[a-f0-9]{64}$/.test(installer?.env?.R2_RCLONE_SHA256 ?? "")
        || !/^v\d+\.\d+\.\d+$/.test(installer?.env?.R2_RCLONE_PIN ?? "")
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
