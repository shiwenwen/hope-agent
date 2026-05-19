---
name: ha-mac-control
description: "Hope Agent native macOS desktop control — the standard `mac_control` status / apps / snapshot / windows / menu / clipboard / dialog loop, target-first action rules, no-blind-coordinate policy, and recovery for stale AX/window/menu/dialog state. Load whenever using `mac_control`, or when the user asks to control local Mac apps, click/type/menu/window/dialog/clipboard, automate Finder/TextEdit/System Settings, or says 控制 Mac, macOS 自动化, 点按钮, 打开应用, 关闭窗口, 菜单点击."
version: 1.0.0
author: Hope Agent
license: MIT
allowed-tools: [mac_control, ask_user_question]
status: active
---

# Hope Agent Mac Control

`mac_control` operates the user's macOS desktop from the authorized Hope Agent app process. macOS UI state is volatile: apps steal focus, AX IDs expire, sheets attach to windows, and multiple windows often share similar titles. Use a fresh observation before every meaningful action.

## Standard Loop

Use this loop unless the user explicitly asks for a single read-only query:

```
1. mac_control(action="status")
2. mac_control(action="apps", op="frontmost" | "search" | "installed")
3. observe: snapshot / windows.list / menu.list / dialog.inspect
4. act: apps.activate/launch, windows.*, act.*, menu.click, dialog.*
5. verify: wait, snapshot, windows.list, or dialog.inspect
```

For a concrete app workflow:

```
apps.launch bundleId=...
apps.frontmost                         # verify focus if the next step depends on menus/input
snapshot or windows.list               # get fresh window/element ids
act/menu/windows/clipboard/dialog      # one action burst
wait or snapshot                       # verify the expected change
```

## Targeting Rules

- Prefer `bundleId` over `appName` for mutations. Use `apps.search` / `apps.installed` when the app name is uncertain, then retry with `bundleId`.
- `appNameMatch` defaults to `exact`. Use `contains` only for read-only discovery or when the user clearly gave a partial name.
- Prefer `windowId` from the latest `windows.list` or `snapshot` for window mutations.
- `target.windowTitleMatch` defaults to `exact`. Use `contains` only after listing windows and confirming a partial title is intentional.
- Prefer `elementId` from the latest `snapshot` for precise clicks and set-value actions.
- If two windows, dialogs, text fields, or buttons match, do not guess. Use a more specific target or ask the user.
- Element mutations reject equally ranked AX candidates instead of choosing the first match. When this happens, take a fresh `snapshot` and retry with `elementId`, `target.windowTitle`, `target.role`, or more specific `target.text`.

## Actions

### Apps

- Use `apps.frontmost` to know what macOS will receive menu and keyboard actions.
- Use `apps.activate bundleId=...` before operating an app that is not frontmost.
- Use `apps.search` or `apps.installed` when launch/activate by name fails.
- `apps.quit` is destructive. Verify the target app and prefer `bundleId`.

### Windows

- Use `windows.list` before `windows.close`, `move`, `resize`, or `minimize` unless the user supplied an exact `windowId`.
- `windowScope` defaults to `frontmost`. Use `windows.list windowScope="all"` to discover background app windows before activating or focusing them.
- Prefer all-scope ids like `win_<pid>_<index>` for cross-app window mutations; they are safer than generic titles.
- For `windows.close`, avoid generic titles like `Untitled` / `未命名` when multiple similar windows exist. Use `windowId`.
- Hope Agent's own window cannot be mutated through the Accessibility worker; if the target is Hope Agent itself, explain the limitation.

### Screenshots

- Use `snapshot includeScreenshot=true` when visual context matters.
- Default screenshots capture the primary display. Use `displayId` from `snapshot.displays` when the user points at a specific monitor.
- For a focused-window image, use `snapshot includeScreenshot=true screenshotTarget="window"`. Pass `windowId` from the latest snapshot/list when several windows are possible.
- Window screenshot matching uses the current AX window state; if it fails, take a fresh snapshot and retry with a precise `windowId`.

### Elements and Text

- `act.click` is for AX targets only. It requires `target` and should not consume raw `x/y`.
- Use `act.click_point` only when the user explicitly wants a coordinate click or AX cannot represent the target. This includes valid coordinates like `(0, 0)`.
- `act.type` and `act.set_value` should target text input roles (`AXTextArea`, `AXTextField`, `AXSearchField`, etc.).
- Use `act.paste` for long text or apps that do not accept `AXValue` reliably. It stages text on the pasteboard, invokes paste, and reports only clipboard restore status.
- Do not type passwords, OTPs, or private credentials unless the user explicitly supplied them in the current flow.

### Menus

- Prefer `menu.click` over hotkeys for app commands.
- `menu.scope` defaults to `app`, which targets the current frontmost app menu bar.
- Use `menu.list scope="system"` before operating macOS menu bar extras/status items. System menu entries may expose useful `description`, `value`, and `actions` even when `title` is empty.
- If a menu path fails, call `menu.list` with the same `scope` and check the localized titles/descriptions of the current menu surface.
- If the user says "do not use shortcuts", never call `act.hotkey`. Use menus or AX actions.

### Clipboard

- `clipboard.get` reads user clipboard text and may expose secrets. Use it only when the user asked for clipboard content or it is clearly necessary, and keep `maxChars` tight.
- `clipboard.set` is useful before a deliberate paste workflow. It does not echo the written text in the result; verify by pasting into the intended target, not by reading the clipboard back unless needed.
- Prefer `act.paste` over separate `clipboard.set` + `act.hotkey` for text insertion; it tries to restore the previous UTF-8 text clipboard.
- Use `clipboard.clear` only when the user asked to clear the clipboard or after a sensitive paste workflow.

### Dialogs and Sheets

- Use `dialog.inspect` before `dialog.accept` or `dialog.dismiss`.
- macOS sheets may appear as `AXSheet` elements attached to normal `AXWindow`s; inspect with higher `maxElements` when needed.
- When several dialogs are present, target by dialog text/window or use the button id from the inspected result.
- `buttonText` should be the visible label. Examples: `取消`, `保存`, `删除`, `Cancel`, `Save`, `Don't Save`.
- `dialog.dismiss` means a cancel/close-style action. If the user wants to discard changes, choose the explicit discard button such as `删除` or `Don't Save`, not a generic dismiss guess.

## Verification and Recovery

- After `apps.launch` / `apps.activate`, verify `frontmost` before menu or input actions.
- After any action that changes UI, re-snapshot or call the relevant list/inspect command before using old ids.
- If an element becomes stale, take a fresh snapshot and reselect by role + label/text + window.
- If `dialog.inspect` returns empty but the UI visibly has a sheet, retry with `maxElements: 300` or `500` and confirm the frontmost app.
- If `menu.click` says a path component was not found, check frontmost app and `menu.list`; do not retry the same path blindly.
- If a mutation succeeds but the expected state did not change, use `wait` or a fresh snapshot to verify before deciding the next action.

## Approval Awareness

Treat these as higher risk and be extra explicit about the target:

- `windows.close`
- `apps.quit`
- `menu.click` on destructive menu items
- `clipboard.get` / `clipboard.set` / `clipboard.clear`
- `dialog.accept` / explicit discard buttons
- raw coordinate clicks and drags

The approval system will enforce policy, but the model should still choose precise targets and explain uncertainty before asking the user to approve.
