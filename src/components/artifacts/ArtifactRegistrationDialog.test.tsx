// @vitest-environment jsdom
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { ArtifactRegistrationDialog } from "./ArtifactRegistrationDialog"
import { ArtifactLifecycle } from "./ArtifactLifecycle"
import type { ArtifactRecord } from "@/lib/transport"

const mocks = vi.hoisted(() => ({
  current: { importArtifact: vi.fn(), uploadFile: vi.fn(), discardFileUpload: vi.fn() },
  t: (key: string) => key,
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => mocks.current,
  useTransport: () => mocks.current,
}))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: mocks.t }) }))

const artifact: ArtifactRecord = {
  id: "artifact-A",
  title: "Report",
  kind: "report",
  contentType: "html",
  lifecycleState: "active",
  privacy: "sensitive",
  currentVersion: 5,
  currentHash: "a".repeat(64),
  payloadKind: "freeform",
  sourceCount: 0,
  sourceSummaries: [],
  evidenceSummary: {},
  capabilities: {},
  projectPath: "/managed/artifact-A",
  createdAt: "2026-10-01T00:00:00Z",
  updatedAt: "2026-10-19T00:00:00Z",
}

beforeEach(() => {
  vi.useFakeTimers()
  mocks.current = {
    importArtifact: vi.fn().mockResolvedValue(artifact),
    uploadFile: vi.fn().mockResolvedValue({ uploadId: "lease-local-file" }),
    discardFileUpload: vi.fn().mockResolvedValue(undefined),
  }
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

test.each(["/workspace/report.md", "C:\\workspace\\report.html", "\\\\server\\share\\report.md"])(
  "explicitly registers a new private snapshot from an absolute runtime-host path %s",
  async (path) => {
    const imported = vi.fn(),
      close = vi.fn()
    render(<ArtifactRegistrationDialog onImported={imported} onClose={close} />)
    expect(mocks.current.importArtifact).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "artifacts.registration.create" })).toBeDisabled()
    fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
    fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
    fireEvent.change(screen.getByRole("textbox", { name: "artifacts.registration.path" }), {
      target: { value: path },
    })
    await act(async () =>
      fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" })),
    )
    expect(mocks.current.importArtifact).toHaveBeenCalledExactlyOnceWith({
      filePath: path,
      privacy: "local_private",
    })
    expect(imported).toHaveBeenCalledExactlyOnceWith(artifact)
    expect(screen.getByRole("button", { name: "common.saved" })).toBeDisabled()
    expect(close).not.toHaveBeenCalled()
    act(() => vi.advanceTimersByTime(1999))
    expect(close).not.toHaveBeenCalled()
    act(() => vi.advanceTimersByTime(1))
    expect(close).toHaveBeenCalledOnce()
  },
)

test("an update uses the selected ID and exact version without changing privacy or guessing by name", async () => {
  const before = structuredClone(artifact)
  render(<ArtifactRegistrationDialog artifact={artifact} onImported={vi.fn()} onClose={vi.fn()} />)
  fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "/workspace/same-title.md" } })
  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.update" })),
  )
  expect(mocks.current.importArtifact).toHaveBeenCalledExactlyOnceWith({
    filePath: "/workspace/same-title.md",
    artifactId: "artifact-A",
    expectedVersion: 5,
  })
  expect(artifact).toEqual(before)
})

test("relative paths and cancellation dispatch no import", () => {
  const close = vi.fn()
  render(<ArtifactRegistrationDialog onImported={vi.fn()} onClose={close} />)
  fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "relative/report.md" } })
  expect(screen.getByRole("button", { name: "artifacts.registration.create" })).toBeDisabled()
  fireEvent.click(screen.getByRole("button", { name: "common.cancel" }))
  expect(close).toHaveBeenCalledOnce()
  expect(mocks.current.importArtifact).not.toHaveBeenCalled()
})

test("a conflict preserves the draft, shows failure for two seconds and never retries automatically", async () => {
  mocks.current.importArtifact.mockRejectedValue(
    new Error("artifact version conflict: synthetic-private-error"),
  )
  const imported = vi.fn()
  render(<ArtifactRegistrationDialog artifact={artifact} onImported={imported} onClose={vi.fn()} />)
  fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "/workspace/revised.md" } })
  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.update" })),
  )
  expect(screen.getByRole("alert")).toHaveTextContent("artifacts.registration.failed")
  expect(screen.queryByText(/synthetic-private-error/)).toBeNull()
  expect(screen.getByRole("button", { name: "common.retry" })).toBeDisabled()
  expect(imported).not.toHaveBeenCalled()
  act(() => vi.advanceTimersByTime(2000))
  expect(screen.getByDisplayValue("/workspace/revised.md")).toBeTruthy()
  expect(screen.getByRole("button", { name: "artifacts.registration.update" })).not.toBeDisabled()
  expect(mocks.current.importArtifact).toHaveBeenCalledOnce()
})

test("a pending write cannot be dismissed or sent twice", async () => {
  let finish!: (value: ArtifactRecord) => void
  mocks.current.importArtifact.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const close = vi.fn()
  render(<ArtifactRegistrationDialog onImported={vi.fn()} onClose={close} />)
  fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "/workspace/report.md" } })
  fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" }))
  expect(screen.getByRole("button", { name: "common.cancel" })).toBeDisabled()
  expect(screen.getByRole("button", { name: "common.saving" })).toBeDisabled()
  fireEvent.keyDown(document, { key: "Escape" })
  expect(close).not.toHaveBeenCalled()
  expect(mocks.current.importArtifact).toHaveBeenCalledOnce()
  await act(async () => finish(artifact))
})

