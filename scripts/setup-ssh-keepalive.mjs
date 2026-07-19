#!/usr/bin/env node
//
// Keep the push connection alive while `.husky/pre-push` runs its multi-minute
// gates.
//
// `git push` opens the connection to the remote and negotiates refs BEFORE it
// invokes pre-push (that negotiation is where the hook's remote-SHA arguments
// come from) — verified with GIT_TRACE:
//
//   15:10:27.383  run_command: ssh git@github.com 'git-receive-pack ...'
//   15:10:32.180  run_command: .husky/pre-push ...     <- 4.8s later
//   15:10:32.536  run_command: git pack-objects ...    <- only after the hook
//
// Our hook runs fmt + clippy + typecheck + eslint + vitest + cargo test, i.e.
// 10-15 minutes, and the connection sits idle that whole time. GitHub's SSH
// server drops it ("Connection to ssh.github.com closed by remote host"), so
// the pack write right after the gates hits a dead socket: git dies of SIGPIPE
// (exit 141) *after* printing "all checks passed", and the push silently never
// happens. Measured: a 5-minute hook survives, a 15-minute one does not.
//
// `ServerAliveInterval` makes ssh send keepalives during that idle window,
// which keeps the connection open across the whole gate run (verified: same
// 15-minute hook, exit 0 with keepalive vs 141 without).
//
// Runs from package.json `prepare`, so a plain `pnpm install` sets it up.

import { execFileSync } from "node:child_process"

const SSH_COMMAND = "ssh -o ServerAliveInterval=30 -o ServerAliveCountMax=10"

function git(args, { allowFailure = false } = {}) {
  try {
    return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim()
  } catch (err) {
    if (allowFailure) return null
    throw err
  }
}

// Not a git checkout (tarball install, Docker build context) — nothing to do.
if (git(["rev-parse", "--is-inside-work-tree"], { allowFailure: true }) !== "true") {
  process.exit(0)
}

// Never clobber an existing setting: contributors using 1Password / a custom
// identity / a proxy command have their own `core.sshCommand`, and silently
// replacing it would break their auth.
//
// Query the EFFECTIVE value (no `--local`): such a setting usually lives in the
// user's global config, and a local write would take precedence over it — so a
// local-only lookup sees nothing, writes, and silently strips their working
// auth setup. We can't safely append our flags to an arbitrary command either
// (it may be a wrapper script that rejects `-o`), so we leave it untouched and
// point at the equivalent ~/.ssh/config knob instead.
const existing = git(["config", "--get", "core.sshCommand"], { allowFailure: true })
if (existing) {
  if (!existing.includes("ServerAliveInterval")) {
    console.log(
      `[setup-ssh-keepalive] core.sshCommand already set (${existing}) — leaving it alone.\n` +
        `[setup-ssh-keepalive] If a long push dies with exit 141 after "all checks passed", add to ~/.ssh/config:\n` +
        `[setup-ssh-keepalive]     Host github.com\n` +
        `[setup-ssh-keepalive]       ServerAliveInterval 30`,
    )
  }
  process.exit(0)
}

try {
  git(["config", "--local", "core.sshCommand", SSH_COMMAND])
  console.log(`[setup-ssh-keepalive] core.sshCommand = ${SSH_COMMAND}`)
} catch {
  // Best-effort: a missing/readonly config must not fail `pnpm install`.
  console.warn("[setup-ssh-keepalive] could not set core.sshCommand; long pushes may fail with SIGPIPE")
}
