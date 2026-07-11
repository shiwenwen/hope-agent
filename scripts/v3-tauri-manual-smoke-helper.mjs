#!/usr/bin/env node
import childProcess from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"

const defaultPlansDir = path.join(
  os.homedir(),
  "Library/Mobile Documents/com~apple~CloudDocs/HopeAI/Hope Agent/Plans/hope-agent-control-plane-plans-2026-07-05/11-agent-control-plane-v3-claude-parity",
)

const args = parseArgs(process.argv.slice(2))

if (args.help) {
  printHelp()
  process.exit(0)
}

const repoRoot = process.cwd()
const plansDir = path.resolve(args.plansDir ?? process.env.HOPE_V3_PLANS_DIR ?? defaultPlansDir)
const artifactRel = args.artifact ?? "evidence/tauri-manual-gui-smoke-2026-07-08.md"
const artifactPath = path.resolve(plansDir, artifactRel)
const timestamp = new Date().toISOString()
const safeTimestamp = timestamp.replace(/[-:]/g, "").replace(/\..+/, "")
const defaultDataDir = path.join(os.tmpdir(), `hope-agent-v3-tauri-manual-smoke-${safeTimestamp}`)
const dataDir = path.resolve(args.dataDir ?? defaultDataDir)
const launcherArgs = [
  "scripts/v3-tauri-smoke-launch.mjs",
  "--json",
  "--data-dir",
  dataDir,
  "--vite-port",
  String(args.vitePort),
  "--server-port",
  String(args.serverPort),
  "--identifier",
  args.identifier ?? `ai.hopeagent.desktop.v3manualsmoke.${process.pid}`,
]
if (args.force) launcherArgs.push("--force")

const launchSummary = JSON.parse(runNode(launcherArgs))
const packageJson = readJson(path.join(repoRoot, "package.json"))
const helperLaunchCommand = buildHelperLaunchCommand({
  dataDir,
  identifier: launchSummary.identifier,
  serverPort: extractPort(launchSummary.serverHealthUrl),
  vitePort: extractPort(launchSummary.viteUrl),
})
const packet = renderPacket({
  artifactRel,
  artifactPath,
  branch: runGit(["branch", "--show-current"]).trim() || "unknown",
  commit: runGit(["rev-parse", "--short=9", "HEAD"]).trim() || "unknown",
  dataDir,
  helperLaunchCommand,
  launchSummary,
  nodeVersion: process.version,
  packageVersion: packageJson.version ?? "unknown",
  plansDir,
  repoRoot,
  reviewer: args.reviewer ?? "manual:<name>",
  timestamp,
})

if (args.append) {
  fs.mkdirSync(path.dirname(artifactPath), { recursive: true })
  fs.appendFileSync(artifactPath, `\n${packet}\n`)
}

if (args.json) {
  process.stdout.write(
    `${JSON.stringify(
      {
        artifactPath,
        dataDir,
        directTauriCommand: launchSummary.command,
        helperLaunchCommand,
        launchSummary,
        packetAppended: args.append,
        plansDir,
      },
      null,
      2,
    )}\n`,
  )
} else {
  process.stdout.write(packet)
  process.stdout.write("\n")
  if (args.append) process.stdout.write(`Appended packet to: ${artifactPath}\n`)
}

function parseArgs(argv) {
  const parsed = {
    append: false,
    artifact: null,
    dataDir: null,
    force: false,
    help: false,
    identifier: null,
    json: false,
    plansDir: null,
    reviewer: null,
    serverPort: 18421,
    vitePort: 1422,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--append") parsed.append = true
    else if (arg === "--artifact") parsed.artifact = argv[++i]
    else if (arg === "--data-dir") parsed.dataDir = argv[++i]
    else if (arg === "--force") parsed.force = true
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--identifier") parsed.identifier = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--plans-dir") parsed.plansDir = argv[++i]
    else if (arg === "--reviewer") parsed.reviewer = argv[++i]
    else if (arg === "--server-port") parsed.serverPort = parsePort(argv[++i], "--server-port")
    else if (arg === "--vite-port") parsed.vitePort = parsePort(argv[++i], "--vite-port")
    else fail(`Unknown argument: ${arg}`)
  }
  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-tauri-manual-smoke-helper.mjs [options]
  node scripts/v3-tauri-manual-smoke-helper.mjs --append --reviewer "<name>"

Purpose:
  Create a manual execution packet for the V3 Tauri desktop GUI smoke proof.
  The helper prepares an isolated launch command and records environment data,
  but it never checks coverage boxes or marks strict proof passed.

