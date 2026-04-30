/**
 * Transport abstraction layer.
 *
 * Provides a unified interface for frontend code to communicate with the
 * backend regardless of whether it runs inside Tauri (IPC) or as a
 * standalone web app (HTTP / WebSocket).
 */

import type { MediaItem, SessionMode } from "@/types/chat";

/**
 * One entry in `ChatStartArgs.attachments`. Snake-case fields are the
 * wire form — both Tauri IPC and `POST /api/chat` serialize this object
 * as-is. `data` and `file_path` are mutually-exclusive: inline images
 * carry base64 in `data`; everything else is persisted to disk and
 * referenced by absolute `file_path`.
 */
export interface ChatAttachment {
  name: string;
  mime_type: string;
  data?: string;
  file_path?: string;
}

/**
 * Args accepted by {@link Transport.startChat}. Mirrors the parameters of
 * the Tauri `chat` command and the `POST /api/chat` body.
 */
export interface ChatStartArgs {
  message: string;
  attachments: ReadonlyArray<ChatAttachment>;
  sessionId: string | null;
  incognito?: boolean;
  modelOverride?: string;
  agentId?: string;
  permissionMode?: SessionMode;
  planMode?: string;
  temperatureOverride?: number;
  displayText?: string;
  workingDir?: string | null;
  // Tauri's invoke serializes extra unknown fields without complaint, and
  // HTTP's POST body is plain JSON — keep this open so HTTP impl can
  // pass-through without an unsafe `as Record<string, unknown>` cast.
  [key: string]: unknown;
}

/**
 * Transport defines the three communication primitives the app needs:
 *
 * 1. `call` – request/response (command invocation)
 * 2. `startChat` – run a chat turn and stream events for it
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
   * Run a chat turn and stream its events. Resolves with the final
   * assistant response text once the turn completes.
   *
   * - Tauri mode: opens a `Channel<string>` internally and forwards each
   *   delta to `onEvent`.
   * - HTTP mode: stream deltas arrive via the global EventBus path
   *   (`/ws/events` → `chat:stream_delta`, consumed by
   *   `useChatStreamReattach`). `onEvent` is invoked only for a
   *   synthesized `session_created` event so callers' `__pending__` cache
   *   gets renamed in place.
   */
  startChat(args: ChatStartArgs, onEvent: (event: string) => void): Promise<string>;

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

  /**
   * Prompt the user to pick a directory and return its absolute path.
   * Returns `null` when the user cancels.
   *
   * - Tauri mode: opens the native directory picker via
   *   `@tauri-apps/plugin-dialog`. The returned path is on the user's local
   *   filesystem.
   * - HTTP mode: the implementation is expected to surface a server-side
   *   directory browser (see {@link listServerDirectory}); the returned path
   *   is on the server machine, not the browser client.
   */
  pickLocalDirectory(): Promise<string | null>;

  /**
   * List a single directory level on the server machine. Used by both the
   * directory-browser modal (HTTP mode) and the chat-input `@` mention popper
   * (path-mode navigation, both modes).
   *
   * @param path Absolute path to list. When omitted, the server returns a
   *             platform default root (`/` on Unix, user profile on Windows).
   */
  listServerDirectory(path?: string): Promise<DirListing>;

  /**
   * Fuzzy-search files & directories under `root`. Used by the chat-input
   * `@` mention popper for tokens that don't contain a `/` (e.g. `@chat`).
   *
   * - `root` MUST be absolute.
   * - `q` is matched as a case-insensitive subsequence against name then path.
   * - `limit` defaults to 50 server-side; results are pre-sorted by score.
   */
  searchFiles(root: string, q: string, limit?: number): Promise<FileSearchResponse>;
}

/**
 * One level of a server-side directory listing. Shape mirrors the
 * `/api/filesystem/list-dir` response.
 */
export interface DirListing {
  path: string;
  parent: string | null;
  entries: DirEntry[];
  /** `true` when the server capped results; show a "results truncated" hint. */
  truncated: boolean;
}

export interface DirEntry {
  name: string;
  /** Absolute path of this entry, pre-joined by the server. */
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number | null;
  modifiedMs: number | null;
}

/** A single result from `searchFiles`, sorted by score descending. */
export interface FileMatch {
  name: string;
  /** Absolute path. */
  path: string;
  /** Path relative to the search root, with `/` separator. */
  relPath: string;
  isDir: boolean;
  /** Subsequence-match score; higher = better. Server-sorted. */
  score: number;
}

/**
 * Response from `searchFiles`. Mirrors the `/api/filesystem/search-files`
 * response shape.
 */
export interface FileSearchResponse {
  /** Canonicalized absolute root that was searched. */
  root: string;
  matches: FileMatch[];
  /** `true` when the walk hit the per-search file cap and stopped early. */
  truncated: boolean;
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
