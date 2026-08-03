import { afterEach, beforeEach, expect, test, vi } from "vitest"

import { HttpTransport } from "./transport-http"
import { TRANSPORT_EVENT_RESYNC_REQUIRED } from "./transport"

const fetchMock = vi.fn()

beforeEach(() => {
  fetchMock.mockReset()
  vi.stubGlobal("fetch", fetchMock)
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

test("HttpTransport builds remote Artifact previews with a project-bound ticket", async () => {
  fetchMock
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "resource-ticket",
          eventTicket: "event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          ticket: "canvas-bound-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
  const transport = new HttpTransport("http://localhost:8420", "secret token")
  await transport.initializeRemoteAccess()
  expect(transport.resolveAssetUrl("/srv/canvas/projects/artifact report 1/index.html")).toBeNull()

  await expect(
    transport.artifactPreviewUrl("artifact report 1", "/srv/private/artifact"),
  ).resolves.toBe(
    "http://localhost:8420/api/resource/canvas-bound-ticket/canvas/projects/artifact%20report%201/index.html",
  )
  expect(fetchMock.mock.calls[1]).toEqual([
    "http://localhost:8420/api/auth/preview-resource-ticket",
    {
      method: "POST",
      headers: {
        Authorization: "Bearer secret token",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ kind: "canvas_project", projectId: "artifact report 1" }),
    },
  ])
  expect(fetchMock.mock.calls[0]?.[1]).toEqual({
    method: "POST",
    headers: { Authorization: "Bearer secret token" },
  })
})

test("HttpTransport binds Design previews to the path-derived artifact subtree", async () => {
  fetchMock.mockResolvedValueOnce(
    new Response(
      JSON.stringify({
        authRequired: true,
        ticket: "design-bound-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  const transport = new HttpTransport("https://agent.example", "owner-token")

  const projectPath = "/srv/design/projects/project 1/artifacts/artifact 2"
  await expect(transport.artifactPreviewUrl("artifact 2", projectPath)).resolves.toBe(
    "https://agent.example/api/resource/design-bound-ticket/design/projects/project%201/artifacts/artifact%202/index.html",
  )
  await expect(transport.artifactPreviewUrl("artifact 2", projectPath)).resolves.toContain(
    "design-bound-ticket",
  )
  expect(fetchMock).toHaveBeenCalledTimes(1)
  expect(fetchMock).toHaveBeenCalledWith("https://agent.example/api/auth/preview-resource-ticket", {
    method: "POST",
    headers: {
      Authorization: "Bearer owner-token",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      kind: "design_artifact",
      projectId: "project 1",
      artifactId: "artifact 2",
    }),
  })
})

test("HttpTransport mints a single-file ticket for remote workspace previews", async () => {
  fetchMock
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "generic-resource-ticket",
          eventTicket: "event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ ticket: "bound-file-ticket", expiresInSecs: 900 }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )
  const transport = new HttpTransport("https://agent.example", "owner-token")

  await expect(
    transport.projectFsRawUrl({
      scope: "project",
      scopeId: "project-1",
      path: "preview/index.html",
    }),
  ).resolves.toBe("https://agent.example/api/resource/bound-file-ticket/fs/raw")

  expect(fetchMock.mock.calls[1]).toEqual([
    "https://agent.example/api/fs/raw-ticket",
    {
      method: "POST",
      headers: {
        Authorization: "Bearer owner-token",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        scope: "project",
        scopeId: "project-1",
        path: "preview/index.html",
      }),
    },
  ])
  expect(fetchMock.mock.calls[1]?.[0]).not.toContain("generic-resource-ticket")
})

test("HttpTransport mints a path-bound ticket for session file previews", async () => {
  fetchMock.mockResolvedValueOnce(
    new Response(
      JSON.stringify({
        authRequired: true,
        ticket: "bound-session-file-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  const transport = new HttpTransport("https://agent.example", "owner-token")

  await expect(
    transport.previewRawUrl("/srv/session/visible.html", { sessionId: "session-1" }),
  ).resolves.toBe("https://agent.example/api/resource/bound-session-file-ticket/fs/raw")

  expect(fetchMock).toHaveBeenCalledWith(
    "https://agent.example/api/sessions/session-1/files/by-path-ticket",
    {
      method: "POST",
      headers: {
        Authorization: "Bearer owner-token",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        path: "/srv/session/visible.html",
        download: false,
      }),
    },
  )
  expect(fetchMock.mock.calls[0]?.[0]).not.toContain("visible.html")
})

test("HttpTransport uses the protected direct session route when authentication is disabled", async () => {
  fetchMock.mockResolvedValueOnce(
    new Response(JSON.stringify({ authRequired: false, ticket: null, expiresInSecs: null }), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  )
  const transport = new HttpTransport("http://localhost:8420")

  const url = await transport.previewRawUrl("/srv/session/report.pdf", {
    sessionId: "session-1",
  })

  expect(url).toBe(
    "http://localhost:8420/api/sessions/session-1/files/by-path?path=%2Fsrv%2Fsession%2Freport.pdf",
  )
})

test("HttpTransport requests durable-state resync on connect and EventBus lag", async () => {
  class MockWebSocket {
    static instances: MockWebSocket[] = []
    readyState = 0
    onopen: (() => void) | null = null
    onmessage: ((event: { data: string }) => void) | null = null
    onerror: (() => void) | null = null
    onclose: (() => void) | null = null

    constructor(url: string, protocols?: string[]) {
      void url
      void protocols
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

  await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1))
  const socket = MockWebSocket.instances[0]
  socket.open()
  socket.message({ name: "_lagged", payload: { missed: 3 } })

  expect(resyncReasons).toEqual([{ reason: "connected" }, { reason: "lagged", missed: 3 }])
  unsubscribe()
})

test("HttpTransport sends the scoped event ticket as a WebSocket subprotocol", async () => {
  class MockWebSocket {
    static calls: Array<{ url: string; protocols?: string[] }> = []
    readyState = 0
    onopen: (() => void) | null = null
    onmessage: ((event: { data: string }) => void) | null = null
    onerror: (() => void) | null = null
    onclose: (() => void) | null = null

    constructor(url: string, protocols?: string[]) {
      MockWebSocket.calls.push({ url, protocols })
    }

    close() {
      this.readyState = 3
    }
  }

  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({
        authRequired: true,
        resourceTicket: "resource-ticket",
        eventTicket: "event-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  vi.stubGlobal("WebSocket", MockWebSocket)
  const transport = new HttpTransport("https://agent.example", "root-secret")
  const unsubscribe = transport.listen("config:changed", () => undefined)

  await vi.waitFor(() => expect(MockWebSocket.calls).toHaveLength(1))
  expect(MockWebSocket.calls[0]).toEqual({
    url: "wss://agent.example/ws/events",
    protocols: ["ha-events.event-ticket"],
  })
  expect(MockWebSocket.calls[0].url).not.toContain("root-secret")
  unsubscribe()
})

test("HttpTransport remints scoped tickets after a rejected WebSocket handshake", async () => {
  vi.useFakeTimers()
  class MockWebSocket {
    static instances: MockWebSocket[] = []
    static calls: Array<{ url: string; protocols?: string[] }> = []
    readyState = 0
    onopen: (() => void) | null = null
    onmessage: ((event: { data: string }) => void) | null = null
    onerror: (() => void) | null = null
    onclose: (() => void) | null = null

    constructor(url: string, protocols?: string[]) {
      MockWebSocket.instances.push(this)
      MockWebSocket.calls.push({ url, protocols })
    }

    close() {
      this.readyState = 3
      this.onclose?.()
    }
  }

  fetchMock
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "old-resource-ticket",
          eventTicket: "old-event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "new-resource-ticket",
          eventTicket: "new-event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
  vi.stubGlobal("WebSocket", MockWebSocket)
  const transport = new HttpTransport("https://agent.example", "root-secret")
  const unsubscribe = transport.listen("config:changed", () => undefined)

  await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(1))
  MockWebSocket.instances[0].close()
  await vi.advanceTimersByTimeAsync(1_000)
  await vi.waitFor(() => expect(MockWebSocket.instances).toHaveLength(2))

  expect(fetchMock).toHaveBeenCalledTimes(2)
  expect(MockWebSocket.calls[1].protocols).toEqual(["ha-events.new-event-ticket"])
  unsubscribe()
})

test("HttpTransport refreshes the same-origin browser session after token replacement", async () => {
  vi.stubGlobal("window", { location: { origin: "http://localhost:8420" } })
  fetchMock.mockResolvedValue(new Response(JSON.stringify({ ok: true }), { status: 200 }))
  const transport = new HttpTransport("http://localhost:8420")

  await transport.activateOwnerToken("replacement-secret")

  expect(fetchMock).toHaveBeenCalledWith("http://localhost:8420/api/auth/session", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ token: "replacement-secret", remember: true }),
  })
  await expect(transport.artifactPreviewUrl("artifact-1")).resolves.toBe(
    "http://localhost:8420/api/canvas/projects/artifact-1/index.html",
  )
})

test("HttpTransport ignores scoped tickets minted for a replaced Owner Token", async () => {
  let resolveOldRequest: ((response: Response) => void) | undefined
  fetchMock
    .mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          resolveOldRequest = resolve
        }),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "new-resource-ticket",
          eventTicket: "new-event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )

  const transport = new HttpTransport("https://agent.example", "old-owner-token")
  const oldRequest = transport.initializeRemoteAccess()
  await transport.activateOwnerToken("new-owner-token")
  resolveOldRequest?.(
    new Response(
      JSON.stringify({
        authRequired: true,
        resourceTicket: "stale-resource-ticket",
        eventTicket: "stale-event-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  await oldRequest

  expect(transport.resolveAssetUrl("/srv/avatars/a.png")).toContain("new-resource-ticket")
  expect(transport.resolveAssetUrl("/srv/avatars/a.png")).not.toContain("stale-resource-ticket")
  expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
    headers: { Authorization: "Bearer old-owner-token" },
  })
  expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({
    headers: { Authorization: "Bearer new-owner-token" },
  })
})

test("HttpTransport ignores a preview ticket minted for a replaced Owner Token", async () => {
  let resolveOldPreview: ((response: Response) => void) | undefined
  fetchMock
    .mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          resolveOldPreview = resolve
        }),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "new-resource-ticket",
          eventTicket: "new-event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          ticket: "new-preview-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )

  const transport = new HttpTransport("https://agent.example", "old-owner-token")
  const stalePreview = transport.artifactPreviewUrl("artifact-1")
  await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

  await transport.activateOwnerToken("new-owner-token")
  resolveOldPreview?.(
    new Response(
      JSON.stringify({
        authRequired: true,
        ticket: "stale-preview-ticket",
        expiresInSecs: 900,
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  )
  await expect(stalePreview).resolves.toBeNull()
  await expect(transport.artifactPreviewUrl("artifact-1")).resolves.toContain("new-preview-ticket")
  expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
    headers: expect.objectContaining({ Authorization: "Bearer old-owner-token" }),
  })
  expect(fetchMock.mock.calls[2]?.[1]).toMatchObject({
    headers: expect.objectContaining({ Authorization: "Bearer new-owner-token" }),
  })
})

test("HttpTransport ignores stale 401 responses after Owner Token replacement", async () => {
  let resolveStaleRequest: ((response: Response) => void) | undefined
  const dispatchEvent = vi.fn()
  vi.stubGlobal("window", {
    location: { origin: "https://ui.example" },
    dispatchEvent,
  })
  fetchMock
    .mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          resolveStaleRequest = resolve
        }),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          authRequired: true,
          resourceTicket: "new-resource-ticket",
          eventTicket: "new-event-ticket",
          expiresInSecs: 900,
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    )

  const transport = new HttpTransport("https://agent.example", "old-owner-token")
  const staleRequest = transport.call("get_server_runtime_status")
  await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1))

  await transport.activateOwnerToken("new-owner-token")
  resolveStaleRequest?.(new Response("unauthorized", { status: 401 }))
  await expect(staleRequest).rejects.toThrow("returned 401")

  expect(dispatchEvent).not.toHaveBeenCalled()
  expect(transport.resolveAssetUrl("/srv/avatars/a.png")).toContain("new-resource-ticket")
  expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({
    headers: { Authorization: "Bearer old-owner-token" },
  })
  expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({
    headers: { Authorization: "Bearer new-owner-token" },
  })
})

