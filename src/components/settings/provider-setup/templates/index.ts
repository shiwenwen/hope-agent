import type { ProviderTemplate } from "../types"
import { internationalTemplates } from "./international"
import { chinaTemplates } from "./china"
import { infrastructureTemplates } from "./infrastructure"
import { localTemplates } from "./local"

export const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  ...internationalTemplates,
  ...chinaTemplates,
  ...infrastructureTemplates,
  ...localTemplates,
]
