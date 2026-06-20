# Privacy Practices Draft

## Single Purpose

The extension's single purpose is to let the Hope Agent desktop app control Chrome tabs selected by the user.

## Data Collection

The extension can read page content, URLs, screenshots, console output, and network response metadata for Hope-controlled tabs. It can also observe Chrome download metadata when Hope Agent invokes an approved download observation action. This data is sent to the local Hope Agent native host through Chrome Native Messaging.

The extension does not sell data, does not use data for advertising, and does not send data directly to third-party services.

## User Control

Users can stop control from:

- The visible page overlay.
- The extension toolbar popup.
- Hope Agent Settings.

When control stops, the extension detaches the debugger from the tab.

## Remote Transfer

The extension itself does not perform remote network transfer. Hope Agent may later send selected context to AI providers configured by the user in the desktop app.