test("HttpTransport keeps provisional remote-auth failures local to the connection flow", async () => {
  const dispatchEvent = vi.fn()
  vi.stubGlobal("window", { dispatchEvent })
  fetchMock.mockResolvedValue(new Response("unauthorized", { status: 401 }))
  const transport = new HttpTransport("https://agent.example", "wrong-owner-token")

  await expect(transport.initializeRemoteAccess(false)).rejects.toThrow(
    "Remote authentication failed (401)",
  )

  expect(dispatchEvent).not.toHaveBeenCalled()
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
  expect(fetchMock.mock.calls[0]?.[0]).toBe("http://localhost:8420/api/chat/ui")
  expect(events).toEqual([
    JSON.stringify({
      type: "session_created",
      session_id: "session-123",
    }),
  ])
})

test.each(["knowledge_chat", "pet_chat"] as const)(
  "HttpTransport carries the %s product surface through the bundled UI endpoint",
  async (uiSurface) => {
    const transport = new HttpTransport("http://localhost:8420")
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ sessionId: "session-123", response: "ok" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

    await transport.startChat(
      {
        message: "hello",
        attachments: [],
        sessionId: "session-123",
        uiSurface,
      },
      () => undefined,
    )

    expect(fetchMock.mock.calls[0]?.[0]).toBe("http://localhost:8420/api/chat/ui")
    const request = fetchMock.mock.calls[0]?.[1] as RequestInit | undefined
    expect(JSON.parse(String(request?.body))).toMatchObject({ uiSurface })
  },
)

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
  const browserTransport = new HttpTransport("http://localhost:8420")
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({ relPath: "report.docx", kind: "office", text: "report", images: [] }),
      {
        status: 200,
        headers: { "content-type": "application/json" },
      },
    ),
  )

  expect(
    browserTransport.resolveMediaUrl({
      url: "/api/attachments/session-a/report.docx?token=legacy-secret&download=1",
      name: "report.docx",
      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      sizeBytes: 100,
      kind: "file",
    }),
  ).toBe("http://localhost:8420/api/attachments/session-a/report.docx?download=1")
  expect(
    transport.resolveMediaUrl({
      url: "/api/attachments/session-a/report.docx?token=legacy-secret",
      name: "report.docx",
      mimeType: "application/octet-stream",
      sizeBytes: 100,
      kind: "file",
    }),
  ).toBeNull()

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
    "http://localhost:8420/api/attachments/session-a/report.docx/extract",
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

