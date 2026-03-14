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

// ── Provider Key → Icon 映射 ──────────────────────────────────────

const ICON_MAP: Record<string, React.ComponentType<{ size?: number | string; className?: string }>> = {
  anthropic: Anthropic,
  openai: OpenAI,
  "openai-chat": OpenAI,
  deepseek: DeepSeek,
  "google-gemini": Gemini,
  xai: Grok,
  mistral: Mistral,
  openrouter: OpenRouter,
  groq: Groq,
  moonshot: Moonshot,
  qwen: Qwen,
  volcengine: Doubao,
  zhipu: Zhipu,
  minimax: Minimax,
  "kimi-coding": Kimi,
  xiaomi: XiaomiMiMo,
  qianfan: Baidu,
  modelstudio: AlibabaCloud,
  nvidia: Nvidia,
  together: Together,
  ollama: Ollama,
  vllm: Vllm,
  "lm-studio": LmStudio,
  codex: Codex,
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
}

export default function ProviderIcon({
  providerKey,
  providerName,
  size = 20,
  className,
}: ProviderIconProps) {
  const key = resolveKey(providerKey, providerName)
  const IconComponent = key ? ICON_MAP[key] : undefined

  if (IconComponent) {
    return <IconComponent size={size} className={className} />
  }

  // Fallback: generic settings icon
  return <Settings2 style={{ width: size, height: size }} className={className} />
}
