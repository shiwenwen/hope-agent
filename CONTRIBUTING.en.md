# Contributing Guide

> English · [简体中文](CONTRIBUTING.md)

Welcome! This guide is for **first-time and regular contributors**—covering bug reports, PRs, translation, and skill / provider / channel contributions.

If you are an **experienced maintainer or AI coding assistant** (Claude Code / Codex / Cursor), the project's cross-PR contracts live in [AGENTS.md](AGENTS.md), and per-subsystem design lives in [`docs/architecture/`](docs/architecture/). This document covers process only and does not duplicate those.

## Code of Conduct

Participation implies agreement with the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). In short: **stay professional, be respectful, focus on the issue not the person**.

## What do you want to do?

| Goal | Path |
|---|---|
| Report a bug | [New issue](https://github.com/shiwenwen/hope-agent/issues/new/choose) → Bug report template |
| Report a **security vulnerability** | **Do not file a public issue**, see [SECURITY.md](SECURITY.md) for the private channel |
| Propose a feature | Open a [discussion](https://github.com/shiwenwen/hope-agent/discussions) first, then an issue |
| Fix a bug / add a feature | fork → branch → PR (details below) |
| Help with translation | See "Translation contributions" below |
| Add a skill / provider / channel | See "Plugin-style contributions" below |
| Fix docs / typos | PR directly, no issue needed |

## PR Workflow

### 1. Fork & branch

```bash
git clone git@github.com:<your-account>/hope-agent.git
cd hope-agent
git remote add upstream git@github.com:shiwenwen/hope-agent.git

git checkout -b feat/xxx   # or fix/xxx, docs/xxx
```

### 2. Set up the environment

```bash
pnpm install              # frontend deps + Husky pre-push hooks
pnpm tauri dev            # desktop dev mode (frontend + Rust backend hot-reload)
```

Full command list in [AGENTS.md "开发命令"](AGENTS.md#开发命令).

### 3. Make changes

Key contracts (full list in AGENTS.md):

- Core business logic must live in `crates/ha-core/` (**zero Tauri dependency**); `src-tauri/` and `crates/ha-server/` are thin adapters
- Frontend: React 19 + TypeScript + Tailwind v4 + shadcn/ui
- Path alias `@/` → `src/`
- No native logging (`console.log` / `log::info!`); use [`app_info!`](crates/ha-core/src/logging.rs) family macros
- Cross-platform branches: prefer `#[cfg(unix)]` / `#[cfg(windows)]`; new primitives go in [`crates/ha-core/src/platform/`](crates/ha-core/src/platform/)

### 4. Pre-push self-check (mandatory)

The [`.husky/pre-push`](.husky/pre-push) hook runs these six checks before `git push`:

```bash
cargo fmt --all --check
cargo clippy -p ha-core -p ha-server --all-targets --locked -- -D warnings
cargo test  -p ha-core -p ha-server --locked
pnpm typecheck
pnpm lint
pnpm test
```

Any failure blocks the push. **Do not use `--no-verify` to bypass**—CI runs the same checks and will block the PR.

### 5. Commit message convention

Follow the existing repo style ([git log](https://github.com/shiwenwen/hope-agent/commits/main)):

```
<type>(<scope>): <one-line description>

<optional details, < 80 chars per line>
```

- `type`: `feat` / `fix` / `docs` / `ci` / `chore` / `refactor` / `perf` / `test`
- `scope`: subsystem name (`chat` / `provider` / `channel` / `mcp` / `skill` / `plan` / `cron` ...)
- Chinese or English both work. **Chinese preferred** (repo's primary language)

✅ `feat(provider): bump built-in model template provider/model list`
✅ `ci(release): fix Linux/Windows release build + rotate updater pubkey`
❌ `update code` / `fix bug`

**DCO**: Sign every commit with `git commit -s` ([Developer Certificate of Origin](https://developercertificate.org/) attestation requirement).

### 6. PR description

The PR template will guide you. Key fields:

- Linked issue (`closes #xxx`)
- Summary of changes
- Testing approach (manual / unit tests)
- Whether subsystem [`docs/architecture/<name>.md`](docs/architecture/) needs updating
- CHANGELOG `Unreleased` updated (see below)

### 7. CI passes + Review

- CI must be green to merge ([`lint.yml`](.github/workflows/lint.yml) + [`rust.yml`](.github/workflows/rust.yml))
- Critical paths (`.github/`, `tauri.conf.json`, `crates/ha-core/src/security/`, `docs/architecture/`) require maintainer review via [CODEOWNERS](.github/CODEOWNERS)
- Other paths: any non-maintainer can review, but merge is performed by a maintainer

### 8. Squash merge

We squash-merge to keep `main` linear—your multiple commits collapse into one. **Keep each PR focused on one thing**; split large changes into multiple PRs.

## Translation contributions

Hope Agent supports 12 languages (zh, en, ja, ko, de, fr, es, pt-BR, ru, it, tr, vi). `zh` and `en` are the source-of-truth; the other 10 are filled from these.

```bash
node scripts/sync-i18n.mjs --check   # check missing translations
node scripts/sync-i18n.mjs --apply   # fill from template
```

Translation files in [`src/i18n/locales/`](src/i18n/locales/). PR guidelines:

- One PR covers one or a few related languages
- Key paths must match `zh` exactly
- Verify by running `pnpm tauri dev` and switching the UI language

## Plugin-style contributions (Skill / Provider / Channel)

Three easy contribution surfaces with mature templates.

### Add a skill

Each directory under [`skills/`](skills/) is a skill containing `SKILL.md` + optional scripts/resources. See [`skills/skill-creator/SKILL.md`](skills/skill-creator/SKILL.md).

For vendored third-party skills: register the upstream source + full license text in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

### Add an LLM provider

New providers integrate with [`crates/ha-core/src/provider/`](crates/ha-core/src/provider/). We currently support 4 ApiType protocols (Anthropic / OpenAIChat / OpenAIResponses / Codex). If your new provider speaks one of these, just add a built-in template config—no Rust code needed. For a brand-new protocol, follow the existing trait implementations.

### Add an IM channel

[`crates/ha-core/src/channel/`](crates/ha-core/src/channel/) already contains 12 channels (Telegram, Slack, Feishu, WeCom, ...) as references. Each implements the `ChannelPlugin` trait + event callbacks. Easiest starting point: a webhook-based channel (see LINE / Discord).

## CHANGELOG maintenance

Every user-visible change (feat / fix / breaking) must add a line under `## Unreleased` in [`CHANGELOG.md`](CHANGELOG.md). At release time we cut to `## vX.Y.Z`.

Format:

```markdown
## Unreleased

### Added
- chat: new XXX feature (#PR_NUMBER)

### Fixed
- channel: fix telegram bot Y issue (#PR_NUMBER)
```

Pure chore / refactor / internal docs changes don't need a CHANGELOG entry.

## Documentation maintenance redlines

If your PR matches any of these, you **must update the corresponding doc in the same PR** (full table in [AGENTS.md "文档维护"](AGENTS.md#文档维护)):

- Add / remove features, commands, modules → `CHANGELOG.md`
- Subsystem architecture change → `docs/architecture/<name>.md`
- New architectural capability → new file in `docs/architecture/` + `docs/README.md` index
- Modify Tauri command / HTTP route → [`docs/architecture/api-reference.md`](docs/architecture/api-reference.md)
- Edit either README → sync the other in the same PR (`README.md` ↔ `README.en.md`)
- Edit release notes → both Chinese and English in the same PR

## For experienced contributors / AI assistants

If you plan to touch cross-PR contracts (Provider / Permission / Plan Mode / Channel streaming / context compaction / memory priority / ...), **read [AGENTS.md](AGENTS.md) end-to-end first**—it lists 30+ cross-PR redlines. Also read the corresponding [`docs/architecture/<name>.md`](docs/architecture/).

## Feedback & discussion

- Bugs / feature requests: [Issues](https://github.com/shiwenwen/hope-agent/issues)
- Design discussion / usage questions: [Discussions](https://github.com/shiwenwen/hope-agent/discussions)
- Security: see [SECURITY.md](SECURITY.md) (**not in public issues**)

Thanks for contributing to Hope Agent! 🎉
