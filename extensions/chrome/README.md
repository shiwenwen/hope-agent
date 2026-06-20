# Hope Agent Browser Control Extension

Production distribution uses Chrome Web Store. Unpacked install stays supported for
Alpha, development, self-use, and enterprise environments where the store path is
blocked.

## Web Store release

1. Run `pnpm chrome:extension:package` from the repo root.
2. Upload `extensions/chrome/dist/hope-agent-chrome-extension-<version>.zip`.
3. After the first upload, record the production extension ID assigned by Chrome Web Store.
4. Add that ID to Hope Agent's default `browser.extension.extensionIds` and set the default `storeUrl`.
5. Keep the unpacked dev ID (`ejafepfkhjdjopjonfgalbkelimgeeji`) in the allowlist for Alpha/dev fallback.

The Web Store package script strips the unpacked-development `manifest.key` so Chrome Web Store can assign the production extension ID. Keep the `key` in `manifest.json` for alpha `Load unpacked` installs; it gives the dev extension a stable ID for native messaging.

Packaged Hope Agent desktop builds run `pnpm prepare:browser-host` from Tauri's `beforeBuildCommand`, bundle `ha-browser-host` as a `browser-host/` resource, and expose that path to Core via `HOPE_AGENT_BROWSER_HOST_PATH`. Users should not need to find the native host binary in normal Web Store installs.

Store listing, permission notes, privacy copy, reviewer notes, and the release checklist live in `store-listing/`.

## Unpacked fallback

1. Build the native host binary: `cargo build -p ha-browser-host`.
2. Load this directory with `chrome://extensions` -> Developer mode -> Load unpacked.
3. Open Hope Agent Settings -> Browser.
4. The Settings panel auto-detects this unpacked extension's stable ID (`ejafepfkhjdjopjonfgalbkelimgeeji`). Confirm the native host path and click Install native host.
5. Reload the extension and refresh Hope Agent Settings -> Browser.

Native host manifest directories:

- macOS: `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`
- Linux: `~/.config/google-chrome/NativeMessagingHosts/`
- Windows: manifest is written under `%LOCALAPPDATA%\\HopeAgent\\extension\\`, and Hope Agent registers `HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\com.hope_agent.chrome`.

The extension action popup includes emergency controls:

- Stop current tab: detaches Hope Agent from the active Chrome tab.
- Stop all controlled tabs: detaches every tab this extension currently tracks.

Run `pnpm exec tsc -p extensions/chrome/tsconfig.json --noEmit` from the repo root to type-check the service worker and popup.

Local smoke pages:

1. Run `pnpm chrome:extension:smoke-pages:check` to verify the local fixture server and test pages.
2. Run `pnpm chrome:extension:smoke-pages` to serve the manual fixture.
3. Open the printed root URL in Chrome, claim the tab from Hope Agent, and verify:
   - root and same-origin iframe snapshot/action/crop/drag,
   - cross-origin iframe snapshot/action/crop/drag,
   - `browser.status` frame tree + matched flat-session diagnostics.
