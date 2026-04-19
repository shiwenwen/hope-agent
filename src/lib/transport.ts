/**
 * Transport abstraction layer.
 *
 * Provides a unified interface for frontend code to communicate with the
 * backend regardless of whether it runs inside Tauri (IPC) or as a
 * standalone web app (HTTP / WebSocket).
 */

import type { MediaItem } from "@/types/chat";

/** A handle returned by `openChatStream` to control the stream lifetime. */
export interface ChatStream {
  /** Close the stream and release resources. */
  close(): void;
}

/**
 * Transport defines the three communication primitives the app needs:
 *
 * 1. `call` – request/response (command invocation)
 * 2. `openChatStream` – streaming chat events for a session
 * 3. `listen` – subscribe to backend-pushed events
 */
export interface Transport {
  /**
   * Invoke a backend command and return the result.
   *
   * In Tauri mode this maps to `invoke()`.
   * In HTTP mode this maps to REST endpoints.
   */
  call<T>(command: string, args?: Record<string, unknown>): Promise<T>;

  /**
   * Prepare file data for transport.
   *
   * Returns a `Blob` (HTTP — multipart, zero-copy) or `number[]`
   * (Tauri IPC — JSON serialization). Callers pass the result as the
   * `data` field in `call()` args.
   */
  prepareFileData(buffer: ArrayBuffer, mimeType: string): Blob | number[];

  /**
   * Open a streaming channel for chat events.
   *
   * @param sessionId - The session to stream (may be `null` for new sessions).
   * @param onEvent   - Called for every streamed event (raw JSON string).
   * @returns A `ChatStream` handle; call `.close()` to terminate.
   */
  openChatStream(
    sessionId: string | null,
    onEvent: (event: string) => void,
  ): ChatStream;

  /**
   * Subscribe to a named backend event.
   *
   * @returns An unsubscribe function.
   */
  listen(eventName: string, handler: (payload: unknown) => void): () => void;

  /**
   * Resolve a {@link MediaItem} into a URL that `<img src>` / `<a href>` /
   * window.open can consume. Returns `null` when the item isn't reachable
   * in the current transport (legacy URL shape, missing `localPath` in
   * Tauri mode, etc.) — callers should render a FileCard fallback instead
   * of a broken `<img src="">`.
   */
  resolveMediaUrl(item: MediaItem): string | null;

  /**
   * Resolve a persisted image reference (avatar path, project-logo data URL,
   * remote image URL) into something `<img src>` can consume in the current
   * transport.
   *
   * Accepted inputs:
   *  - `data:` URL  → passthrough (works in both modes)
   *  - `http(s)://` URL → passthrough
   *  - Absolute filesystem path (typical for avatars, e.g.
   *    `~/.hope-agent/avatars/foo.png`):
   *       - Tauri mode → wrapped via `convertFileSrc`
   *       - HTTP mode  → rewritten to a server route
   *         (`/api/avatars/{basename}?token=...`) when the path's parent
   *         directory matches a known asset category; otherwise `null`
   *  - `null` / empty string → `null`
   *
   * Callers should fall back to their emoji / initials / default-icon
   * rendering when the result is `null`.
   */
  resolveAssetUrl(path: string | null | undefined): string | null;

  /**
   * Trigger the user-facing "open" action for a media item.
   * - Tauri: opens the file with the OS default handler.
   * - HTTP:  downloads via a transient `<a download>`.
   */
  openMedia(item: MediaItem): Promise<void>;

  /**
   * Show the media file in the OS file manager (Finder / Explorer).
   * No-op on HTTP/Web — UIs should gate on {@link supportsLocalFileOps}.
   */
  revealMedia(item: MediaItem): Promise<void>;

  /**
   * Whether the transport supports local-file ops (open-in-app, reveal-in-folder).
   * False in HTTP/Web mode — UIs should hide the "Reveal" action.
   */
  supportsLocalFileOps(): boolean;

  /**
   * Prompt the user to pick a local image and return a {@link PickedImage}
   * the caller can feed into a preview / crop dialog. Returns `null` when
   * the user cancels. See {@link PickLocalImageFn} for transport-specific
   * behaviour.
   */
  pickLocalImage(): Promise<PickedImage | null>;
}

/**
 * Returns `true` when the app is running inside a Tauri webview.
 *
 * Detection is based on the presence of `window.__TAURI_INTERNALS__` which
 * Tauri injects before any user script executes.
 */
export function isTauriMode(): boolean {
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    return typeof window !== "undefined" && !!(window as any).__TAURI_INTERNALS__;
  } catch {
    return false;
  }
}

/**
 * Outcome of a local image picker interaction.
 *
 * - `src`: URL the caller can pass to `<img src>` or into `AvatarCropDialog`.
 *   In Tauri mode this is a `tauri://` asset URL (safe to display but the
 *   final Blob comes from the crop dialog's canvas); in HTTP mode this is
 *   a `blob:` URL created from a `<input type="file">` selection.
 * - `file`: the underlying `File` in HTTP mode. Absent in Tauri mode (the
 *   crop dialog re-encodes whatever the canvas produces before upload).
 * - `revoke`: release the URL when the caller is done previewing. Safe to
 *   call multiple times. Must be called on crop confirm, cancel, and on
 *   component unmount to avoid leaking Blob-backed memory in long HTTP
 *   sessions.
 */
export interface PickedImage {
  src: string;
  file?: File;
  revoke?: () => void;
}

/**
 * Prompt the user to pick a single local image, returning either an
 * HTTP-displayable `src` plus the underlying `File`, or `null` when the
 * user cancels.
 *
 * Transport-specific implementations are provided by:
 *  - [`pickLocalImage`](./transport-tauri.ts) — `@tauri-apps/plugin-dialog.open` + `convertFileSrc`.
 *  - [`pickLocalImage`](./transport-http.ts) — hidden `<input type="file">`.
 *
 * Callers obtain the right one via `getTransport().pickLocalImage()` (this
 * method is on the Transport interface below; there's also a re-export
 * here so the type is co-located with `PickedImage`).
 */
export type PickLocalImageFn = () => Promise<PickedImage | null>;

/**
 * Normalize a `listen()` payload into its decoded form.
 *
 * Tauri 2 and the HTTP/WS transports both deliver already-parsed JS values,
 * but older backend paths that explicitly `serde_json::to_string(...)` before
 * emitting still arrive as a JSON string. This helper handles both shapes so
 * call sites don't need to repeat the `typeof raw === "string"` check.
 */
export function parsePayload<T>(raw: unknown): T {
  return (typeof raw === "string" ? JSON.parse(raw) : raw) as T;
}
