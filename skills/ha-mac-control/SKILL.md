---
name: ha-mac-control
description: "Hope Agent native macOS desktop control — the standard `mac_control` status / diagnostics / apps / dock / spaces / snapshot / visual / windows / menu / clipboard / dialog loop, target-first action rules, no-blind-coordinate policy, and recovery for stale AX/window/menu/dialog state. Load whenever using `mac_control`, or when the user asks to control local Mac apps, Dock, Spaces, click/type/menu/window/dialog/clipboard, automate Finder/TextEdit/System Settings, visually locate UI, or says 控制 Mac, macOS 自动化, 点按钮, 打开应用, Dock, Space, 关闭窗口, 菜单点击, 视觉定位."
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
3. observe: snapshot / visual.observe / elements.find / windows.list / dock.list / spaces.list / menu.list / menu.popover / dialog.inspect
4. act: apps.activate/launch, dock.launch, spaces.switch, windows.*, act.*, menu.click, dialog.*
5. verify: wait, snapshot, windows.list, or dialog.inspect
```

For a concrete app workflow:

```
apps.launch bundleId=...
apps.frontmost                         # verify focus if the next step depends on menus/input
snapshot, elements.find, or windows.list # get fresh window/element ids
act/menu/windows/clipboard/dialog      # one action burst
wait or snapshot                       # verify the expected change
```

## Targeting Rules

- Prefer `bundleId` over `appName` for mutations. Use `apps.search` / `apps.installed` when the app name is uncertain, then retry with `bundleId`.
- `appNameMatch` defaults to `exact`. Use `contains` only for read-only discovery or when the user clearly gave a partial name.
- Prefer `windowId` from the latest `windows.list` or `snapshot` for window mutations.
- `target.windowTitleMatch` defaults to `exact`. Use `contains` only after listing windows and confirming a partial title is intentional.
- Prefer `elementId` from the latest `snapshot` / `visual.observe` / `elements.find` for precise clicks and set-value actions, and pass the matching `target.snapshotId` with it. `snapshotId + elementId` lets the runtime verify the original AX fingerprint and re-resolve stale `el_N` ids instead of blindly trusting a new traversal.
- Use `elements.find` when a full snapshot is too noisy or when an action target is ambiguous. It is read-only and returns scored candidates with reasons; retry mutations with `target.elementId` from the chosen candidate plus the result `snapshotId`.
- If two windows, dialogs, text fields, or buttons match, do not guess. Use a more specific target or ask the user.
- Element mutations reject equally ranked AX candidates instead of choosing the first match. When this happens, take a fresh `snapshot` and retry with `elementId`, `target.windowTitle`, `target.role`, or more specific `target.text`.

## Actions

### Apps

- Use `apps.frontmost` to know what macOS will receive menu and keyboard actions.
- Use `apps.activate bundleId=...` before operating an app that is not frontmost.
- Use `apps.search` or `apps.installed` when launch/activate by name fails.
- `apps.quit` is destructive. Verify the target app and prefer `bundleId`.

### Dock and Spaces

- Use `dock.list` before `dock.launch`; prefer `dockItemId` or `bundleId` over a loose app name.
- Use `dock.menu` to open a Dock item's context menu and inspect `menuItems`; use `dock.select_menu` with `menuItem` when possible, or `menuIndex` only when titles are unavailable. If both are present, `menuItem` is treated as the intended target.
- `dock.hide` and `dock.show` change the user's Dock autohide setting and restart Dock, so be explicit before approval.
- Use `spaces.list` before `spaces.switch` when targeting a numbered Space. `spaceIndex` is 1-based.
- `spaces.switch direction="left"|"right"` / `spaceIndex` / `spaceId` pass exactly one selector. Direction and adjacent targets use Mission Control Control+Left/Right first; non-adjacent exact targets may fall back to Control+number or SkyLight/CGS. Verify with `spaces.list` or a fresh screenshot after switching.
- `spaces.move_window` moves one explicit window to `spaceIndex` / `spaceId` through SkyLight/CGS. Resolve the window first with `windows.list windowScope="all"` and prefer `windowId`; if post-move verification warns, use `spaces.list` or a fresh screenshot to confirm.

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

- Use `elements.find op="find"` before clicking or typing into ambiguous UI. Useful examples: `target.role="AXButton"`, `target.text="Save"`, `target.windowTitle="Untitled"`.
- `elements.find` returns `totalMatches` plus candidate `score`, `reasons`, `element`, and `window`. Prefer high-score candidates whose reasons include the user's intended text/role/window.
- Browser/WebView snapshots may focus the dominant `AXWebArea` and re-traverse when no text input is exposed. If a result warning mentions this fallback, use the refreshed candidates first; if it still exposes only web/canvas content, switch to `visual.observe annotate=true`, OCR, or `visual.point`.
- Use `act.dry_run` when the next mutation should use the exact same target resolver, but you want to verify the resolved element first. Pass `dryRunOp` for the intended real op, such as `click`, `type`, or `set_value`; the result returns resolved `target` plus `preview.executionPlan`, `fallbackPlan`, `verificationPlan`, and `warnings` without changing the UI.
- Read mutation `verification` when present. `verified` means the low-level expected state was observed, `failed` means the action returned but the observed state did not match, and `unverified` means the tool could not prove the result. For ordinary clicks without a clear state change, still verify with `wait`, `snapshot`, `elements.find`, or `dialog.inspect`.
- Use `explain=true` only when you need the same preview attached to an executed action result; for pre-approval review, prefer `act.dry_run`.
- Use `act.perform_action` for a named AX action when a higher-level op is not enough. It requires `target` and `axAction`; common aliases such as `press` and `show_menu` normalize to AX names, while other valid AX action strings are attempted directly even if the target did not advertise them in `actions[]`.
- `act.click` is for AX targets only. It requires `target` and should not consume raw `x/y`.
- `act.click` first attempts `AXPress`; if that fails and the target has bounds, the runtime may click the target center and report an `AXPressFailed+CGEventFallback(...)` execution marker.
- Use `act.click_point` only when the user explicitly wants a coordinate click or AX cannot represent the target. This includes valid coordinates like `(0, 0)`.
- Use `act.move_cursor` when the user wants the pointer moved without clicking. It accepts either `x/y` or a target, and can smooth the path with `durationMs` / `steps` / `motionProfile`.
- Use `act.press` for single-key or repeated key presses. Use `hotkey` for one chord such as Cmd+N; use `press` when you need sequential keys, repeat, holdMs, intervalMs, or shared modifiers.
- Use `act.swipe` for smooth pointer drag gestures from `x/y`, `fromX/fromY`, or a target to `deltaX/deltaY`, `toX/toY`, or `toTarget`; use `act.drag` for deliberate drag/drop between coordinate or AX element endpoints. Pass `motionProfile="human"` only when the gesture benefits from eased, less mechanical pointer motion.
- `act.type` and `act.set_value` should target text input roles (`AXTextArea`, `AXTextField`, `AXSearchField`, etc.).
- `act.type` defaults to AXSetValue. Only pass `typingProfile` / `typingDelayMs` when the app needs real character-by-character keyboard input.
- For replacement-style text entry, failed `AXSetValue` can fall back to focus + Cmd+A + protected pasteboard replace; still inspect the returned `verification` before assuming the text changed.
- Use `act.paste` for long text or apps that do not accept `AXValue` reliably. It stages text on the pasteboard, invokes paste, and reports only clipboard restore status.
- `act`, `wait`, and `dialog` results are compact by default and do not return a full AX snapshot. Set `includeSnapshot=true` only when full AX tree debugging is needed; otherwise verify with `wait`, `elements.find`, `windows.list`, or `dialog.inspect`.
- Do not type passwords, OTPs, or private credentials unless the user explicitly supplied them in the current flow.

### Visual Positioning

Use visual positioning when AX labels are missing, the UI is canvas-like, or the user refers to something visible on screen rather than a stable element.

Standard visual loop:

```
visual.observe screenshotTarget="window" | "display" annotate=true
act.click target.elementId="el_..." target.snapshotId="macsnap_..."  # when the annotated id is clear
visual.ocr or visual.find_text text="..."      # when the target is visible text
read the returned image and choose an image pixel point # when OCR is not enough
visual.point snapshotId=... coordinateSpace="image_pixels" x=... y=...
act.click target=<suggestedAction.target>       # if suggestedAction.op is click
act.click_point x=<suggestedAction.x> y=<suggestedAction.y> # if suggestedAction.op is click_point
verify with snapshot, visual.observe, wait, or elements.find
```

Rules:

- `visual.observe` is read-only. It returns an image file marker for model vision plus a compact JSON payload with `snapshotId`, screenshot metadata, displays, and windows.
- Prefer `visual.observe annotate=true` for ambiguous visual UI. The returned image is labeled with AX element ids and includes `uiMap`; when an id clearly identifies the target, use `act.click target.elementId=... target.snapshotId=<observe snapshotId>` instead of a coordinate click.
- If the annotated id is unclear or the target is not in `uiMap`, use OCR or image-pixel positioning.
- `visual.ocr` is read-only. Use it when visible text matters but you do not need to filter for one phrase yet.
- `visual.find_text` is read-only. Use it before coordinate clicking visible words or text-only buttons; pass `textMatch="contains"` only for intentional partial text.
- `visual.find_text` returns OCR `textMatches` with center points, AX `hitElements` / `nearestElements`, a top-level `suggestedAction`, and `suggestedActions[]` ordered from stable AX target to coordinate fallback.
- Image pixel coordinates use the screenshot top-left as origin. `(0, 0)` is valid. Never pass image pixels directly to `act.click_point`.
- Always call `visual.point` before coordinate clicks chosen from a screenshot. It converts image pixels to macOS screen points and returns AX `hitElements` / `nearestElements`.
- Prefer `suggestedAction` from `visual.point` or `visual.find_text`; follow its `op`. If it includes `target`, call `act.click` with that target. If it is `click_point`, use its `x/y`. If `insideFrame=false`, do not click; adjust the point or observe again.
- If `suggestedActions[]` has multiple entries, use the first clear AX target first and keep `click_point` as a fallback after re-observing uncertainty.
- If OCR returns no match, do not click blindly. Retry with `textMatch="contains"`, OCR `languages`, a fresh window screenshot, or use image-pixel visual positioning.
- If the snapshot expired or lacks screenshot metadata, call `visual.observe` again instead of reusing old points.

### Menus

- Prefer `menu.click` over hotkeys for app commands.
- `menu.scope` defaults to `app`, which targets the current frontmost app menu bar.
- Use `menu.list scope="system"` before operating macOS menu bar extras/status items. System menu entries include 0-based `index`, optional `boundsPoints`, and may expose useful `description`, `value`, and `actions` even when `title` is empty.
- For status items, prefer `menu.click scope="system" menuIndex=<index> verify=true` after listing when the title is empty or localized. Verification returns likely popovers and OCR screenshot metadata when available.
- Menu clicks use a native chain before giving up: `AXShowMenu`, then `AXPress`, then center-point click when bounds are available.
- After opening a status item or menu bar extra popover, use `menu.popover appHint="..."` to identify the floating panel. It ranks all-app AX windows with menu-bar geometry, host app hints, and optional OCR text; it does not click anything.
- If a menu path fails, call `menu.list` with the same `scope` and check the localized titles/descriptions of the current menu surface.
- If the user says "do not use shortcuts", never call `act.hotkey`. Use menus or AX actions.

### Clipboard

- `clipboard.get` reads user clipboard text and may expose secrets. Use it only when the user asked for clipboard content or it is clearly necessary, and keep `maxChars` tight.
- `clipboard.set` is useful before a deliberate paste workflow. It does not echo the written text in the result; verify by pasting into the intended target, not by reading the clipboard back unless needed.
- Prefer `act.paste` over separate `clipboard.set` + `act.hotkey` for text insertion; it backs up and restores the previous pasteboard items.
- Use `clipboard.clear` only when the user asked to clear the clipboard or after a sensitive paste workflow.

### Dialogs and Sheets

- Use `dialog.list` / `dialog.inspect` before mutating dialogs when the button or field label is not already known.
- macOS sheets and lightweight prompts may appear as `AXSheet` or `AXPopover` elements attached to normal `AXWindow`s; inspect with higher `maxElements` when needed.
- When several dialogs are present, target by dialog text/window or use the button id from the inspected result.
- `dialog.click` requires `buttonText`; use the visible label. Examples: `取消`, `保存`, `删除`, `Cancel`, `Save`, `Don't Save`.
- `dialog.input` requires `text`; use `field`, `fieldIndex`, or `target.elementId` when more than one dialog field exists. Set `clear=true` to replace the value.
- Dialog button presses and `clear=true` text input share the same fallbacks as `act.click` / `act.set_value`.
- `dialog.file` can enter `filePath`, set `fileName`, then click `selectButton` (or the default accept button). It returns `fileDialog.nameField`, `requestedButton`, and the actual `selectedButton` when clicked. Use `selectButton="none"` when you only want to fill path/name.
- `dialog.dismiss` means a cancel/close-style action. If the user wants to discard changes, choose the explicit discard button such as `删除` or `Don't Save`, not a generic dismiss guess.

