import { useCallback, useState } from "react"
import { useTranslation } from "react-i18next"

import { getTransport } from "@/lib/transport-provider"
import { useAudioRecorder } from "@/hooks/useAudioRecorder"
import type { RecorderState } from "@/hooks/useAudioRecorder"
import { logger } from "@/lib/logger"
import { unwrapActiveSttModel } from "@/lib/stt"

interface Transcript {
  text: string
  language?: string | null
  durationMs?: number | null
}

export type VoiceInputState =
  | RecorderState
  | "transcribing"
  | "ready"

export interface UseVoiceInputResult {
  state: VoiceInputState
  durationMs: number
  audioLevel: number
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
 * Composite hook: drives MediaRecorder + posts the final blob to
 * `stt_transcribe_blob`. Phase 4 ships the batch path (record-then-
 * transcribe); the streaming session API from Phase 2 is wired into
 * the hook in a later iteration.
 */
export function useVoiceInput(): UseVoiceInputResult {
  const { t } = useTranslation()
  const recorder = useAudioRecorder()
  const [transcribing, setTranscribing] = useState(false)
  const [errorMessage, setErrorMessage] = useState<string | null>(null)

  const start = useCallback(async () => {
    setErrorMessage(null)
    // Pre-flight the active STT model so we don't burn a mic-permission
    // prompt + recording session only to fail at the transcribe step.
    try {
      const raw = await getTransport().call<unknown>("get_active_stt_model", {})
      const normActive = unwrapActiveSttModel(raw, "activeModel")
      if (!normActive?.providerId || !normActive?.modelId) {
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
        `preflight ok: provider=${normActive.providerId} model=${normActive.modelId}`,
      )
    } catch (e) {
      // Pre-flight call itself failed (network / transport) — fall through
      // to the recorder; transcribe-time error handling will surface it.
      const m = e instanceof Error ? e.message : String(e)
      logger.warn("voice", "useVoiceInput::start", `preflight call failed raw=${m}`)
    }
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
      // recorder.error fills in
    }
  }, [recorder, t])

  const stopAndTranscribe = useCallback(async (): Promise<string> => {
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
      // Backend `SttError::Display` always emits `stt:<code>: <body>`.
      // HTTP transport may wrap that inside `[HttpTransport] POST ... 400:
      // {"error":"stt:..."}` so scan anywhere in the message, not anchored.
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
    }
  }, [recorder, t])

  const cancel = useCallback(() => {
    setErrorMessage(null)
    recorder.cancel()
  }, [recorder])

  const clearError = useCallback(() => setErrorMessage(null), [])

  const state: VoiceInputState = transcribing
    ? "transcribing"
    : recorder.state === "stopped"
      ? "ready"
      : recorder.state

  // Surface getUserMedia denial via i18n.
  const surfacedRecorderError =
    recorder.error && !errorMessage
      ? (() => {
          const name = (recorder.error as DOMException | Error).name ?? ""
          if (name === "NotAllowedError" || name === "SecurityError") {
            return t("voice.permissionDenied")
          }
          return t("voice.failed")
        })()
      : null

  return {
    state,
    durationMs: recorder.durationMs,
    audioLevel: recorder.audioLevel,
    errorMessage: errorMessage ?? surfacedRecorderError,
    start,
    stopAndTranscribe,
    cancel,
    clearError,
  }
}
