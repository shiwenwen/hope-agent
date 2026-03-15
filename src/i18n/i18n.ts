import i18n from "i18next"
import { initReactI18next } from "react-i18next"
import LanguageDetector from "i18next-browser-languagedetector"

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

i18n
  .use(LanguageDetector)
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
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
      lookupLocalStorage: "i18nextLng",
    },
  })

const STORAGE_KEY = "i18nextLng"

/**
 * Check whether the app is currently in "follow system" mode.
 * When the user explicitly picks a language, it's stored in localStorage.
 * If there's no stored preference, we're following the system.
 */
export function isFollowingSystem(): boolean {
  return !localStorage.getItem(STORAGE_KEY)
}

/**
 * Switch to "follow system" language mode.
 * Removes the stored preference and re-detects the browser/system language.
 */
export function setFollowSystemLanguage() {
  localStorage.removeItem(STORAGE_KEY)
  // Re-detect language from navigator
  const detected =
    navigator.language ||
    (navigator.languages && navigator.languages[0]) ||
    "en"
  // Resolve to a supported language code
  const supported = SUPPORTED_LANGUAGES.map((l) => l.code)
  const exact = supported.find((c) => c === detected)
  const prefix = supported.find((c) => detected.startsWith(c + "-"))
  const lang = exact || prefix || "en"
  i18n.changeLanguage(lang)
  // Remove again because changeLanguage will re-set it
  localStorage.removeItem(STORAGE_KEY)
}

export default i18n
