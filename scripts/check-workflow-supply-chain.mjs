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

  return errors
}

// The release paths share one reviewed installer instead of independently
// drifting version/digest pins. This is a bounded structural guard, not a
// general Shell analyzer.
export function inspectRcloneWorkflow(name, source) {
  if (!credentialSteps.has(name)) return []
  try {
    const workflow = parseDocument(source).toJS({ maxAliasCount: 0 })
    const steps = Object.values(workflow.jobs ?? {}).flatMap((job) => job.steps ?? [])
    const installers = steps.filter((step) => typeof step.run === "string"
      && step.run.includes('bash scripts/install-rclone.sh "$RUNNER_TEMP/hope-rclone-bin"'))
    const firstIo = steps.findIndex((step) => credentialSteps.get(name).has(step.name))
    if (installers.length !== 1 || (firstIo >= 0 && steps.indexOf(installers[0]) >= firstIo)
        || !installers[0].run.includes('echo "$RUNNER_TEMP/hope-rclone-bin" >> "$GITHUB_PATH"')
        || steps.some((step) => /apt-get install[^\n]*\brclone\b/.test(step.run ?? ""))) {
      return [`${name}: shared pinned rclone installer and PATH publication required; no distro rclone`]
    }
    return []
  } catch {
    return [`${name}: invalid rclone workflow structure`]
  }
}

export function inspectRcloneInstaller(source) {
  const size = source.indexOf('"$R2_RCLONE_BYTES"')
  const verify = source.indexOf('printf \'%s  %s\\n\' "$R2_RCLONE_SHA256" "$tmp/rclone.zip" | sha256sum -c - >/dev/null')
  const unpack = source.indexOf('unzip -q "$tmp/rclone.zip"')
  const execute = source.indexOf('version_output="$("$binary" version)"')
  const flags = source.indexOf('flags_help="$("$binary" help flags)"')
  const publish = source.indexOf('mv -f "$candidate" "$destination/rclone"')
  if (!/^R2_RCLONE_PIN=v\d+\.\d+\.\d+$/m.test(source)
      || !/^R2_RCLONE_SHA256=[a-f0-9]{64}$/m.test(source)
      || !/^R2_RCLONE_BYTES=[1-9]\d*$/m.test(source)
      || size < 0 || verify <= size || unpack <= verify || execute <= unpack || flags <= execute || publish <= flags) {
    return ["rclone SHA-256, size and executable checks must precede publication"]
  }
  return []
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const directory = path.join(repoRoot, ".github/workflows")
  const errors = readdirSync(directory).filter((name) => /\.ya?ml$/.test(name))
    .flatMap((name) => {
      const source = readFileSync(path.join(directory, name), "utf8")
      return [...inspectWorkflow(name, source), ...inspectRcloneWorkflow(name, source)]
    })
  errors.push(...inspectRcloneInstaller(readFileSync(path.join(repoRoot, "scripts/install-rclone.sh"), "utf8")))
  if (errors.length) {
    console.error(errors.join("\n"))
    process.exitCode = 1
  } else {
    console.log("Workflow supply-chain guards passed")
  }
}
