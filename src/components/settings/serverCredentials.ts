type ServerMode = "embedded" | "remote"

interface RemoteApiKeyForSaveArgs {
  currentMode: ServerMode
  previousMode: ServerMode
  currentRemoteServerUrl: string
  previousRemoteServerUrl: string
  remoteApiKey: string
  replacementOwnerToken: string | null
  activeRemoteMatchesDestination: boolean
}

interface OwnerTokenWillExistArgs {
  replacementOwnerToken: string | null
  hasManagedOwnerToken: boolean
  externallyManaged: boolean
}

interface RemoteValidationOrderArgs {
  currentMode: ServerMode
  previousMode: ServerMode
  currentRemoteServerUrl: string
  previousRemoteServerUrl: string
  replacementOwnerToken: string | null
  activeRemoteMatchesDestination: boolean
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
  activeRemoteMatchesDestination,
}: RemoteApiKeyForSaveArgs): string | null {
  // Replacing the Owner Token while already connected to a remote server
  // must carry the replacement into that connection. When switching server
  // destinations, however, the remote-token field is the credential for the
  // destination; the server being left may legitimately have no token.
  const keepsCurrentRemoteServer =
    currentMode === "remote" &&
    previousMode === "remote" &&
    activeRemoteMatchesDestination &&
    normalizedRemoteServerUrl(currentRemoteServerUrl) ===
      normalizedRemoteServerUrl(previousRemoteServerUrl)
  if (keepsCurrentRemoteServer && replacementOwnerToken !== null) {
    return replacementOwnerToken.trim() || null
  }
  return remoteApiKey.trim() || null
}

export function ownerTokenWillExist({
  replacementOwnerToken,
  hasManagedOwnerToken,
  externallyManaged,
}: OwnerTokenWillExistArgs): boolean {
  if (externallyManaged) return true
  return replacementOwnerToken === null ? hasManagedOwnerToken : replacementOwnerToken.length > 0
}

/**
 * A different destination can always be validated before touching the current
 * server. The only deferred case is replacing the Owner Token of the same
 * active remote, because that new credential is invalid until the mutation.
 */
export function shouldPrepareRemoteBeforeServerMutation({
  currentMode,
  previousMode,
  currentRemoteServerUrl,
  previousRemoteServerUrl,
  replacementOwnerToken,
  activeRemoteMatchesDestination,
}: RemoteValidationOrderArgs): boolean {
  if (currentMode !== "remote") return false
  const keepsCurrentRemoteServer =
    previousMode === "remote" &&
    activeRemoteMatchesDestination &&
    normalizedRemoteServerUrl(currentRemoteServerUrl) ===
      normalizedRemoteServerUrl(previousRemoteServerUrl)
  return !keepsCurrentRemoteServer || replacementOwnerToken === null
}
