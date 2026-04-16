/**
 * Tauri IPC transport implementation.
 *
 * Wraps `@tauri-apps/api/core` invoke / Channel and
 * `@tauri-apps/api/event` listen into the Transport interface.
 */

import { invoke, Channel } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { Transport, ChatStream } from "@/lib/transport";

export class TauriTransport implements Transport {
  // ----- call -----

  async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return invoke<T>(command, args);
  }

  // ----- prepareFileData -----

  prepareFileData(buffer: ArrayBuffer): number[] {
    return Array.from(new Uint8Array(buffer));
  }

  // ----- openChatStream -----

  openChatStream(
    _sessionId: string | null,
    onEvent: (event: string) => void,
  ): ChatStream {
    const channel = new Channel<string>();
    channel.onmessage = (raw: string) => {
      onEvent(raw);
    };

    // The Tauri Channel is passed as an argument to the `chat` command.
    // The actual `invoke("chat", { ..., onEvent: channel })` call is
    // performed by the caller (e.g. useChatStream). Here we just expose
    // the channel so callers can attach it.
    //
    // We store the channel reference on the returned handle so the caller
    // can pass `handle.tauriChannel` to invoke().
    const handle: TauriChatStream = {
      tauriChannel: channel,
      close() {
        // Tauri Channels are closed when the invoke promise resolves or
        // the caller drops the reference. Explicit close is a no-op but
        // we null out the callback to prevent late deliveries.
        channel.onmessage = () => {};
      },
    };

    return handle;
  }

  // ----- listen -----

  listen(eventName: string, handler: (payload: unknown) => void): () => void {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    tauriListen(eventName, (event) => {
      handler(event.payload);
    }).then((fn) => {
      if (cancelled) {
        // The caller already unsubscribed before the async setup finished.
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }
}

/**
 * Extended ChatStream that exposes the underlying Tauri Channel.
 *
 * Callers that need to pass the channel to `invoke("chat", { onEvent })`
 * can narrow the type:
 *
 * ```ts
 * const stream = transport.openChatStream(sid, handler);
 * invoke("chat", { ..., onEvent: (stream as TauriChatStream).tauriChannel });
 * ```
 */
export interface TauriChatStream extends ChatStream {
  readonly tauriChannel: Channel<string>;
}
