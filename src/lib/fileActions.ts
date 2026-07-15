/** UI metadata for the unified file actions resolved by `useFileResource`. */

import {
  Download,
  ExternalLink,
  Eye,
  FilePenLine,
  FilePlus,
  FolderOpen,
  FolderPlus,
  Save,
  Trash2,
  Upload,
  type LucideIcon,
} from "lucide-react"
import type { FileAction } from "@/components/chat/files/types"

export type { FileAction } from "@/components/chat/files/types"

/** i18n key + fallback label + icon for each action (UI rendering metadata). */
export const FILE_ACTION_META: Record<
  FileAction,
  { labelKey: string; defaultLabel: string; icon: LucideIcon }
> = {
  preview: { labelKey: "fileActions.preview", defaultLabel: "Preview", icon: Eye },
  open: { labelKey: "fileActions.open", defaultLabel: "Open", icon: ExternalLink },
  download: { labelKey: "fileActions.download", defaultLabel: "Download", icon: Download },
  reveal: {
    labelKey: "fileActions.revealInFolder",
    defaultLabel: "Reveal in folder",
    icon: FolderOpen,
  },
  edit: { labelKey: "fileActions.edit", defaultLabel: "Edit", icon: FilePenLine },
  remove: { labelKey: "fileActions.remove", defaultLabel: "Remove", icon: Trash2 },
  rename: { labelKey: "fileActions.rename", defaultLabel: "Rename", icon: FilePenLine },
  delete: { labelKey: "fileActions.delete", defaultLabel: "Delete", icon: Trash2 },
  createFile: { labelKey: "fileActions.createFile", defaultLabel: "New file", icon: FilePlus },
  createFolder: {
    labelKey: "fileActions.createFolder",
    defaultLabel: "New folder",
    icon: FolderPlus,
  },
  upload: { labelKey: "fileActions.upload", defaultLabel: "Upload", icon: Upload },
  saveAs: { labelKey: "fileActions.saveAs", defaultLabel: "Save as", icon: Save },
}