test("a host switch prevents an old-host result from updating the new gallery", async () => {
  let finish!: (value: ArtifactRecord) => void
  const old = mocks.current
  old.importArtifact.mockReturnValue(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const imported = vi.fn(),
    close = vi.fn()
  const view = render(<ArtifactRegistrationDialog onImported={imported} onClose={close} />)
  fireEvent.click(screen.getByRole("radio", { name: "artifacts.registration.pathSource" }))
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "/workspace/report.md" } })
  fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" }))
  mocks.current = { importArtifact: vi.fn(), uploadFile: vi.fn(), discardFileUpload: vi.fn() }
  view.rerender(<ArtifactRegistrationDialog onImported={imported} onClose={close} />)
  await act(async () => finish(artifact))
  expect(screen.getByRole("alert")).toHaveTextContent("artifacts.registration.hostChanged")
  expect(imported).not.toHaveBeenCalled()
  expect(mocks.current.importArtifact).not.toHaveBeenCalled()
})

test("lifecycle labels distinguish an initial snapshot from later versions independently of identical dates", () => {
  const view = render(<ArtifactLifecycle artifact={{ ...artifact, currentVersion: 1 }} />)
  expect(screen.getByText("artifacts.registration.firstVersion")).toBeTruthy()
  expect(view.container.querySelector("time")?.dateTime).toBe(artifact.createdAt)
  view.rerender(<ArtifactLifecycle artifact={{ ...artifact, updatedAt: artifact.createdAt }} />)
  expect(screen.getByText("artifacts.registration.updatedVersion")).toBeTruthy()
  expect(screen.queryByText("artifacts.registration.firstVersion")).toBeNull()
})

test("default owner-selected file uses an upload lease and never sends its local path", async () => {
  const file = new File(["# Delivered report"], "report.md", { type: "text/markdown" })
  const imported = vi.fn()
  render(<ArtifactRegistrationDialog artifact={artifact} onImported={imported} onClose={vi.fn()} />)
  expect(screen.queryByRole("textbox")).toBeNull()
  fireEvent.change(screen.getByLabelText("artifacts.registration.file"), {
    target: { files: [file] },
  })
  expect(mocks.current.uploadFile).not.toHaveBeenCalled()
  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.update" })),
  )
  expect(mocks.current.uploadFile).toHaveBeenCalledExactlyOnceWith(
    file,
    "artifact_source",
    undefined,
    expect.any(AbortSignal),
  )
  expect(mocks.current.importArtifact).toHaveBeenCalledExactlyOnceWith({
    uploadId: "lease-local-file",
    artifactId: artifact.id,
    expectedVersion: 5,
  })
  expect(imported).toHaveBeenCalledExactlyOnceWith(artifact)
  expect(mocks.current.discardFileUpload).not.toHaveBeenCalled()
})

test("unsupported selected files dispatch neither upload nor import", () => {
  render(<ArtifactRegistrationDialog onImported={vi.fn()} onClose={vi.fn()} />)
  fireEvent.change(screen.getByLabelText("artifacts.registration.file"), {
    target: { files: [new File(["secret"], "private.txt")] },
  })
  expect(screen.getByRole("button", { name: "artifacts.registration.create" })).toBeDisabled()
  expect(mocks.current.uploadFile).not.toHaveBeenCalled()
  expect(mocks.current.importArtifact).not.toHaveBeenCalled()
})

test("failed registration discards only its original-host unclaimed lease and retains the file without retry", async () => {
  mocks.current.importArtifact.mockRejectedValue(new Error("private upload failure"))
  const file = new File(["# Report"], "report.md")
  render(<ArtifactRegistrationDialog onImported={vi.fn()} onClose={vi.fn()} />)
  fireEvent.change(screen.getByLabelText("artifacts.registration.file"), {
    target: { files: [file] },
  })
  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" })),
  )
  expect(mocks.current.discardFileUpload).toHaveBeenCalledExactlyOnceWith("lease-local-file")
  expect(screen.getByRole("alert")).toHaveTextContent("artifacts.registration.failed")
  expect(screen.queryByText(/private upload failure/)).toBeNull()
  act(() => vi.advanceTimersByTime(2000))
  expect(mocks.current.uploadFile).toHaveBeenCalledOnce()
  expect(screen.getByRole("button", { name: "artifacts.registration.create" })).not.toBeDisabled()
})

test.each(["host", "unmount"])(
  "a late upload after %s change is discarded on the original host without import",
  async (change) => {
    let finish!: (lease: { uploadId: string }) => void
    const old = mocks.current
    old.uploadFile.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve
      }),
    )
    const imported = vi.fn(),
      close = vi.fn()
    const view = render(<ArtifactRegistrationDialog onImported={imported} onClose={close} />)
    fireEvent.change(screen.getByLabelText("artifacts.registration.file"), {
      target: { files: [new File(["# Report"], "report.md")] },
    })
    fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" }))
    const signal = old.uploadFile.mock.calls[0][3] as AbortSignal
    if (change === "unmount") {
      view.unmount()
      expect(signal.aborted).toBe(true)
    } else {
      mocks.current = { importArtifact: vi.fn(), uploadFile: vi.fn(), discardFileUpload: vi.fn() }
      view.rerender(<ArtifactRegistrationDialog onImported={imported} onClose={close} />)
    }
    await act(async () => finish({ uploadId: "late-original-lease" }))
    expect(old.discardFileUpload).toHaveBeenCalledExactlyOnceWith("late-original-lease")
    expect(old.importArtifact).not.toHaveBeenCalled()
    expect(mocks.current.importArtifact).not.toHaveBeenCalled()
    expect(imported).not.toHaveBeenCalled()
  },
)
