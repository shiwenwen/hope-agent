# Permission Justification

## `activeTab`

Required for Chrome to grant the extension access to the currently selected tab when the user opens the extension popup or Hope Agent attaches control. Hope Agent still requires an explicit tab claim or agent-created tab before it sends control commands.

## `nativeMessaging`

Required to connect the extension to the local Hope Agent native host. This is the only transport between Chrome and the desktop app.

## `alarms`

Required to keep the connection to the local Hope Agent native host healthy. Chrome evicts idle MV3 service workers, and the desktop app cannot dial in, so a periodic alarm wakes the service worker to re-establish the native connection after the app restarts. It is used only for this local reconnection heartbeat.

## `debugger`

Required for Chrome DevTools Protocol control of user-approved tabs: navigation, screenshots, accessibility snapshots, input events, PDF generation, console/network/page-error observation, and dialog handling.

Hope Agent exposes high-level browser actions and an advanced raw CDP path. Advanced calls go through Hope Agent's normal tool approval system. Page content, console, network, and page-error inspection are scoped to Hope-controlled or claimed tabs.

## `scripting`

Required for the visible control overlay, emergency stop UI, and frame-local actions in cross-origin iframes where root-frame JavaScript cannot access the DOM.

## `tabs`

Required to list Chrome tabs for explicit user selection, create controlled tabs, activate tabs, and close only tabs that Hope Agent owns.

## `downloads`

Required to observe Chrome download activity and cancel downloads by id after Hope Agent approval. Each action is surfaced through Hope Agent's user approval flow before any download is observed or cancelled.

## `webNavigation`

Required to read Chrome's frame tree metadata (`frameId`, `parentFrameId`, URL, and document id) for Hope-controlled tabs. This lets Hope Agent map cross-origin iframe debugger sessions back to the correct frame without guessing from page content. It is used for diagnostics and safer iframe control only; Hope Agent does not monitor browsing history outside controlled tabs.

## `host_permissions`

`http://*/*` and `https://*/*` are required because Hope Agent may control arbitrary web pages selected by the user. Host permissions are technical capability only; Hope Agent still applies its own permission and ownership checks before operating a tab.