Options:
  --append                 Append the packet to the Tauri GUI smoke artifact.
  --json                   Print machine-readable metadata.
  --plans-dir <path>       Override the V3 Plans directory.
  --artifact <path>        Artifact path relative to the Plans directory.
  --reviewer <name>        Reviewer label to include in the packet.
  --data-dir <path>        Isolated smoke data dir.
  --force                  Allow reusing an existing data dir for launcher prep.
  --identifier <id>        Unique Tauri app identifier.
  --vite-port <port>       Preferred Vite dev port. Defaults to 1422.
  --server-port <port>     Preferred embedded server port. Defaults to 18421.
  --help, -h               Show this help.
`)
}

function renderPacket({
  artifactRel,
  artifactPath,
  branch,
  commit,
  dataDir,
  launchSummary,
  helperLaunchCommand,
  nodeVersion,
  packageVersion,
  plansDir,
  repoRoot,
  reviewer,
  timestamp,
}) {
  const passCommand = [
    "node scripts/v3-strict-proof-record.mjs \\",
    "  --requirement tauri_manual_gui_smoke \\",
    "  --id tauri_manual_gui_smoke_2026_07_08 \\",
    "  --status passed \\",
    `  --artifact ${artifactRel} \\`,
    `  --reviewer ${shellQuote(reviewer)} \\`,
    '  --summary "Tauri Desktop Manual GUI Smoke proof completed." \\',
    "  --confirm-reviewed",
  ].join("\n")

  return [
    `## Manual GUI Smoke Execution Packet - ${timestamp}`,
    "",
    "This packet prepares the real desktop smoke. It does not prove coverage by itself.",
    "",
    "### Environment",
    "",
    `- Hope commit: \`${commit}\``,
    `- Branch: \`${branch}\``,
    `- App build/version: \`${packageVersion}\``,
    `- Node: \`${nodeVersion}\``,
    `- Workspace/worktree: \`${repoRoot}\``,
    `- Plans dir: \`${plansDir}\``,
    `- Artifact: \`${artifactPath}\``,
    `- Reviewer: \`${reviewer}\``,
    "",
    "### Isolated Launch",
    "",
    `- Data dir: \`${dataDir}\``,
    `- Identifier: \`${launchSummary.identifier}\``,
    `- Vite URL: \`${launchSummary.viteUrl}\``,
    `- Server health URL: \`${launchSummary.serverHealthUrl}\``,
    "",
    "```bash",
    helperLaunchCommand,
    "```",
    "",
    "### Manual Checklist",
    "",
    "- [ ] Open the real Tauri desktop window and record a screenshot or manual observation.",
    "- [ ] Verify input_plus_menu.",
    "- [ ] Verify goal_plan_mutex from GUI and slash command paths.",
    "- [ ] Verify workflow_menu.",
    "- [ ] Verify loop_create_status.",
    "- [ ] Verify workspace_default_advanced.",
    "- [ ] Verify responsive_layout.",
    "- [ ] Verify key_locales.",
    "- [ ] Fill the artifact Coverage Notes and Manual Execution Worksheet.",
    "- [ ] Change Required Coverage and Reviewer Decision checkboxes only after real observations exist.",
    "",
    "### Mark Passed After Manual Review",
    "",
    "```bash",
    passCommand,
    "```",
    "",
  ].join("\n")
}

function buildHelperLaunchCommand({ dataDir, identifier, serverPort, vitePort }) {
  return [
    "node scripts/v3-tauri-smoke-launch.mjs",
    "--run",
    "--force",
    "--data-dir",
    shellQuote(dataDir),
    "--vite-port",
    String(vitePort),
    "--server-port",
    String(serverPort),
    "--identifier",
    shellQuote(identifier),
  ].join(" ")
}

function runNode(argv) {
  return childProcess.execFileSync(process.execPath, argv, {
    cwd: process.cwd(),
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  })
}

function runGit(argv) {
  try {
    return childProcess.execFileSync("git", argv, {
      cwd: process.cwd(),
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    })
  } catch {
    return ""
  }
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"))
}

function parsePort(value, flag) {
  const port = Number.parseInt(value, 10)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail(`${flag} must be a TCP port.`)
  return port
}

function extractPort(url) {
  return new URL(url).port
}

function shellQuote(value) {
  const text = String(value)
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(text)) return text
  return `'${text.replaceAll("'", "'\\''")}'`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
