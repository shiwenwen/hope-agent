import i18n from "i18next"
import { initReactI18next } from "react-i18next"
import { invoke } from "@tauri-apps/api/core"

import zh from "./locales/zh.json"
import zhTW from "./locales/zh-TW.json"
import en from "./locales/en.json"
import ja from "./locales/ja.json"
import ko from "./locales/ko.json"
import tr from "./locales/tr.json"
import vi from "./locales/vi.json"
import pt from "./locales/pt.json"
import ru from "./locales/ru.json"
import ar from "./locales/ar.json"
import es from "./locales/es.json"
import ms from "./locales/ms.json"

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

/** Resolve a raw locale string to one of our supported language codes. */
function resolveLanguage(raw: string): string {
  const exact = supportedCodes.find((c) => c === raw)
  if (exact) return exact
  const prefix = supportedCodes.find((c) => raw.startsWith(c + "-"))
  return prefix || "en"
}

/** Detect the system/browser language and resolve to a supported code. */
function detectSystemLanguage(): string {
  const detected = navigator.language || (navigator.languages && navigator.languages[0]) || "en"
  return resolveLanguage(detected)
}

// Initialize i18next with system language as default (will be overridden by backend config)
i18n
  .use(initReactI18next)
  .init({
    resources: {
      zh: { translation: zh },
      "zh-TW": { translation: zhTW },
      en: { translation: en },
      ja: { translation: ja },
      ko: { translation: ko },
      tr: { translation: tr },
      vi: { translation: vi },
      pt: { translation: pt },
      ru: { translation: ru },
      ar: { translation: ar },
      es: { translation: es },
      ms: { translation: ms },
    },
    lng: detectSystemLanguage(),
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
    },
  })

// Internal state: tracks whether user selected "auto" (follow system)
let _followingSystem = true

/**
 * Load saved language preference from backend config.json and apply it.
 * Should be called once at app startup.
 */
export async function initLanguageFromConfig() {
  try {
    const saved = await invoke<string>("get_language")
    if (saved && saved !== "auto") {
      _followingSystem = false
      const lang = resolveLanguage(saved)
      i18n.changeLanguage(lang)
    } else {
      _followingSystem = true
      i18n.changeLanguage(detectSystemLanguage())
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
  const lang = detectSystemLanguage()
  i18n.changeLanguage(lang)
  invoke("set_language", { language: "auto" }).catch(() => {})
}

/**
 * Set a specific language.
 * Persists the language code to backend config.json.
 */
export function setLanguage(code: string) {
  _followingSystem = false
  i18n.changeLanguage(code)
  invoke("set_language", { language: code }).catch(() => {})
}

export default i18n
