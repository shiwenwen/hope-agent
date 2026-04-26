import {
  User,
  MessageSquareHeart,
  FolderKanban,
  BookOpen,
} from "lucide-react"

// ── Types ─────────────────────────────────────────────────────────

export interface MemoryEntry {
  id: number
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string }
  content: string
  tags: string[]
  source: string
  sourceSessionId?: string | null
  createdAt: string
  updatedAt: string
  relevanceScore?: number | null
  pinned?: boolean
}

export interface MemorySearchQuery {
  query: string
  types?: string[] | null
  scope?: { kind: "global" } | { kind: "agent"; id: string } | null
  agentId?: string | null
  limit?: number | null
}

export interface NewMemory {
  memoryType: "user" | "feedback" | "project" | "reference"
  scope: { kind: "global" } | { kind: "agent"; id: string }
  content: string
  tags: string[]
  source: string
}

export interface EmbeddingConfig {
  enabled: boolean
  providerType: string
  apiBaseUrl?: string | null
  apiKey?: string | null
  apiModel?: string | null
  apiDimensions?: number | null
  localModelId?: string | null
}

export interface EmbeddingPreset {
  name: string
  providerType: string
  baseUrl: string
  defaultModel: string
  defaultDimensions: number
}

export interface LocalEmbeddingModel {
  id: string
  name: string
  dimensions: number
  sizeMb: number
  minRamGb: number
  languages: string[]
  downloaded: boolean
}

export interface OllamaEmbeddingModel {
  id: string
  displayName: string
  dimensions: number
  sizeMb: number
  contextWindow: number
  languages: string[]
  minOllamaVersion?: string | null
  installed: boolean
  recommended: boolean
}

export type { AgentInfo } from "@/types/chat"

export interface MemoryStats {
  total: number
  byType: Record<string, number>
  withEmbedding: number
}

export type MemoryView = "list" | "add" | "edit" | "embedding"

// ── Constants ─────────────────────────────────────────────────────

export const MEMORY_TYPES = ["user", "feedback", "project", "reference"] as const

export const MEMORY_TYPE_ICONS: Record<string, typeof User> = {
  user: User,
  feedback: MessageSquareHeart,
  project: FolderKanban,
  reference: BookOpen,
}
