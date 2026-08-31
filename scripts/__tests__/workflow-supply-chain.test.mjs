import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { inspectWorkflow } from "../check-workflow-supply-chain.mjs"

test("only local paths and complete action commits are accepted", () => {
  for (const ref of ["actions/checkout@v6", "actions/checkout@main", "actions/checkout@1234567", "${{ inputs.action }}"]) {
    assert.equal(inspectWorkflow("test.yml", `      - uses: ${ref}`).length, 1)
  }
  for (const ref of ["./.github/actions/local", `actions/checkout@${"a".repeat(40)} # v6`, `"actions/checkout@${"a".repeat(40)}"`]) {
    assert.deepEqual(inspectWorkflow("test.yml", `      - uses: ${ref}`), [])
  }
})

test("R2 credentials cannot leak to global env or setup steps", () => {
  const secret = "RCLONE_CONFIG_R2_SECRET_ACCESS_KEY: ${{ secrets.R2_SECRET_ACCESS_KEY }}"
  for (const source of [`env:\n  ${secret}`, `      - name: Install dependencies\n        env:\n          ${secret}`]) {
    assert.equal(inspectWorkflow("update-linux-repo.yml", source).length, 1)
  }
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", `      - name: Publish to R2\n        env:\n          ${secret}`), [])
})

test("rclone must have a pinned digest and verify it before extraction", () => {
  const source = readFileSync(new URL("../../.github/workflows/mirror-release-r2.yml", import.meta.url), "utf8")
  assert.deepEqual(inspectWorkflow("mirror-release-r2.yml", source), [])
  for (const broken of [source.replace(/R2_RCLONE_SHA256: [a-f0-9]+/, "R2_RCLONE_SHA256: latest"), source.replace("sha256sum --check --status", "true")]) {
    assert.ok(inspectWorkflow("mirror-release-r2.yml", broken).some((error) => error.includes("rclone SHA-256")))
  }
})

test("setup receives booleans, never the underlying upload keys", () => {
  const expression = "          R2_ACCESS_KEY_PRESENT: ${{ secrets.R2_ACCESS_KEY_ID != '' }}"
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", expression), [])
  assert.equal(inspectWorkflow("update-linux-repo.yml", expression.replace(" != ''", "")).length, 1)
  const workflow = readFileSync(new URL("../../.github/workflows/update-linux-repo.yml", import.meta.url), "utf8")
  assert.deepEqual(inspectWorkflow("update-linux-repo.yml", workflow), [])
  const upload = workflow.slice(workflow.indexOf("      - name: Publish to R2"))
  assert.ok(!upload.includes("npm install"))
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
