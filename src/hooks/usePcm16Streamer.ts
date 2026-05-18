import { useCallback, useEffect, useRef, useState } from "react"

import { useAnalyserLevels } from "./useAnalyserLevels"

/**
 * Streaming audio capture hook for the STT WS providers.
 *
 * Mic → AudioContext → AudioWorkletNode → onChunk(Int16Array).
 *
 * Every WS STT provider hope-agent supports (Deepgram / AssemblyAI / Azure
 * Speech / Volcengine / iFlytek) consumes 16 kHz mono PCM16 little-endian
 * frames. The worklet downsamples whatever rate the OS hands us (44.1 kHz /
 * 48 kHz on most hardware) to 16 kHz with the simplest possible decimator
 * — adequate for speech because the upstream ASR re-resamples anyway.
 *
 * Chunks are emitted ~100 ms apart (1600 samples / 3200 bytes), small
 * enough for sub-second perceived partial latency and large enough that
 * the IPC overhead is amortised.
 */

export type Pcm16StreamerState =
  | "idle"
  | "requesting-permission"
  | "streaming"
  | "stopped"
  | "error"

export interface UsePcm16StreamerResult {
  state: Pcm16StreamerState
  durationMs: number
  audioLevel: number
  /** Rolling RMS history (48 bins, ~50 ms each) for the waveform UI.
   * Oldest first; newest at the end. Zero-padded when not streaming. */
  levels: number[]
  error: Error | null
  /** Begin streaming. `onChunk` receives each 100 ms PCM16 frame. */
  start: (onChunk: (chunk: Int16Array) => void) => Promise<void>
  /** Stop the worklet + tear down resources. Idempotent. */
  stop: () => void
  /** Same as `stop` but signals "discard the session" semantically. */
  cancel: () => void
}


// 16 kHz target rate / 1600 sample-per-frame contract is hard-coded into
// the worklet processor (sample rates are immutable once the AudioContext
// is created, so there's no reason to template these on the TS side).
const MAX_RECORD_MS = 5 * 60 * 1000

// AudioWorklet processor that downsamples to 16 kHz mono and emits PCM16
// frames of `FRAME_SAMPLES` samples each. Lives in a separate global scope
// (worklet thread) — `sampleRate` and `registerProcessor` are worklet
// globals so we ship this as a string and load via Blob URL.
const WORKLET_SOURCE = `
class Pcm16Downsampler extends AudioWorkletProcessor {
  constructor() {
    super()
    this.targetRate = 16000
    this.frameSamples = 1600
    this.buffer = new Int16Array(this.frameSamples)
    this.write = 0
    this.sourceCursor = 0
    this.step = sampleRate / this.targetRate
  }
  process(inputs) {
    const input = inputs[0]
    if (!input || !input[0]) return true
    const ch = input[0]
    for (let i = 0; i < ch.length; i++) {
      this.sourceCursor += 1
      if (this.sourceCursor >= this.step) {
        this.sourceCursor -= this.step
        const f = Math.max(-1, Math.min(1, ch[i]))
        this.buffer[this.write++] = f < 0 ? f * 0x8000 : f * 0x7FFF
        if (this.write >= this.frameSamples) {
          this.port.postMessage(this.buffer.slice(0))
          this.write = 0
        }
      }
    }
    return true
  }
}
registerProcessor("pcm16-downsampler", Pcm16Downsampler)
`

