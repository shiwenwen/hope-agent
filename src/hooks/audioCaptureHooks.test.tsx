// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useAudioRecorder } from "./useAudioRecorder"
import { usePcm16Streamer } from "./usePcm16Streamer"

const mocks = vi.hoisted(() => ({
  loggerError: vi.fn(),
  startLevels: vi.fn(),
  stopLevels: vi.fn(),
}))

vi.mock("@/lib/logger", () => ({
  logger: { error: mocks.loggerError },
}))

vi.mock("./useAnalyserLevels", () => ({
  useAnalyserLevels: () => ({
    audioLevel: 0,
    levels: [],
    start: mocks.startLevels,
    stop: mocks.stopLevels,
  }),
}))

function setMediaDevices(getUserMedia: ReturnType<typeof vi.fn>) {
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: { getUserMedia },
  })
}

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  Reflect.deleteProperty(navigator, "mediaDevices")
})

beforeEach(() => {
  vi.clearAllMocks()
})

describe("audio capture start failures", () => {
  it("loads the PCM worklet from a same-origin asset and rejects when loading fails", async () => {
    const stopTrack = vi.fn()
    setMediaDevices(
      vi.fn().mockResolvedValue({
        getTracks: () => [{ stop: stopTrack }],
      }),
    )

    const workletError = new DOMException("Unable to load the worklet module", "AbortError")
    const addModule = vi.fn().mockRejectedValue(workletError)
    const close = vi.fn().mockResolvedValue(undefined)
    const audioContext = {
      state: "running",
      audioWorklet: { addModule },
      close,
    }
    function AudioContextMock() {
      return audioContext
    }
    vi.stubGlobal("AudioContext", AudioContextMock)

    const { result } = renderHook(() => usePcm16Streamer())
    let rejected: unknown
    await act(async () => {
      try {
        await result.current.start(vi.fn())
      } catch (error) {
        rejected = error
      }
    })

    expect(addModule).toHaveBeenCalledWith("/pcm16-downsampler.worklet.js")
    expect(rejected).toMatchObject({ name: "AbortError" })
    await waitFor(() => expect(result.current.state).toBe("error"))
    expect(result.current.error).toMatchObject({ name: "AbortError" })
    expect(stopTrack).toHaveBeenCalledOnce()
    expect(close).toHaveBeenCalledOnce()
    expect(mocks.loggerError).toHaveBeenCalledWith(
      "voice",
      "usePcm16Streamer::start",
      expect.stringContaining("name=AbortError"),
    )
  })

  it("makes batch recorder setup failures observable to its caller", async () => {
    const permissionError = new DOMException("Permission denied", "NotAllowedError")
    setMediaDevices(vi.fn().mockRejectedValue(permissionError))

    const { result } = renderHook(() => useAudioRecorder())
    let rejected: unknown
    await act(async () => {
      try {
        await result.current.start()
      } catch (error) {
        rejected = error
      }
    })

    expect(rejected).toMatchObject({ name: "NotAllowedError" })
    await waitFor(() => expect(result.current.state).toBe("error"))
    expect(result.current.error).toMatchObject({ name: "NotAllowedError" })
    expect(mocks.loggerError).toHaveBeenCalledWith(
      "voice",
      "useAudioRecorder::start",
      expect.stringContaining("name=NotAllowedError"),
    )
  })

  it("surfaces and cleans up AudioWorklet processor failures after startup", async () => {
    const stopTrack = vi.fn()
    setMediaDevices(
      vi.fn().mockResolvedValue({
        getTracks: () => [{ stop: stopTrack }],
      }),
    )

    const source = { connect: vi.fn(), disconnect: vi.fn() }
    const analyser = { fftSize: 0, disconnect: vi.fn() }
    const audioContext = {
      state: "running",
      audioWorklet: { addModule: vi.fn().mockResolvedValue(undefined) },
      createMediaStreamSource: vi.fn(() => source),
      createAnalyser: vi.fn(() => analyser),
      close: vi.fn().mockResolvedValue(undefined),
    }
    function AudioContextMock() {
      return audioContext
    }
    const worklet = {
      port: { onmessage: null, close: vi.fn() },
      onprocessorerror: null as (() => void) | null,
      disconnect: vi.fn(),
    }
    function AudioWorkletNodeMock() {
      return worklet
    }
    vi.stubGlobal("AudioContext", AudioContextMock)
    vi.stubGlobal("AudioWorkletNode", AudioWorkletNodeMock)

    const { result } = renderHook(() => usePcm16Streamer())
    await act(async () => {
      await result.current.start(vi.fn())
    })
    expect(result.current.state).toBe("streaming")

    act(() => worklet.onprocessorerror?.())

    await waitFor(() => expect(result.current.state).toBe("error"))
    expect(result.current.error).toMatchObject({ name: "AudioWorkletProcessorError" })
    expect(stopTrack).toHaveBeenCalledOnce()
    expect(mocks.loggerError).toHaveBeenCalledWith(
      "voice",
      "usePcm16Streamer::processor",
      expect.stringContaining("name=AudioWorkletProcessorError"),
    )
  })

  it("logs and cleans up MediaRecorder runtime failures", async () => {
    const stopTrack = vi.fn()
    setMediaDevices(
      vi.fn().mockResolvedValue({
        getTracks: () => [{ stop: stopTrack }],
      }),
    )

    const recorder = {
      state: "inactive",
      mimeType: "audio/webm",
      ondataavailable: null,
      onstop: null,
      onerror: null as ((event: { error: Error }) => void) | null,
      start: vi.fn(() => {
        recorder.state = "recording"
      }),
      stop: vi.fn(),
    }
    function MediaRecorderMock() {
      return recorder
    }
    MediaRecorderMock.isTypeSupported = vi.fn(() => true)
    vi.stubGlobal("MediaRecorder", MediaRecorderMock)

    const { result } = renderHook(() => useAudioRecorder())
    await act(async () => {
      await result.current.start()
    })
    expect(result.current.state).toBe("recording")

    act(() => recorder.onerror?.({ error: new Error("device disconnected") }))

    await waitFor(() => expect(result.current.state).toBe("error"))
    expect(stopTrack).toHaveBeenCalledOnce()
    expect(mocks.loggerError).toHaveBeenCalledWith(
      "voice",
      "useAudioRecorder::recording",
      expect.stringContaining("device disconnected"),
    )
  })
})