test("HttpTransport turns Bearer-authenticated remote media into a temporary Blob URL", async () => {
  const transport = new HttpTransport("https://agent.example", "root-secret")
  const createObjectUrl = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:secure-media")
  const revokeObjectUrl = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined)
  fetchMock.mockResolvedValue(new Response("image-bytes", { status: 200 }))

  const lease = await transport.loadMediaUrl({
    url: "/api/attachments/session-a/image.png?token=legacy-secret",
    name: "image.png",
    mimeType: "image/png",
    sizeBytes: 11,
    kind: "image",
  })

  expect(lease.url).toBe("blob:secure-media")
  const [url, init] = fetchMock.mock.lastCall!
  expect(url.toString()).toBe("https://agent.example/api/attachments/session-a/image.png")
  expect(init).toEqual({ headers: { Authorization: "Bearer root-secret" } })
  expect(createObjectUrl).toHaveBeenCalledOnce()
  lease.release()
  expect(revokeObjectUrl).toHaveBeenCalledWith("blob:secure-media")
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

test("HttpTransport unwraps Agent markdown and template content like Tauri", async () => {
  const transport = new HttpTransport("http://localhost:8420")

  fetchMock
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ content: "# Agent instructions" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ content: null }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ content: "# Built-in template" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

  await expect(
    transport.call<string | null>("get_agent_markdown", {
      id: "agent-1",
      file: "agent.md",
    }),
  ).resolves.toBe("# Agent instructions")
  await expect(
    transport.call<string | null>("get_agent_markdown", {
      id: "agent-1",
      file: "persona.md",
    }),
  ).resolves.toBeNull()
  await expect(
    transport.call<string>("get_agent_template", { name: "agent", locale: "zh" }),
  ).resolves.toBe("# Built-in template")

  expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
    "http://localhost:8420/api/agents/agent-1/markdown?file=agent.md",
    "http://localhost:8420/api/agents/agent-1/markdown?file=persona.md",
    "http://localhost:8420/api/agents/template?name=agent&locale=zh",
  ])
})

