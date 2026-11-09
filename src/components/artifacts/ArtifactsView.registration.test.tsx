// @vitest-environment jsdom
import { useEffect, type ReactNode } from "react"
import { afterEach, beforeEach, expect, test, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import ArtifactsView from "./ArtifactsView"
import type { ArtifactRecord } from "@/lib/transport"

const mocks = vi.hoisted(() => ({
  listArtifacts: vi.fn(),
  getArtifact: vi.fn(),
  listArtifactVersions: vi.fn(),
  verifyArtifact: vi.fn(),
  reset: vi.fn(),
  transitionTo: vi.fn(),
  drag: vi.fn(),
  viewerRef: { current: null },
  t: (key: string) => key,
  returned: null as unknown as ArtifactRecord,
  activeTransport: null as null | Record<string, unknown>,
  revision: 0,
  dialogUnmount: vi.fn(),
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => mocks.activeTransport ?? mocks,
  useTransport: () => mocks.activeTransport ?? mocks,
  getTransportRevision: () => mocks.revision,
  useTransportRevision: () => mocks.revision,
}))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: mocks.t }) }))
vi.mock("@/hooks/useDragWidth", () => ({ useDragWidth: () => mocks.drag }))
vi.mock("@/hooks/useFullscreenTransition", () => ({
  useFullscreenTransition: () => ({
    ref: mocks.viewerRef,
    reset: mocks.reset,
    transitionTo: mocks.transitionTo,
    animating: false,
  }),
}))
vi.mock("@/components/chat/files/FileActionMenu", () => ({
  FileContextMenu: ({ children }: { children: ReactNode }) => children,
}))
vi.mock("@/components/chat/files/useFileResource", () => ({
  useFileResource: (_target: unknown, options?: { onPreviewFile?: () => void }) => ({
    primary: "preview",
    run: () => options?.onPreviewFile?.(),
  }),
}))
vi.mock("./ArtifactViewer", () => ({ default: () => <div>Managed preview</div> }))
vi.mock("./ArtifactRegistrationDialog", () => ({
  ArtifactRegistrationDialog: ({
    artifact,
    onImported,
  }: {
    artifact?: ArtifactRecord
    onImported: (artifact: ArtifactRecord) => void
  }) => {
    useEffect(() => () => mocks.dialogUnmount(), [])
    return (
      <div>
        <input aria-label="runtime-host-path" defaultValue="" />
        {artifact && (
          <p>
            Target {artifact.id} v{artifact.currentVersion}
          </p>
        )}
        <button onClick={() => onImported(mocks.returned)}>complete registration</button>
      </div>
    )
  },
}))

const saved: ArtifactRecord = {
  id: "artifact-new",
  title: "Registered report",
  kind: "report",
  contentType: "html",
  lifecycleState: "active",
  privacy: "local_private",
  currentVersion: 1,
  currentHash: "a".repeat(64),
  payloadKind: "freeform",
  sourceCount: 0,
  sourceSummaries: [],
  evidenceSummary: {},
  capabilities: {},
  projectPath: "/managed/new",
  createdAt: "2026-10-19T00:00:00Z",
  updatedAt: "2026-10-19T00:00:00Z",
}

beforeEach(() => {
  window.localStorage?.clear()
  mocks.listArtifacts.mockReset()
  mocks.getArtifact.mockReset().mockResolvedValue(saved)
  mocks.listArtifactVersions.mockReset().mockResolvedValue([])
  mocks.returned = saved
  mocks.activeTransport = null
  mocks.revision = 0
  mocks.dialogUnmount.mockClear()
})
afterEach(cleanup)

