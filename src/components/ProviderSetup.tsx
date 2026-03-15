import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import ProviderIcon from "@/components/ProviderIcon"
import TestResultDisplay, { parseTestResult, type TestResult } from "@/components/TestResultDisplay"
import {
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCircle2,
  ChevronDown,
  Clock,
  Globe,
  Info,
  Key,
  Loader2,
  Play,
  Plus,
  Search,
  Settings2,
  Trash2,
  Type,
  Image,
  Video,
  X,
  XCircle,
} from "lucide-react"

// ── Types ─────────────────────────────────────────────────────────

type ApiType = "anthropic" | "openai-chat" | "openai-responses" | "codex"

export interface ModelConfig {
  id: string
  name: string
  inputTypes: string[]
  contextWindow: number
  maxTokens: number
  reasoning: boolean
  costInput: number
  costOutput: number
}

interface ProviderConfig {
  id: string
  name: string
  apiType: ApiType
  baseUrl: string
  apiKey: string
  models: ModelConfig[]
  enabled: boolean
  userAgent: string
}

// ── Built-in Provider Templates ───────────────────────────────────

interface ProviderTemplate {
  key: string
  name: string
  description: string
  icon: string // emoji
  apiType: ApiType
  baseUrl: string
  apiKeyPlaceholder: string
  requiresApiKey: boolean
  models: ModelConfig[]
}

