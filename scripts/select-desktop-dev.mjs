import { spawnSync } from "node:child_process"
import { dirname, resolve } from "node:path"
import { createInterface } from "node:readline/promises"
import { fileURLToPath } from "node:url"

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")

const modes = [
  {
    id: "1",
    alias: "default",
    script: "dev:desktop",
    description: "Default UI / business development (no optional binaries)",
  },
  {
    id: "2",
    alias: "browser",
    script: "dev:desktop:browser",
    description: "Chrome extension integration (Browser Host)",
  },
  {
    id: "3",
    alias: "eval",
    script: "dev:desktop:eval",
    description: "Evaluation feature development (Eval Sidecar)",
  },
  {
    id: "4",
    alias: "full",
    script: "dev:desktop:full",
    description: "Full desktop capability check (both optional binaries)",
  },
]

process.stdout.write("Select desktop development mode:\n")
for (const mode of modes) {
  process.stdout.write(`  ${mode.id}) ${mode.alias.padEnd(7)} ${mode.description}\n`)
}

const prompt = createInterface({ input: process.stdin, output: process.stdout })
prompt.on("SIGINT", () => {
  prompt.close()
  process.exit(130)
})

let selected
try {
  while (!selected) {
    const answer = (await prompt.question("Choose a mode [1]: ")).trim().toLowerCase() || "1"
    selected = modes.find((mode) => mode.id === answer || mode.alias === answer)
    if (!selected) {
      process.stderr.write("Invalid mode. Enter 1-4 or default/browser/eval/full.\n")
    }
  }
} finally {
  prompt.close()
}

process.stdout.write(`\nStarting pnpm ${selected.script}...\n\n`)
const pnpm = process.platform === "win32" ? "pnpm.cmd" : "pnpm"
const forwardedArgs = process.argv.slice(2)
const run = spawnSync(
  pnpm,
  ["run", selected.script, ...forwardedArgs],
  {
    cwd: repoRoot,
    stdio: "inherit",
    env: process.env,
  },
)

if (run.error) {
  process.stderr.write(`Failed to start pnpm ${selected.script}: ${run.error.message}\n`)
  process.exit(1)
}
process.exit(run.status ?? 1)
