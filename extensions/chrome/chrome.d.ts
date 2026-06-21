// The MV3 service worker uses the broad chrome.* API surface; a precise
// ambient type isn't worth maintaining here, so the global is intentionally `any`.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
declare const chrome: any;
