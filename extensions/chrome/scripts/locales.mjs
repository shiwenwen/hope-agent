// Single source of truth for the Chrome `_locales` the extension ships.
//
// `package-webstore.mjs` and `verify-webstore-package.mjs` both keep HARDCODED
// file allowlists (exact-set matching is the packaging safety net). Importing
// this list in both places means the locale set can never silently drift the
// way two hand-maintained arrays would.
//
// Folder names use Chrome's locale codes (underscores, BCP-47-ish). The desktop
// app ships `zh` (Simplified) and `zh-TW` (Traditional); Chrome has no bare
// `zh` folder, so Simplified maps to `zh_CN` and Traditional to `zh_TW`.
export const DEFAULT_LOCALE = "en"

export const LOCALES = [
  "ar",
  "en",
  "es",
  "ja",
  "ko",
  "ms",
  "pt",
  "ru",
  "tr",
  "vi",
  "zh_CN",
  "zh_TW",
]

export const LOCALE_MESSAGE_FILES = LOCALES.map((code) => `_locales/${code}/messages.json`)
