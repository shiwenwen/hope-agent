import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { getTransport } from "@/lib/transport-provider"
import { useAudioRecorder } from "@/hooks/useAudioRecorder"
import type { RecorderState } from "@/hooks/useAudioRecorder"
import { usePcm16Streamer, pcm16ToBase64 } from "@/hooks/usePcm16Streamer"
import { logger } from "@/lib/logger"
import {
  fetchActiveProviderKind,
  STREAMING_KINDS,
  unwrapSessionId,
  type SttProviderKind,
} from "@/lib/stt"

interface Transcript {
  text: string
  language?: string | null
  durationMs?: number | null
}

interface TranscriptDeltaPayload {
  sessionId: string
  text: string
  isFinal: boolean
  accumulated?: string | null
  language?: string | null
}

interface SessionErrorPayload {
  sessionId: string
  code: string
  message: string
}

export type VoiceInputState =
  | RecorderState
  | "transcribing"
  | "ready"

export interface UseVoiceInputResult {
  state: VoiceInputState
  durationMs: number
  audioLevel: number
  /** Live partial transcript while streaming. Empty in batch mode. */
  partialText: string
  /** Human-readable last error (already localized). `null` when idle / OK. */
  errorMessage: string | null
  /** Begin recording. */
  start: () => Promise<void>
  /** Stop recording and run STT. Resolves with the transcript text (or empty on error). */
  stopAndTranscribe: () => Promise<string>
  /** Discard current recording without running STT. */
  cancel: () => void
  /** Clear the error message (called after the caller surfaced it). */
  clearError: () => void
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onerror = () => reject(reader.error ?? new Error("FileReader failed"))
    reader.onload = () => {
      const r = reader.result
      if (typeof r !== "string") {
        reject(new Error("Unexpected FileReader result"))
        return
      }
      const idx = r.indexOf(",")
      resolve(idx >= 0 ? r.slice(idx + 1) : r)
    }
    reader.readAsDataURL(blob)
  })
}

function deriveFilename(mimeType: string): string {
  if (mimeType.includes("webm")) return "voice.webm"
  if (mimeType.includes("ogg")) return "voice.ogg"
  if (mimeType.includes("mp4")) return "voice.m4a"
  if (mimeType.includes("mpeg")) return "voice.mp3"
  return "voice.bin"
}

/**
 * Voice input composite hook. Selects between two paths at `start()`
 * time based on the active provider's wire protocol:
 *
 * - **Batch** (`/v1/audio/transcriptions` or `/v1/chat/completions` with
 *   `input_audio`): record via `MediaRecorder` into a webm/opus blob and
 *   hand the whole thing to `stt_transcribe_blob` on `stop()`.
 * - **Streaming** (Deepgram / AssemblyAI / Azure / Volcengine / iFlytek
 *   WebSocket): capture 16 kHz PCM16 frames via `usePcm16Streamer`,
 *   open a session with `stt_start_session`, push each frame through
 *   `stt_push_chunk`, subscribe `stt:transcript_partial/final/session_error`
 *   for live partial preview, and `stt_finalize_session` on `stop()`.
 *
 * The picked path is opaque to the caller (`ChatInput`); the only new
 * surface is `partialText` for showing live preview while streaming.
 */
