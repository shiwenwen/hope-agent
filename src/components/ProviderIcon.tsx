import {
  Anthropic,
  OpenAI,
  DeepSeek,
  Gemini,
  Grok,
  Mistral,
  OpenRouter,
  Groq,
  Moonshot,
  Qwen,
  Doubao,
  Zhipu,
  Minimax,
  Kimi,
  XiaomiMiMo,
  Baidu,
  AlibabaCloud,
  Nvidia,
  Together,
  Ollama,
  Vllm,
  LmStudio,
  Codex,
} from "@lobehub/icons"
import { Settings2 } from "lucide-react"

// ── Types ──────────────────────────────────────────────────────────

type IconComponent = React.ComponentType<{ size?: number | string; className?: string; style?: React.CSSProperties }>

interface IconEntry {
  mono: IconComponent
  color?: IconComponent        // .Color sub-component (brand original colors)
  colorPrimary?: string        // fallback: tint Mono with this color
}

// ── Provider Key → Icon 映射 ──────────────────────────────────────

const ICON_MAP: Record<string, IconEntry> = {
  anthropic:      { mono: Anthropic },
  openai:         { mono: OpenAI },
  "openai-chat":  { mono: OpenAI },
  deepseek:       { mono: DeepSeek,       color: DeepSeek.Color,       colorPrimary: "#4D6BFE" },
  "google-gemini":{ mono: Gemini,         color: Gemini.Color },
  xai:            { mono: Grok },
  mistral:        { mono: Mistral,        color: Mistral.Color },
  openrouter:     { mono: OpenRouter,     colorPrimary: "#6566F1" },
  groq:           { mono: Groq,           colorPrimary: "#F55036" },
  moonshot:       { mono: Moonshot },
  qwen:           { mono: Qwen,           color: Qwen.Color },
  volcengine:     { mono: Doubao,         color: Doubao.Color },
  zhipu:          { mono: Zhipu,          color: Zhipu.Color },
  minimax:        { mono: Minimax,        color: Minimax.Color },
  "kimi-coding":  { mono: Kimi,           color: Kimi.Color },
  xiaomi:         { mono: XiaomiMiMo },
  qianfan:        { mono: Baidu,          color: Baidu.Color },
  modelstudio:    { mono: AlibabaCloud,   color: AlibabaCloud.Color },
  nvidia:         { mono: Nvidia,         color: Nvidia.Color },
  together:       { mono: Together,       color: Together.Color },
  ollama:         { mono: Ollama },
  vllm:           { mono: Vllm,           color: Vllm.Color },
  "lm-studio":    { mono: LmStudio,      colorPrimary: "#4338CA" },
  codex:          { mono: Codex,          color: Codex.Color },
}

// ── Name → Key 模糊匹配（用于已持久化的 Provider） ────────────────

const NAME_KEY_MAP: [RegExp, string][] = [
  [/anthropic/i, "anthropic"],
  [/openai/i, "openai"],
  [/deepseek/i, "deepseek"],
  [/gemini/i, "google-gemini"],
  [/grok|xai/i, "xai"],
  [/mistral/i, "mistral"],
  [/openrouter/i, "openrouter"],
  [/groq/i, "groq"],
  [/moonshot|kimi/i, "moonshot"],
  [/qwen|千问|dashscope/i, "qwen"],
  [/doubao|豆包|火山|volcengine/i, "volcengine"],
  [/zhipu|智谱|glm|z\.ai/i, "zhipu"],
  [/minimax/i, "minimax"],
  [/kimi.*coding/i, "kimi-coding"],
  [/xiaomi|mimo/i, "xiaomi"],
  [/qianfan|千帆|百度|baidu|ernie/i, "qianfan"],
  [/modelstudio|阿里云|alibaba/i, "modelstudio"],
  [/nvidia/i, "nvidia"],
  [/together/i, "together"],
  [/ollama/i, "ollama"],
  [/vllm/i, "vllm"],
  [/lm.?studio/i, "lm-studio"],
  [/codex/i, "codex"],
]

function resolveKey(providerKey?: string, providerName?: string): string | undefined {
  if (providerKey && ICON_MAP[providerKey]) return providerKey
  if (providerName) {
    for (const [re, key] of NAME_KEY_MAP) {
      if (re.test(providerName)) return key
    }
  }
  return undefined
}

// ── Component ─────────────────────────────────────────────────────

interface ProviderIconProps {
  providerKey?: string
  providerName?: string
  size?: number
  className?: string
  /** true = 渲染品牌原始颜色（优先 Color 变体，fallback 用 colorPrimary 着色 Mono） */
  color?: boolean
}

export default function ProviderIcon({
  providerKey,
  providerName,
  size = 20,
  className,
  color = false,
}: ProviderIconProps) {
  const key = resolveKey(providerKey, providerName)
  const entry = key ? ICON_MAP[key] : undefined

  if (!entry) {
    // Fallback: generic settings icon
    return <Settings2 style={{ width: size, height: size }} className={className} />
  }

  if (color) {
    // Prefer .Color sub-component if available
    if (entry.color) {
      const ColorIcon = entry.color
      return <ColorIcon size={size} className={className} />
    }
    // Fallback: tint Mono with colorPrimary
    if (entry.colorPrimary) {
      const MonoIcon = entry.mono
      return <MonoIcon size={size} className={className} style={{ color: entry.colorPrimary }} />
    }
  }

  // Default: Mono (inherits parent color)
  const MonoIcon = entry.mono
  return <MonoIcon size={size} className={className} />
}