test.each([
  {
    command: "get_system_prompt",
    args: { agentId: "agent-1" },
    payload: { system_prompt: "# System prompt" },
    expected: "# System prompt",
  },
  {
    command: "render_persona_to_soul_md",
    args: { id: "agent-1", persona: {} },
    payload: { content: "# Soul" },
    expected: "# Soul",
  },
  {
    command: "read_log_file_cmd",
    args: { filename: "app.log" },
    payload: { content: "log line" },
    expected: "log line",
  },
  {
    command: "get_log_file_path_cmd",
    args: undefined,
    payload: { path: "/tmp/app.log" },
    expected: "/tmp/app.log",
  },
  {
    command: "export_logs_cmd",
    args: { filter: {}, format: "json" },
    payload: { data: "[]" },
    expected: "[]",
  },
  {
    command: "install_skill_dependency",
    args: { skillName: "demo", specIndex: 0 },
    payload: { ok: true, output: "installed" },
    expected: "installed",
  },
  {
    command: "channel_validate_credentials",
    args: { channelId: "telegram", credentials: {}, settings: {} },
    payload: { info: "Hope Bot" },
    expected: "Hope Bot",
  },
  {
    command: "create_backup_cmd",
    args: undefined,
    payload: { name: "backup.zip" },
    expected: "backup.zip",
  },
])(
  "HttpTransport unwraps $command string payload like Tauri",
  async ({ command, args, payload, expected }) => {
    const transport = new HttpTransport("http://localhost:8420")
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify(payload), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

    await expect(
      transport.call<string>(command, args as Record<string, unknown> | undefined),
    ).resolves.toBe(expected)
  },
)

