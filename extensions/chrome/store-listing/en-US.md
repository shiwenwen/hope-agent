# Hope Agent Browser Control

## Short Description

Control Chrome tabs from the Hope Agent desktop app after explicit local setup.

## Detailed Description

Hope Agent Browser Control connects Google Chrome to the Hope Agent desktop app through Chrome Native Messaging. It lets Hope Agent operate Chrome tabs that you create, select, or claim from the app, including page navigation, screenshots, form entry, frame-aware clicks, download observation, and emergency stop controls.

The extension does not work by itself. It requires the Hope Agent native host installed on the same computer. When Hope Agent is not running or the native host is not installed, the extension stays idle.

Visible controls:

- A page overlay appears while Hope Agent controls a tab.
- The toolbar popup can stop control for the current tab or all controlled tabs.
- Hope Agent Settings shows connection status, version diagnostics, and repair steps.

Data handling:

- Browser data is sent only to the local Hope Agent app through Native Messaging.
- The extension does not send browsing data to a third-party server.
- Hope Agent applies its own permission checks before real Chrome access and advanced actions such as observing or interrupting downloads and using raw Chrome DevTools Protocol commands.

## Category

Productivity

## Language

English
