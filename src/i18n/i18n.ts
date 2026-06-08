import i18n from "i18next"
import { initReactI18next } from "react-i18next"
import { getTransport } from "@/lib/transport-provider"
import { parsePayload } from "@/lib/transport"

// 仅 en 同步内联作为首屏兜底（fallbackLng）；其余 11 种语言按需懒加载，避免把
// 12 份翻译（~2.8MB）全量打进主 bundle。其余语言见下方 localeLoaders。
import en from "./locales/en.json"

export const SUPPORTED_LANGUAGES = [
  { code: "zh", label: "简体中文", shortLabel: "ZH" },
  { code: "zh-TW", label: "繁體中文", shortLabel: "TW" },
  { code: "en", label: "English", shortLabel: "EN" },
  { code: "ja", label: "日本語", shortLabel: "JA" },
  { code: "tr", label: "Türkçe", shortLabel: "TR" },
  { code: "vi", label: "Tiếng Việt", shortLabel: "VI" },
  { code: "pt", label: "Português", shortLabel: "PT" },
  { code: "ko", label: "한국어", shortLabel: "KO" },
  { code: "ru", label: "Русский", shortLabel: "RU" },
  { code: "ar", label: "العربية", shortLabel: "AR" },
  { code: "es", label: "Español", shortLabel: "ES" },
  { code: "ms", label: "Bahasa Melayu", shortLabel: "MY" },
] as const

const supportedCodes = SUPPORTED_LANGUAGES.map((l) => l.code)

// 各语言翻译正文的懒加载器。必须用显式映射（而非模板字符串拼接），Vite/Rolldown
// 才能把每种语言静态分析成独立 chunk，按当前语言 fetch。
const localeLoaders: Record<string, () => Promise<{ default: Record<string, unknown> }>> = {
  zh: () => import("./locales/zh.json"),
  "zh-TW": () => import("./locales/zh-TW.json"),
  ja: () => import("./locales/ja.json"),
  ko: () => import("./locales/ko.json"),
  tr: () => import("./locales/tr.json"),
  vi: () => import("./locales/vi.json"),
  pt: () => import("./locales/pt.json"),
  ru: () => import("./locales/ru.json"),
  ar: () => import("./locales/ar.json"),
  es: () => import("./locales/es.json"),
  ms: () => import("./locales/ms.json"),
}

// en 首屏已同步内联，标记为已加载。
const loadedLocales = new Set<string>(["en"])

/** 确保某语言翻译已挂载到 i18next（幂等）。未知 code 静默忽略，由 fallback 兜底。 */
async function ensureLocale(code: string): Promise<void> {
  if (loadedLocales.has(code)) return
  const loader = localeLoaders[code]
  if (!loader) return
  const mod = await loader()
  i18n.addResourceBundle(code, "translation", mod.default, true, true)
  loadedLocales.add(code)
}

/** 异步切语言：先确保 bundle 就位再 changeLanguage，避免闪 key。 */
async function loadAndSetLanguage(code: string): Promise<void> {
  await ensureLocale(code)
  await i18n.changeLanguage(code)
}

/** Resolve a raw locale string to one of our supported language codes. */
function resolveLanguage(raw: string): string {
  const exact = supportedCodes.find((c) => c === raw)
  if (exact) return exact
  const prefix = supportedCodes.find((c) => raw.startsWith(c + "-"))
  return prefix || "en"
}

/** Detect the system/browser language and resolve to a supported code. */
function detectSystemLanguage(): string {
  if (typeof navigator === "undefined") {
    return "en"
  }

  const detected = navigator.language || navigator.languages?.[0] || "en"
  return resolveLanguage(detected)
}

// 首屏只内联 en（fallbackLng），以 en 起步保证第一帧不空白；随后异步拉取系统语言
// 的翻译 bundle，到位后 changeLanguage 触发 react-i18next 重渲染。
const _initialLang = detectSystemLanguage()
i18n
  .use(initReactI18next)
  .init({
    resources: { en: { translation: en } },
    lng: "en",
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
    },
  })

if (_initialLang !== "en") {
  void loadAndSetLanguage(_initialLang)
}

// Internal state: tracks whether user selected "auto" (follow system)
let _followingSystem = true

/**
 * Load saved language preference from backend config.json and apply it.
 * Should be called once at app startup.
 */
export async function initLanguageFromConfig() {
  try {
    const saved = await getTransport().call<string>("get_language")
    if (saved && saved !== "auto") {
      _followingSystem = false
      await loadAndSetLanguage(resolveLanguage(saved))
    } else {
      _followingSystem = true
      await loadAndSetLanguage(detectSystemLanguage())
    }
  } catch {
    // Backend not ready yet, keep system default
    _followingSystem = true
  }
}

/**
 * Check whether the app is currently in "follow system" mode.
 */
export function isFollowingSystem(): boolean {
  return _followingSystem
}

/**
 * Switch to "follow system" language mode.
 * Persists "auto" to backend config.json.
 */
export function setFollowSystemLanguage() {
  _followingSystem = true
  void loadAndSetLanguage(detectSystemLanguage())
  getTransport().call("set_language", { language: "auto" }).catch(() => {})
}

/**
 * Set a specific language.
 * Persists the language code to backend config.json.
 */
export function setLanguage(code: string) {
  _followingSystem = false
  void loadAndSetLanguage(code)
  getTransport().call("set_language", { language: code }).catch(() => {})
}

/**
 * Listen for backend config:changed events and hot-reload language.
 * Returns an unlisten function. Should be called once in App.tsx useEffect.
 */
export function listenLanguageConfigChange(): () => void {
  return getTransport().listen("config:changed", (raw) => {
    try {
      const payload = parsePayload<{ category?: string }>(raw)
      if (payload?.category === "language") {
        initLanguageFromConfig()
      }
    } catch { /* ignore */ }
  })
}

export default i18n
