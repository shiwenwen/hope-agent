# Self-Diagnosis & Issue Reporting

Hope Agent exposes a conversational self-understanding path through the bundled
`ha-self-diagnosis` skill and a core `issue_report` tool. The v1 scope is
user/conversation triggered: it does not run background health scans.

## Responsibilities

- `skills/ha-self-diagnosis/` is the workflow entrypoint. It runs in
  `context: fork` so source reads, log queries, and diagnostic snippets do not
  bloat the parent conversation.
- Self-study mode answers questions about Hope Agent's implementation by
  reading `docs/architecture/`, live source files, settings, and read-only
  runtime databases.
- Issue-report mode turns bugs, feature requests, and improvements into
  GitHub issue drafts. A bug is not required: explicit user requests for a
  requirement or improvement may go straight to draft/submission flow.
- `issue_report` is the core tool for duplicate search, draft generation, and
  issue creation.

## Configuration

`AppConfig.issue_reporting` controls the default target and payload shaping:

- `enabled`
- `owner`
- `repo`
- `apiBaseUrl`
- `labelsByKind.{bug,feature,improvement}`
- `maxEvidenceChars`
- `duplicateCheckEnabled`

The default repository is `shiwenwen/hope-agent`.

The GitHub token is optional and is not stored in `config.json`. If configured,
it lives at `~/.hope-agent/credentials/github-issue.json` and must be written
with `platform::write_secure_file`. Settings exposes token save/clear/test
commands for both Tauri and HTTP transports.

When no token is configured, `issue_report` falls back to the user's
authenticated GitHub CLI (`gh`). This uses the identity from `gh auth login`;
Hope Agent does not read or persist that credential.

## Tool Contract

`issue_report` supports:

- `action: "search"`: searches open GitHub issues in the configured repo.
- `action: "draft"`: creates a sanitized draft and does not require a token.
- `action: "create"`: asks the user to confirm through `ask_user_question`,
  then posts to GitHub with the configured token or authenticated `gh` CLI.

Issue kinds are `bug`, `feature`, and `improvement`.

The tool sanitizes issue titles, bodies, evidence, and GitHub error bodies with
the shared sensitive-data redactor. Bodies are capped by
`maxEvidenceChars`; GitHub error excerpts use a smaller internal cap.

## Boundaries

- Creation always requires a user confirmation inside the tool, even if the
  caller already showed a draft.
- Outbound GitHub requests must pass `security::ssrf::check_url`.
- The `gh` fallback is used only when no Hope Agent issue-reporting token is
  configured.
- `ha-self-diagnosis` must query SQLite databases read-only.
- Background monitoring or auto-filing is out of scope for v1.
- The issue-reporting token should never be echoed in chat, logs, or issue
  bodies.
