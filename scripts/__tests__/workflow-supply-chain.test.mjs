import assert from "node:assert/strict"
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { inspectWorkflow, inspectRcloneWorkflow, inspectRcloneInstaller } from "../check-workflow-supply-chain.mjs"

const workflowWithSteps = (steps) => `jobs:\n  test:\n    steps:\n${steps}\n`

test("only local paths and complete action commits are accepted", () => {
  for (const ref of ["actions/checkout@v6", "actions/checkout@main", "actions/checkout@1234567", "${{ inputs.action }}"]) {
    assert.equal(inspectWorkflow("test.yml", workflowWithSteps(`      - uses: ${ref}`)).length, 1)
  }
  for (const ref of ["./.github/actions/local", `actions/checkout@${"a".repeat(40)} # v6`, `"actions/checkout@${"a".repeat(40)}"`]) {
    assert.deepEqual(inspectWorkflow("test.yml", workflowWithSteps(`      - uses: ${ref}`)), [])
  }
})

test("flow mappings, quoted keys and multiline refs cannot hide unpinned actions", () => {
  for (const source of [
    workflowWithSteps("      - { uses: actions/checkout@v6 }"),
    workflowWithSteps("      - 'uses': actions/checkout@main"),
    workflowWithSteps("      - uses: >-\n          actions/checkout@v6"),
    'jobs: { test: { steps: [{ name: Checkout, uses: actions/checkout@v6 }] } }',
    'jobs: { reusable: { uses: owner/repo/.github/workflows/build.yml@main } }',
  ]) {
    assert.ok(inspectWorkflow("test.yml", source).some((error) => error.includes("full commit SHA")))
    assert.deepEqual(inspectWorkflow("test.yml", source.replace(/@(v6|main)/g, `@${"a".repeat(40)}`)), [])
  }
})

test("ambiguous or unsupported YAML fails closed without source excerpts", () => {
  for (const source of [
    workflowWithSteps("      - uses: actions/checkout@v6\n        uses: ./safe"),
    "shared: &step { uses: actions/checkout@v6 }\njobs: { test: { steps: [*step] } }",
    workflowWithSteps("      - <<: { uses: actions/checkout@v6 }"),
    workflowWithSteps("      - uses: !unknown actions/checkout@v6"),
    "jobs: [",
  ]) {
    const errors = inspectWorkflow("test.yml", source)
    assert.ok(errors.length > 0)
    assert.ok(errors.every((error) => !error.includes("actions/checkout")))
  }
})

test("R2 credentials cannot leak to global env or setup steps", () => {
  const secret = "RCLONE_CONFIG_R2_SECRET_ACCESS_KEY: ${{ secrets.R2_SECRET_ACCESS_KEY }}"
  for (const source of [`env:\n  ${secret}`, workflowWithSteps(`      - name: Install dependencies\n        env:\n          ${secret}`)]) {
    assert.equal(inspectWorkflow("update-linux-repo.yml", source).length, 1)
  }
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", workflowWithSteps(`      - name: Publish to R2\n        env:\n          ${secret}`)), [])
})

test("flow env and alternate secret syntax retain the credential boundary", () => {
  const source = JSON.stringify({ jobs: { test: { steps: [{ name: "Publish to R2", env: {
    RCLONE_CONFIG_R2_SECRET_ACCESS_KEY: "${{ secrets.R2_SECRET_ACCESS_KEY }}",
  } }] } } })
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", source), [])
  for (const broken of [
    source.replace("Publish to R2", "Install dependencies"),
    source.replace("RCLONE_CONFIG_R2_SECRET_ACCESS_KEY", "UNRELATED"),
    source.replace("secrets.R2_SECRET_ACCESS_KEY", "secrets['R2_SECRET_ACCESS_KEY']"),
  ]) assert.equal(inspectWorkflow("update-linux-repo.yml", broken).length, 1)
})

test("both release paths require the shared pinned installer", () => {
  for (const name of ["mirror-release-r2.yml", "update-linux-repo.yml"]) {
    const source = readFileSync(new URL(`../../.github/workflows/${name}`, import.meta.url), "utf8")
    assert.deepEqual(inspectRcloneWorkflow(name, source), [])
    for (const broken of [source.replace("bash scripts/install-rclone.sh", "true"),
      source.replace('>> "$GITHUB_PATH"', "> /dev/null"),
      source.replace("sudo apt-get install -y -qq", "sudo apt-get install -y -qq rclone")]) {
      assert.ok(inspectRcloneWorkflow(name, broken).length > 0)
    }
  }
})

