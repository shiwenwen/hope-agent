# apt / yum repo templates

Single source of truth for the Hope Agent Debian + RPM repository, hosted on
**Cloudflare R2** and served at **`https://repo.hopeagent.ai/`**.

> **Why R2, not GitHub Pages.** apt/dnf indexes reference the package files by
> URL under one base, so the `.deb`/`.rpm` must live at that base. GitHub's
> 100 MB per-file limit rejected every package once the app crossed ~100 MB
> (v0.20.1's arm64 `.deb` was the first casualty; by v0.21.0 all four packages
> were over), so the old git-committed Pages bucket was structurally stuck. R2
> has no per-object size limit and **zero egress fees** — the natural host for
> a download-heavy package repo. The reprepro / createrepo_c / GPG build logic
> is unchanged; only the publish target moved.

## Files

- [`rpm/hope-agent.repo`](rpm/hope-agent.repo) — the `.repo` file dropped into `/etc/yum.repos.d/`. CI also uploads this file to the bucket at `rpm/hope-agent.repo` so `curl …/rpm/hope-agent.repo` serves it.
- The apt `conf/distributions` reprepro config is **generated on the fly inside CI** (the `SignWith:` line embeds the GPG fingerprint imported from the `GPG_SIGNING_KEY` secret), not committed here — that way rotating the signing key never needs a code change.
- `pubkey.gpg` is **no longer a committed file**: CI exports the signing key's public half from `GPG_SIGNING_KEY` and uploads it to the bucket root every run, so it always matches the active key.

## What CI does on every release ([`update-linux-repo.yml`](../.github/workflows/update-linux-repo.yml))

See [`../docs/release-process.md`](../docs/release-process.md) §1.9 for the full flow. Summary:

1. `gh release download` pulls every deb + rpm whose filename matches the release version — both amd64 / x86_64 and arm64 / aarch64 when present. Only the amd64 deb + x86_64 rpm are required as the minimum baseline.
2. Import `GPG_SIGNING_KEY` into a fresh `GNUPGHOME`.
3. **`rclone copy r2:$R2_BUCKET → ./bucket`** — mirror the currently-published tree DOWN so reprepro's `apt/db` state and createrepo_c's existing repodata are intact for an incremental update. Empty on the very first seed run.
4. `reprepro -b apt includedeb stable …` files each deb into the right `binary-<arch>` index and signs `InRelease` / `Release.gpg` via `SignWith:`.
5. `createrepo_c --update rpm/stable/<arch>/` + `gpg --detach-sign --armor` on each `repodata/repomd.xml`. Per-arch subdirs (`x86_64` + `aarch64`); dnf picks via `$basearch`.
6. Export `pubkey.gpg` from the imported key.
7. **`rclone copy ./bucket → r2:$R2_BUCKET`** — non-destructive upload (`copy`, never `sync`): regenerated indexes overwrite, historical packages are skipped by checksum, nothing is deleted.
8. **Verify** — fetch `InRelease`, `repomd.xml` and `pubkey.gpg` back over `https://repo.hopeagent.ai/…` and assert they are live and well-formed. A broken publish (or an unwired custom domain) fails the job here instead of silently leaving users on a stale source.

`rpm --addsign` is **not** used — reprepro/createrepo only require *repo metadata* signatures; the rpm is trusted via repo-level `repo_gpgcheck=1`.

## R2 first-time setup (one-time, on the Cloudflare side)

All of this is done in the Cloudflare dashboard + `gh` CLI; none of it is in code.

