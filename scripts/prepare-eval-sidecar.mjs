import { copyFileSync, existsSync, mkdirSync, statSync, chmodSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

const DEV = process.argv.includes("--dev")
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const targetTriple = DEV
  ? hostTriple()
  : process.env.HA_EVAL_SIDECAR_TARGET ||
    process.env.TAURI_ENV_TARGET_TRIPLE ||
    process.env.CARGO_BUILD_TARGET ||
    inferTargetTriple(process.env.TAURI_PLATFORM, process.env.TAURI_ARCH) ||
    hostTriple()

if (!targetTriple) {
  console.error("[prepare-eval-sidecar] unable to determine Rust target triple")
  process.exit(1)
}

const executableName = targetTriple.includes("windows")
  ? "hope-agent-eval.exe"
  : "hope-agent-eval"
const cargoArgs = ["build", "-p", "ha-eval", "--locked"]
if (!DEV) cargoArgs.push("--profile", "eval-sidecar")
if (!DEV && targetTriple) cargoArgs.push("--target", targetTriple)

const build = spawnSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
  env: process.env,
})
if (build.status !== 0) process.exit(build.status ?? 1)

const targetDirectory = cargoMetadataTargetDir()
const profile = DEV ? "debug" : "eval-sidecar"
const source = DEV
  ? join(targetDirectory, profile, executableName)
  : join(targetDirectory, targetTriple, profile, executableName)
if (!existsSync(source) || !statSync(source).isFile()) {
  console.error(`[prepare-eval-sidecar] missing built Sidecar: ${source}`)
  process.exit(1)
}

const binariesDirectory = join(repoRoot, "src-tauri", "binaries")
mkdirSync(binariesDirectory, { recursive: true })
const suffix = targetTriple.includes("windows") ? ".exe" : ""
const destination = join(
  binariesDirectory,
  `hope-agent-eval-${targetTriple}${suffix}`,
)
copyFileSync(source, destination)
if (!targetTriple.includes("windows")) chmodSync(destination, 0o755)
console.log(`[prepare-eval-sidecar] copied ${source} -> ${destination}`)

function hostTriple() {
  const output = spawnSync("rustc", ["-vV"], { encoding: "utf8" })
  if (output.status !== 0) return ""
  return output.stdout.match(/^host:\s*(.+)$/m)?.[1]?.trim() || ""
}

function inferTargetTriple(platform, arch) {
  if (!platform || !arch) return ""
  const normalizedArch = arch === "arm64" ? "aarch64" : arch
  if (platform === "darwin") {
    if (normalizedArch === "x86_64") return "x86_64-apple-darwin"
    if (normalizedArch === "aarch64") return "aarch64-apple-darwin"
  }
  if (platform === "linux") {
    if (normalizedArch === "x86_64") return "x86_64-unknown-linux-gnu"
    if (normalizedArch === "aarch64") return "aarch64-unknown-linux-gnu"
  }
  if (platform === "windows") {
    if (normalizedArch === "x86_64") return "x86_64-pc-windows-msvc"
    if (normalizedArch === "aarch64") return "aarch64-pc-windows-msvc"
  }
  return ""
}

function cargoMetadataTargetDir() {
  const metadata = spawnSync(
    "cargo",
    ["metadata", "--format-version=1", "--no-deps"],
    { cwd: repoRoot, encoding: "utf8", env: process.env },
  )
  if (metadata.status !== 0) {
    process.stderr.write(metadata.stderr || "")
    process.exit(metadata.status ?? 1)
  }
  const parsed = JSON.parse(metadata.stdout)
  if (typeof parsed.target_directory !== "string" || !parsed.target_directory) {
    console.error("[prepare-eval-sidecar] cargo metadata has no target_directory")
    process.exit(1)
  }
  return parsed.target_directory
}
