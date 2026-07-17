// ── Types ─────────────────────────────────────────────────────────

export type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"
export type ThinkingStyleType = "openai" | "anthropic" | "zai" | "qwen" | "none"

export interface ModelConfig {
  id: string
  name: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  thinkingStyle?: ThinkingStyleType | null
  /**
   * 每百万输入 token 单价。`null` = 未标价（厂商单价未知），`0` = 明确不按 token 计费
   * （本地模型、包月端点）。二者对成本统计含义不同：未标价回退内置估算表，明确免费如实记 $0。
   * 币种：字段不带货币维度，各 Provider 混用（如 qwen 存的是人民币价）；新增条目请与同一
   * Provider 内的兄弟条目保持同一口径。
   */
  costInput: number | null
  /** 每百万输出 token 单价。语义同 `costInput`。 */
  costOutput: number | null
}

export interface AuthProfile {
  id: string
  label: string
  apiKey: string
  baseUrl?: string
  enabled: boolean
}

/** 模型单价币种。与后端 `provider::Currency` 对齐；缺省 = USD（历史默认）。 */
export type Currency = "USD" | "CNY"

export interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  authProfiles: AuthProfile[]
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
  thinkingStyle: ThinkingStyleType
  allowPrivateNetwork?: boolean
  /**
   * 本 Provider 模型单价的币种。单价照厂商价目页原文录入（如 qwen 存人民币价），
   * 成本入账在后端 `resolve_cost` 单点换算成 USD；这里只声明口径不做换算。
   */
  currency?: Currency
}

export interface ProviderTemplate {
  key: string
  name: string
  description: string
  icon: string // emoji
  apiType: ApiType
  baseUrl: string
  apiKeyPlaceholder: string
  requiresApiKey: boolean
  models: ModelConfig[]
  thinkingStyle?: ThinkingStyleType
  /** 模板单价币种，实例化时透传进 ProviderConfig.currency。缺省 = USD。 */
  currency?: Currency
}
