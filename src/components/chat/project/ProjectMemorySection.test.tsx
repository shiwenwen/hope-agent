// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"

import { ProjectMemorySection } from "./ProjectMemorySection"

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: () => {} },
  useTranslation: () => ({
    t: (key: string) =>
      ({
        "common.delete": "删除",
        "common.loading": "加载中",
        "common.save": "保存",
        "common.saving": "保存中",
        "project.autoMemory.content": "详细内容",
        "project.autoMemory.deleteConfirm": "确认删除？",
        "project.autoMemory.description": "只注入索引",
        "project.autoMemory.editorHint": "详情按需读取",
        "project.autoMemory.editorTitle": "项目自动记忆",
        "project.autoMemory.empty": "还没有主题",
        "project.autoMemory.fileNameGenerated": "自动生成文件名",
        "project.autoMemory.name": "标题",
        "project.autoMemory.newTopic": "新建主题",
        "project.autoMemory.rebuildIndex": "重建索引",
        "project.autoMemory.summary": "索引摘要",
        "project.autoMemory.summaryHint": "保持简短",
        "project.autoMemory.type": "类型",
        "project.autoMemory.types.project": "项目进展",
        "project.autoMemory.types.feedback": "工作反馈",
        "project.autoMemory.types.reference": "参考资料",
        "project.autoMemory.types.user": "用户背景",
      })[key] ?? key,
  }),
}))

afterEach(() => {
  cleanup()
  transportMock.call.mockReset()
  transportMock.listen.mockClear()
})

const entry = {
  fileName: "project_architecture.md",
  name: "架构",
  description: "当前模块边界",
  memoryType: "project" as const,
  sizeBytes: 128,
}

describe("ProjectMemorySection", () => {
  it("loads topic bodies on demand and saves through the owner API", async () => {
    transportMock.call.mockImplementation((command: string) => {
      if (command === "list_project_memory_files_cmd") return Promise.resolve([entry])
      if (command === "read_project_memory_file_cmd") {
        return Promise.resolve({ ...entry, content: "详细架构", fileHash: "hash-v1" })
      }
      if (command === "write_project_memory_file_cmd") {
        return Promise.resolve({
          ...entry,
          name: "新架构",
          content: "详细架构",
          fileHash: "hash-v2",
        })
      }
      return Promise.resolve(null)
    })

    render(<ProjectMemorySection projectId="project-1" />)
    fireEvent.click(await screen.findByText("架构"))

    const name = await screen.findByLabelText("标题")
    expect((name as HTMLInputElement).value).toBe("架构")
    fireEvent.change(name, { target: { value: "新架构" } })
    fireEvent.click(screen.getByRole("button", { name: "保存" }))

    await waitFor(() => {
      expect(transportMock.call).toHaveBeenCalledWith("write_project_memory_file_cmd", {
        id: "project-1",
        input: {
          fileName: "project_architecture.md",
          expectedFileHash: "hash-v1",
          name: "新架构",
          description: "当前模块边界",
          memoryType: "project",
          content: "详细架构",
        },
      })
    })
  })

  it("redacts secrets from owner API failures", async () => {
    transportMock.call.mockRejectedValue(
      new Error("Authorization: Bearer bearer-secret api_key=sk-live-secret"),
    )

    render(<ProjectMemorySection projectId="project-1" />)

    expect(await screen.findByText(/Authorization: Bearer \[redacted\]/)).toBeTruthy()
    expect(screen.queryByText(/bearer-secret|sk-live-secret/)).toBeNull()
  })
})
