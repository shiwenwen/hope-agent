import { copyFileSync, existsSync, mkdirSync, statSync, chmodSync } from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const targetTriple =
  process.env.HA_BROWSER_HOST_TARGET ||
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  process.env.CARGO_BUILD_TARGET ||
  inferTargetTriple(process.env.TAURI_PLATFORM, process.env.TAURI_ARCH) ||
  ""
const hostName =
  process.platform === "win32" || targetTriple.includes("windows")
    ? "ha-browser-host.exe"
    : "ha-browser-host"

const cargoArgs = ["build", "-p", "ha-browser-host", "--release", "--locked"]
if (targetTriple) {
  cargoArgs.push("--target", targetTriple)
}

const build = spawnSync("cargo", cargoArgs, {
  cwd: repoRoot,
  stdio: "inherit",
  env: process.env,
})

if (build.status !== 0) {
  process.exit(build.status ?? 1)
}

const cargoTargetDir = cargoMetadataTargetDir()
const targetDir = targetTriple
  ? join(cargoTargetDir, targetTriple, "release")
  : join(cargoTargetDir, "release")
const source = join(targetDir, hostName)
if (!existsSync(source) || !statSync(source).isFile()) {
  console.error(`[prepare-browser-host] missing built host binary: ${source}`)
  process.exit(1)
}

const outDir = join(repoRoot, "src-tauri", "resources", "browser-host")
mkdirSync(outDir, { recursive: true })
const dest = join(outDir, basename(hostName))
copyFileSync(source, dest)
if (process.platform !== "win32") {
  chmodSync(dest, 0o755)
}
console.log(`[prepare-browser-host] copied ${source} -> ${dest}`)

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
  const metadata = spawnSync("cargo", ["metadata", "--format-version=1", "--no-deps"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: process.env,
  })
  if (metadata.status !== 0) {
    process.stderr.write(metadata.stderr || "")
    console.error("[prepare-browser-host] cargo metadata failed")
    process.exit(metadata.status ?? 1)
  }
  try {
    const parsed = JSON.parse(metadata.stdout)
    if (typeof parsed.target_directory === "string" && parsed.target_directory) {
      return parsed.target_directory
    }
  } catch (error) {
    console.error(`[prepare-browser-host] parsing cargo metadata failed: ${error}`)
    process.exit(1)
  }
  console.error("[prepare-browser-host] cargo metadata did not include target_directory")
  process.exit(1)
}
