import { describe, expect, test } from "vitest"

import {
  ownerTokenWillExist,
  remoteApiKeyForSave,
  shouldPrepareRemoteBeforeServerMutation,
} from "./serverCredentials"

describe("remoteApiKeyForSave", () => {
  test("keeps the destination token when switching from embedded to remote", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "embedded",
        currentRemoteServerUrl: "https://remote.example",
        previousRemoteServerUrl: "",
        remoteApiKey: " remote-secret ",
        replacementOwnerToken: "",
        activeRemoteMatchesDestination: false,
      }),
    ).toBe("remote-secret")
  })

  test("keeps the destination token when changing remote servers", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://new.example",
        previousRemoteServerUrl: "https://old.example",
        remoteApiKey: " new-remote-secret ",
        replacementOwnerToken: "",
        activeRemoteMatchesDestination: false,
      }),
    ).toBe("new-remote-secret")
  })

  test("carries an active remote server token replacement into the connection", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://AGENT.example:443/",
        previousRemoteServerUrl: "https://agent.example",
        remoteApiKey: "old-secret",
        replacementOwnerToken: " new-secret ",
        activeRemoteMatchesDestination: true,
      }),
    ).toBe("new-secret")
  })

  test("preserves the remote field when its active Owner Token was not edited", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://agent.example",
        previousRemoteServerUrl: "https://agent.example",
        remoteApiKey: " remote-secret ",
        replacementOwnerToken: null,
        activeRemoteMatchesDestination: true,
      }),
    ).toBe("remote-secret")
  })

  test("does not reuse a token replacement for a differently targeted active client", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://target.example",
        previousRemoteServerUrl: "https://target.example",
        remoteApiKey: " target-secret ",
        replacementOwnerToken: "new-current-token",
        activeRemoteMatchesDestination: false,
      }),
    ).toBe("target-secret")
  })
})

describe("ownerTokenWillExist", () => {
  test("accepts externally managed credentials for public binding", () => {
    expect(
      ownerTokenWillExist({
        replacementOwnerToken: null,
        hasManagedOwnerToken: false,
        externallyManaged: true,
      }),
    ).toBe(true)
  })

  test("uses the credential-store state when the field was not edited", () => {
    expect(
      ownerTokenWillExist({
        replacementOwnerToken: null,
        hasManagedOwnerToken: true,
        externallyManaged: false,
      }),
    ).toBe(true)
  })

  test("uses an explicit replacement when credentials are locally managed", () => {
    expect(
      ownerTokenWillExist({
        replacementOwnerToken: "",
        hasManagedOwnerToken: true,
        externallyManaged: false,
      }),
    ).toBe(false)
  })
})

describe("shouldPrepareRemoteBeforeServerMutation", () => {
  test("validates a different remote before changing the current server token", () => {
    expect(
      shouldPrepareRemoteBeforeServerMutation({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://new.example",
        previousRemoteServerUrl: "https://old.example",
        replacementOwnerToken: "new-current-token",
        activeRemoteMatchesDestination: false,
      }),
    ).toBe(true)
  })

  test("validates an unchanged remote early when its Owner Token was not edited", () => {
    expect(
      shouldPrepareRemoteBeforeServerMutation({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://AGENT.example:443/",
        previousRemoteServerUrl: "https://agent.example",
        replacementOwnerToken: null,
        activeRemoteMatchesDestination: true,
      }),
    ).toBe(true)
  })

  test("defers validation only for a same-remote Owner Token replacement", () => {
    expect(
      shouldPrepareRemoteBeforeServerMutation({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://agent.example/%7E/",
        previousRemoteServerUrl: "https://agent.example/~",
        replacementOwnerToken: "new-token",
        activeRemoteMatchesDestination: true,
      }),
    ).toBe(false)
  })

  test("validates first when the active HTTP client targets another origin", () => {
    expect(
      shouldPrepareRemoteBeforeServerMutation({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://target.example",
        previousRemoteServerUrl: "https://target.example",
        replacementOwnerToken: "new-current-token",
        activeRemoteMatchesDestination: false,
      }),
    ).toBe(true)
  })
})