test("registration selects its durable result and ignores a pre-registration list response", async () => {
  let finishOld!: (rows: ArtifactRecord[]) => void
  mocks.listArtifacts
    .mockReturnValueOnce(
      new Promise((resolve) => {
        finishOld = resolve
      }),
    )
    .mockResolvedValue([saved])
  render(<ArtifactsView isViewVisible />)
  fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.create" }))
  fireEvent.click(screen.getByRole("button", { name: "complete registration" }))
  await screen.findAllByText("Registered report")
  await act(async () => finishOld([]))
  expect(screen.getAllByText("Registered report")).toHaveLength(2)
  expect(screen.getAllByText("artifacts.registration.firstVersion")).toHaveLength(2)
  expect(mocks.getArtifact).toHaveBeenCalledWith("artifact-new")
})

test("update is bound to the selected version and refreshes it without a duplicate library row", async () => {
  const current = { ...saved, id: "existing", title: "Existing report", currentVersion: 5 }
  const next = { ...current, currentVersion: 6 }
  mocks.listArtifacts.mockResolvedValue([current])
  mocks.getArtifact.mockResolvedValue(current)
  render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Existing report/ }))
  fireEvent.click(await screen.findByRole("button", { name: "artifacts.registration.update" }))
  expect(screen.getByText("Target existing v5")).toBeTruthy()
  mocks.returned = next
  mocks.listArtifacts.mockResolvedValue([next])
  mocks.getArtifact.mockResolvedValue(next)
  fireEvent.click(screen.getByRole("button", { name: "complete registration" }))
  await waitFor(() => expect(screen.getByText("v6")).toBeTruthy())
  expect(screen.getAllByText("Existing report")).toHaveLength(2)
  expect(screen.getAllByText("artifacts.registration.updatedVersion")).toHaveLength(2)
  expect(current.currentVersion).toBe(5)
})

test("switching selection blocks updates until the new artifact identity has loaded", async () => {
  const second = { ...saved, id: "second", title: "Second report", currentVersion: 4 }
  let finishSecond!: (value: ArtifactRecord) => void
  mocks.listArtifacts.mockResolvedValue([saved, second])
  mocks.getArtifact.mockImplementation((id: string) =>
    id === "second"
      ? new Promise((resolve) => {
          finishSecond = resolve
        })
      : Promise.resolve(saved),
  )
  render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
  await screen.findByRole("button", { name: "artifacts.registration.update" })
  fireEvent.click(screen.getByRole("button", { name: /Second report/ }))
  const update = screen.getByRole("button", { name: "artifacts.registration.update" })
  expect(update).toBeDisabled()
  fireEvent.click(update)
  expect(screen.queryByText("Target artifact-new v1")).toBeNull()
  await act(async () => finishSecond(second))
  await waitFor(() => expect(update).not.toBeDisabled())
  fireEvent.click(update)
  expect(screen.getByText("Target second v4")).toBeTruthy()
})

test("a hidden runtime switch clears old selections even when both hosts use the same artifact ID", async () => {
  mocks.listArtifacts.mockResolvedValue([saved])
  const view = render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
  await screen.findByRole("button", { name: "artifacts.registration.update" })
  view.rerender(<ArtifactsView isViewVisible={false} />)
  const other = { ...saved, title: "Other host report", currentVersion: 8 }
  let finishList!: (rows: ArtifactRecord[]) => void
  const otherTransport = {
    listArtifacts: vi.fn(
      () =>
        new Promise<ArtifactRecord[]>((resolve) => {
          finishList = resolve
        }),
    ),
    getArtifact: vi.fn().mockResolvedValue(other),
    listArtifactVersions: vi.fn().mockResolvedValue([]),
  }
  mocks.activeTransport = otherTransport
  mocks.revision++
  view.rerender(<ArtifactsView isViewVisible={false} />)
  expect(screen.queryByRole("button", { name: "artifacts.registration.update" })).toBeNull()
  view.rerender(<ArtifactsView isViewVisible />)
  expect(screen.queryByRole("button", { name: "artifacts.registration.update" })).toBeNull()
  await act(async () => finishList([other]))
  fireEvent.click(await screen.findByRole("button", { name: /Other host report/ }))
  fireEvent.click(await screen.findByRole("button", { name: "artifacts.registration.update" }))
  expect(screen.getByText("Target artifact-new v8")).toBeTruthy()
  expect(otherTransport.getArtifact).toHaveBeenCalledWith(saved.id)
})

