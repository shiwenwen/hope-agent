import { expect, test } from "vitest"

import { hasKnownLocalBackend, type KnownLocalBackend } from "./provider-detection"

const ollamaBackend: KnownLocalBackend = {
  key: "ollama",
  name: "Ollama",
  apiType: "openai-chat",
  baseUrl: "http://127.0.0.1:11434",
  hosts: ["127.0.0.1", "localhost", "::1", "ollama.local"],
  port: 11434,
}

test("matches known local backend from catalog instead of regex", () => {
  expect(
    hasKnownLocalBackend(
      [
        {
          enabled: true,
          apiType: "openai-chat",
          baseUrl: "http://ollama.local:11434",
        },
      ],
      [ollamaBackend],
      "ollama",
    ),
  ).toBe(true)
})

test("treats localhost v1 and 127 loopback as the same known backend", () => {
  const providers = [
    {
      enabled: true,
      apiType: "openai-chat",
      baseUrl: "http://localhost:11434/v1",
    },
    {
      enabled: true,
      apiType: "openai-chat",
      baseUrl: "http://127.0.0.1:11434",
    },
  ]

  expect(hasKnownLocalBackend(providers, [ollamaBackend], "ollama")).toBe(true)
})

test("requires matching api type and port", () => {
  expect(
    hasKnownLocalBackend(
      [
        {
          enabled: true,
          apiType: "openai-responses",
          baseUrl: "http://localhost:11434/v1",
        },
        {
          enabled: true,
          apiType: "openai-chat",
          baseUrl: "http://localhost:1234",
        },
      ],
      [ollamaBackend],
      "ollama",
    ),
  ).toBe(false)
})
