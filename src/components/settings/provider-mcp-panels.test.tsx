// @vitest-environment jsdom

import { afterEach, describe, expect, test, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import ProviderSetup from "./ProviderSetup"
import McpServersPanel from "./mcp-panel/McpServersPanel"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}))

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}))

vi.mock("@/lib/logger", () => ({
  logger: {
    error: vi.fn(),
    warn: vi.fn(),
  },
}))

const transportMock = vi.hoisted(() => ({
  call: vi.fn(),
  listen: vi.fn(() => () => {}),
}))

vi.mock("@/lib/transport-provider", () => ({
  getTransport: () => transportMock,
}))

const mcpMock = vi.hoisted(() => ({
  listServers: vi.fn(),
  removeServer: vi.fn(),
  reconnectServer: vi.fn(),
  testConnection: vi.fn(),
  startOauth: vi.fn(),
  signOut: vi.fn(),
}))

vi.mock("@/lib/mcp", () => ({
  listServers: mcpMock.listServers,
  removeServer: mcpMock.removeServer,
  reconnectServer: mcpMock.reconnectServer,
  testConnection: mcpMock.testConnection,
  startOauth: mcpMock.startOauth,
  signOut: mcpMock.signOut,
  MCP_EVENTS: {
    SERVERS_CHANGED: "mcp:servers_changed",
    SERVER_STATUS_CHANGED: "mcp:server_status_changed",
    AUTH_REQUIRED: "mcp:auth_required",
    AUTH_COMPLETED: "mcp:auth_completed",
  },
}))

vi.mock("./provider-setup/TemplateGrid", () => ({
  TemplateGrid: ({ configuredProviders }: { configuredProviders: unknown[] }) => (
    <div>
      <span data-testid="configured-provider-count">{configuredProviders.length}</span>
    </div>
  ),
}))

vi.mock("./provider-setup/TemplateConfig", () => ({
  TemplateConfig: () => <div data-testid="template-config" />,
}))

vi.mock("./provider-setup/CustomWizard", () => ({
  CustomWizard: () => <div data-testid="custom-wizard" />,
}))

vi.mock("./mcp-panel/McpServerEditDialog", () => ({
  default: () => <div data-testid="mcp-edit-dialog" />,
}))

vi.mock("./mcp-panel/McpImportDialog", () => ({
  default: () => <div data-testid="mcp-import-dialog" />,
}))

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe("settings provider and MCP panels", () => {
  test("ProviderSetup passes loaded providers into the chooser", async () => {
    transportMock.call.mockImplementation(async (command: string) => {
      if (command === "get_providers") {
        return [
          {
            id: "p1",
            name: "Provider 1",
            apiType: "openai-chat",
            baseUrl: "https://example.test",
            apiKey: "",
            models: [],
            enabled: true,
          },
        ]
      }
      return null
    })

    render(<ProviderSetup onComplete={vi.fn()} onCodexAuth={vi.fn(() => Promise.resolve())} />)

    expect((await screen.findByTestId("configured-provider-count")).textContent).toBe("1")
  })

  test("McpServersPanel renders the empty state and subscribes to MCP events", async () => {
    mcpMock.listServers.mockResolvedValue([])

    render(<McpServersPanel />)

    expect(await screen.findByText("settings.mcp.emptyTitle")).toBeTruthy()
    expect(
      screen.getAllByRole("button", { name: "settings.mcp.addServer" }).length,
    ).toBeGreaterThan(0)
    expect(transportMock.listen).toHaveBeenCalledWith("mcp:servers_changed", expect.any(Function))
    expect(transportMock.listen).toHaveBeenCalledWith(
      "mcp:server_status_changed",
      expect.any(Function),
    )
  })
})
