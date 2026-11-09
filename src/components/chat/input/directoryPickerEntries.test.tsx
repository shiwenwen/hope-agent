// @vitest-environment jsdom

import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react"
import WorkingDirectoryButton from "./WorkingDirectoryButton"
import ProjectDialog from "../project/ProjectDialog"
import { useDirectoryPicker } from "./useDirectoryPicker"
import type { DirListing } from "@/lib/transport"

const mocks = vi.hoisted(() => ({
  desktop: true,
  pickLocalDirectory: vi.fn(),
  listServerDirectory: vi.fn<(path?: string) => Promise<DirListing>>(),
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
  fileRuntime: () => ({ workspaceHost: "local" }),
  t: (key: string, fallback?: string | { defaultValue?: string }) =>
    typeof fallback === "string" ? fallback : (fallback?.defaultValue ?? key),
}))

vi.mock("@/lib/transport", async (original) => ({
  ...(await original<typeof import("@/lib/transport")>()),
  isTauriMode: () => mocks.desktop,
}))
vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => mocks,
  useTransport: () => mocks,
}))
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: mocks.t }) }))
vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
}))
vi.mock("../project/ProjectInstructionsField", () => ({ default: () => null }))
vi.mock("../project/ProjectKnowledgeSection", () => ({ default: () => null }))

beforeEach(() => {
  vi.clearAllMocks()
  mocks.desktop = true
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (X11; Linux x86_64)")
  mocks.pickLocalDirectory.mockResolvedValue("/native/project")
  mocks.call.mockResolvedValue({
    path: "/home/hope/chosen-dir/AGENTS.md",
    content: "",
    contentHash: "",
    exists: false,
  })
  mocks.listServerDirectory.mockImplementation(async (path) => ({
    path: path ?? "/home/hope",
    parent: "/home/hope",
    entries: [
      {
        name: "chosen-dir",
        path: "/home/hope/chosen-dir",
        isDir: true,
        isSymlink: false,
        size: null,
        modifiedMs: null,
      },
      {
        name: "notes.txt",
        path: "/home/hope/notes.txt",
        isDir: false,
        isSymlink: false,
        size: 10,
        modifiedMs: null,
      },
    ],
    truncated: false,
  }))
})

afterEach(async () => {
  cleanup()
  await new Promise<void>((resolve) => setTimeout(resolve, 0))
  vi.restoreAllMocks()
})

describe("working-directory and project-folder entry points", () => {
  for (const entry of ["toolbar", "menu", "project"] as const) {
    it.each([
      ["Linux desktop", true, "X11; Linux x86_64", true],
      ["macOS desktop", true, "Macintosh; Intel Mac OS X", false],
      ["Windows desktop", true, "Windows NT 10.0; Win64; x64", false],
      ["Linux web", false, "X11; Linux x86_64", true],
    ])(
      `${entry}: %s uses the expected picker and returns the selected directory`,
      async (_name, desktop, agent, appBrowser) => {
        mocks.desktop = desktop
        vi.spyOn(navigator, "userAgent", "get").mockReturnValue(`Mozilla/5.0 (${agent})`)
        const onChange = vi.fn()
        if (entry === "project") {
          render(<ProjectDialog open mode="create" agents={[]} onOpenChange={() => {}} />)
        } else {
          render(<WorkingDirectoryButton variant={entry} workingDir={null} onChange={onChange} />)
        }
        const trigger =
          entry === "project"
            ? "project.linkedDirs.add"
            : entry === "menu"
              ? "chat.addWorkingDirectory"
              : "chat.workingDir.select"
        fireEvent.click(screen.getByRole("button", { name: trigger }))

        if (!appBrowser) {
          await waitFor(() => expect(mocks.pickLocalDirectory).toHaveBeenCalledOnce())
          expect(mocks.listServerDirectory).not.toHaveBeenCalled()
          if (entry === "project") await screen.findByText("/native/project")
          else expect(onChange).toHaveBeenCalledWith("/native/project")
          return
        }
        fireEvent.click(await screen.findByRole("button", { name: "chosen-dir" }))
        await screen.findByDisplayValue("/home/hope/chosen-dir")
        expect(mocks.pickLocalDirectory).not.toHaveBeenCalled()
        // Browser Linux keeps the existing file rows; only Linux desktop opts in.
        expect(Boolean(screen.queryByText("notes.txt"))).toBe(!desktop)
        fireEvent.click(screen.getByRole("button", { name: "chat.workingDir.selectCurrent" }))
        if (entry === "project") await screen.findByText("/home/hope/chosen-dir")
        else expect(onChange).toHaveBeenCalledWith("/home/hope/chosen-dir")
      },
    )
  }

  it("keeps unmodified shared-hook callers on the native picker on Linux", async () => {
    const onPicked = vi.fn()
    const { result } = renderHook(() =>
      useDirectoryPicker({ onPicked, errorTitle: "invalid", loggerSource: "test" }),
    )
    await act(async () => result.current.pick())
    expect(mocks.pickLocalDirectory).toHaveBeenCalledOnce()
    expect(onPicked).toHaveBeenCalledWith("/native/project")
    expect(result.current.browserOpen).toBe(false)
  })
})