export function usePcm16Streamer(): UsePcm16StreamerResult {
  const [state, setState] = useState<Pcm16StreamerState>("idle")
  const [durationMs, setDurationMs] = useState(0)
  const [error, setError] = useState<Error | null>(null)

  const streamRef = useRef<MediaStream | null>(null)
  const audioCtxRef = useRef<AudioContext | null>(null)
  const sourceRef = useRef<MediaStreamAudioSourceNode | null>(null)
  const workletRef = useRef<AudioWorkletNode | null>(null)
  const analyserRef = useRef<AnalyserNode | null>(null)
  const { audioLevel, levels, start: startLevels, stop: stopLevels } =
    useAnalyserLevels(analyserRef)
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const startedAtRef = useRef<number>(0)
  const workletBlobUrlRef = useRef<string | null>(null)

  const cleanup = useCallback(() => {
    if (intervalRef.current !== null) clearInterval(intervalRef.current)
    intervalRef.current = null
    stopLevels()
    try {
      workletRef.current?.disconnect()
      workletRef.current?.port.close()
    } catch {
      // ignore
    }
    workletRef.current = null
    try {
      analyserRef.current?.disconnect()
    } catch {
      // ignore
    }
    analyserRef.current = null
    try {
      sourceRef.current?.disconnect()
    } catch {
      // ignore
    }
    sourceRef.current = null
    if (audioCtxRef.current && audioCtxRef.current.state !== "closed") {
      void audioCtxRef.current.close().catch(() => {})
    }
    audioCtxRef.current = null
    streamRef.current?.getTracks().forEach((t) => t.stop())
    streamRef.current = null
    if (workletBlobUrlRef.current) {
      URL.revokeObjectURL(workletBlobUrlRef.current)
      workletBlobUrlRef.current = null
    }
  }, [stopLevels])

  const start = useCallback(
    async (onChunk: (chunk: Int16Array) => void) => {
      if (state === "streaming" || state === "requesting-permission") return
      setError(null)
      setState("requesting-permission")
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: {
            echoCancellation: true,
            noiseSuppression: true,
            channelCount: 1,
          },
        })
        streamRef.current = stream

        const AudioCtxCtor: typeof AudioContext | undefined =
          (window as unknown as { AudioContext?: typeof AudioContext }).AudioContext ??
          (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
        if (!AudioCtxCtor) throw new Error("AudioContext not supported in this browser")
        const ctx = new AudioCtxCtor()
        audioCtxRef.current = ctx

        const workletBlob = new Blob([WORKLET_SOURCE], { type: "application/javascript" })
        const workletUrl = URL.createObjectURL(workletBlob)
        workletBlobUrlRef.current = workletUrl
        await ctx.audioWorklet.addModule(workletUrl)

        const source = ctx.createMediaStreamSource(stream)
        sourceRef.current = source

        const analyser = ctx.createAnalyser()
        analyser.fftSize = 1024
        analyserRef.current = analyser

        const worklet = new AudioWorkletNode(ctx, "pcm16-downsampler")
        worklet.port.onmessage = (e) => {
          // Worklet posts Int16Array slices; pass straight through.
          const data = e.data as Int16Array | undefined
          if (data && data.length > 0) onChunk(data)
        }
        workletRef.current = worklet

        // Source feeds both the analyser (level meter, no downsample) and
        // the worklet (downsamples + emits PCM16). Worklet output is NOT
        // connected to the destination — we only want the messages.
        source.connect(analyser)
        source.connect(worklet)

        startedAtRef.current = Date.now()
        setDurationMs(0)
        intervalRef.current = setInterval(() => {
          const elapsed = Date.now() - startedAtRef.current
          setDurationMs(elapsed)
          if (elapsed >= MAX_RECORD_MS) {
            // Hard cap: tear everything down. Earlier behaviour only
            // disconnected the worklet, leaving the mic track, audio
            // context, analyser interval, and `state === "streaming"`
            // alive — UI stayed in recording mode while the upstream
            // STT session leaked. Caller observes the state flip and
            // is expected to finalize / cancel its session.
            cleanup()
            setState("stopped")
          }
        }, 100)
        startLevels()
        setState("streaming")
      } catch (e) {
        cleanup()
        setError(e instanceof Error ? e : new Error(String(e)))
        setState("error")
      }
    },
    [state, cleanup, startLevels],
  )

  const stop = useCallback(() => {
    cleanup()
    setState("stopped")
  }, [cleanup])

  const cancel = useCallback(() => {
    cleanup()
    setState("idle")
  }, [cleanup])

  useEffect(() => {
    return () => {
      cleanup()
    }
  }, [cleanup])

  return { state, durationMs, audioLevel, levels, error, start, stop, cancel }
}

/** Pack an `Int16Array` PCM16 frame into a base64 string for transport. */
export function pcm16ToBase64(chunk: Int16Array): string {
  // Build a `Uint8Array` view over the same buffer to feed btoa via String.
  const bytes = new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength)
  // Chunked btoa avoids the call-stack limit on long byte arrays (~100k+).
  let binary = ""
  const CHUNK = 0x8000
  for (let i = 0; i < bytes.length; i += CHUNK) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + CHUNK)))
  }
  return btoa(binary)
}
