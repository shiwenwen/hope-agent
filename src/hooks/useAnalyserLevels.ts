import { useCallback, useEffect, useRef, useState } from "react"

/**
 * Shared audio-level + history hook for the two recorder hooks
 * (`useAudioRecorder` for batch / `usePcm16Streamer` for WS streaming).
 * Owns the RAF RMS sampler and the rolling 50 ms history ring; consumers
 * just hand it an `AnalyserNode` ref.
 *
 * Returns:
 * - `audioLevel`: latest RMS (0-1), throttled — updates only when the
 *   level moves by `LEVEL_DELTA` from the last published value. Drives
 *   the recording-dot pulse / mic LED.
 * - `levels`: rolling history (LEVELS_HISTORY_SIZE samples, oldest
 *   first). Drives the waveform UI.
 * - `start()`: begin sampling the analyser (must be called once it's
 *   wired up).
 * - `stop()`: cancel sampling, reset state.
 */

export const LEVELS_HISTORY_SIZE = 48
const LEVELS_TICK_MS = 50
const LEVEL_DELTA = 0.02
const ZERO_LEVELS = Object.freeze(new Array<number>(LEVELS_HISTORY_SIZE).fill(0))

export interface UseAnalyserLevelsResult {
  audioLevel: number
  levels: number[]
  start: () => void
  stop: () => void
}

export function useAnalyserLevels(
  analyserRef: React.RefObject<AnalyserNode | null>,
): UseAnalyserLevelsResult {
  const [audioLevel, setAudioLevel] = useState(0)
  const [levels, setLevels] = useState<number[]>(() => ZERO_LEVELS.slice())

  const rafRef = useRef<number | null>(null)
  const tickRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const lastEmittedLevelRef = useRef<number>(0)
  const latestLevelRef = useRef<number>(0)
  const levelsRef = useRef<number[]>(ZERO_LEVELS.slice())
  /** Trailing-zero counter so the silence short-circuit doesn't have to
   * scan the ring each tick. Increments when we push 0, resets to 0
   * when we push a non-zero value. When it exceeds LEVELS_HISTORY_SIZE
   * the entire visible ring is zeros and we can stop re-rendering. */
  const trailingZerosRef = useRef<number>(LEVELS_HISTORY_SIZE)

  const stop = useCallback(() => {
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current)
    rafRef.current = null
    if (tickRef.current !== null) clearInterval(tickRef.current)
    tickRef.current = null
    latestLevelRef.current = 0
    lastEmittedLevelRef.current = 0
    trailingZerosRef.current = LEVELS_HISTORY_SIZE
    const fresh = ZERO_LEVELS.slice()
    levelsRef.current = fresh
    setLevels(fresh)
    setAudioLevel(0)
  }, [])

  const start = useCallback(() => {
    const loop = () => {
      const analyser = analyserRef.current
      if (!analyser) return
      const buf = new Uint8Array(analyser.frequencyBinCount)
      analyser.getByteTimeDomainData(buf)
      let sum = 0
      for (let i = 0; i < buf.length; i++) {
        const v = (buf[i] - 128) / 128
        sum += v * v
      }
      const rms = Math.sqrt(sum / buf.length)
      const next = Math.min(1, rms * 2.5)
      latestLevelRef.current = next
      if (Math.abs(next - lastEmittedLevelRef.current) >= LEVEL_DELTA) {
        lastEmittedLevelRef.current = next
        setAudioLevel(next)
      }
      rafRef.current = requestAnimationFrame(loop)
    }
    loop()
    // 50 ms tick samples the latest RMS into the rolling history. 20 Hz
    // is the right rate for the waveform — fast enough to feel live,
    // far below RAF cost. When the ring is fully zeroed and the new
    // sample is also zero, skip the state update so consumers don't
    // re-render through silence.
    tickRef.current = setInterval(() => {
      const ring = levelsRef.current
      const next = latestLevelRef.current
      ring.shift()
      ring.push(next)
      if (next === 0) {
        trailingZerosRef.current = Math.min(
          LEVELS_HISTORY_SIZE,
          trailingZerosRef.current + 1,
        )
        if (trailingZerosRef.current >= LEVELS_HISTORY_SIZE) return
      } else {
        trailingZerosRef.current = 0
      }
      setLevels(ring.slice())
    }, LEVELS_TICK_MS)
  }, [analyserRef])

  useEffect(() => () => stop(), [stop])

  return { audioLevel, levels, start, stop }
}
