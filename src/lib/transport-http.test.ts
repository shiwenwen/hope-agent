import { afterEach, beforeEach, expect, test, vi } from "vitest"

import { HttpTransport } from "./transport-http"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "./transport"

const fetchMock = vi.fn()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal("fetch", fetchMock)
})

afterEach(() => {
  vi.unstubAllGlobals()
})

test("HttpTransport builds Artifact previews from opaque ids", () => {
  const transport = new HttpTransport("http://localhost:8420", "secret token")

  expect(transport.artifactPreviewUrl("artifact/report 1", "/srv/private/artifact")).toBe(
    "http://localhost:8420/api/canvas/projects/artifact%2Freport%201/index.html?token=secret%20token",
  )
})

test("HttpTransport requests durable-state resync on connect and EventBus lag", () => {
  class MockWebSocket {
    static instances: MockWebSocket[] = []
    readyState = 0
    onopen: (() => void) | null = null
    onmessage: ((event: { data: string }) => void) | null = null
    onerror: (() => void) | null = null
    onclose: (() => void) | null = null

    constructor(url: string) {
      void url
      MockWebSocket.instances.push(this)
    }

    close() {
      this.readyState = 3
      this.onclose?.()
    }

    open() {
      this.readyState = 1
      this.onopen?.()
    }

    message(value: unknown) {
      this.onmessage?.({ data: JSON.stringify(value) })
    }
  }

  vi.stubGlobal("WebSocket", MockWebSocket)
  const transport = new HttpTransport("http://localhost:8420")
  const resyncReasons: unknown[] = []
  const unsubscribe = transport.listen(TRANSPORT_EVENT_RESYNC_REQUIRED, (payload) => {
    resyncReasons.push(payload)
  })

  const socket = MockWebSocket.instances[0]
  socket.open()
  socket.message({ name: "_lagged", payload: { missed: 3 } })

  expect(resyncReasons).toEqual([{ reason: "connected" }, { reason: "lagged", missed: 3 }])
  unsubscribe()
})

test("HttpTransport.startChat only bridges session_created and not late turn_started", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  const events: string[] = []

  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({
        sessionId: "session-123",
        response: "assistant reply",
        turnId: "turn-456",
      }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      },
    ),
  )

  const response = await transport.startChat(
    {
      message: "hello",
      attachments: [],
      sessionId: null,
    },
    (event) => events.push(event),
  )

  expect(response).toBe("assistant reply")
  expect(events).toEqual([
    JSON.stringify({
      type: "session_created",
      session_id: "session-123",
    }),
  ])
})

test("HttpTransport.save_attachment unwraps path from multipart response", async () => {
  const transport = new HttpTransport("http://localhost:8420")

  fetchMock.mockResolvedValue(
    new Response(JSON.stringify({ path: "/tmp/attachment.txt" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  )

  const path = await transport.call<string>("save_attachment", {
    fileName: "attachment.txt",
    mimeType: "text/plain",
    data: new Blob(["hello"], { type: "text/plain" }),
  })

  expect(path).toBe("/tmp/attachment.txt")
})

test("HttpTransport extracts only attachments owned by the active session", async () => {
  const transport = new HttpTransport("http://localhost:8420", "secret")
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({ relPath: "report.docx", kind: "office", text: "report", images: [] }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      },
    ),
  )

  await expect(
    transport.extractMediaDocument(
      {
        url: "/api/attachments/session-a/report.docx?token=secret&download=1",
        name: "report.docx",
        mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        sizeBytes: 100,
        kind: "file",
      },
      { sessionId: "session-a" },
    ),
  ).resolves.toMatchObject({ text: "report" })
  const [extractUrl, extractInit] = fetchMock.mock.lastCall!
  expect(extractUrl.toString()).toBe(
    "http://localhost:8420/api/attachments/session-a/report.docx/extract?token=secret",
  )
  expect(extractInit).toEqual({ headers: { Authorization: "Bearer secret" } })

  fetchMock.mockClear()
  await expect(
    transport.extractMediaDocument(
      {
        url: "/api/attachments/session-b/report.docx",
        name: "report.docx",
        mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        sizeBytes: 100,
        kind: "file",
      },
      { sessionId: "session-a" },
    ),
  ).rejects.toThrow("outside the active session")
  expect(fetchMock).not.toHaveBeenCalled()
})

test("HttpTransport.try_restore_session unwraps HTTP restored payload", async () => {
  const transport = new HttpTransport("http://localhost:8420")

  fetchMock.mockResolvedValue(
    new Response(JSON.stringify({ restored: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  )

  const restored = await transport.call<boolean>("try_restore_session")

  expect(restored).toBe(true)
})

test("HttpTransport unwraps the Git auto-merge input for the HTTP owner API", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  fetchMock.mockResolvedValue(
    new Response(JSON.stringify({ message: "enabled" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  )

  await transport.call("enable_session_git_pr_auto_merge_cmd", {
    sessionId: "s1",
    input: {
      requestId: "request-1",
      expectedRevision: "revision-1",
      method: "squash",
      confirmAutoMerge: true,
    },
  })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/git/pull-request/auto-merge",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify({
        requestId: "request-1",
        expectedRevision: "revision-1",
        method: "squash",
        confirmAutoMerge: true,
      }),
    }),
  )
})

test("HttpTransport maps the enhanced focus preference consistently", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  fetchMock.mockImplementation(() =>
    Promise.resolve(
      new Response(JSON.stringify(true), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  )

  await expect(transport.call<boolean>("get_enhanced_focus_indicators")).resolves.toBe(true)
  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/config/enhanced-focus-indicators",
    expect.objectContaining({ method: "GET", body: undefined }),
  )

  await transport.call("set_enhanced_focus_indicators", { enabled: true })
  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/config/enhanced-focus-indicators",
    expect.objectContaining({ method: "POST", body: JSON.stringify({ enabled: true }) }),
  )
})

