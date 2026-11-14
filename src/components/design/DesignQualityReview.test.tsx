// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import { DesignQualityReview } from "./DesignQualityReview"
import type { DesignQualityRun } from "@/types/design"

const { call, success, error } = vi.hoisted(() => ({
  call: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}))
let transport = { call }
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transport }))
vi.mock("@/lib/logger", () => ({ logger: { warn: vi.fn() } }))
vi.mock("sonner", () => ({ toast: { success, error } }))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}))
const run: DesignQualityRun = {
  artifactId: "A",
  artifactHash: "source",
  scenariosHash: "scenes",
  reviewId: "review-1",
  baselineRevision: "R1",
  baselineRevisions: [
    { id: "R1", parentId: null, restoredFrom: null, createdAt: "2026-11-14T00:00:00Z" },
  ],
  deterministicPassed: false,
  findings: [],
  diffs: [
    {
      scenarioId: "empty",
      scenarioTitle: "Empty",
      viewport: { width: 390, height: 844 },
      currentHash: "current",
      baselineHash: "previous",
      changedRatio: 0.5,
      passed: false,
    },
    {
      scenarioId: "error",
      scenarioTitle: "Error",
      viewport: { width: 390, height: 844 },
      currentHash: "error-current",
      passed: false,
    },
  ],
}
function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}
function fixture(command: string, args: Record<string, unknown>) {
  if (command === "review_design_artifact_cmd") return Promise.resolve([])
  if (command === "run_design_visual_regression_cmd") return Promise.resolve(run)
  if (command === "get_design_quality_image_cmd") {
    const input = args.input as { side: string; scenarioId: string }
    const hash =
      input.side === "baseline"
        ? "previous"
        : input.scenarioId === "empty"
          ? "current"
          : "error-current"
    return Promise.resolve({ imageHash: hash, dataUrl: `data:image/png;base64,${hash}` })
  }
  if (command === "accept_design_visual_baseline_cmd")
    return Promise.resolve({
      revisionId: "R2",
      revisions: [
        ...run.baselineRevisions,
        { ...run.baselineRevisions[0], id: "R2", parentId: "R1" },
      ],
    })
  if (command === "restore_design_visual_baseline_cmd")
    return Promise.resolve({ revisionId: "R3", revisions: [...run.baselineRevisions] })
  throw new Error(command)
}
beforeEach(() => {
  vi.clearAllMocks()
  transport = { call }
  call.mockImplementation(fixture)
})
afterEach(cleanup)
async function ready() {
  await screen.findByRole("button", { name: "design.review.acceptBaseline" })
}

it("loads a selected pair on demand and accepts its saved receipt without recapturing", async () => {
  render(<DesignQualityReview artifactId="A" onClose={vi.fn()} />)
  await ready()
  expect(call.mock.calls.filter(([cmd]) => cmd === "get_design_quality_image_cmd")).toHaveLength(0)
  fireEvent.click(screen.getByRole("button", { name: /Empty/ }))
  await screen.findByAltText("Empty 本轮截图")
  expect(screen.getByAltText("Empty 审查时基线")).toHaveAttribute(
    "src",
    "data:image/png;base64,previous",
  )
  fireEvent.click(screen.getByRole("button", { name: "design.review.acceptBaseline" }))
  await waitFor(() => expect(success).toHaveBeenCalledOnce())
  expect(call).toHaveBeenCalledWith("accept_design_visual_baseline_cmd", {
    input: {
      artifactId: "A",
      reviewId: "review-1",
      expectedArtifactHash: "source",
      expectedScenariosHash: "scenes",
      expectedBaselineRevision: "R1",
    },
  })
  expect(
    call.mock.calls.filter(([cmd]) => cmd === "run_design_visual_regression_cmd"),
  ).toHaveLength(1)
  expect(screen.getByRole("button", { name: "design.review.acceptBaseline" })).toBeDisabled()
  fireEvent.click(screen.getByRole("button", { name: /恢复基线/ }))
  await waitFor(() =>
    expect(call).toHaveBeenCalledWith("restore_design_visual_baseline_cmd", {
      artifactId: "A",
      input: { artifactId: "A", revisionId: "R1", expectedRevisionId: "R2" },
    }),
  )
})

