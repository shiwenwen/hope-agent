// @vitest-environment jsdom

import { useState, type ComponentProps, type ReactNode } from "react"
import { cleanup, fireEvent, render, screen, within, waitFor } from "@testing-library/react"
import { afterEach, expect, test, vi } from "vitest"
import { FolderOpen, Globe } from "lucide-react"
import en from "@/i18n/locales/en.json"
import { PersonalAssistantHeader } from "./PersonalAssistantHeader"

vi.mock("@/lib/transport-provider", () => ({
  useTransport: () => ({ resolveAssetUrl: (path: string) => path }),
}))
vi.mock("@/components/ui/tooltip", () => ({
  IconTip: ({ children }: { children: ReactNode }) => children,
}))
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const value = key
        .split(".")
        .reduce<unknown>(
          (node, segment) =>
            node && typeof node === "object"
              ? (node as Record<string, unknown>)[segment]
              : undefined,
          en,
        )
      return typeof value === "string" ? value : (fallback ?? key)
    },
  }),
}))

afterEach(cleanup)

const tabs = [
  {
    id: "browser",
    panelId: "browser" as const,
    label: "Browser",
    icon: Globe,
    open: true,
    windowMode: "docked" as const,
  },
  { id: "files", panelId: "files" as const, label: "Files", icon: FolderOpen, open: true },
]

function HeaderHarness({
  layoutMode = "docked",
  docked = true,
  onFloat = vi.fn(),
  onMaximize = vi.fn(),
  onSearch = vi.fn(),
  attentionCount = 0,
}: {
  layoutMode?: "docked" | "stage"
  docked?: boolean
  onFloat?: (id: string) => void
  onMaximize?: () => void
  onSearch?: () => void
  attentionCount?: number
}) {
  const [openTabs, setTabs] = useState(tabs)
  const [activeId, setActive] = useState("browser")
  const [collapsed, setCollapsed] = useState(false)
  const workbench: ComponentProps<typeof PersonalAssistantHeader>["workbench"] = {
    width: 500,
    layoutMode,
    docked,
    collapsed,
    tabs: openTabs,
    launchItems: tabs,
    activeId,
    onSelect: setActive,
    onOpen: vi.fn(),
    onCloseTab: (id) => setTabs((current) => current.filter((tab) => tab.id !== id)),
    onReorder: vi.fn(),
    onToggleTabWindow: onFloat,
    onToggleMaximize: onMaximize,
    onCloseWorkbench: () => setCollapsed(true),
    onExpand: () => setCollapsed(false),
  }
  return (
    <PersonalAssistantHeader
      profile={{ name: "Hope", avatar: null }}
      attentionCount={attentionCount}
      itemsOpen={collapsed}
      onOpenSettings={vi.fn()}
      onSearch={onSearch}
      onToggleItems={() => setCollapsed(true)}
      workbench={workbench}
    />
  )
}

test("assistant workbench can switch and close tabs, float the browser and maximize", () => {
  const onFloat = vi.fn()
  const onMaximize = vi.fn()
  render(<HeaderHarness onFloat={onFloat} onMaximize={onMaximize} />)
  fireEvent.click(screen.getByRole("button", { name: en.chat.controlPanel.floatWindow }))
  expect(onFloat).toHaveBeenCalledWith("browser")
  fireEvent.click(screen.getByRole("button", { name: en.fileBrowser.maximize }))
  expect(onMaximize).toHaveBeenCalledOnce()
  const files = screen.getByRole("tab", { name: /Files/ })
  fireEvent.click(files)
  expect(files).toHaveAttribute("aria-selected", "true")
  fireEvent.click(within(files).getByRole("button", { name: en.common.close }))
  expect(screen.queryByRole("tab", { name: /Files/ })).toBeNull()
  expect(screen.getByRole("tab", { name: /Browser/ })).toBeTruthy()
})

test("opening Items or collapsing the workbench keeps a working reopen entry", () => {
  render(<HeaderHarness />)
  for (const label of [en.personalAssistant.items, en.chat.rightPanel.collapse]) {
    if (label === en.personalAssistant.items) {
      fireEvent.keyDown(screen.getByRole("button", { name: en.chat.moreActions }), { key: "Enter" })
      fireEvent.click(screen.getByRole("menuitem", { name: label }))
    } else fireEvent.click(screen.getByRole("button", { name: label }))
    expect(screen.queryByRole("tab", { name: /Browser/ })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: en.chat.rightPanel.expand }))
    expect(screen.getByRole("tab", { name: /Browser/ })).toBeTruthy()
  }
})

test("stage workbench retains controls and restores the assistant header when collapsed", () => {
  render(<HeaderHarness layoutMode="stage" />)
  expect(screen.getByRole("banner", { hidden: true })).toHaveClass("hidden")
  fireEvent.click(screen.getByRole("button", { name: en.chat.rightPanel.collapse }))
  expect(screen.getByRole("banner")).toHaveTextContent("Hope")
  fireEvent.click(screen.getByRole("button", { name: en.chat.rightPanel.expand }))
  expect(screen.getByRole("tab", { name: /Browser/ })).toBeTruthy()
})

test("floating-only panels keep a reopen entry without a docked tab strip", () => {
  render(<HeaderHarness docked={false} />)
  expect(screen.queryByRole("tablist")).toBeNull()
  expect(screen.getByRole("button", { name: en.chat.rightPanel.expand })).toBeTruthy()
})

test("search and all Items are secondary actions while attention stays directly reachable", async () => {
  const onSearch = vi.fn()
  const { rerender } = render(<HeaderHarness onSearch={onSearch} />)
  expect(screen.queryByRole("button", { name: en.common.search })).toBeNull()
  expect(screen.queryByRole("button", { name: en.personalAssistant.items })).toBeNull()
  fireEvent.keyDown(screen.getByRole("button", { name: en.chat.moreActions }), { key: "Enter" })
  fireEvent.click(screen.getByRole("menuitem", { name: en.common.search }))
  expect(onSearch).toHaveBeenCalledOnce()
  rerender(<HeaderHarness attentionCount={2} />)
  await waitFor(() =>
    expect(screen.getByRole("button", { name: `${en.personalAssistant.items} 2` })).toBeTruthy(),
  )
})
