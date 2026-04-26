import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import { spawnSync } from "node:child_process"
import test from "node:test"
import assert from "node:assert/strict"
import { fileURLToPath } from "node:url"

const testDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(testDir, "../..")
const syncVersionScript = path.join(repoRoot, "scripts", "sync-version.mjs")
const prePushHook = path.join(repoRoot, ".husky", "pre-push")

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    ...options,
  })

  return {
    ...result,
    stdout: result.stdout ?? "",
    stderr: result.stderr ?? "",
  }
}

function writeExecutable(filePath, contents) {
  writeFileSync(filePath, contents)
  chmodSync(filePath, 0o755)
}

test("sync-version stages synced files without package-lock.json", () => {
  const tempRoot = mkdtempSync(path.join(os.tmpdir(), "hope-agent-sync-version-"))

  try {
    mkdirSync(path.join(tempRoot, "src-tauri"), { recursive: true })

    writeFileSync(
      path.join(tempRoot, "package.json"),
      JSON.stringify(
        {
          name: "hope-agent-test",
          version: "0.1.0",
        },
        null,
        2,
      ),
    )
    writeFileSync(
      path.join(tempRoot, "src-tauri", "Cargo.toml"),
      '[package]\nname = "hope-agent-test"\nversion = "0.1.0"\n',
    )
    writeFileSync(
      path.join(tempRoot, "src-tauri", "tauri.conf.json"),
      `${JSON.stringify({ version: "0.1.0" }, null, 2)}\n`,
    )

    assert.equal(run("git", ["init", "-q"], { cwd: tempRoot }).status, 0)
    assert.equal(run("git", ["config", "user.email", "test@example.com"], { cwd: tempRoot }).status, 0)
    assert.equal(run("git", ["config", "user.name", "Test User"], { cwd: tempRoot }).status, 0)
    assert.equal(run("git", ["add", "."], { cwd: tempRoot }).status, 0)
    assert.equal(run("git", ["commit", "-qm", "init"], { cwd: tempRoot }).status, 0)

    writeFileSync(
      path.join(tempRoot, "package.json"),
      JSON.stringify(
        {
          name: "hope-agent-test",
          version: "0.2.0",
        },
        null,
        2,
      ),
    )

    const result = run("node", [syncVersionScript], {
      cwd: tempRoot,
      env: {
        ...process.env,
        npm_lifecycle_event: "version",
      },
    })

    assert.equal(result.status, 0, result.stderr || result.stdout)

    const staged = run("git", ["diff", "--cached", "--name-only"], { cwd: tempRoot })
    assert.equal(staged.status, 0, staged.stderr)
    assert.deepEqual(staged.stdout.trim().split("\n").filter(Boolean).sort(), [
      "package.json",
      "src-tauri/Cargo.toml",
      "src-tauri/tauri.conf.json",
    ])

    const cargoToml = readFileSync(path.join(tempRoot, "src-tauri", "Cargo.toml"), "utf8")
    const tauriConfig = JSON.parse(
      readFileSync(path.join(tempRoot, "src-tauri", "tauri.conf.json"), "utf8"),
    )

    assert.match(cargoToml, /^version = "0.2.0"$/m)
    assert.equal(tauriConfig.version, "0.2.0")
  } finally {
    rmSync(tempRoot, { recursive: true, force: true })
  }
})

test(
  "pre-push lint step does not pass --silent through to eslint",
  { skip: process.platform === "win32" ? "shell hook regression test is Unix-only" : false },
  () => {
    const tempRoot = mkdtempSync(path.join(os.tmpdir(), "hope-agent-pre-push-"))

    try {
      const binDir = path.join(tempRoot, "bin")
      const logPath = path.join(tempRoot, "calls.log")
      mkdirSync(binDir, { recursive: true })

      writeExecutable(
        path.join(binDir, "cargo"),
        `#!/usr/bin/env sh
echo "cargo:$*" >> "$HOOK_LOG"
exit 0
`,
      )
      writeExecutable(
        path.join(binDir, "pnpm"),
        `#!/usr/bin/env sh
echo "pnpm:$*" >> "$HOOK_LOG"
if [ "$1" = "lint" ] && [ "$2" = "--silent" ]; then
  echo "lint received invalid --silent forwarding" >&2
  exit 86
fi
exit 0
`,
      )

      const result = run("sh", [prePushHook], {
        cwd: tempRoot,
        env: {
          ...process.env,
          PATH: `${binDir}:${process.env.PATH}`,
          HOOK_LOG: logPath,
        },
      })

      assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)

      const log = readFileSync(logPath, "utf8")
      assert.match(log, /pnpm:typecheck/)
      assert.doesNotMatch(log, /pnpm:lint --silent/)
    } finally {
      rmSync(tempRoot, { recursive: true, force: true })
    }
  },
)
