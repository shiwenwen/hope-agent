#!/usr/bin/env node
import childProcess from "node:child_process"
import fs from "node:fs"
import net from "node:net"
import os from "node:os"
import path from "node:path"

const args = parseArgs(process.argv.slice(2))

if (args.help) {
  printHelp()
  process.exit(0)
}

const timestamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "")
const dataDir = path.resolve(args.dataDir ?? path.join(os.tmpdir(), `hope-agent-v3-tauri-smoke-${timestamp}`))
const vitePort = await choosePort(args.vitePort)
const serverPort = await choosePort(args.serverPort)
const identifier = args.identifier ?? `ai.hopeagent.desktop.v3smoke.p${process.pid}`
const devUrl = `http://localhost:${vitePort}`
const serverBindAddr = `127.0.0.1:${serverPort}`
const beforeDevCommand = [
  "pnpm dev:browser-host",
  `pnpm dev --host 127.0.0.1 --port ${vitePort} --strictPort true`,
].join(" && ")
const tauriConfig = {
  identifier,
  build: {
    devUrl,
    beforeDevCommand,
  },
}

prepareDataDir(dataDir, serverBindAddr, args.force)

const command = [
  "pnpm",
  "exec",
  "tauri",
  "dev",
  "--config",
  JSON.stringify(tauriConfig),
]

const summary = {
  dataDir,
  identifier,
  viteUrl: `http://127.0.0.1:${vitePort}/`,
  serverHealthUrl: `http://127.0.0.1:${serverPort}/api/health`,
  serverBindAddr,
  command: `HA_DATA_DIR=${shellQuote(dataDir)} ${command.map(shellQuote).join(" ")}`,
}

if (args.json) {
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`)
} else {
  process.stdout.write(
    [
      "V3 Tauri smoke launch prepared.",
      `Data dir: ${dataDir}`,
      `Identifier: ${identifier}`,
      `Vite URL: ${summary.viteUrl}`,
      `Server health URL: ${summary.serverHealthUrl}`,
      "",
      "Launch command:",
      summary.command,
      "",
      args.run
        ? "Starting Tauri now. Use Ctrl+C to stop the isolated smoke instance."
        : "Dry run only. Add --run to start the isolated smoke instance.",
      "",
    ].join("\n"),
  )
}

if (args.run) {
  const child = childProcess.spawn(command[0], command.slice(1), {
    cwd: process.cwd(),
    env: {
      ...process.env,
      HA_DATA_DIR: dataDir,
    },
    stdio: "inherit",
  })

  child.on("exit", (code, signal) => {
    if (signal) process.kill(process.pid, signal)
    process.exit(code ?? 0)
  })
}

function parseArgs(argv) {
  const parsed = {
    dataDir: null,
    force: false,
    help: false,
    identifier: null,
    json: false,
    run: false,
    serverPort: 18421,
    vitePort: 1422,
  }

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--data-dir") parsed.dataDir = argv[++i]
    else if (arg === "--force") parsed.force = true
    else if (arg === "--help" || arg === "-h") parsed.help = true
    else if (arg === "--identifier") parsed.identifier = argv[++i]
    else if (arg === "--json") parsed.json = true
    else if (arg === "--run") parsed.run = true
    else if (arg === "--server-port") parsed.serverPort = parsePort(argv[++i], "--server-port")
    else if (arg === "--vite-port") parsed.vitePort = parsePort(argv[++i], "--vite-port")
    else fail(`Unknown argument: ${arg}`)
  }

  return parsed
}

function printHelp() {
  process.stdout.write(`Usage:
  node scripts/v3-tauri-smoke-launch.mjs [options]
  node scripts/v3-tauri-smoke-launch.mjs --run

Purpose:
  Prepare and optionally launch an isolated Tauri desktop instance for V3 strict
  proof manual GUI smoke. The launcher avoids collisions with an existing Hope
  Agent desktop instance by using a unique Tauri identifier, a dedicated
  HA_DATA_DIR, an alternate Vite port, and an alternate embedded-server port.

Options:
  --run                   Start Tauri after preparing the isolated data dir.
  --json                  Print machine-readable launch metadata.
  --data-dir <path>       Smoke data dir. Defaults to /tmp/hope-agent-v3-tauri-smoke-<timestamp>.
  --force                 Allow reusing an existing data dir.
  --identifier <id>       Tauri app identifier. Defaults to ai.hopeagent.desktop.v3smoke.p<PID>.
  --vite-port <port>      Preferred Vite dev port. Defaults to 1422; next free port is used.
  --server-port <port>    Preferred embedded server port. Defaults to 18421; next free port is used.
  --help, -h              Show this help.
`)
}

function prepareDataDir(dir, bindAddr, force) {
  if (fs.existsSync(dir) && !force && fs.readdirSync(dir).length > 0) {
    fail(`Data dir already exists and is not empty: ${dir}\nUse --force or choose --data-dir.`)
  }
  fs.mkdirSync(dir, { recursive: true })
  const configPath = path.join(dir, "config.json")
  if (fs.existsSync(configPath) && !force) {
    fail(`config.json already exists: ${configPath}\nUse --force or choose --data-dir.`)
  }
  fs.writeFileSync(
    configPath,
    `${JSON.stringify(
      {
        providers: [],
        server: {
          bindAddr,
        },
      },
      null,
      2,
    )}\n`,
  )
}

async function choosePort(preferred) {
  let port = preferred
  while (!(await isPortFree(port))) port += 1
  return port
}

function isPortFree(port) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.once("listening", () => {
      server.close(() => resolve(true))
    })
    server.listen(port, "127.0.0.1")
  })
}

function parsePort(value, flag) {
  const port = Number.parseInt(value, 10)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail(`${flag} must be a TCP port.`)
  return port
}

function shellQuote(value) {
  if (/^[A-Za-z0-9_./:=@+-]+$/.test(value)) return value
  return `'${value.replaceAll("'", "'\\''")}'`
}

function fail(message) {
  process.stderr.write(`${message}\n`)
  process.exit(1)
}
