---
name: ha-pet-import
description: Safely import, select, switch, or enable a compatible desktop pet in Hope Agent. Resolve packages from any origin, including local folders, zip archives, pet.json plus a sprite, PNG/WebP atlases, chat attachments, repository or cloud files, direct HTTPS artifact URLs, and download pages. Use whenever a user asks to install, add, migrate, import, activate, use, select, switch to, wake, or enable a pet in Hope, regardless of which website, tool, or community produced it.
---

# Import a Hope Pet

Route every source through Hope's validator and atomic installer. Treat the origin as provenance, never as the destination or trust boundary.

## Non-negotiable rules

- Default the destination to Hope Agent unless the user explicitly names another product.
- Inspect arbitrary download pages when necessary. Reading inert HTML/JavaScript and tracing referenced resources is valid source resolution; the page's domain is not an import protocol or trust signal.
- Never run a source website's installer, package-manager command, or setup script. A page advertising Codex, VS Code, or another host still supplies only input bytes.
- Never copy files directly into `~/.hope-agent/pets` or manufacture `hope.json`. Hope creates metadata, hashes, staging, and the final directory.
- Never write to `~/.codex/pets` for a Hope import.
- Never search, read, print, or shell out for Hope credentials or the Server Owner Token. In particular, do not inspect `~/.hope-agent/credentials`, run `hope-agent server token show`, or put a Bearer token in an `exec` command.
- Always preview, show the result, and obtain confirmation before commit. Do not treat the original request as confirmation of bytes not yet inspected.
- Never enable or select a pet merely because it was imported. Change the active desktop pet only when the user explicitly asks to enable, use, activate, select, or switch to it.

## Select a safe Hope handoff

Use the exact `hope-agent pet ...` command prefix through Hope's `exec` tool. Hope recognizes only a strictly parsed Pet CLI argv, rejects shell operators and expansions, and routes it to the owning Hope binary on the host after normal exec approval. This sealed control-plane handoff also works when the conversation's shell commands are sandboxed; it never copies the platform binary or Owner Token into the container. Do not prepend a discovered executable path, add custom environment variables, request PTY/background mode, or wrap the command in another shell.

Local `--source` paths must already exist in the durable conversation workspace (for example, as an attachment or a file materialized by a trusted connector). Files created only inside an `isolated` exec command are discarded and cannot cross into the sealed host handoff; if Hope reports `pet_cli_local_source_unavailable_to_host`, materialize the package durably or use a direct HTTPS artifact URL. Never claim that an isolated-only file was imported.

Before inspecting or downloading the source, run:

```text
hope-agent pet capabilities --json
```

Continue only when stdout is JSON with `status: "capabilities"`, `schemaVersion >= 1`, and the capabilities needed below. Enabling requires `activateInstalled: true`. Exit status alone is not proof: an older desktop binary may silently open or delegate to an existing GUI and produce no CLI output. On empty, malformed, or incompatible output, do not retry pet subcommands and do not bypass the mismatch with raw HTTP calls. Explain that the running Hope build lacks the bundled pet protocol and guide the user to upgrade or use Settings → Pets.

## Select or enable an installed pet

Use this flow when the user names an already installed pet. After a requested import succeeds, skip lookup and use the exact `pet.petRef` returned by commit.

1. Run `hope-agent pet list --json` and resolve the target by exact `petRef` first, then exact `manifest.displayName`. Do not choose from a fuzzy or ambiguous name; ask the user when more than one installed pet matches.
2. For “enable”, “use”, “activate”, or “wake”, call the dedicated desktop command:

```text
hope-agent pet activate --pet-ref <PET_REF> --json
```

Require JSON with `status: "activated"`, the exact requested `petRef`, and `enabled: true`. The CLI securely calls the running desktop's authenticated Pet API; it never exposes the Owner Token to the model.

For “select” or “switch” without a request to show the overlay, use `update_settings` with only `selectedPetRef` and preserve the current enabled state.

Pet enablement is desktop-only because the current Tauri process owns the native PetWindow. If `pet activate` reports unavailable, unsupported, or desktop-only, guide the user to the desktop control. Never fall back to editing `config.json` or treating an offline config write as a live overlay.

## Resolve the source

Choose the actual package artifact without restricting its origin:

| Given source | Feed to Hope |
| --- | --- |
| Folder containing `pet.json` or legacy `avatar.json` and its sprite | The folder path |
| Zip / `.codex-pet.zip` | The archive path or direct HTTPS URL |
| `pet.json` with its relative PNG/WebP available beside it | The manifest path or direct HTTPS URL |
| Standalone 1536×1872 or 1536×2288 PNG/WebP atlas | The image path or direct HTTPS URL; optionally preserve a user-supplied display name |
| Chat attachment, repository file, cloud-drive file, or other connector resource | Materialize the file locally with the available trusted connector, then feed its local path |
| Ordinary download/web page | Inspect its HTML, scripts, and referenced resources to find the real zip, manifest, sprite, or client-side packaging inputs. Feed those bytes to Hope, not the HTML page and not the site's install command |

Direct HTTPS artifacts may come from any public origin. Supported package shapes and size, archive, path, image, SSRF, and atlas checks still apply; “any source” does not mean executing arbitrary content.

When a page constructs an archive in the browser instead of publishing one, reproduce only the inert file collection/archiving step or download its manifest and sprite inputs. Never execute an installer to obtain them.

For multiple loose files, materialize them under one temporary directory and pass every file as a repeated `--source`. Preserve the server-provided filenames because the manifest's relative `spritesheetPath` is part of the package. Do not rewrite the manifest merely to make it pass. If a source provides only a character picture rather than a valid atlas, report that it needs pet creation/conversion rather than pretending it is importable.

## Use the CLI

Use the capability-checked sealed `hope-agent pet` handoff above. Do not install a substitute CLI.

### 1. Preview

Run with a local path or direct artifact URL as a safely quoted argument:

```text
hope-agent pet preview --source <SOURCE> --json
```

For loose resources discovered on a page, repeat the flag for files in the same temporary directory:

```text
hope-agent pet preview --source <PET_JSON> --source <SPRITE> --json
```

Add `--display-name <NAME>` only for a standalone atlas or an explicit user override. Preserve the identical option during commit.

Require `canCommit: true`. Show the user:

- `manifest.displayName` and sprite version
- `width` × `height`
- every validation warning or error
- `duplicatePetRef`, when present
- the exact `packageHash`

Stop on validation errors and explain them. Warnings remain visible but do not silently become errors.

### 2. Confirm

Ask whether to import this exact reviewed package into Hope. If the user originally requested “install and enable/use”, make the confirmation explicitly cover both importing these reviewed bytes and selecting/enabling the returned pet. Otherwise do not add enablement to the confirmation.

### 3. Commit

After confirmation, reuse the same source and optional display name:

```text
hope-agent pet import --source <SOURCE> --expected-package-hash <PACKAGE_HASH> --json
```

Reuse the identical ordered `--source` list for loose files.

The second command re-reads or re-downloads the artifact. On `pet_cli_source_changed`, preview the new bytes and ask again; never substitute the new hash automatically.

Report the returned status and `pet.petRef`. `already_present` is success without another copy. Import only adds the package to Hope's library; select or enable it separately and only when explicitly requested in a desktop GUI conversation.

For an explicitly requested “import and enable/use” flow, call `pet activate` only after import succeeds, using the returned `pet.petRef` rather than guessing from the manifest id. If enablement fails, report that the package is installed but not enabled; never roll back or repeat the successful import.

## Use the HTTP API

Use this path only through an existing authenticated Hope connector or client whose call surface does not expose its credentials to the model. Never use `exec`/`curl` for authenticated Hope API calls, retrieve a token from disk or CLI, or ask the user to paste one into chat. If no such client is already available, use the capability-checked local CLI or guide the user to Settings → Pets.

For any direct HTTPS zip, manifest, or sprite URL, preview with:

```json
{
  "request": {
    "source": { "kind": "link", "link": "https://files.example/pet-package.zip" },
    "displayName": null
  }
}
```

Send it to `POST /api/pets/import/preview`. Browser/remote local files must first use that authenticated client's staged upload flow, then use `uploadedPath` or `uploadedFiles`; HTTP intentionally rejects server-local paths. Use `uploadedFiles` for a loose manifest and sprite so the import service receives the set as one package.

After showing the preview and receiving confirmation, send its token only in the JSON body:

```json
{
  "request": {
    "previewToken": "<PREVIEW_TOKEN>",
    "enableAfterImport": false
  }
}
```

Send that to `POST /api/pets/import/commit`. If the user declines or the workflow is abandoned, call `POST /api/pets/import/preview/cancel` with `{"previewToken":"<PREVIEW_TOKEN>"}`. Never request `enableAfterImport: true` over HTTP.

To activate an already installed pet through the same authenticated client in a desktop runtime, send `{"petRef":"custom:example"}` to `POST /api/pets/activate`. Require the returned config to contain the same `selectedPetRef` and `enabled: true`. Headless Server deliberately returns desktop-only.
