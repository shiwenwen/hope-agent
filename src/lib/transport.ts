/**
 * Transport abstraction layer.
 *
 * Provides a unified interface for frontend code to communicate with the
 * backend regardless of whether it runs inside Tauri (IPC) or as a
 * standalone web app (HTTP / WebSocket).
 */

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