test("switching runtime closes an already opened update registration", async () => {
  mocks.listArtifacts.mockResolvedValue([saved])
  const view = render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
  fireEvent.click(await screen.findByRole("button", { name: "artifacts.registration.update" }))
  expect(screen.getByText("Target artifact-new v1")).toBeTruthy()
  mocks.activeTransport = { listArtifacts: vi.fn().mockResolvedValue([]) }
  mocks.revision++
  view.rerender(<ArtifactsView isViewVisible={false} />)
  expect(screen.queryByText("Target artifact-new v1")).toBeNull()
})

test("an old-host verification response cannot replace the new host update target", async () => {
  let finishVerify!: (value: unknown) => void
  mocks.verifyArtifact.mockReturnValue(
    new Promise((resolve) => {
      finishVerify = resolve
    }),
  )
  mocks.listArtifacts.mockResolvedValue([saved])
  const view = render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
  fireEvent.click(await screen.findByRole("button", { name: "artifacts.verify" }))
  const other = { ...saved, title: "Other host report", currentVersion: 8 }
  mocks.activeTransport = {
    listArtifacts: vi.fn().mockResolvedValue([other]),
    getArtifact: vi.fn().mockResolvedValue(other),
    listArtifactVersions: vi.fn().mockResolvedValue([]),
  }
  mocks.revision++
  view.rerender(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Other host report/ }))
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "artifacts.registration.update" }),
    ).not.toBeDisabled(),
  )
  await act(async () => finishVerify({ status: "passed" }))
  fireEvent.click(screen.getByRole("button", { name: "artifacts.registration.update" }))
  expect(screen.getByText("Target artifact-new v8")).toBeTruthy()
})

test("routine access ticket renewal preserves the same host registration and typed path", async () => {
  mocks.listArtifacts.mockResolvedValue([saved])
  const view = render(<ArtifactsView isViewVisible />)
  fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
  fireEvent.click(await screen.findByRole("button", { name: "artifacts.registration.update" }))
  fireEvent.change(screen.getByRole("textbox", { name: "runtime-host-path" }), {
    target: { value: "/managed/new-report.html" },
  })
  mocks.revision++ // HTTP ticket renewal keeps the exact same singleton/owner namespace.
  view.rerender(<ArtifactsView isViewVisible />)
  expect(screen.getByText("Target artifact-new v1")).toBeTruthy()
  expect(screen.getByRole("textbox", { name: "runtime-host-path" })).toHaveValue(
    "/managed/new-report.html",
  )
})

test.each(["create", "update"])(
  "leaving the persistent view unmounts %s registration and does not reopen it",
  async (mode) => {
    mocks.listArtifacts.mockResolvedValue([saved])
    const view = render(<ArtifactsView isViewVisible />)
    if (mode === "update") {
      fireEvent.click(await screen.findByRole("button", { name: /Registered report/ }))
      await waitFor(() =>
        expect(
          screen.getByRole("button", { name: "artifacts.registration.update" }),
        ).not.toBeDisabled(),
      )
    }
    fireEvent.click(screen.getByRole("button", { name: `artifacts.registration.${mode}` }))
    expect(screen.getByRole("textbox", { name: "runtime-host-path" })).toBeTruthy()
    view.rerender(<ArtifactsView isViewVisible={false} />)
    expect(screen.queryByRole("textbox", { name: "runtime-host-path" })).toBeNull()
    expect(mocks.dialogUnmount).toHaveBeenCalledOnce()
    view.rerender(<ArtifactsView isViewVisible />)
    expect(screen.queryByRole("textbox", { name: "runtime-host-path" })).toBeNull()
    expect(mocks.dialogUnmount).toHaveBeenCalledOnce()
  },
)