test("HttpTransport maps execution mode and workflow owner commands", async () => {
  const transport = new HttpTransport("http://localhost:8420")

  fetchMock.mockImplementation(() =>
    Promise.resolve(
      new Response(JSON.stringify({ id: "wf-1" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  )

  await transport.call("get_execution_mode", { sessionId: "s1" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/execution-mode",
    expect.objectContaining({ method: "GET", body: undefined }),
  )

  await transport.call("set_execution_mode", { sessionId: "s1", mode: "deep" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/execution-mode",
    expect.objectContaining({ method: "POST", body: JSON.stringify({ mode: "deep" }) }),
  )

  await transport.call("get_workflow_mode", { sessionId: "s1" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/workflow-mode",
    expect.objectContaining({ method: "GET", body: undefined }),
  )

  await transport.call("set_workflow_mode", { sessionId: "s1", mode: "ultracode" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/workflow-mode",
    expect.objectContaining({ method: "POST", body: JSON.stringify({ mode: "ultracode" }) }),
  )

  await transport.call("list_workflow_runs", { sessionId: "s1" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/workflow-runs",
    expect.objectContaining({ method: "GET", body: undefined }),
  )

  await transport.call("preview_workflow_script", {
    sessionId: "s1",
    scriptSource: "export default async function main(workflow) {}",
    executionMode: "guarded",
  })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/workflow-runs/preview",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify({
        scriptSource: "export default async function main(workflow) {}",
        executionMode: "guarded",
      }),
    }),
  )

  await transport.call("create_workflow_run", {
    sessionId: "s1",
    kind: "general.workflow",
    executionMode: "guarded",
    scriptSource: "export default async function main(workflow) {}",
    budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
    parentRunId: "wf-parent",
    origin: "repair",
    runImmediately: true,
  })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/sessions/s1/workflow-runs",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify({
        kind: "general.workflow",
        executionMode: "guarded",
        scriptSource: "export default async function main(workflow) {}",
        budget: { maxScriptSecs: 180, maxOps: 24, maxOutputTokens: 10000 },
        parentRunId: "wf-parent",
        origin: "repair",
        runImmediately: true,
      }),
    }),
  )

  await transport.call("get_workflow_run", { runId: "wf-1" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/workflow-runs/wf-1",
    expect.objectContaining({ method: "GET", body: undefined }),
  )

  await transport.call("run_workflow_run", { runId: "wf-1" })

  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/workflow-runs/wf-1/run",
    expect.objectContaining({ method: "POST", body: JSON.stringify({}) }),
  )

  for (const [command, suffix] of [
    ["approve_workflow_run", "approve"],
    ["pause_workflow_run", "pause"],
    ["resume_workflow_run", "resume"],
    ["cancel_workflow_run", "cancel"],
  ] as const) {
    await transport.call(command, { runId: "wf-1" })

    expect(fetchMock).toHaveBeenLastCalledWith(
      `http://localhost:8420/api/workflow-runs/wf-1/${suffix}`,
      expect.objectContaining({ method: "POST", body: JSON.stringify({}) }),
    )
  }
})

test("HttpTransport unwraps Tauri-style provider config bodies for HTTP routes", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  const provider = {
    id: "provider-1",
    name: "Smoke Provider",
    apiType: "openai-chat",
    baseUrl: "https://example.invalid/v1",
    apiKey: "key",
    authProfiles: [],
    models: [
      {
        id: "smoke-model",
        name: "Smoke Model",
        inputTypes: ["text"],
        contextWindow: 128000,
        maxTokens: 4096,
        reasoning: false,
        thinkingStyle: "openai",
        costInput: 0,
        costOutput: 0,
      },
    ],
    enabled: true,
    userAgent: "claude-code/0.1.0",
    thinkingStyle: "openai",
  }

  fetchMock.mockImplementation(() =>
    Promise.resolve(
      new Response(JSON.stringify(provider), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    ),
  )

  await transport.call("add_provider", { config: provider })
  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/providers",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify(provider),
    }),
  )

  await transport.call("test_provider", { config: provider })
  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/providers/test",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify(provider),
    }),
  )

  await transport.call("update_provider", { config: provider })
  expect(fetchMock).toHaveBeenLastCalledWith(
    "http://localhost:8420/api/providers/provider-1",
    expect.objectContaining({
      method: "PUT",
      body: JSON.stringify(provider),
    }),
  )
})