test.each([
  {
    command: "get_agent_memory_md",
    args: { id: "agent-1" },
    payload: { content: "# Agent memory" },
    expected: "# Agent memory",
  },
  {
    command: "get_global_memory_md",
    args: undefined,
    payload: { content: null },
    expected: null,
  },
])(
  "HttpTransport unwraps $command optional content like Tauri",
  async ({ command, args, payload, expected }) => {
    const transport = new HttpTransport("http://localhost:8420")
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify(payload), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

    await expect(
      transport.call<string | null>(command, args as Record<string, unknown> | undefined),
    ).resolves.toBe(expected)
  },
)

test.each([
  ["get_skill_env_check", { enabled: false }, false],
  ["get_skills_auto_review_enabled", { enabled: true }, true],
  ["get_skills_auto_review_promotion", { auto: false }, false],
])("HttpTransport unwraps %s boolean payload like Tauri", async (command, payload, expected) => {
  const transport = new HttpTransport("http://localhost:8420")
  fetchMock.mockResolvedValue(
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  )

  await expect(transport.call<boolean>(command)).resolves.toBe(expected)
})

test.each(["test_provider", "test_model"])(
  "HttpTransport preserves the %s JSON-string contract",
  async (command) => {
    const transport = new HttpTransport("http://localhost:8420")
    const payload = { success: false, message: "connection failed", latencyMs: 12 }
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify(payload), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

    const args = command === "test_provider" ? { config: {} } : { config: {}, modelId: "m1" }
    await expect(transport.call<string>(command, args)).resolves.toBe(JSON.stringify(payload))
  },
)

test.each(["test_provider", "test_model"])(
  "HttpTransport rejects a scalar %s error like Tauri",
  async (command) => {
    const transport = new HttpTransport("http://localhost:8420")
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify("Client error: invalid user agent"), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

    const args = command === "test_provider" ? { config: {} } : { config: {}, modelId: "m1" }
    await expect(transport.call<string>(command, args)).rejects.toThrow(
      "Client error: invalid user agent",
    )
  },
)

test("HttpTransport aligns proxy probe success and failure with Tauri", async () => {
  const transport = new HttpTransport("http://localhost:8420")
  fetchMock
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ success: true, message: "Proxy connected" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ success: false, message: "Proxy unavailable" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    )

  await expect(transport.call<string>("test_proxy", { config: {} })).resolves.toBe(
    "Proxy connected",
  )
  await expect(transport.call<string>("test_proxy", { config: {} })).rejects.toThrow(
    "Proxy unavailable",
  )
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
