// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, expect, it, vi } from "vitest"
import MissingModelDialog from "./MissingModelDialog"
import { LOCAL_MODEL_ALERT_EVENT, type LocalModelMissingAlert } from "@/types/local-model-jobs"

const mocks = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
  openExternalUrl: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}))
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => mocks }))
vi.mock("@/lib/openExternalUrl", () => ({ openExternalUrl: mocks.openExternalUrl }))
vi.mock("sonner", () => ({ toast: { success: mocks.success, error: mocks.error } }))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }))

afterEach(() => { cleanup(); vi.resetAllMocks() })

it.each(["chat", "embedding"] as const)("keeps the %s alert open after redirecting to install Ollama", async (kind) => {
  let onAlert: ((raw: unknown) => void) | undefined
  mocks.listen.mockImplementation((...args: unknown[]) => {
    if (args[0] === LOCAL_MODEL_ALERT_EVENT) onAlert = args[1] as typeof onAlert
    return () => {}
  })
  mocks.call.mockImplementation(async (command: string) => {
    if (command === "local_llm_detect_ollama") return { phase: "not-installed", installScriptSupported: false }
    throw new Error(`Unexpected command: ${command}`)
  })
  render(<MissingModelDialog />)
  const alert: LocalModelMissingAlert = {
    kind, missingModelId: "local-test", missingDisplayName: "Local test",
    alternatives: [], canRedownload: true, canDisableEmbedding: false,
  }
  act(() => onAlert?.(alert))
  fireEvent.click(screen.getByRole("button", { name: /redownload/i }))
  await waitFor(() => expect(mocks.openExternalUrl).toHaveBeenCalledWith("https://ollama.com/download"))
  expect(mocks.call).toHaveBeenCalledExactlyOnceWith("local_llm_detect_ollama")
  expect(mocks.success).not.toHaveBeenCalled()
  expect(mocks.error).not.toHaveBeenCalled()
  expect(screen.getByRole("dialog")).toBeTruthy()
})