test("rclone pins and verification order cannot silently disappear", () => {
  const source = readFileSync(new URL("../install-rclone.sh", import.meta.url), "utf8")
  assert.deepEqual(inspectRcloneInstaller(source), [])
  for (const broken of [source.replace(/R2_RCLONE_SHA256=[a-f0-9]+/, "R2_RCLONE_SHA256=latest"),
    source.replace(/R2_RCLONE_BYTES=\d+/, "R2_RCLONE_BYTES=0"),
    source.replace("sha256sum -c - >/dev/null", "true"),
    source.replace('flags_help="$("$binary" help flags)"', "flags_help=ignored")]) {
    assert.ok(inspectRcloneInstaller(broken).length > 0)
  }
})

test("rclone installer rejects corruption and failed smoke before replacing a binary", () => {
  const source = readFileSync(new URL("../install-rclone.sh", import.meta.url), "utf8")
  for (const scenario of ["valid", "truncated", "bad-digest", "wrong-version", "missing-flags", "failed-help"]) {
    const root = mkdtempSync(path.join(os.tmpdir(), "hope-rclone-fixture-"))
    try {
      const executable = `#!/bin/sh
echo called >> "$HA_FIXTURE_EXECUTED"
case "$1" in
  version) echo 'rclone ${scenario === "wrong-version" ? "v0.0.0" : "v1.75.2"}' ;;
  help) ${scenario === "failed-help" ? "exit 1" : `echo '${scenario === "missing-flags" ? "--metadata-set --checksum --s3-no-check-bucket" : "--metadata --metadata-set --checksum --s3-no-check-bucket"}'`} ;;
esac
`
      const archive = path.join(root, "fixture.zip")
      const zip = spawnSync("python3", ["-c", "import sys,zipfile; z=zipfile.ZipFile(sys.argv[1],'w'); z.writestr('rclone-v1.75.2-linux-amd64/rclone',sys.argv[2]); z.close()", archive, executable], { encoding: "utf8" })
      assert.equal(zip.status, 0, zip.stderr)
      const bytes = readFileSync(archive)
      const digest = createHash("sha256").update(bytes).digest("hex")
      if (scenario === "truncated") writeFileSync(archive, bytes.subarray(0, -1))
      if (scenario === "bad-digest") {
        bytes[40] ^= 1
        writeFileSync(archive, bytes)
      }
      const script = path.join(root, "install.sh")
      writeFileSync(script, source.replace(/^R2_RCLONE_SHA256=.+$/m, `R2_RCLONE_SHA256=${digest}`)
        .replace(/^R2_RCLONE_BYTES=.+$/m, `R2_RCLONE_BYTES=${bytes.length}`))
      const curl = path.join(root, "curl")
      writeFileSync(curl, `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.copyFileSync(process.env.HA_FIXTURE_ARCHIVE, args[args.indexOf('-o') + 1]);
`, { mode: 0o755 })
      // No production network, keys, data or installation directory.
      const destination = path.join(root, "bin")
      const setup = spawnSync("mkdir", [destination])
      assert.equal(setup.status, 0)
      writeFileSync(path.join(destination, "rclone"), "previous executable")
      const marker = path.join(root, "executed")
      const result = spawnSync("bash", [script, destination], {
        env: { PATH: `${root}:${process.env.PATH}`, HA_FIXTURE_ARCHIVE: archive, HA_FIXTURE_EXECUTED: marker },
        encoding: "utf8",
      })
      if (scenario === "valid") {
        assert.equal(result.status, 0, result.stderr)
        assert.equal(readFileSync(path.join(destination, "rclone"), "utf8"), executable)
      } else {
        assert.notEqual(result.status, 0, scenario)
        assert.equal(readFileSync(path.join(destination, "rclone"), "utf8"), "previous executable", scenario)
      }
      if (["truncated", "bad-digest"].includes(scenario)) {
        assert.ok(!readdirSync(root).includes("executed"), "unverified binary executed")
      }
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  }
})

test("setup receives booleans, never the underlying upload keys", () => {
  const expression = workflowWithSteps("      - name: Preflight\n        env:\n          R2_ACCESS_KEY_PRESENT: ${{ secrets.R2_ACCESS_KEY_ID != '' }}")
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", expression), [])
  assert.equal(inspectWorkflow("update-linux-repo.yml", expression.replace(" != ''", "")).length, 1)
  const workflow = readFileSync(new URL("../../.github/workflows/update-linux-repo.yml", import.meta.url), "utf8")
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", workflow), [])
  const upload = workflow.slice(workflow.indexOf("      - name: Publish to R2"))
  assert.ok(!upload.includes("npm install"))
})

test("all repository workflows pass the structural guard", () => {
  const directory = new URL("../../.github/workflows/", import.meta.url)
  for (const name of readdirSync(directory).filter((file) => /\.ya?ml$/.test(file))) {
    assert.deepEqual(inspectWorkflow(name, readFileSync(new URL(name, directory), "utf8")), [], name)
  }
})

test("endpoint resolver diagnoses the boolean key/account mixup without receiving a key", () => {
  const script = new URL("../r2-endpoint.sh", import.meta.url)
  for (const [same, status] of [["true", 1], ["false", 0]]) {
    const result = spawnSync("bash", [fileURLToPath(script)], {
      env: { PATH: process.env.PATH, R2_ACCOUNT_ID: "a".repeat(32), R2_ACCOUNT_IS_ACCESS_KEY: same },
      encoding: "utf8",
    })
    assert.equal(result.status, status, result.stderr)
    if (same === "true") assert.ok(result.stdout.includes("equals R2_ACCESS_KEY_ID"))
  }
})

function runManifestRewrite({ platformUrl, assetMap }) {
  const tempRoot = mkdtempSync(path.join(os.tmpdir(), "hope-agent-mirror-manifest-"))
  const manifestPath = path.join(tempRoot, "latest.json")
  const outputPath = path.join(tempRoot, "rewritten.json")
  const assetMapPath = path.join(tempRoot, "asset-map.json")
  writeFileSync(
    manifestPath,
    JSON.stringify({
      version: "0.40.0",
      notes: "[English](https://github.com/shiwenwen/hope-agent/blob/v0.40.0/docs/release-notes/v0.40.0.en.md)",
      platforms: { "windows-x86_64": { url: platformUrl, signature: "signed" } },
      bare_binary: {
        platforms: {
          "linux-x86_64": {
            url: "https://github.com/shiwenwen/hope-agent/releases/download/v0.40.0/hope-agent-0.40.0-linux-x86_64.tar.gz",
            signature: "signed",
          },
        },
      },
    }),
  )
  writeFileSync(assetMapPath, JSON.stringify(assetMap))
  const script = fileURLToPath(new URL("../rewrite-manifest-for-mirror.mjs", import.meta.url))
  const result = spawnSync(process.execPath, [script, manifestPath, "v0.40.0", "https://repo.hopeagent.ai", `--asset-map=${assetMapPath}`, `--out=${outputPath}`], { encoding: "utf8" })
  return { tempRoot, outputPath, result }
}

test("mirror manifest resolves tauri-action v1 asset API URLs through the release map", () => {
  const apiUrl = "https://api.github.com/repos/shiwenwen/hope-agent/releases/assets/540118898"
  const run = runManifestRewrite({
    platformUrl: apiUrl,
    assetMap: { [apiUrl]: "Hope.Agent_0.40.0_x64-setup.exe" },
  })
  try {
    assert.equal(run.result.status, 0, run.result.stderr)
    const rewritten = JSON.parse(readFileSync(run.outputPath, "utf8"))
    assert.equal(rewritten.platforms["windows-x86_64"].url, "https://repo.hopeagent.ai/download/v0.40.0/Hope.Agent_0.40.0_x64-setup.exe")
    assert.equal(rewritten.bare_binary.platforms["linux-x86_64"].url, "https://repo.hopeagent.ai/download/v0.40.0/hope-agent-0.40.0-linux-x86_64.tar.gz")
    assert.match(rewritten.notes, /repo\.hopeagent\.ai\/download\/v0\.40\.0\/docs\/release-notes/)
  } finally {
    rmSync(run.tempRoot, { recursive: true, force: true })
  }
})

test("mirror manifest rejects unmapped asset API URLs", () => {
  const run = runManifestRewrite({
    platformUrl: "https://api.github.com/repos/shiwenwen/hope-agent/releases/assets/540118898",
    assetMap: {},
  })
  try {
    assert.equal(run.result.status, 1)
    assert.match(run.result.stderr, /absent from the release asset map/)
  } finally {
    rmSync(run.tempRoot, { recursive: true, force: true })
  }
})

test("mirror manifest rejects unsafe filenames in the asset map", () => {
  const apiUrl = "https://api.github.com/repos/shiwenwen/hope-agent/releases/assets/540118898"
  const run = runManifestRewrite({ platformUrl: apiUrl, assetMap: { [apiUrl]: "../latest.json" } })
  try {
    assert.equal(run.result.status, 1)
    assert.match(run.result.stderr, /unsafe asset-map entry/)
  } finally {
    rmSync(run.tempRoot, { recursive: true, force: true })
  }
})
