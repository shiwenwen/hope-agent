// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { TooltipProvider } from "@/components/ui/tooltip"
import ProjectKnowledgeSection from "./ProjectKnowledgeSection"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

const toastMock = vi.hoisted(() => ({
  error: vi.fn(),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("sonner", () => ({
  toast: toastMock,
}))

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string, fallbackOrOptions?: string | Record<string, unknown>, values?: Record<string, unknown>) => {
      const translations: Record<string, string> = {
        "knowledge.picker.empty": "还没有知识空间",
        "knowledge.picker.noteCount": "{{count}} 篇笔记",
        "knowledge.picker.accessOff": "关闭",
        "knowledge.picker.accessRead": "只读",
        "knowledge.picker.accessWrite": "读写",
        "knowledge.picker.accessOffTip": "不挂载",
        "knowledge.picker.accessReadTip": "可读取",
        "knowledge.picker.accessWriteTip": "可写入",
        "project.knowledge.label": "知识空间",
        "project.knowledge.hint": "项目对话可检索这些笔记。",
        "project.knowledge.loadFailed": "加载项目知识空间失败",
        "project.knowledge.updateFailed": "更新项目知识空间失败",
        "project.knowledge.errorDetail": "详细信息：{{error}}",
      }
      const options =
        typeof fallbackOrOptions === "object" && fallbackOrOptions !== null
          ? fallbackOrOptions
          : values
      let text =
        translations[key] ??
        (typeof fallbackOrOptions === "string"
          ? fallbackOrOptions
          : typeof options?.defaultValue === "string"
            ? options.defaultValue
            : key)
      for (const [name, value] of Object.entries(options ?? {})) {
        text = text.replace(`{{${name}}}`, String(value))
      }
      return text
    },
  }),
}))

afterEach(() => {
  cleanup()
  transportMock.call.mockReset()
  transportMock.listen.mockClear()
  toastMock.error.mockReset()
})

function renderSection() {
  return render(
    <TooltipProvider>
      <ProjectKnowledgeSection projectId="p1" />
    </TooltipProvider>,
  )
}

function kb() {
  return {
    id: "kb1",
    name: "Docs",
    emoji: "📚",
    rootDir: null,
    allowExternalWrites: false,
    externalRawSync: "disabled",
    archived: false,
    createdAt: 1,
    updatedAt: 1,
    noteCount: 3,
    external: false,
  }
}

describe("ProjectKnowledgeSection", () => {
  it("shows project knowledge load failures instead of an empty state", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_kbs_cmd") return Promise.resolve([])
      if (name === "list_project_kbs_cmd") {
        return Promise.reject(
          new Error(
            "Authorization: Bearer bearer-secret token=query-secret api_key=sk-live-secret",
          ),
        )
      }
      return Promise.resolve(null)
    })

    renderSection()

    expect(await screen.findByText("加载项目知识空间失败")).toBeTruthy()
    expect(await screen.findByText(/Authorization: Bearer \[redacted\]/)).toBeTruthy()
    expect(screen.queryByText("还没有知识空间")).toBeNull()
    expect(screen.queryByText(/bearer-secret|query-secret|sk-live-secret/)).toBeNull()
  })

  it("shows redacted detail when updating project knowledge access fails", async () => {
    transportMock.call.mockImplementation((name: string) => {
      if (name === "list_kbs_cmd") return Promise.resolve([kb()])
      if (name === "list_project_kbs_cmd") return Promise.resolve([])
      if (name === "attach_project_kb_cmd") {
        return Promise.reject(new Error("permission denied Authorization: Bearer bearer-secret"))
      }
      return Promise.resolve(null)
    })

    renderSection()

    fireEvent.click(await screen.findByRole("radio", { name: "只读" }))

    await waitFor(() => {
      expect(toastMock.error).toHaveBeenCalledWith("更新项目知识空间失败", {
        description: "详细信息：permission denied Authorization: Bearer [redacted]",
      })
    })
  })
})