export function useVoiceInput(): UseVoiceInputResult {
  const { t } = useTranslation()
  const recorder = useAudioRecorder()
  const streamer = usePcm16Streamer()
  const [transcribing, setTranscribing] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)
  const [partialText, setPartialText] = useState("")
  const [mode, setMode] = useState<"batch" | "streaming" | null>(null)

  const sessionIdRef = useRef<string | null>(null)
  const finalAccumulatorRef = useRef<string>("")
  const sessionErrorRef = useRef<string | null>(null)
  const unsubsRef = useRef<Array<() => void>>([])

  const teardownSession = useCallback(() => {
    for (const off of unsubsRef.current) {
      try {
        off()
      } catch {
        // ignore
      }
    }
    unsubsRef.current = []
    sessionIdRef.current = null
    finalAccumulatorRef.current = ""
    sessionErrorRef.current = null
    setPartialText("")
  }, [])

  const subscribeSessionEvents = useCallback((sessionId: string) => {
    const transport = getTransport()
    const onPartial = (payload: unknown) => {
      const p = payload as TranscriptDeltaPayload | null
      if (!p || p.sessionId !== sessionId) return
      // `accumulated` is the cumulative buffer some providers emit
      // (Deepgram / AssemblyAI); when absent, `text` is the latest delta.
      const preview = (p.accumulated ?? "").trim() || p.text || ""
      setPartialText(preview)
    }
    const onFinal = (payload: unknown) => {
      const p = payload as TranscriptDeltaPayload | null
      if (!p || p.sessionId !== sessionId) return
      finalAccumulatorRef.current += p.text
      // Show the running tally so the user sees stable text accumulating.
      setPartialText(finalAccumulatorRef.current)
    }
    const onError = (payload: unknown) => {
      const p = payload as SessionErrorPayload | null
      if (!p || p.sessionId !== sessionId) return
      sessionErrorRef.current = p.message || p.code
      logger.error(
        "voice",
        "useVoiceInput::session",
        `session error code=${p.code} raw=${p.message}`,
      )
    }
    unsubsRef.current = [
      transport.listen("stt:transcript_partial", onPartial),
      transport.listen("stt:transcript_final", onFinal),
      transport.listen("stt:session_error", onError),
    ]
  }, [])

  const startBatch = useCallback(async () => {
    setMode("batch")
    try {
      await recorder.start()
    } catch (e) {
      const m = e instanceof Error ? e.message : String(e)
      const n = e instanceof Error ? e.name : "?"
      logger.error(
        "voice",
        "useVoiceInput::start",
        `recorder.start failed name=${n} raw=${m}`,
      )
    }
  }, [recorder])

  const startStreaming = useCallback(
    async (providerId: string, modelId: string, kind: SttProviderKind) => {
      setMode("streaming")
      try {
        const raw = await getTransport().call<unknown>("stt_start_session", {
          providerId,
          modelId,
          options: { sampleRateHz: 16000 },
        })
        const sessionId = unwrapSessionId(raw)
        sessionIdRef.current = sessionId
        finalAccumulatorRef.current = ""
        sessionErrorRef.current = null
        subscribeSessionEvents(sessionId)
        logger.info(
          "voice",
          "useVoiceInput::start",
          `streaming session opened sessionId=${sessionId} kind=${kind}`,
        )
        await streamer.start((chunk) => {
          const id = sessionIdRef.current
          if (!id) return
          const base64 = pcm16ToBase64(chunk)
          // Fire-and-forget — chunk volume is high (~10/s) and individual
          // failures are surfaced via the EventBus session_error stream.
          void getTransport()
            .call("stt_push_chunk", { sessionId: id, base64 })
            .catch((e) => {
              logger.warn(
                "voice",
                "useVoiceInput::streamer",
                `push_chunk failed raw=${e instanceof Error ? e.message : String(e)}`,
              )
            })
        })
      } catch (e) {
        const m = e instanceof Error ? e.message : String(e)
        logger.error(
          "voice",
          "useVoiceInput::start",
          `streaming start failed raw=${m}`,
        )
        teardownSession()
        setMode(null)
        setErrorMessage(t("voice.failed"))
      }
    },
    [streamer, subscribeSessionEvents, teardownSession, t],
  )

  const start = useCallback(async () => {
    setErrorMessage(null)
    setPartialText("")
    // Pre-flight the active STT model + its provider kind so we can pick
    // the right path (batch vs streaming) without burning a mic-permission
    // prompt only to fail at transcribe time.
    let kind: SttProviderKind | null = null
    let active: { providerId: string; modelId: string } | null = null
    try {
      const meta = await fetchActiveProviderKind()
      active = meta.active
      kind = meta.kind
      if (!active) {
        logger.warn(
          "voice",
          "useVoiceInput::start",
          "preflight: no active STT model configured",
        )
        setErrorMessage(t("voice.noProvider"))
        return
      }
      logger.info(
        "voice",
        "useVoiceInput::start",
        `preflight ok: provider=${active.providerId} model=${active.modelId} kind=${kind ?? "?"}`,
      )
    } catch (e) {
      const m = e instanceof Error ? e.message : String(e)
      logger.warn("voice", "useVoiceInput::start", `preflight call failed raw=${m}`)
    }
    if (active && kind && STREAMING_KINDS.has(kind)) {
      await startStreaming(active.providerId, active.modelId, kind)
    } else {
      await startBatch()
    }
  }, [t, startStreaming, startBatch])

  const finalizeStreaming = useCallback(async (): Promise<string> => {
    const sessionId = sessionIdRef.current
    if (!sessionId) return ""
    setTranscribing(true)
    try {
      streamer.stop()
      const transcript = await getTransport().call<Transcript>(
        "stt_finalize_session",
        { sessionId },
      )
      setTranscribing(false)
      logger.info(
        "voice",
        "useVoiceInput::stopAndTranscribe",
        `streaming finalize ok: chars=${transcript?.text?.length ?? 0}`,
      )
      teardownSession()
      setMode(null)
      const text = (transcript?.text ?? "").trim()
      if (!text) {
        setErrorMessage(t("voice.empty"))
        return ""
      }
      return text
    } catch (e) {
      setTranscribing(false)
      const msg = e instanceof Error ? e.message : String(e)
      const code = msg.match(/stt:([a-z_]+):/)?.[1]
      logger.error(
        "voice",
        "useVoiceInput::stopAndTranscribe",
        `streaming finalize failed code=${code ?? "?"} raw=${msg}`,
      )
      teardownSession()
      setMode(null)
      if (code === "no_active_model") {
        setErrorMessage(t("voice.noProvider"))
      } else {
        setErrorMessage(t("voice.failed"))
      }
      return ""
    }
  }, [streamer, teardownSession, t])

  const stopAndTranscribeBatch = useCallback(async (): Promise<string> => {
    try {
      const { blob, mimeType } = await recorder.stop()
      logger.info(
        "voice",
        "useVoiceInput::stopAndTranscribe",
        `recorder stopped: bytes=${blob.size} mime=${mimeType}`,
      )
      if (!blob.size) {
        logger.warn(
          "voice",
          "useVoiceInput::stopAndTranscribe",
          "empty blob from recorder.stop — likely cancelled or zero-duration",
        )
        setErrorMessage(t("voice.failed"))
        return ""
      }
      setTranscribing(true)
      const base64 = await blobToBase64(blob)
      const transcript = await getTransport().call<Transcript>("stt_transcribe_blob", {
        mimeType,
        filename: deriveFilename(mimeType),
        base64,
        options: {},
      })
      setTranscribing(false)
      logger.info(
        "voice",
        "useVoiceInput::stopAndTranscribe",
        `transcribe ok: chars=${transcript?.text?.length ?? 0} lang=${transcript?.language ?? "?"} duration=${transcript?.durationMs ?? "?"}`,
      )
      if (!transcript || !transcript.text || !transcript.text.trim()) {
        setErrorMessage(t("voice.empty"))
        return ""
      }
      return transcript.text.trim()
    } catch (e) {
      setTranscribing(false)
      const msg = e instanceof Error ? e.message : String(e)
      const name = e instanceof Error ? e.name : "?"
      const code = msg.match(/stt:([a-z_]+):/)?.[1]
      logger.error(
        "voice",
        "useVoiceInput::stopAndTranscribe",
        `transcribe failed code=${code ?? "?"} name=${name} raw=${msg}`,
      )
      if (code === "no_active_model") {
        setErrorMessage(t("voice.noProvider"))
      } else if (msg.toLowerCase().includes("permission")) {
        setErrorMessage(t("voice.permissionDenied"))
      } else {
        setErrorMessage(t("voice.failed"))
      }
      return ""
    } finally {
      setMode(null)
    }
  }, [recorder, t])

  const stopAndTranscribe = useCallback(async (): Promise<string> => {
    if (mode === "streaming") return finalizeStreaming()
    return stopAndTranscribeBatch()
  }, [mode, finalizeStreaming, stopAndTranscribeBatch])

  const cancel = useCallback(() => {
    setErrorMessage(null)
    setPartialText("")
    if (mode === "streaming") {
      const sessionId = sessionIdRef.current
      streamer.cancel()
      if (sessionId) {
        void getTransport()
          .call("stt_cancel_session", { sessionId })
          .catch(() => {
            // best-effort; backend GC will reap idle sessions anyway
          })
      }
      teardownSession()
      setMode(null)
    } else {
      recorder.cancel()
      setMode(null)
    }
  }, [mode, streamer, recorder, teardownSession])

  const clearError = useCallback(() => setErrorMessage(null), [])

  // Map child-hook state back to the original `VoiceInputState` union so
  // VoiceRecordButton's existing switch keeps working unchanged.
  const baseState: RecorderState =
    mode === "streaming"
      ? streamer.state === "streaming"
        ? "recording"
        : streamer.state === "stopped"
          ? "stopped"
          : streamer.state === "requesting-permission"
            ? "requesting-permission"
            : streamer.state === "error"
              ? "error"
              : "idle"
      : recorder.state
  const state: VoiceInputState = transcribing
    ? "transcribing"
    : baseState === "stopped"
      ? "ready"
      : baseState

  // Surface getUserMedia denial via i18n.
  const surfacedError =
    mode === "streaming"
      ? streamer.error
      : recorder.error
  const surfacedErrorMessage =
    surfacedError && !errorMessage
      ? (() => {
          const name = (surfacedError as DOMException | Error).name ?? ""
          if (name === "NotAllowedError" || name === "SecurityError") {
            return t("voice.permissionDenied")
          }
          return t("voice.failed")
        })()
      : null

  // Always tear down the live session on unmount so a stale subscription
  // doesn't keep firing into a dead component.
  useEffect(() => {
    return () => {
      teardownSession()
    }
  }, [teardownSession])

  const durationMs = mode === "streaming" ? streamer.durationMs : recorder.durationMs
  const audioLevel = mode === "streaming" ? streamer.audioLevel : recorder.audioLevel

  return {
    state,
    durationMs,
    audioLevel,
    partialText,
    errorMessage: errorMessage ?? surfacedErrorMessage,
    start,
    stopAndTranscribe,
    cancel,
    clearError,
  }
}
