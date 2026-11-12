// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { DesignScenariosControl } from "@/components/design/DesignScenariosControl"

const { call, success, error } = vi.hoisted(() => ({
  call: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}))
const transport = { call }
vi.mock("@/lib/transport-provider", () => ({ getTransport: () => transport }))
vi.mock("@/lib/logger", () => ({ logger: { warn: vi.fn(), error: vi.fn() } }))
vi.mock("sonner", () => ({ toast: { success, error } }))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback?: string) => fallback ?? _key }),
}))

function fixture(id: string) {
  return {
    hash: `hash-${id}`,
    manifest: {
      version: 1,
      viewports: { phone: { width: 390, height: 844 } },
      scenarios: [
        {
          id,
          title: id,
          route: "/",
          state: { label: id },
          viewportIds: ["phone"],
          ready: { selector: "#state", text: id },
        },
      ],
    },
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

beforeEach(() => {
  vi.clearAllMocks()
  call.mockImplementation((command: string, { artifactId }: { artifactId: string }) => {
    if (command === "get_design_scenarios_cmd") return Promise.resolve(fixture(artifactId))
    throw new Error(`Unexpected command: ${command}`)
  })
})
afterEach(cleanup)

async function openEditor() {
  await waitFor(() => expect(screen.getByRole("button")).toBeEnabled())
  fireEvent.click(screen.getByRole("button"))
  return screen.getByRole("textbox")
}

describe("DesignScenariosControl", () => {
  it("saves the edited readiness condition with the loaded revision and applies its viewport", async () => {
    const onApply = vi.fn()
    render(<DesignScenariosControl artifactId="A" onApply={onApply} />)
    const editor = await openEditor()
    const updated = fixture("empty")
    fireEvent.change(editor, { target: { value: JSON.stringify(updated.manifest) } })
    call.mockResolvedValueOnce(updated)
    fireEvent.click(screen.getByRole("button", { name: "保存" }))
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(call).toHaveBeenLastCalledWith("save_design_scenarios_cmd", {
      artifactId: "A",
      expectedHash: "hash-A",
      manifest: updated.manifest,
    })
    expect(onApply).toHaveBeenLastCalledWith(updated.manifest.scenarios[0], {
      width: 390,
      height: 844,
    })
    expect(success).toHaveBeenCalledOnce()
  })

  it("discards an open draft when switching artifacts", async () => {
    const onApply = vi.fn()
    const view = render(<DesignScenariosControl artifactId="A" onApply={onApply} />)
    const editor = await openEditor()
    fireEvent.change(editor, { target: { value: JSON.stringify(fixture("unsaved-A").manifest) } })
    view.rerender(<DesignScenariosControl artifactId="B" onApply={onApply} />)
    await waitFor(() =>
      expect(onApply).toHaveBeenLastCalledWith(fixture("B").manifest.scenarios[0], {
        width: 390,
        height: 844,
      }),
    )
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    const nextEditor = await openEditor()
    expect(JSON.parse((nextEditor as HTMLTextAreaElement).value)).toEqual(fixture("B").manifest)
    expect(call.mock.calls.every(([command]) => command !== "save_design_scenarios_cmd")).toBe(true)
  })

  it.each(["resolve", "reject"] as const)(
    "ignores a late save %s after switching artifacts",
    async (outcome) => {
      const pending = deferred<ReturnType<typeof fixture>>()
      const onApply = vi.fn()
      const view = render(<DesignScenariosControl artifactId="A" onApply={onApply} />)
      await openEditor()
      call.mockReturnValueOnce(pending.promise)
      fireEvent.click(screen.getByRole("button", { name: "保存" }))
      view.rerender(<DesignScenariosControl artifactId="B" onApply={onApply} />)
      await waitFor(() =>
        expect(onApply).toHaveBeenLastCalledWith(fixture("B").manifest.scenarios[0], {
          width: 390,
          height: 844,
        }),
      )
      onApply.mockClear()
      await act(async () => {
        if (outcome === "resolve") pending.resolve(fixture("saved-A"))
        else pending.reject(new Error("stale save failed"))
      })
      expect(onApply).not.toHaveBeenCalled()
      expect(success).not.toHaveBeenCalled()
      expect(error).not.toHaveBeenCalled()
      expect(screen.getByRole("combobox")).toHaveTextContent("B")
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
    },
  )

  it("keeps invalid JSON open without issuing a save", async () => {
    render(<DesignScenariosControl artifactId="A" onApply={vi.fn()} />)
    const editor = await openEditor()
    fireEvent.change(editor, { target: { value: "{" } })
    fireEvent.click(screen.getByRole("button", { name: "保存" }))
    expect(screen.getByRole("dialog")).toBeInTheDocument()
    expect(error).toHaveBeenCalledWith("场景清单不是有效 JSON")
    expect(call).toHaveBeenCalledOnce()
  })

  it("does not revive an old save when returning to the same artifact", async () => {
    const pending = deferred<ReturnType<typeof fixture>>()
    const onApply = vi.fn()
    const view = render(<DesignScenariosControl artifactId="A" onApply={onApply} />)
    await openEditor()
    call.mockReturnValueOnce(pending.promise)
    fireEvent.click(screen.getByRole("button", { name: "保存" }))
    view.rerender(<DesignScenariosControl artifactId="B" onApply={onApply} />)
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveTextContent("B"))
    view.rerender(<DesignScenariosControl artifactId="A" onApply={onApply} />)
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveTextContent("A"))
    onApply.mockClear()
    await act(async () => pending.resolve(fixture("old-A")))
    expect(onApply).not.toHaveBeenCalled()
    expect(success).not.toHaveBeenCalled()
    expect(screen.getByRole("combobox")).toHaveTextContent("A")
  })

  it("ignores an earlier load that finishes after the new artifact loads", async () => {
    const pending = deferred<ReturnType<typeof fixture>>()
    call.mockReturnValueOnce(pending.promise)
    const onApply = vi.fn()
    const view = render(<DesignScenariosControl artifactId="A" onApply={onApply} />)
    view.rerender(<DesignScenariosControl artifactId="B" onApply={onApply} />)
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveTextContent("B"))
    onApply.mockClear()
    await act(async () => pending.resolve(fixture("A")))
    expect(onApply).not.toHaveBeenCalled()
    expect(screen.getByRole("combobox")).toHaveTextContent("B")
  })
})