it("displays missing baselines without requesting a nonexistent image", async () => {
  render(<DesignQualityReview artifactId="A" onClose={vi.fn()} />)
  await ready()
  fireEvent.click(screen.getByRole("button", { name: /Error/ }))
  await screen.findByAltText("Error 本轮截图")
  expect(call.mock.calls.filter(([cmd]) => cmd === "get_design_quality_image_cmd")).toHaveLength(1)
})

it("ignores late image pairs after selecting another scenario", async () => {
  const pending = deferred<{ imageHash: string; dataUrl: string }>()
  call.mockImplementation((cmd, args) =>
    cmd === "get_design_quality_image_cmd" && args.input.scenarioId === "empty"
      ? pending.promise
      : fixture(cmd, args),
  )
  render(<DesignQualityReview artifactId="A" onClose={vi.fn()} />)
  await ready()
  fireEvent.click(screen.getByRole("button", { name: /Empty/ }))
  fireEvent.click(screen.getByRole("button", { name: /Error/ }))
  await screen.findByAltText("Error 本轮截图")
  await act(async () =>
    pending.resolve({ imageHash: "current", dataUrl: "data:image/png;base64,old" }),
  )
  expect(screen.queryByAltText("Empty 本轮截图")).not.toBeInTheDocument()
})

it("rejects an image whose returned identity differs from the reviewed capture", async () => {
  call.mockImplementation((cmd, args) =>
    cmd === "get_design_quality_image_cmd"
      ? Promise.resolve({ imageHash: "wrong", dataUrl: "data:image/png;base64,wrong" })
      : fixture(cmd, args),
  )
  render(<DesignQualityReview artifactId="A" onClose={vi.fn()} />)
  await ready()
  fireEvent.click(screen.getByRole("button", { name: /Empty/ }))
  await screen.findByRole("alert")
  expect(screen.queryByRole("img")).not.toBeInTheDocument()
})

it("does not issue mutations after the active connection changes before rerender", async () => {
  render(<DesignQualityReview artifactId="A" onClose={vi.fn()} />)
  await ready()
  transport = { call: vi.fn() }
  fireEvent.click(screen.getByRole("button", { name: "design.review.acceptBaseline" }))
  expect(call.mock.calls.some(([cmd]) => cmd === "accept_design_visual_baseline_cmd")).toBe(false)
})

it("ignores a late acceptance after closing the dialog", async () => {
  const pending = deferred<{ revisionId: string; revisions: [] }>()
  call.mockImplementation((cmd, args) =>
    cmd === "accept_design_visual_baseline_cmd" ? pending.promise : fixture(cmd, args),
  )
  const close = vi.fn()
  render(<DesignQualityReview artifactId="A" onClose={close} />)
  await ready()
  fireEvent.click(screen.getByRole("button", { name: "design.review.acceptBaseline" }))
  fireEvent.click(screen.getByRole("button", { name: "关闭" }))
  await act(async () => pending.resolve({ revisionId: "R2", revisions: [] }))
  expect(close).toHaveBeenCalledOnce()
  expect(success).not.toHaveBeenCalled()
  expect(error).not.toHaveBeenCalled()
})

it("drops old results on A to B to A remounts", async () => {
  const pending = deferred<DesignQualityRun>()
  let count = 0
  call.mockImplementation((cmd, args) =>
    cmd === "run_design_visual_regression_cmd" && count++ === 0
      ? pending.promise
      : fixture(cmd, args),
  )
  const view = render(<DesignQualityReview key="A-1" artifactId="A" onClose={vi.fn()} />)
  view.rerender(<DesignQualityReview key="B" artifactId="B" onClose={vi.fn()} />)
  view.rerender(<DesignQualityReview key="A-2" artifactId="A" onClose={vi.fn()} />)
  await ready()
  await act(async () => pending.resolve({ ...run, reviewId: "stale-review" }))
  expect(screen.queryByText(/stale-review/)).not.toBeInTheDocument()
})