## Verification and Recovery

- After `apps.launch` / `apps.activate`, verify `frontmost` before menu or input actions.
- After any action that changes UI, re-snapshot or call the relevant list/inspect command before using old ids.
- Tool approval restores the previously frontmost app and focused window before the approved `mac_control` mutation runs, but treat it as best-effort. After an approval, verify `frontmost` or take a fresh observation before chaining another focus-sensitive action.
- If an element becomes stale, take a fresh snapshot and reselect by role + label/text + window.
- If `act.perform_action` returns an AX unsupported/action error, do not retry the same call blindly. Use fresh `elements.find` / `snapshot` and choose a supported action, or switch to `act.click` / `act.click_point` fallback.
- If `dialog.inspect` returns empty but the UI visibly has a sheet, retry with `maxElements: 300` or `500` and confirm the frontmost app.
- If `menu.click` says a path component was not found, check frontmost app and `menu.list`; do not retry the same path blindly.
- If a mutation succeeds but the expected state did not change, use `wait` or a fresh snapshot to verify before deciding the next action.
- If a failure is hard to reproduce, call `diagnostics.summary` to inspect readiness, recent errors, cached snapshot summaries, and the focus anchor. Use `diagnostics.export` when the user/developer needs a managed JSON bundle under `~/.hope-agent/mac-control/diagnostics/` for replay analysis.

## Approval Awareness

Treat these as higher risk and be extra explicit about the target:

- `windows.close`
- `apps.quit`
- `dock.hide` / `dock.show`
- `spaces.switch`
- `menu.click` on destructive menu items
- `clipboard.get` / `clipboard.set` / `clipboard.clear`
- `dialog.accept` / explicit discard buttons
- raw coordinate clicks, cursor moves, swipes, and drags

The approval system will enforce policy, but the model should still choose precise targets and explain uncertainty before asking the user to approve.
