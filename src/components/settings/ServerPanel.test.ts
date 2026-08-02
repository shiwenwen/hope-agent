import { describe, expect, test } from "vitest"

import { remoteApiKeyForSave } from "./serverCredentials"

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
      }),
    ).toBe("new-remote-secret")
  })

  test("carries an active remote server token replacement into the connection", () => {
    expect(
      remoteApiKeyForSave({
        currentMode: "remote",
        previousMode: "remote",
        currentRemoteServerUrl: "https://agent.example/",
        previousRemoteServerUrl: "https://agent.example",
        remoteApiKey: "old-secret",
        replacementOwnerToken: " new-secret ",
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
      }),
    ).toBe("remote-secret")
  })
})
