export type PetRef = string

export const BUILTIN_DEFAULT_PET_REF = "builtin:hope-default" as const
export const BUILTIN_DEBUG_PET_REF = "builtin:hope-debug" as const
export const BUILTIN_DEBUG_PET_ASSET_ID = "builtin/hope-debug" as const

export interface PetConfig {
  enabled: boolean
  selectedPetRef: PetRef
}

export interface PetManifest {
  id: string
  displayName: string
  description?: string
  spriteVersionNumber: 1 | 2
  spritesheetPath: string
}

export interface PetSummary {
  petRef: PetRef
  manifest: PetManifest
  assetId: string
  sourceKind: "builtin" | "codex" | "file" | "zip" | "url" | "created"
  packageHash: string
  builtin: boolean
}

export interface PetLibrarySnapshot {
  revision: number
  selectedPetRef: PetRef
  selectedPetAvailable: boolean
  pets: PetSummary[]
}

export type PetActivityStatus = "needs_input" | "blocked" | "ready" | "running"

export type PetNavigationTarget =
  | { kind: "regular"; sessionId: string; projectId?: string | null }
  | { kind: "side"; sessionId: string; sourceSessionId: string }
  | {
      kind: "knowledge"
      sessionId: string
      kbId: string
      anchorNotePath?: string | null
    }
  | {
      kind: "design"
      sessionId: string
      projectId: string
      artifactId?: string | null
    }

export interface PetActivity {
  activityId: string
  status: PetActivityStatus
  title?: string | null
  titleKind: "session" | "incognito" | "untitled"
  agentId?: string | null
  updatedAt: string
  boundary?: number | null
  preview?: string | null
  target: PetNavigationTarget
}

export interface PetActivitySnapshot {
  revision: number
  generatedAt: string
  stale: boolean
  dominant?: PetActivityStatus | null
  activities: PetActivity[]
  total: number
  truncated: boolean
}

export interface PetValidationIssue {
  code: string
  severity: "warning" | "error"
  message: string
}

export interface PetImportCandidate {
  candidateId: string
  displayName: string
  sourceKind: string
  width: number
  height: number
  inferredVersion?: 1 | 2
  issues: PetValidationIssue[]
}

export interface PetCandidatePage {
  candidates: PetImportCandidate[]
  total: number
  truncated: boolean
}

export type PetImportSource =
  | { kind: "candidate"; candidateId: string }
  | { kind: "localPath"; path: string }
  | { kind: "localPaths"; paths: string[] }
  | { kind: "uploadedPath"; uploadId: string }
  | { kind: "uploadedFiles"; uploadIds: string[] }
  | { kind: "link"; link: string }

export interface PetImportPreview {
  previewToken: string
  manifest: PetManifest
  width: number
  height: number
  issues: PetValidationIssue[]
  assetHash: string
  packageHash: string
  duplicatePetRef?: PetRef
}

export interface PetUpgradeResult {
  pet: PetSummary
  upgraded: boolean
}
