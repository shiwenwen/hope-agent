# Chrome Web Store Release Checklist

## Before upload

- Run `pnpm chrome:extension:package`.
- Confirm verifier output is ok.
- Confirm the generated zip is `extensions/chrome/dist/hope-agent-chrome-extension-<version>.zip`.
- Upload only the generated zip. Do not upload the unpacked source directory.
- Use the copy in `en-US.md`, `permissions.md`, `privacy.md`, and `review-notes.md`.

## First Web Store upload

- Chrome Web Store assigns the production extension ID after the first upload.
- Record that production ID.
- Update Hope Agent default config so `browser.extension.extensionIds` includes the production ID.
- Update Hope Agent default config so `browser.extension.storeUrl` points to the published listing.
- Keep the unpacked development ID `ejafepfkhjdjopjonfgalbkelimgeeji` allowed for Alpha/dev fallback.

## Native host trust

- Native host `allowed_origins` must include the production extension ID.
- Alpha/dev native host manifests may also include the unpacked development ID.
- Desktop release builds must bundle `ha-browser-host` as the Tauri `browser-host/` resource (`pnpm prepare:browser-host` runs from `beforeBuildCommand`).
- If a user installed the native host before the production ID was known, Settings -> Browser -> Repair native host must rewrite the manifest with the production ID.

## After publish

- Install from Chrome Web Store in a clean Chrome profile.
- Open Hope Agent Settings -> Browser.
- Install or repair the native host.
- Verify Settings reports Connected.
- Run a manual smoke on the local fixture with `pnpm chrome:extension:smoke-pages`.
- Verify `tabs.open_user_tabs`, `tabs.claim`, `snapshot.role`, `snapshot.screenshot`, `act.click`, and `tabs.finalize`.

## Fallback path

- Keep unpacked install documented and working for Alpha, development, self-use, and enterprise environments where Web Store install is blocked.
- Do not distribute `.crx` as a normal user path.
