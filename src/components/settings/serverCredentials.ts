type ServerMode = "embedded" | "remote"

interface RemoteApiKeyForSaveArgs {
  currentMode: ServerMode
  previousMode: ServerMode
  currentRemoteServerUrl: string
  previousRemoteServerUrl: string
  remoteApiKey: string
  replacementOwnerToken: string | null
}

function normalizedRemoteServerUrl(value: string): string {
  return value.trim().replace(/\/+$/, "")
}

export function remoteApiKeyForSave({
  currentMode,
  previousMode,
  currentRemoteServerUrl,
  previousRemoteServerUrl,
  remoteApiKey,
  replacementOwnerToken,
}: RemoteApiKeyForSaveArgs): string | null {
  // Replacing the Owner Token while already connected to a remote server
  // must carry the replacement into that connection. When switching server
  // destinations, however, the remote-token field is the credential for the
  // destination; the server being left may legitimately have no token.
  const keepsCurrentRemoteServer =
    currentMode === "remote" &&
    previousMode === "remote" &&
    normalizedRemoteServerUrl(currentRemoteServerUrl) ===
      normalizedRemoteServerUrl(previousRemoteServerUrl)
  if (keepsCurrentRemoteServer && replacementOwnerToken !== null) {
    return replacementOwnerToken.trim() || null
  }
  return remoteApiKey.trim() || null
}