1. **Create the bucket.** R2 → Create bucket, name it `hope-agent-linux-repo` (any name; it becomes the `R2_BUCKET` secret). Location: Automatic.
2. **Connect the custom domain.** Bucket → Settings → Public access → **Custom Domains** → Connect `repo.hopeagent.ai`. This requires `hopeagent.ai`'s DNS to be on Cloudflare; Cloudflare auto-creates the CNAME. (Do **not** enable the `r2.dev` public URL for production — it is rate-limited.) Wait until `https://repo.hopeagent.ai/` resolves before running the seed.
3. **Create an R2 API token.** R2 → Manage R2 API Tokens → Create → **Object Read & Write**, scoped to this one bucket. Note the **Access Key ID**, **Secret Access Key**, and your **Account ID** (shown on the R2 overview page / in the S3 endpoint `https://<account_id>.r2.cloudflarestorage.com`).
4. **Add the four GitHub secrets** on `shiwenwen/hope-agent`:
   ```bash
   gh secret set R2_ACCOUNT_ID        --repo shiwenwen/hope-agent   # Cloudflare account id
   gh secret set R2_ACCESS_KEY_ID     --repo shiwenwen/hope-agent   # from the API token
   gh secret set R2_SECRET_ACCESS_KEY --repo shiwenwen/hope-agent   # from the API token
   gh secret set R2_BUCKET            --repo shiwenwen/hope-agent   # e.g. hope-agent-linux-repo
   ```
   `GPG_SIGNING_KEY` is unchanged and stays. `LINUX_REPO_TOKEN` and the old
   `shiwenwen/hope-agent-linux-repo` Pages repo are **retired** — you may leave
   the old repo up (its stale packages don't hurt) or archive it.
5. **Seed the repo.** With the domain live, run the workflow once against the current release:
   ```bash
   gh workflow run update-linux-repo.yml --repo shiwenwen/hope-agent -f tag=v0.21.0
   ```
   The verify step confirms `https://repo.hopeagent.ai/apt/dists/stable/InRelease` etc. are live. From then on it auto-fires on every `release.published`.

> **New-account gotcha — `tls: handshake failure` on the first seed.** Cloudflare
> provisions the **per-account TLS certificate for the S3 API endpoint**
> (`<account_id>.r2.cloudflarestorage.com`) with a delay on **brand-new R2
> accounts** — often ~20 minutes, sometimes a few hours (Cloudflare-side, see
> cloudflare/cloudflare-docs#6252). Until it lands, rclone/aws-cli/curl all fail
> at the TLS handshake (`alert 40`, no peer certificate) **even though the
> account id, keys, bucket, and custom domain are all correct** — the custom
> domain works immediately because it uses a different (Universal SSL) cert
> path. This is not a misconfiguration: **wait and re-run the seed.** A quick way
> to check readiness from a clean network: `curl -sS -o /dev/null -w '%{http_code}\n'
> https://<account_id>.r2.cloudflarestorage.com/` — once it returns an HTTP status
> (e.g. 400) instead of an SSL error, the endpoint is ready.
>
> **Can't wait? Seed via the Wrangler bridge.** The workflow accepts a
> `via` input: `gh workflow run update-linux-repo.yml -f tag=vX.Y.Z -f via=wrangler`.
> This uploads the built tree through the **Cloudflare API** (`api.cloudflare.com`,
> which has a valid cert) with `wrangler r2 object put` instead of the S3
> endpoint, bypassing the unprovisioned cert entirely. It is **seed-only** (a
> fresh/empty bucket, no pull) and needs a `CLOUDFLARE_API_TOKEN` secret — an API
> token with **Account → Workers R2 Storage → Edit**. Once the S3-endpoint cert
> provisions, drop the flag; normal releases go back to the default rclone path
> (which does the efficient incremental pull+push the bridge can't).

### Signing key info

- Signing key: ed25519, fingerprint `5F80 16D5 0633 E725 909E  9AD7 F0F6 6A31 DFAA EA08`, expires 2027-05-11.
- `GPG_SIGNING_KEY` — ASCII-armored private key. Stored 2026-05-11. Renew before 2027-05-11.

## Key rotation (every 12 months)

1. Generate a new ed25519 keypair (one-shot `gpg --batch --gen-key` in a docker container).
2. `gh secret set GPG_SIGNING_KEY --repo shiwenwen/hope-agent < new-privkey.asc` (replaces the old secret).
3. Re-run `gh workflow run update-linux-repo.yml -f tag=v<latest>` — CI re-exports `pubkey.gpg` from the new key and re-signs the index automatically (no manual `pubkey.gpg` PUT needed anymore).
4. Update the fingerprint above.
5. Users will see "key changed" warnings on `apt update` / `dnf update` after rotation and must re-import the public key:
   ```bash
   curl -fsSL https://repo.hopeagent.ai/pubkey.gpg | \
     sudo gpg --dearmor -o /usr/share/keyrings/hope-agent.gpg --yes
   ```

## Manual re-sync

If the template changes (e.g. new arch, retention tweak) and you want it to take effect against an already-published release:

```bash
gh workflow run update-linux-repo.yml -f tag=vX.Y.Z
```

Same-tag re-runs are idempotent — reprepro removes the version first and the R2
upload is non-destructive.
