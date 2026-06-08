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
  Bailian,
  Nvidia,
  Together,
  Ollama,
  Vllm,
  LmStudio,
  Codex,
  GithubCopilot,
  Novita,
  KiloCode,
  OpenCode,
  Arcee,
  Cerebras,
  Cloudflare,
  DeepInfra,
  Fireworks,
  HuggingFace,
  Stepfun,
  Hunyuan,
  Vercel,
  AssemblyAI,
  Azure,
  IFlyTekCloud,
} from "@lobehub/icons"
import { Mic, Settings2 } from "lucide-react"

// ── Types ──────────────────────────────────────────────────────────

type IconComponent = React.ComponentType<{
  size?: number | string
  className?: string
  style?: React.CSSProperties
}>

interface IconEntry {
  mono: IconComponent
  color?: IconComponent // .Color sub-component (brand original colors)
  colorPrimary?: string // fallback: tint Mono with this color
}

// ── Provider Key → Icon 映射 ──────────────────────────────────────

const ICON_MAP: Record<string, IconEntry> = {
  anthropic: { mono: Anthropic },
  "anthropic-vertex": { mono: Anthropic },
  openai: { mono: OpenAI },
  "openai-chat": { mono: OpenAI },
  deepseek: { mono: DeepSeek, color: DeepSeek.Color, colorPrimary: "#4D6BFE" },
  "google-gemini": { mono: Gemini, color: Gemini.Color },
  xai: { mono: Grok },
  mistral: { mono: Mistral, color: Mistral.Color },
  openrouter: { mono: OpenRouter, colorPrimary: "#6566F1" },
  groq: { mono: Groq, colorPrimary: "#F55036" },
  moonshot: { mono: Moonshot },
  qwen: { mono: Qwen, color: Qwen.Color },
  volcengine: { mono: Doubao, color: Doubao.Color },
  zhipu: { mono: Zhipu, color: Zhipu.Color },
  minimax: { mono: Minimax, color: Minimax.Color },
  "kimi-coding": { mono: Kimi, color: Kimi.Color },
  xiaomi: { mono: XiaomiMiMo },
  qianfan: { mono: Baidu, color: Baidu.Color },
  modelstudio: { mono: Bailian, color: Bailian.Color },
  nvidia: { mono: Nvidia, color: Nvidia.Color },
  together: { mono: Together, color: Together.Color },
  ollama: { mono: Ollama },
  vllm: { mono: Vllm, color: Vllm.Color },
  "lm-studio": { mono: LmStudio, colorPrimary: "#4338CA" },
  codex: { mono: Codex, color: Codex.Color },
  "github-copilot": { mono: GithubCopilot },
  novita: { mono: Novita, color: Novita.Color },
  kilocode: { mono: KiloCode },
  "opencode-go": { mono: OpenCode },
  arcee: { mono: Arcee, color: Arcee.Color },
  cerebras: { mono: Cerebras, color: Cerebras.Color },
  "cloudflare-ai": { mono: Cloudflare, color: Cloudflare.Color },
  deepinfra: { mono: DeepInfra, color: DeepInfra.Color },
  fireworks: { mono: Fireworks, color: Fireworks.Color },
  huggingface: { mono: HuggingFace, color: HuggingFace.Color },
  stepfun: { mono: Stepfun, color: Stepfun.Color },
  tencent: { mono: Hunyuan, color: Hunyuan.Color },
  "vercel-ai-gateway": { mono: Vercel },
  // ── STT (voice input) providers ───────────────────────────────────
  assemblyai: { mono: AssemblyAI, color: AssemblyAI.Color },
  azure: { mono: Azure, color: Azure.Color },
  iflytek: { mono: IFlyTekCloud, color: IFlyTekCloud.Color },
  /** LobeHub doesn't ship a Deepgram brand mark; use a generic mic. */
  deepgram: { mono: Mic, colorPrimary: "#13EF93" },
  /** Alias for the OpenAI ASR endpoints. */
  "openai-whisper": { mono: OpenAI },
  /** DashScope (Bailian) is Alibaba Cloud's model gateway — Qwen mark
   * is the appropriate brand since Qwen3-ASR is the actual model. */
  dashscope: { mono: Qwen, color: Qwen.Color },
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
  [/github.?copilot|copilot/i, "github-copilot"],
  [/novita/i, "novita"],
  [/kilo.?code/i, "kilocode"],
  [/opencode/i, "opencode-go"],
  [/arcee/i, "arcee"],
  [/cerebras/i, "cerebras"],
  [/cloudflare/i, "cloudflare-ai"],
  [/deepinfra/i, "deepinfra"],
  [/fireworks/i, "fireworks"],
  [/hugging.?face/i, "huggingface"],
  [/stepfun|阶跃/i, "stepfun"],
  [/tencent|腾讯|混元|hunyuan/i, "tencent"],
  [/vercel/i, "vercel-ai-gateway"],
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
    return <Settings2 size={size} className={className} />
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
