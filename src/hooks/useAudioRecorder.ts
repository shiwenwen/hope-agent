import { useCallback, useEffect, useRef, useState } from "react"

import { useAnalyserLevels } from "./useAnalyserLevels"

/**
 * Audio recorder hook backed by MediaRecorder + AnalyserNode.
 *
 * Returns a state machine: `idle → requesting-permission → recording →
 * stopped | error`. `cancel` discards the buffered audio; `stop` resolves
 * with the final Blob so the caller can ship it to STT. `audioLevel`
 * (0-1 RMS) feeds the on-screen waveform / red-dot pulse.
 */
export type RecorderState =
  | "idle"
  | "requesting-permission"
  | "recording"
  | "stopped"
  | "error"

export interface UseAudioRecorderResult {
  state: RecorderState
  durationMs: number
  audioLevel: number
  /** Rolling RMS history (48 bins, ~50 ms each) for the waveform UI.
   * Oldest first; newest at the end. Zero-padded when not recording. */
  levels: number[]
  error: Error | null
  /** Begin a new recording. Throws via `error` state if permission is denied. */
  start: () => Promise<void>
  /** Stop and return the recorded Blob. */
  stop: () => Promise<{ blob: Blob; mimeType: string; durationMs: number }>
  /** Discard the current recording and reset to idle. */
  cancel: () => void
}

/** Auto-stop after 5 minutes — guards against forgotten / runaway recordings
 * accumulating chunks in memory indefinitely. */
const MAX_RECORD_MS = 5 * 60 * 1000

/** Preferred MediaRecorder MIME types in order of compatibility. */
function pickMimeType(): string {
  if (typeof window === "undefined" || typeof MediaRecorder === "undefined") return ""
  const candidates = [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/ogg;codecs=opus",
    "audio/mp4",
    "audio/mpeg",
  ]
  for (const m of candidates) {
    if (MediaRecorder.isTypeSupported?.(m)) return m
  }
  return ""
}

