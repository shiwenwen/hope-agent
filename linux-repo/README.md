# apt / yum repo templates

Single source of truth for the [shiwenwen/hope-agent-linux-repo](https://github.com/shiwenwen/hope-agent-linux-repo) Debian + RPM repository that hosts Hope Agent on GitHub Pages.

## Files

- [`rpm/hope-agent.repo`](rpm/hope-agent.repo) — the `.repo` file dropped into `/etc/yum.repos.d/` by `dnf config-manager --add-repo …`. CI also copies this file into the bucket repo at `rpm/hope-agent.repo`.
- The apt `conf/distributions` reprepro config is **generated on the fly inside CI** (the `SignWith:` line embeds the GPG fingerprint imported from the `GPG_SIGNING_KEY` secret), not committed here — that way rotating the signing key never needs a code change.
- The bucket repo's own `README.md` is the user-facing install guide; this directory is for maintainers.

## What CI does on every release

See [`../docs/release-process.md`](../docs/release-process.md) §1.9 for the full flow. Summary:

1. `gh release download` pulls `Hope.Agent_<v>_amd64.deb` + `Hope.Agent-<v>-1.x86_64.rpm`
2. Import `GPG_SIGNING_KEY` secret into a fresh `GNUPGHOME`
3. `reprepro -b apt includedeb stable …` builds the apt index + signs `InRelease` / `Release.gpg`
4. `createrepo_c rpm/stable/x86_64` + `gpg --detach-sign --armor` on `repomd.xml` builds the rpm index + signs the metadata
5. `rpm --addsign` is **not** used — package-level signatures are skipped because reprepro/createrepo only require *repo metadata* signatures, and the rpm Tauri produces is already trusted via repo-level `repo_gpgcheck=1`
6. `git commit + push` to `shiwenwen/hope-agent-linux-repo` via `LINUX_REPO_TOKEN`
7. GitHub Pages re-publishes within ~1 minute

## Bucket repo first-time setup

Already done; recorded here for posterity.

- Repo: `shiwenwen/hope-agent-linux-repo` (public)
- Pages: `https://shiwenwen.github.io/hope-agent-linux-repo/` (from `main` branch root)
- Public key: `pubkey.gpg` at repo root, also fetched by `gpgkey=…` in the rpm `.repo` file
- Signing key: ed25519, fingerprint `5F80 16D5 0633 E725 909E  9AD7 F0F6 6A31 DFAA EA08`, expires 2027-05-11

### Required secrets (in main repo)

- `GPG_SIGNING_KEY` — ASCII-armored private key for the ed25519 signing key. Stored 2026-05-11. Renew before 2027-05-11.
- `LINUX_REPO_TOKEN` — fine-grained PAT with `Contents: Read and write` on `shiwenwen/hope-agent-linux-repo` only. Renew per token expiry.

## Key rotation (every 12 months)

1. Generate a new ed25519 keypair (same procedure as the original — a one-shot `gpg --batch --gen-key` inside a docker container, see `docs/release-process.md` §1.9).
2. Export both armored pubkey and privkey.
3. `gh secret set GPG_SIGNING_KEY --repo shiwenwen/hope-agent < new-privkey.asc` (replaces the old secret).
4. Replace `pubkey.gpg` at the linux-repo root with the new armored pubkey (`gh api -X PUT ...`).
5. Update the fingerprint in [`hope-agent-linux-repo/README.md`](https://github.com/shiwenwen/hope-agent-linux-repo/blob/main/README.md) "Key info".
6. Re-run `gh workflow run update-linux-repo.yml -f tag=v<latest>` to re-sign the existing index with the new key.
7. Users will see "key changed" warnings on `apt update` / `dnf update` after rotation. They need to re-import the public key:
   ```bash
   curl -fsSL https://shiwenwen.github.io/hope-agent-linux-repo/pubkey.gpg | \
     sudo gpg --dearmor -o /usr/share/keyrings/hope-agent.gpg --yes
   ```

## Manual re-sync

If the template changes (e.g. adding `arm64` architecture support) and you want it to take effect against an already-published release:

```bash
gh workflow run update-linux-repo.yml -f tag=vX.Y.Z
```
