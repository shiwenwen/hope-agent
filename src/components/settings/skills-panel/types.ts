export {
  type SkillSummary,
  type SkillInstallSpec,
  type SkillStatus,
} from "../types"

export interface SkillFileInfo {
  name: string
  size: number
  is_dir: boolean
}

export interface SkillRequires {
  bins: string[]
  any_bins?: string[]
  env: string[]
  os: string[]
  config?: string[]
  always?: boolean
  primary_env?: string
}

export interface SkillDetail {
  name: string
  description: string
  source: string
  file_path: string
  base_dir: string
  content: string
  enabled: boolean
  files: SkillFileInfo[]
  requires: SkillRequires
  skill_key?: string
  user_invocable?: boolean
  disable_model_invocation?: boolean
  command_dispatch?: string
  command_tool?: string
  install?: import("../types").SkillInstallSpec[]
  status?: import("../types").SkillStatus
  authored_by?: string
  rationale?: string
}
