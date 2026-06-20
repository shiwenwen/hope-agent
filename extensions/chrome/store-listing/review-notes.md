# Chrome Web Store Review Notes

## What This Extension Does

This extension is a local bridge between Chrome and Hope Agent, a desktop AI assistant. It uses:

- `nativeMessaging` to connect to the local host named `com.hope_agent.chrome`.
- `debugger` to control only tabs that Hope Agent creates or the user explicitly claims inside Hope Agent.
- `scripting` to inject a visible stop overlay and to operate elements inside frames.
- `downloads` to observe Chrome download activity and cancel downloads by id after Hope Agent approval.
- `tabs` and host permissions to list/select tabs and run controlled page actions.
- `webNavigation` to read frame tree metadata for controlled tabs, so cross-origin iframe debugger sessions can be matched to the correct frame without guessing.

## Required Native Host

The extension requires the Hope Agent native host. During review, install Hope Agent, open Settings -> Browser, and use the native host install/repair action. The extension will show disconnected status until the native host is installed and Hope Agent is running.

Native host name:

```text
com.hope_agent.chrome
```

## Reviewer Smoke Test

1. Install Hope Agent and start the app.
2. Open Hope Agent Settings -> Browser.
3. Install or repair the native host.
4. Install this extension.
5. Return to Settings -> Browser and refresh status.
6. Use Hope Agent to create a controlled browser tab.
7. Confirm the tab shows the Hope Agent control overlay.
8. Use the extension toolbar popup to stop control.

## Privacy Boundary

The extension does not call remote APIs directly. It communicates with the local native host only. Any model/provider traffic is handled by Hope Agent according to the user's configured providers and permission settings.

## High-Risk Permissions Justification

See `permissions.md` in this directory.