const PROVIDER_TEMPLATES: ProviderTemplate[] = [
  // ── 国际 Provider ──
  {
    key: "anthropic",
    name: "Anthropic",
    description: "Claude 系列模型",
    icon: "🟤",
    apiType: "anthropic",
    baseUrl: "https://api.anthropic.com",
    apiKeyPlaceholder: "sk-ant-...",
    requiresApiKey: true,
    models: [
      { id: "claude-sonnet-4-6", name: "Claude Sonnet 4.6", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 8192, reasoning: false, costInput: 3.0, costOutput: 15.0 },
      { id: "claude-opus-4-6", name: "Claude Opus 4.6", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 8192, reasoning: true, costInput: 15.0, costOutput: 75.0 },
      { id: "claude-haiku-3-5", name: "Claude Haiku 3.5", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 8192, reasoning: false, costInput: 0.25, costOutput: 1.25 },
    ],
  },
  {
    key: "openai",
    name: "OpenAI",
    description: "GPT 系列模型 (Responses API)",
    icon: "🟢",
    apiType: "openai-responses",
    baseUrl: "https://api.openai.com",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "gpt-4o", name: "GPT-4o", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 16384, reasoning: false, costInput: 2.5, costOutput: 10.0 },
      { id: "gpt-4o-mini", name: "GPT-4o Mini", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 16384, reasoning: false, costInput: 0.15, costOutput: 0.6 },
      { id: "o3", name: "GPT o3", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 100000, reasoning: true, costInput: 10.0, costOutput: 40.0 },
      { id: "o4-mini", name: "GPT o4-mini", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 100000, reasoning: true, costInput: 1.1, costOutput: 4.4 },
    ],
  },
  {
    key: "openai-chat",
    name: "OpenAI (Chat)",
    description: "GPT 系列 — Chat Completions API",
    icon: "💬",
    apiType: "openai-chat",
    baseUrl: "https://api.openai.com",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "gpt-4o", name: "GPT-4o", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 16384, reasoning: false, costInput: 2.5, costOutput: 10.0 },
      { id: "gpt-4o-mini", name: "GPT-4o Mini", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 16384, reasoning: false, costInput: 0.15, costOutput: 0.6 },
      { id: "o3", name: "GPT o3", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 100000, reasoning: true, costInput: 10.0, costOutput: 40.0 },
    ],
  },
  {
    key: "deepseek",
    name: "DeepSeek",
    description: "DeepSeek 系列推理模型",
    icon: "🔵",
    apiType: "openai-chat",
    baseUrl: "https://api.deepseek.com",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "deepseek-chat", name: "DeepSeek V3", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0.27, costOutput: 1.1 },
      { id: "deepseek-reasoner", name: "DeepSeek R1", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: true, costInput: 0.55, costOutput: 2.19 },
    ],
  },
  {
    key: "google-gemini",
    name: "Google Gemini",
    description: "Gemini 系列多模态模型",
    icon: "💎",
    apiType: "openai-chat",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    apiKeyPlaceholder: "AIza...",
    requiresApiKey: true,
    models: [
      { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro", inputTypes: ["text", "image", "video"], contextWindow: 1000000, maxTokens: 65536, reasoning: true, costInput: 1.25, costOutput: 10.0 },
      { id: "gemini-2.5-flash", name: "Gemini 2.5 Flash", inputTypes: ["text", "image", "video"], contextWindow: 1000000, maxTokens: 65536, reasoning: true, costInput: 0.15, costOutput: 0.6 },
    ],
  },
  {
    key: "xai",
    name: "xAI",
    description: "Grok 系列模型",
    icon: "✖️",
    apiType: "openai-chat",
    baseUrl: "https://api.x.ai",
    apiKeyPlaceholder: "xai-...",
    requiresApiKey: true,
    models: [
      { id: "grok-3", name: "Grok 3", inputTypes: ["text"], contextWindow: 131072, maxTokens: 16384, reasoning: false, costInput: 3.0, costOutput: 15.0 },
      { id: "grok-3-mini", name: "Grok 3 Mini", inputTypes: ["text"], contextWindow: 131072, maxTokens: 16384, reasoning: true, costInput: 0.3, costOutput: 0.5 },
    ],
  },
  {
    key: "mistral",
    name: "Mistral",
    description: "Mistral 系列欧洲模型",
    icon: "🟣",
    apiType: "openai-chat",
    baseUrl: "https://api.mistral.ai",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "mistral-large-latest", name: "Mistral Large", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 2.0, costOutput: 6.0 },
      { id: "codestral-latest", name: "Codestral", inputTypes: ["text"], contextWindow: 256000, maxTokens: 8192, reasoning: false, costInput: 0.3, costOutput: 0.9 },
    ],
  },
  {
    key: "openrouter",
    name: "OpenRouter",
    description: "多模型聚合网关，一个 Key 用数百个模型",
    icon: "🔀",
    apiType: "openai-chat",
    baseUrl: "https://openrouter.ai/api/v1",
    apiKeyPlaceholder: "sk-or-...",
    requiresApiKey: true,
    models: [
      { id: "anthropic/claude-sonnet-4-5", name: "Claude Sonnet 4.5", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 8192, reasoning: false, costInput: 3.0, costOutput: 15.0 },
      { id: "openai/gpt-4o", name: "GPT-4o", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 16384, reasoning: false, costInput: 2.5, costOutput: 10.0 },
      { id: "google/gemini-2.5-pro-preview", name: "Gemini 2.5 Pro", inputTypes: ["text", "image"], contextWindow: 1000000, maxTokens: 65536, reasoning: true, costInput: 1.25, costOutput: 10.0 },
      { id: "deepseek/deepseek-r1", name: "DeepSeek R1", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: true, costInput: 0.55, costOutput: 2.19 },
      { id: "openrouter/hunter-alpha", name: "Hunter Alpha", inputTypes: ["text"], contextWindow: 1048576, maxTokens: 65536, reasoning: true, costInput: 0, costOutput: 0 },
      { id: "openrouter/healer-alpha", name: "Healer Alpha", inputTypes: ["text", "image"], contextWindow: 262144, maxTokens: 65536, reasoning: true, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "groq",
    name: "Groq",
    description: "超高速 LPU 推理",
    icon: "🚀",
    apiType: "openai-chat",
    baseUrl: "https://api.groq.com/openai",
    apiKeyPlaceholder: "gsk_...",
    requiresApiKey: true,
    models: [
      { id: "llama-3.3-70b-versatile", name: "Llama 3.3 70B", inputTypes: ["text"], contextWindow: 128000, maxTokens: 32768, reasoning: false, costInput: 0.59, costOutput: 0.79 },
      { id: "mixtral-8x7b-32768", name: "Mixtral 8x7B", inputTypes: ["text"], contextWindow: 32768, maxTokens: 32768, reasoning: false, costInput: 0.24, costOutput: 0.24 },
    ],
  },
  // ── 国内 Provider ──
  {
    key: "moonshot",
    name: "Moonshot AI (Kimi)",
    description: "Kimi 系列长上下文模型",
    icon: "🌙",
    apiType: "openai-chat",
    baseUrl: "https://api.moonshot.ai/v1",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "kimi-k2.5", name: "Kimi K2.5", inputTypes: ["text", "image"], contextWindow: 256000, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "kimi-k2-thinking", name: "Kimi K2 Thinking", inputTypes: ["text"], contextWindow: 256000, maxTokens: 8192, reasoning: true, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "qwen",
    name: "通义千问 (Qwen)",
    description: "阿里云 DashScope 大模型",
    icon: "☁️",
    apiType: "openai-chat",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "qwen-max", name: "Qwen Max", inputTypes: ["text"], contextWindow: 32768, maxTokens: 8192, reasoning: false, costInput: 2.4, costOutput: 9.6 },
      { id: "qwen-plus", name: "Qwen Plus", inputTypes: ["text"], contextWindow: 131072, maxTokens: 8192, reasoning: false, costInput: 0.8, costOutput: 2.0 },
      { id: "qwen-turbo", name: "Qwen Turbo", inputTypes: ["text"], contextWindow: 131072, maxTokens: 8192, reasoning: false, costInput: 0.3, costOutput: 0.6 },
      { id: "qwq-plus", name: "QwQ Plus (推理)", inputTypes: ["text"], contextWindow: 131072, maxTokens: 16384, reasoning: true, costInput: 1.6, costOutput: 4.0 },
    ],
  },
  {
    key: "volcengine",
    name: "火山引擎 (豆包)",
    description: "字节跳动 Doubao 系列模型",
    icon: "🌋",
    apiType: "openai-chat",
    baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "doubao-seed-1-8-251228", name: "Doubao Seed 1.8", inputTypes: ["text", "image"], contextWindow: 256000, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "doubao-seed-code-preview-251028", name: "Doubao Seed Code", inputTypes: ["text", "image"], contextWindow: 256000, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "kimi-k2-5-260127", name: "Kimi K2.5", inputTypes: ["text", "image"], contextWindow: 256000, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "glm-4-7-251222", name: "GLM 4.7", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "deepseek-v3-2-251201", name: "DeepSeek V3.2", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "zhipu",
    name: "智谱 AI (Z.AI)",
    description: "GLM 系列模型",
    icon: "🧠",
    apiType: "openai-chat",
    baseUrl: "https://open.bigmodel.cn/api/paas",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "glm-5", name: "GLM-5", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "glm-4-plus", name: "GLM-4 Plus", inputTypes: ["text", "image"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0.5, costOutput: 0.5 },
    ],
  },
  {
    key: "minimax",
    name: "MiniMax",
    description: "MiniMax M2 系列 (Anthropic 兼容)",
    icon: "🔶",
    apiType: "anthropic",
    baseUrl: "https://api.minimax.io/anthropic",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "MiniMax-VL-01", name: "MiniMax VL 01", inputTypes: ["text", "image"], contextWindow: 200000, maxTokens: 8192, reasoning: false, costInput: 0.3, costOutput: 1.2 },
      { id: "MiniMax-M2.5", name: "MiniMax M2.5", inputTypes: ["text"], contextWindow: 200000, maxTokens: 8192, reasoning: true, costInput: 0.3, costOutput: 1.2 },
      { id: "MiniMax-M2.5-highspeed", name: "MiniMax M2.5 Highspeed", inputTypes: ["text"], contextWindow: 200000, maxTokens: 8192, reasoning: true, costInput: 0.3, costOutput: 1.2 },
    ],
  },
  {
    key: "kimi-coding",
    name: "Kimi Coding",
    description: "Kimi for Coding (Anthropic 兼容)",
    icon: "🌑",
    apiType: "anthropic",
    baseUrl: "https://api.kimi.com/coding",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "k2p5", name: "Kimi for Coding", inputTypes: ["text", "image"], contextWindow: 262144, maxTokens: 32768, reasoning: true, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "xiaomi",
    name: "小米 MiMo",
    description: "MiMo 系列模型 (Anthropic 兼容)",
    icon: "📱",
    apiType: "anthropic",
    baseUrl: "https://api.xiaomimimo.com/anthropic",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "mimo-v2-flash", name: "MiMo V2 Flash", inputTypes: ["text"], contextWindow: 262144, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "qianfan",
    name: "百度千帆",
    description: "百度智能云 千帆大模型平台",
    icon: "🔴",
    apiType: "openai-chat",
    baseUrl: "https://qianfan.baidubce.com/v2",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "deepseek-v3.2", name: "DeepSeek V3.2", inputTypes: ["text"], contextWindow: 98304, maxTokens: 32768, reasoning: true, costInput: 0, costOutput: 0 },
      { id: "ernie-5.0-thinking-preview", name: "ERNIE 5.0 Thinking", inputTypes: ["text", "image"], contextWindow: 119000, maxTokens: 64000, reasoning: true, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "modelstudio",
    name: "ModelStudio (DashScope)",
    description: "阿里云 Coding 专用端点",
    icon: "🏗️",
    apiType: "openai-chat",
    baseUrl: "https://coding-intl.dashscope.aliyuncs.com/v1",
    apiKeyPlaceholder: "sk-...",
    requiresApiKey: true,
    models: [
      { id: "qwen3.5-plus", name: "Qwen 3.5 Plus", inputTypes: ["text", "image"], contextWindow: 1000000, maxTokens: 65536, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "qwen3-coder-plus", name: "Qwen 3 Coder Plus", inputTypes: ["text"], contextWindow: 1000000, maxTokens: 65536, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "qwen3-coder-next", name: "Qwen 3 Coder Next", inputTypes: ["text"], contextWindow: 262144, maxTokens: 65536, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "qwen3-max-2026-01-23", name: "Qwen 3 Max", inputTypes: ["text"], contextWindow: 262144, maxTokens: 65536, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "MiniMax-M2.5", name: "MiniMax M2.5", inputTypes: ["text"], contextWindow: 1000000, maxTokens: 65536, reasoning: true, costInput: 0, costOutput: 0 },
      { id: "glm-5", name: "GLM-5", inputTypes: ["text"], contextWindow: 202752, maxTokens: 16384, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "glm-4.7", name: "GLM 4.7", inputTypes: ["text"], contextWindow: 202752, maxTokens: 16384, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "kimi-k2.5", name: "Kimi K2.5", inputTypes: ["text", "image"], contextWindow: 262144, maxTokens: 32768, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "nvidia",
    name: "NVIDIA",
    description: "NVIDIA AI Endpoints",
    icon: "💚",
    apiType: "openai-chat",
    baseUrl: "https://integrate.api.nvidia.com/v1",
    apiKeyPlaceholder: "nvapi-...",
    requiresApiKey: true,
    models: [
      { id: "nvidia/llama-3.1-nemotron-70b-instruct", name: "Llama 3.1 Nemotron 70B", inputTypes: ["text"], contextWindow: 131072, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "meta/llama-3.3-70b-instruct", name: "Llama 3.3 70B", inputTypes: ["text"], contextWindow: 131072, maxTokens: 4096, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "nvidia/mistral-nemo-minitron-8b-8k-instruct", name: "Mistral NeMo Minitron 8B", inputTypes: ["text"], contextWindow: 8192, maxTokens: 2048, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "together",
    name: "Together AI",
    description: "开源模型云推理平台",
    icon: "🤝",
    apiType: "openai-chat",
    baseUrl: "https://api.together.xyz/v1",
    apiKeyPlaceholder: "...",
    requiresApiKey: true,
    models: [
      { id: "meta-llama/Llama-3.3-70B-Instruct-Turbo", name: "Llama 3.3 70B Turbo", inputTypes: ["text"], contextWindow: 131072, maxTokens: 8192, reasoning: false, costInput: 0.88, costOutput: 0.88 },
      { id: "moonshotai/Kimi-K2.5", name: "Kimi K2.5", inputTypes: ["text", "image"], contextWindow: 262144, maxTokens: 32768, reasoning: true, costInput: 0.5, costOutput: 2.8 },
      { id: "zai-org/GLM-4.7", name: "GLM 4.7 Fp8", inputTypes: ["text"], contextWindow: 202752, maxTokens: 8192, reasoning: false, costInput: 0.45, costOutput: 2.0 },
      { id: "meta-llama/Llama-4-Scout-17B-16E-Instruct", name: "Llama 4 Scout 17B", inputTypes: ["text", "image"], contextWindow: 10000000, maxTokens: 32768, reasoning: false, costInput: 0.18, costOutput: 0.59 },
      { id: "deepseek-ai/DeepSeek-V3.1", name: "DeepSeek V3.1", inputTypes: ["text"], contextWindow: 131072, maxTokens: 8192, reasoning: false, costInput: 0.6, costOutput: 1.25 },
      { id: "deepseek-ai/DeepSeek-R1", name: "DeepSeek R1", inputTypes: ["text"], contextWindow: 131072, maxTokens: 8192, reasoning: true, costInput: 3.0, costOutput: 7.0 },
      { id: "moonshotai/Kimi-K2-Instruct-0905", name: "Kimi K2 Instruct", inputTypes: ["text"], contextWindow: 262144, maxTokens: 8192, reasoning: false, costInput: 1.0, costOutput: 3.0 },
    ],
  },
  // ── 本地 Provider ──
  {
    key: "ollama",
    name: "Ollama",
    description: "本地模型推理，无需 API Key",
    icon: "🦙",
    apiType: "openai-chat",
    baseUrl: "http://127.0.0.1:11434",
    apiKeyPlaceholder: "（无需填写）",
    requiresApiKey: false,
    models: [
      { id: "llama3.3", name: "Llama 3.3", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
      { id: "qwen3:32b", name: "Qwen 3 32B", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: true, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "vllm",
    name: "vLLM",
    description: "高性能本地推理引擎",
    icon: "⚙️",
    apiType: "openai-chat",
    baseUrl: "http://127.0.0.1:8000",
    apiKeyPlaceholder: "（无需填写）",
    requiresApiKey: false,
    models: [
      { id: "your-model-id", name: "Your Model", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
  {
    key: "lm-studio",
    name: "LM Studio",
    description: "桌面端本地模型推理",
    icon: "🖥️",
    apiType: "openai-chat",
    baseUrl: "http://127.0.0.1:1234",
    apiKeyPlaceholder: "（无需填写）",
    requiresApiKey: false,
    models: [
      { id: "your-model-id", name: "Your Model", inputTypes: ["text"], contextWindow: 128000, maxTokens: 8192, reasoning: false, costInput: 0, costOutput: 0 },
    ],
  },
]

// ── ModelEditor ───────────────────────────────────────────────────

export function ModelEditor({
  model,
  onChange,
  onRemove,
  onTest,
}: {
  model: ModelConfig
  onChange: (m: ModelConfig) => void
  onRemove: () => void
  onTest?: (modelId: string) => Promise<string>
}) {
  const { t } = useTranslation()
  const inputTypes = ["text", "image", "video"]
  const [testLoading, setTestLoading] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; data: any } | null>(null)
  const [logExpanded, setLogExpanded] = useState(false)

  function toggleInput(type: string) {
    const current = model.inputTypes
    if (current.includes(type)) {
      onChange({ ...model, inputTypes: current.filter((t) => t !== type) })
    } else {
      onChange({ ...model, inputTypes: [...current, type] })
    }
  }

  return (
    <div className="border border-border rounded-lg p-3.5 space-y-3 bg-secondary/60">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-medium text-muted-foreground/70 uppercase tracking-wider">
          {t("model.modelConfig")}
        </span>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6 text-muted-foreground hover:text-red-400"
          onClick={onRemove}
        >
          <Trash2 className="h-3 w-3" />
        </Button>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.modelId")}</label>
          <Input
            value={model.id}
            onChange={(e) => onChange({ ...model, id: e.target.value })}
            placeholder="model-id"
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">{t("model.displayName")}</label>
          <Input
            value={model.name}
            onChange={(e) => onChange({ ...model, name: e.target.value })}
            placeholder="Model Name"
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      <div className="space-y-1.5">
        <label className="text-[10px] text-muted-foreground">
          {t("model.supportedInputTypes")}
        </label>
        <div className="flex gap-2">
          {inputTypes.map((type) => (
            <button
              key={type}
              onClick={() => toggleInput(type)}
              className={`px-2.5 py-1 text-[11px] rounded-md border transition-colors flex items-center gap-1.5 ${model.inputTypes.includes(type)
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border bg-background text-muted-foreground hover:border-primary/40"
                }`}
            >
              {type === "text" && <Type className="h-3 w-3" />}
              {type === "image" && <Image className="h-3 w-3" />}
              {type === "video" && <Video className="h-3 w-3" />}
              {type === "text" ? t("model.text") : type === "image" ? t("model.image") : t("model.video")}
            </button>
          ))}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">
            Context Window
          </label>
          <Input
            type="number"
            value={model.contextWindow}
            onChange={(e) =>
              onChange({
                ...model,
                contextWindow: parseInt(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">
            Max Tokens
          </label>
          <Input
            type="number"
            value={model.maxTokens}
            onChange={(e) =>
              onChange({ ...model, maxTokens: parseInt(e.target.value) || 0 })
            }
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-muted-foreground">{t("model.reasoning")}</label>
        <button
          onClick={() => onChange({ ...model, reasoning: !model.reasoning })}
          className={`w-9 h-5 rounded-full transition-colors relative ${model.reasoning ? "bg-primary" : "bg-secondary border border-border"
            }`}
        >
          <span
            className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${model.reasoning ? "left-[18px]" : "left-0.5"
              }`}
          />
        </button>
      </div>

      <div className="grid grid-cols-2 gap-2.5">
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">
            {t("model.inputCost")}
          </label>
          <Input
            type="number"
            step="0.01"
            value={model.costInput}
            onChange={(e) =>
              onChange({
                ...model,
                costInput: parseFloat(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
        <div className="space-y-1">
          <label className="text-[10px] text-muted-foreground">
            {t("model.outputCost")}
          </label>
          <Input
            type="number"
            step="0.01"
            value={model.costOutput}
            onChange={(e) =>
              onChange({
                ...model,
                costOutput: parseFloat(e.target.value) || 0,
              })
            }
            className="bg-background text-xs h-8"
          />
        </div>
      </div>

      {/* Per-model test */}
      {onTest && model.id && (
        <div className="space-y-1.5 pt-1 border-t border-border/50">
          <div className="flex items-center gap-2">
            <button
              onClick={async () => {
                if (!onTest || !model.id) return
                setTestLoading(true)
                setTestResult(null)
                setLogExpanded(false)
                try {
                  const msg = await onTest(model.id)
                  const data = JSON.parse(msg)
                  setTestResult({ ok: data.success ?? true, data })
                } catch (e) {
                  try {
                    const data = JSON.parse(String(e))
                    setTestResult({ ok: false, data })
                  } catch {
                    setTestResult({ ok: false, data: { success: false, message: String(e) } })
                  }
                } finally {
                  setTestLoading(false)
                }
              }}
              disabled={testLoading}
              className="flex items-center gap-1 text-[10px] text-primary/70 hover:text-primary transition-colors disabled:opacity-50"
            >
              {testLoading ? <Loader2 className="h-3 w-3 animate-spin" /> : <Play className="h-3 w-3 fill-current" />}
              发送 "Hi" 测试
            </button>
            {testResult && (
              <span className={`flex items-center gap-1 text-[10px] ${testResult.ok ? "text-green-400" : "text-red-400"}`}>
                {testResult.ok ? <CheckCircle2 className="h-3 w-3" /> : <XCircle className="h-3 w-3" />}
                {testResult.data.message}
                {testResult.data.latencyMs != null && testResult.data.latencyMs > 0 && (
                  <span className="text-muted-foreground flex items-center gap-0.5">
                    <Clock className="h-2.5 w-2.5" />
                    {testResult.data.latencyMs}ms
                  </span>
                )}
                <button
                  onClick={() => setLogExpanded(!logExpanded)}
                  className="text-muted-foreground hover:text-foreground transition-colors ml-0.5"
                  title="查看完整日志"
                >
                  <Info className="h-3 w-3" />
                </button>
              </span>
            )}
          </div>
          {testResult?.ok && testResult.data.reply && (
            <div className="px-2.5 py-1.5 rounded-md bg-secondary/50 text-[10px] text-muted-foreground border border-border/50">
              <span className="text-[9px] font-medium text-foreground/60">AI 回复: </span>
              {testResult.data.reply}
            </div>
          )}
          {logExpanded && testResult && (() => {
            const d = testResult.data
            return (
              <div className="px-2.5 py-2 rounded-md bg-secondary/30 border border-border/50 overflow-hidden space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[9px] font-medium text-foreground/60">完整日志</span>
                  <button onClick={() => setLogExpanded(false)} className="text-muted-foreground hover:text-foreground">
                    <X className="h-2.5 w-2.5" />
                  </button>
                </div>
                {d.request && (
                  <div>
                    <span className="text-[9px] font-semibold text-blue-400">▸ 请求</span>
                    <pre className="text-[10px] text-muted-foreground whitespace-pre-wrap break-all max-h-32 overflow-y-auto font-mono mt-0.5 pl-2 border-l-2 border-blue-500/30">
                      {JSON.stringify(d.request, null, 2)}
                    </pre>
                  </div>
                )}
                <div>
                  <span className={`text-[9px] font-semibold ${d.success ? "text-green-400" : "text-red-400"}`}>▸ 响应 {d.status ? `(${d.status})` : ""}</span>
                  <pre className={`text-[10px] text-muted-foreground whitespace-pre-wrap break-all max-h-40 overflow-y-auto font-mono mt-0.5 pl-2 border-l-2 ${d.success ? "border-green-500/30" : "border-red-500/30"}`}>
                    {d.response ? JSON.stringify(d.response, null, 2) : JSON.stringify({ success: d.success, message: d.message, model: d.model, latencyMs: d.latencyMs }, null, 2)}
                  </pre>
                </div>
              </div>
            )
          })()}
        </div>
      )}
    </div>
  )
}

// ── Main Component ────────────────────────────────────────────────

export default function ProviderSetup({
  onComplete,
  onCodexAuth,
  onCancel,
}: {
  onComplete: () => void
  onCodexAuth: () => Promise<void>
  onCancel?: () => void
}) {
  const [mode, setMode] = useState<"choose" | "template-config" | "custom">(
    "choose",
  )
  const [codexLoading, setCodexLoading] = useState(false)
  const [codexError, setCodexError] = useState("")
  const { t } = useTranslation()

  // Template selection
  const [selectedTemplate, setSelectedTemplate] =
    useState<ProviderTemplate | null>(null)
  const [searchQuery, setSearchQuery] = useState("")

  // Config form (for both template & custom)
  const [customStep, setCustomStep] = useState(0) // 0=type, 1=connection, 2=models
  const [apiType, setApiType] = useState<ApiType>("openai-chat")
  const [providerName, setProviderName] = useState("")
  const [baseUrl, setBaseUrl] = useState("")
  const [apiKey, setApiKey] = useState("")
  const [models, setModels] = useState<ModelConfig[]>([])
  const [testResult, setTestResult] = useState<TestResult | null>(null)
  const [testLoading, setTestLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState("")
  const [modelsExpanded, setModelsExpanded] = useState(false)

  // ── Actions ─────────────────────────────────────────────────────

  function selectTemplate(template: ProviderTemplate) {
    setSelectedTemplate(template)
    setProviderName(template.name)
    setBaseUrl(template.baseUrl)
    setApiType(template.apiType)
    setModels([...template.models])
    setApiKey("")
    setTestResult(null)
    setError("")
    setModelsExpanded(false)
    setMode("template-config")
  }

  function startCustom() {
    setSelectedTemplate(null)
    setProviderName("")
    setBaseUrl("https://api.example.com")
    setApiType("openai-chat")
    setModels([])
    setApiKey("")
    setTestResult(null)
    setError("")
    setCustomStep(0)
    setMode("custom")
  }

  async function handleTest() {
    setTestLoading(true)
    setTestResult(null)
    try {
      const msg = await invoke<string>("test_provider", {
        config: {
          id: "",
          name: providerName,
          apiType,
          baseUrl,
          apiKey,
          userAgent: "claude-code/0.1.0",
          models: [],
          enabled: true,
        },
      })
      setTestResult(parseTestResult(msg, false))
    } catch (e) {
      setTestResult(parseTestResult(String(e), true))
    } finally {
      setTestLoading(false)
    }
  }

  async function handleSave() {
    if (models.length === 0) return
    setSaving(true)
    setError("")
    try {
      await invoke("add_provider", {
        config: {
          id: "",
          name: providerName,
          apiType,
          baseUrl,
          apiKey: apiKey || "ollama",
          userAgent: "claude-code/0.1.0",
          models,
          enabled: true,
        },
      })
      // Set the first model as active
      const providers = await invoke<ProviderConfig[]>("get_providers")
      const latest = providers[providers.length - 1]
      if (latest && latest.models.length > 0) {
        await invoke("set_active_model", {
          providerId: latest.id,
          modelId: latest.models[0].id,
        })
      }
      onComplete()
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  async function handleCodexAuth() {
    setCodexLoading(true)
    setCodexError("")
    try {
      await onCodexAuth()
    } catch (e) {
      setCodexError(String(e))
      setCodexLoading(false)
    }
  }

  // ── Filtered Templates ──────────────────────────────────────────

  const filteredTemplates = searchQuery.trim()
    ? PROVIDER_TEMPLATES.filter(
      (t) =>
        t.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        t.description.toLowerCase().includes(searchQuery.toLowerCase()),
    )
    : PROVIDER_TEMPLATES

  // ── Choose Mode Screen (Template Grid) ──────────────────────────

  if (mode === "choose") {
    return (
    <div className="flex flex-col h-full bg-background">
        {/* Header with optional back button */}
        {onCancel && (
          <div className="h-11 flex items-center px-4 border-b border-border shrink-0">
            <button
              onClick={onCancel}
              className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
            >
              <ArrowLeft className="h-4 w-4" />
              {t("common.back")}
            </button>
          </div>
        )}

        {/* Scrollable content area */}
        <div className="flex-1 overflow-y-auto">
        {/* Title */}
        <div className="text-center pt-10 pb-5 px-4">
          <h1 className="text-2xl font-semibold tracking-tight text-foreground">
            OpenComputer
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("provider.selectProvider")}
          </p>
        </div>

        {/* Codex Quick Auth */}
        <div className="px-6 pb-4 max-w-xl mx-auto w-full">
          <Button
            onClick={handleCodexAuth}
            disabled={codexLoading}
            className="w-full h-11 text-sm font-medium bg-primary hover:bg-primary/90"
          >
            {codexLoading ? (
              <span className="flex items-center gap-2">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("provider.waitingBrowserLogin")}
              </span>
            ) : (
              t("provider.codexSignIn")
            )}
          </Button>
          {codexError && (
            <p className="text-xs text-red-400 text-center mt-2">
              {codexError}
            </p>
          )}
          <div className="flex items-center gap-3 mt-4">
            <div className="flex-1 h-px bg-border" />
            <span className="text-xs text-muted-foreground">
              {t("provider.orSelectProvider")}
            </span>
            <div className="flex-1 h-px bg-border" />
          </div>
        </div>

        {/* Search */}
        <div className="px-6 pb-3 max-w-xl mx-auto w-full">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
            <Input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t("provider.searchProviders")}
              className="bg-card pl-9 h-9 text-xs"
            />
          </div>
        </div>

        {/* Template Grid */}
        <div className="px-6 pb-6 max-w-xl mx-auto w-full">
          <div className="grid grid-cols-2 gap-2">
            {filteredTemplates.map((template) => (
              <button
                key={template.key}
                onClick={() => selectTemplate(template)}
                className="flex items-center gap-2.5 p-3 rounded-xl border border-border bg-card hover:border-primary/40 hover:bg-secondary/50 text-left transition-all duration-200"
              >
                <ProviderIcon providerKey={template.key} size={24} className="shrink-0" color />
                <div className="min-w-0">
                  <div className="text-xs font-medium text-foreground truncate">
                    {template.name}
                  </div>
                  <div className="text-[10px] text-muted-foreground truncate">
                    {template.description}
                  </div>
                </div>
              </button>
            ))}

            {/* Custom Provider */}
            <button
              onClick={startCustom}
              className="flex items-center gap-2.5 p-3 rounded-xl border border-dashed border-border bg-card/50 hover:border-primary/40 hover:bg-secondary/50 text-left transition-all duration-200"
            >
              <div className="w-7 h-7 rounded-lg flex items-center justify-center bg-secondary text-muted-foreground shrink-0">
                <Settings2 className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <div className="text-xs font-medium text-foreground">
                  {t("provider.custom")}
                </div>
                <div className="text-[10px] text-muted-foreground">
                  {t("provider.customDescription")}
                </div>
              </div>
            </button>
          </div>
        </div>
        </div>
      </div>
    )
  }

  // ── Template Config (simple: just API key + optional tweaks) ─────

  if (mode === "template-config" && selectedTemplate) {
    const canSave =
      (!selectedTemplate.requiresApiKey || apiKey.trim()) &&
      models.length > 0 &&
      models.every((m) => m.id.trim() && m.name.trim())

    return (
    <div className="flex flex-col h-full bg-background">
        {/* Header */}
        <div className="h-11 flex items-center px-4 border-b border-border shrink-0">
          <button
            onClick={() => setMode("choose")}
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <ArrowLeft className="h-4 w-4" />
            {t("common.back")}
          </button>
          <span className="text-sm font-semibold text-foreground mx-auto flex items-center gap-1.5">
            <ProviderIcon providerKey={selectedTemplate.key} size={18} color />
            {selectedTemplate.name}
          </span>
          <div className="w-12" /> {/* spacer */}
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-6 max-w-lg mx-auto w-full space-y-4">
          {/* Provider info */}
          <div className="bg-card border border-border rounded-xl p-4 space-y-3">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground">
                {t("provider.name")}
              </label>
              <Input
                value={providerName}
                onChange={(e) => setProviderName(e.target.value)}
                className="bg-background"
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground">
                {t("provider.apiType")}
              </label>
              <div className="relative">
                <select
                  value={apiType}
                  onChange={(e) => setApiType(e.target.value as ApiType)}
                  className="w-full appearance-none bg-background text-foreground text-xs font-medium px-3 py-2 rounded-md border border-border cursor-pointer hover:bg-secondary/50 transition-colors focus:outline-none focus:ring-1 focus:ring-ring"
                >
                  <option value="openai-chat">OpenAI Chat Completions</option>
                  <option value="openai-responses">OpenAI Responses API</option>
                  <option value="anthropic">Anthropic Messages API</option>
                </select>
                <ChevronDown className="absolute right-2.5 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground pointer-events-none" />
              </div>
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                <Key className="h-3 w-3" />
                API Key
                {!selectedTemplate.requiresApiKey && (
                  <span className="text-[10px] text-muted-foreground/60 font-normal">({t("provider.optional")})</span>
                )}
              </label>
              <Input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={selectedTemplate.requiresApiKey ? selectedTemplate.apiKeyPlaceholder : t("provider.leaveEmptyNoAuth")}
                className="bg-background font-mono text-xs"
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                <Globe className="h-3 w-3" />
                Base URL
              </label>
              <Input
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                className="bg-background font-mono text-xs"
              />
            </div>

            {/* Test Connection */}
            <Button
              variant="secondary"
              size="sm"
              onClick={handleTest}
              disabled={
                testLoading ||
                (selectedTemplate.requiresApiKey && !apiKey.trim()) ||
                !baseUrl.trim()
              }
              className="w-full"
            >
              {testLoading ? (
                <span className="flex items-center gap-2">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("common.testing")}
                </span>
              ) : (
                t("provider.testConnection")
              )}
            </Button>

            {testResult && (
              <TestResultDisplay result={testResult} />
            )}
          </div>

          {/* Models (collapsed by default for templates, shows summary) */}
          <div className="bg-card border border-border rounded-xl overflow-hidden">
            <button
              onClick={() => setModelsExpanded(!modelsExpanded)}
              className="w-full flex items-center justify-between px-4 py-3 text-left hover:bg-secondary/30 transition-colors"
            >
              <div className="flex items-center gap-1.5">
                <span className="text-sm font-semibold text-foreground">
                  {t("model.modelList")}
                </span>
                <span className="text-[10px] text-muted-foreground/60 bg-secondary/80 px-1.5 py-0.5 rounded-md">
                  {models.length}
                </span>
              </div>
              <ArrowRight
                className={`h-3.5 w-3.5 text-muted-foreground transition-transform ${modelsExpanded ? "rotate-90" : ""
                  }`}
              />
            </button>

            {!modelsExpanded && (
              <div className="px-4 pb-3 flex flex-wrap gap-1.5">
                {models.map((m) => (
                  <span
                    key={m.id}
                    className="px-2 py-0.5 text-[10px] rounded-md bg-secondary text-muted-foreground"
                  >
                    {m.name}
                  </span>
                ))}
              </div>
            )}

            {modelsExpanded && (
              <div className="px-4 pb-4 space-y-2.5">
                {models.map((model, i) => (
                  <ModelEditor
                    key={i}
                    model={model}
                    onChange={(m) => {
                      const updated = [...models]
                      updated[i] = m
                      setModels(updated)
                    }}
                    onRemove={() =>
                      setModels(models.filter((_, j) => j !== i))
                    }
                    onTest={baseUrl.trim() ? (modelId) => invoke<string>("test_model", {
                      config: { id: "", name: providerName, apiType, baseUrl, apiKey: apiKey || "ollama", userAgent: "claude-code/0.1.0", models: [], enabled: true },
                      modelId,
                    }) : undefined}
                  />
                ))}
                <Button
                  variant="secondary"
                  size="sm"
                  className="w-full"
                  onClick={() =>
                    setModels([
                      ...models,
                      {
                        id: "",
                        name: "",
                        inputTypes: ["text"],
                        contextWindow: 128000,
                        maxTokens: 8192,
                        reasoning: false,
                        costInput: 0,
                        costOutput: 0,
                      },
                    ])
                  }
                >
                  <Plus className="h-3.5 w-3.5 mr-1" />
                  {t("model.addModel")}
                </Button>
              </div>
            )}
          </div>

          {error && <p className="text-xs text-red-400">{error}</p>}
        </div>

        {/* Footer */}
        <div className="border-t border-border px-6 py-3 flex justify-end shrink-0">
          <Button onClick={handleSave} disabled={!canSave || saving}>
            {saving ? (
              <span className="flex items-center gap-2">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.saving")}
              </span>
            ) : (
              <>
                <Check className="h-4 w-4 mr-1" />
                {t("common.done")}
              </>
            )}
          </Button>
        </div>
      </div>
    )
  }

  // ── Custom Provider (full 3-step wizard) ─────────────────────────

  const API_TYPE_OPTIONS: {
    value: ApiType
    label: string
    description: string
  }[] = [
      {
        value: "anthropic",
        label: "Anthropic Messages API",
        description: t("wizard.anthropicDesc"),
      },
      {
        value: "openai-chat",
        label: "OpenAI Chat Completions",
        description: t("wizard.openaiChatDesc"),
      },
      {
        value: "openai-responses",
        label: "OpenAI Responses API",
        description: t("wizard.openaiResponsesDesc"),
      },
    ]

  const canNext =
    customStep === 0
      ? true
      : customStep === 1
        ? baseUrl.trim() && providerName.trim()
        : models.length > 0 &&
        models.every((m) => m.id.trim() && m.name.trim())

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="h-11 flex items-center px-4 border-b border-border shrink-0">
        <button
          onClick={() => {
            if (customStep > 0) setCustomStep(customStep - 1)
            else setMode("choose")
          }}
          className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
        >
          <ArrowLeft className="h-4 w-4" />
          {t("common.back")}
        </button>

        <div className="flex items-center gap-2 mx-auto">
          {[t("wizard.apiType"), t("wizard.connectionConfig"), t("wizard.models")].map((label, i) => (
            <div key={i} className="flex items-center gap-2">
              <div
                className={`w-6 h-6 rounded-full flex items-center justify-center text-[10px] font-medium transition-colors ${i === customStep
                    ? "bg-primary text-primary-foreground"
                    : i < customStep
                      ? "bg-primary/20 text-primary"
                      : "bg-secondary text-muted-foreground"
                  }`}
              >
                {i < customStep ? <Check className="h-3 w-3" /> : i + 1}
              </div>
              <span
                className={`text-xs hidden sm:inline ${i === customStep ? "text-foreground font-medium" : "text-muted-foreground"}`}
              >
                {label}
              </span>
              {i < 2 && (
                <div className="w-6 h-px bg-border hidden sm:block" />
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-6 max-w-lg mx-auto w-full">
        {customStep === 0 && (
          <div className="space-y-3">
            <h3 className="text-base font-semibold text-foreground">
              {t("wizard.selectApiType")}
            </h3>
            <div className="grid gap-2.5">
              {API_TYPE_OPTIONS.map((opt) => (
                <button
                  key={opt.value}
                  onClick={() => setApiType(opt.value)}
                  className={`flex items-center gap-3 p-3.5 rounded-xl border text-left transition-all duration-200 ${apiType === opt.value
                      ? "border-primary bg-primary/5 ring-1 ring-primary/30"
                      : "border-border bg-card hover:border-primary/40 hover:bg-secondary/50"
                    }`}
                >
                  <div className="min-w-0">
                    <div className="text-sm font-medium text-foreground">
                      {opt.label}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {opt.description}
                    </div>
                  </div>
                  {apiType === opt.value && (
                    <Check className="h-4 w-4 text-primary ml-auto shrink-0" />
                  )}
                </button>
              ))}
            </div>
          </div>
        )}

        {customStep === 1 && (
          <div className="space-y-4">
            <h3 className="text-base font-semibold text-foreground">
              {t("wizard.connectionConfig")}
            </h3>
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground">
                  {t("provider.name")}
                </label>
                <Input
                  value={providerName}
                  onChange={(e) => setProviderName(e.target.value)}
                  placeholder={t("provider.myCustomProvider")}
                  className="bg-card"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Globe className="h-3 w-3" />
                  Base URL
                </label>
                <Input
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="https://api.example.com"
                  className="bg-card font-mono text-xs"
                />
              </div>
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-muted-foreground flex items-center gap-1.5">
                  <Key className="h-3 w-3" />
                  API Key
                  <span className="text-[10px] text-muted-foreground/60 font-normal">({t("provider.optional")})</span>
                </label>
                <Input
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder={t("provider.authRequired")}
                  className="bg-card font-mono text-xs"
                />
              </div>
              <Button
                variant="secondary"
                size="sm"
                onClick={handleTest}
                disabled={testLoading || !apiKey.trim() || !baseUrl.trim()}
                className="w-full"
              >
                {testLoading ? (
                  <span className="flex items-center gap-2">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("common.testing")}
                  </span>
                ) : (
                  t("provider.testConnection")
                )}
              </Button>
              {testResult && (
                <TestResultDisplay result={testResult} />
              )}
            </div>
          </div>
        )}

        {customStep === 2 && (
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-base font-semibold text-foreground">
                  {t("model.addModels")}
                </h3>
                <p className="text-xs text-muted-foreground mt-0.5">
                  {t("model.configModels")}
                </p>
              </div>
              <Button
                variant="secondary"
                size="sm"
                onClick={() =>
                  setModels([
                    ...models,
                    {
                      id: "",
                      name: "",
                      inputTypes: ["text"],
                      contextWindow: 128000,
                      maxTokens: 8192,
                      reasoning: false,
                      costInput: 0,
                      costOutput: 0,
                    },
                  ])
                }
              >
                <Plus className="h-3.5 w-3.5 mr-1" />
                {t("common.add")}
              </Button>
            </div>
            <div className="space-y-2.5 max-h-[400px] overflow-y-auto pr-1">
              {models.map((model, i) => (
                <ModelEditor
                  key={i}
                  model={model}
                  onChange={(m) => {
                    const updated = [...models]
                    updated[i] = m
                    setModels(updated)
                  }}
                  onRemove={() =>
                    setModels(models.filter((_, j) => j !== i))
                  }
                  onTest={baseUrl.trim() ? (modelId) => invoke<string>("test_model", {
                    config: { id: "", name: providerName, apiType, baseUrl, apiKey: apiKey || "ollama", userAgent: "claude-code/0.1.0", models: [], enabled: true },
                    modelId,
                  }) : undefined}
                />
              ))}
              {models.length === 0 && (
                <div className="text-center py-8 text-muted-foreground text-xs">
                  {t("model.atLeastOneModel")}
                </div>
              )}
            </div>
          </div>
        )}

        {error && <p className="text-xs text-red-400 mt-3">{error}</p>}
      </div>

      {/* Footer */}
      <div className="border-t border-border px-6 py-3 flex justify-end gap-2 shrink-0">
        {customStep < 2 ? (
          <Button onClick={() => setCustomStep(customStep + 1)} disabled={!canNext}>
            {t("common.nextStep")}
            <ArrowRight className="h-4 w-4 ml-1" />
          </Button>
        ) : (
          <Button onClick={handleSave} disabled={!canNext || saving}>
            {saving ? (
              <span className="flex items-center gap-2">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("common.saving")}
              </span>
            ) : (
              <>
                <Check className="h-4 w-4 mr-1" />
                {t("common.done")}
              </>
            )}
          </Button>
        )}
      </div>
    </div>
  )
}
