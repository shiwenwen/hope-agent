// @vitest-environment jsdom

import { act, cleanup, renderHook, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { useVoiceInput } from "./useVoiceInput"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => vi.fn()),
  loggerError: vi.fn(),
  loggerInfo: vi.fn(),
  loggerWarn: vi.fn(),
  recorderStart: vi.fn(),
  recorderStop: vi.fn(),
  recorderCancel: vi.fn(),
  streamerStart: vi.fn(),
  streamerStop: vi.fn(),
  streamerCancel: vi.fn(),
  providerKind: "azure-ws" as string,
  streamerState: "idle" as string,
  streamerError: null as Error | null,
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => ({ call: mocks.call, listen: mocks.listen }),
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: mocks.loggerError,
    info: mocks.loggerInfo,
    warn: mocks.loggerWarn,
  },
}))

vi.mock("@/hooks/useAudioRecorder", () => ({
  useAudioRecorder: () => ({
    state: "idle",
    durationMs: 0,
    audioLevel: 0,
    levels: [],
    error: null,
    start: mocks.recorderStart,
    stop: mocks.recorderStop,
    cancel: mocks.recorderCancel,
  }),
}))

vi.mock("@/hooks/usePcm16Streamer", () => ({
  usePcm16Streamer: () => ({
    state: mocks.streamerState,
    durationMs: 0,
    audioLevel: 0,
    levels: [],
    error: mocks.streamerError,
    start: mocks.streamerStart,
    stop: mocks.streamerStop,
    cancel: mocks.streamerCancel,
  }),
  pcm16ToBase64: vi.fn(() => "encoded"),
}))

beforeEach(() => {
  vi.clearAllMocks()
  mocks.call.mockImplementation(async (command: string) => {
    if (command === "get_stt_providers") {
      return [{ id: "azure", name: "Azure", kind: mocks.providerKind, enabled: true }]
    }
    if (command === "get_active_stt_model") {
      return { providerId: "azure", modelId: "speech" }
    }
    if (command === "stt_start_session") return "stt_opened"
    return undefined
  })
  mocks.recorderStart.mockResolvedValue(undefined)
  mocks.streamerStart.mockResolvedValue(undefined)
  mocks.providerKind = "azure-ws"
  mocks.streamerState = "idle"
  mocks.streamerError = null
})

afterEach(() => {
  cleanup()
})

describe("useVoiceInput streaming session lifecycle", () => {
  it("cancels an opened backend session when audio capture fails", async () => {
    mocks.streamerStart.mockRejectedValue(
      new DOMException("Unable to load the worklet module", "AbortError"),
    )
    const { result } = renderHook(() => useVoiceInput("chat-1"))

    await act(async () => {
      await result.current.start()
    })

    expect(mocks.call).toHaveBeenCalledWith("stt_cancel_session", {
      sessionId: "stt_opened",
    })
    await waitFor(() => expect(result.current.errorMessage).toBe("voice.failed"))
    expect(mocks.loggerError).toHaveBeenCalledWith(
      "voice",
      "useVoiceInput::start",
      expect.stringContaining("name=AbortError"),
    )
  })

  it("cancels a live backend session when the composer unmounts", async () => {
    const { result, unmount } = renderHook(() => useVoiceInput("chat-1"))

    await act(async () => {
      await result.current.start()
    })
    unmount()

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("stt_cancel_session", {
        sessionId: "stt_opened",
      })
    })
  })

  it("cancels a live backend session when the worklet fails after startup", async () => {
    const { result, rerender } = renderHook(() => useVoiceInput("chat-1"))

    await act(async () => {
      await result.current.start()
    })
    mocks.streamerState = "error"
    mocks.streamerError = new Error("processor failed")
    rerender()

    await waitFor(() => {
      expect(mocks.call).toHaveBeenCalledWith("stt_cancel_session", {
        sessionId: "stt_opened",
      })
      expect(result.current.errorMessage).toBe("voice.failed")
    })
  })

  it("surfaces batch microphone permission failures through the same contract", async () => {
    mocks.providerKind = "openai-transcriptions"
    mocks.recorderStart.mockRejectedValue(new DOMException("Permission denied", "NotAllowedError"))
    const { result } = renderHook(() => useVoiceInput("chat-1"))

    await act(async () => {
      await result.current.start()
    })

    await waitFor(() => {
      expect(result.current.errorMessage).toBe("voice.permissionDenied")
    })
    expect(mocks.call).not.toHaveBeenCalledWith("stt_start_session", expect.anything())
  })
})