export function useAudioRecorder(): UseAudioRecorderResult {
  const [state, setState] = useState<RecorderState>("idle")
  const [durationMs, setDurationMs] = useState(0)
  const [error, setError] = useState<Error | null>(null)

  const recorderRef = useRef<MediaRecorder | null>(null)
  const chunksRef = useRef<Blob[]>([])
  const streamRef = useRef<MediaStream | null>(null)
  const audioCtxRef = useRef<AudioContext | null>(null)
  const analyserRef = useRef<AnalyserNode | null>(null)
  const { audioLevel, levels, start: startLevels, stop: stopLevels } =
    useAnalyserLevels(analyserRef)
  const startedAtRef = useRef<number>(0)
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const cancelledRef = useRef<boolean>(false)
  /** Final recording result, populated by `onstop` whether the stop was
   * user-triggered or watchdog-triggered. `stop()` returns it on the
   * already-stopped path so an auto-stop never silently discards audio. */
  const lastResultRef = useRef<{
    blob: Blob
    mimeType: string
    durationMs: number
  } | null>(null)
  const stopPromiseRef = useRef<{
    resolve: (v: { blob: Blob; mimeType: string; durationMs: number }) => void
    reject: (e: Error) => void
  } | null>(null)

  const cleanup = useCallback(() => {
    if (intervalRef.current !== null) clearInterval(intervalRef.current)
    intervalRef.current = null
    stopLevels()
    try {
      analyserRef.current?.disconnect()
    } catch {
      // ignore
    }
    analyserRef.current = null
    if (audioCtxRef.current && audioCtxRef.current.state !== "closed") {
      void audioCtxRef.current.close().catch(() => {})
    }
    audioCtxRef.current = null
    streamRef.current?.getTracks().forEach((t) => t.stop())
    streamRef.current = null
    recorderRef.current = null
  }, [stopLevels])

  const start = useCallback(async () => {
    if (state === "recording" || state === "requesting-permission") return
    setError(null)
    setState("requesting-permission")
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      streamRef.current = stream

      const mimeType = pickMimeType()
      const recorder = mimeType
        ? new MediaRecorder(stream, { mimeType })
        : new MediaRecorder(stream)
      recorderRef.current = recorder
      chunksRef.current = []

      cancelledRef.current = false
      lastResultRef.current = null
      recorder.ondataavailable = (e) => {
        if (e.data && e.data.size > 0) chunksRef.current.push(e.data)
      }
      recorder.onstop = () => {
        if (cancelledRef.current) {
          chunksRef.current = []
          cleanup()
          setState("idle")
          return
        }
        const finalMime =
          recorder.mimeType || mimeType || (chunksRef.current[0]?.type ?? "audio/webm")
        const blob = new Blob(chunksRef.current, { type: finalMime })
        const finalDuration = Date.now() - startedAtRef.current
        chunksRef.current = []
        const result = { blob, mimeType: finalMime, durationMs: finalDuration }
        cleanup()
        setState("stopped")
        if (stopPromiseRef.current) {
          stopPromiseRef.current.resolve(result)
          stopPromiseRef.current = null
        } else {
          // Watchdog-initiated stop (no awaiting caller) — cache so the
          // next stop() call can drain it.
          lastResultRef.current = result
        }
      }
      recorder.onerror = (e) => {
        const err = (e as ErrorEvent).error ?? new Error("MediaRecorder error")
        setError(err instanceof Error ? err : new Error(String(err)))
        cleanup()
        setState("error")
        stopPromiseRef.current?.reject(err instanceof Error ? err : new Error(String(err)))
        stopPromiseRef.current = null
      }

      const AudioCtxCtor: typeof AudioContext | undefined =
        (window as unknown as { AudioContext?: typeof AudioContext }).AudioContext ??
        (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
      if (AudioCtxCtor) {
        const ctx = new AudioCtxCtor()
        audioCtxRef.current = ctx
        const source = ctx.createMediaStreamSource(stream)
        const analyser = ctx.createAnalyser()
        analyser.fftSize = 1024
        source.connect(analyser)
        analyserRef.current = analyser
        startLevels()
      }

      recorder.start(1000)
      startedAtRef.current = Date.now()
      setDurationMs(0)
      intervalRef.current = setInterval(() => {
        const elapsed = Date.now() - startedAtRef.current
        setDurationMs(elapsed)
        if (elapsed >= MAX_RECORD_MS && recorderRef.current?.state === "recording") {
          recorderRef.current.stop()
        }
      }, 100)
      setState("recording")
    } catch (e) {
      cleanup()
      setError(e instanceof Error ? e : new Error(String(e)))
      setState("error")
    }
  }, [state, cleanup, startLevels])

  const stop = useCallback(() => {
    return new Promise<{ blob: Blob; mimeType: string; durationMs: number }>(
      (resolve, reject) => {
        // Watchdog (MAX_RECORD_MS) may have already stopped the recorder
        // and cached its result. Drain that first so the audio isn't lost.
        if (lastResultRef.current) {
          const cached = lastResultRef.current
          lastResultRef.current = null
          resolve(cached)
          return
        }
        const recorder = recorderRef.current
        if (!recorder || recorder.state === "inactive") {
          reject(new Error("Not recording"))
          return
        }
        stopPromiseRef.current = { resolve, reject }
        recorder.stop()
      },
    )
  }, [])

  const cancel = useCallback(() => {
    lastResultRef.current = null
    const recorder = recorderRef.current
    if (recorder && recorder.state !== "inactive") {
      cancelledRef.current = true
      stopPromiseRef.current = null
      recorder.stop()
    } else {
      cleanup()
      setState("idle")
    }
  }, [cleanup])

  // Tear down on unmount.
  useEffect(() => {
    return () => {
      stopPromiseRef.current = null
      lastResultRef.current = null
      cleanup()
    }
  }, [cleanup])

  return { state, durationMs, audioLevel, levels, error, start, stop, cancel }
}
