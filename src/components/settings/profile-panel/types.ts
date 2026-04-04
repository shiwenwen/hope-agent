export interface UserConfig {
  name?: string | null
  avatar?: string | null
  gender?: string | null
  birthday?: string | null
  role?: string | null
  timezone?: string | null
  language?: string | null
  aiExperience?: string | null
  responseStyle?: string | null
  customInfo?: string | null
  weatherEnabled?: boolean
  weatherCity?: string | null
  weatherLatitude?: number | null
  weatherLongitude?: number | null
}

export const GENDER_PRESETS = ["male", "female"]

export const TIMEZONE_OPTIONS: { groupKey: string; zones: { value: string; labelKey: string }[] }[] = [
  {
    groupKey: "Asia",
    zones: [
      { value: "Asia/Shanghai", labelKey: "tz.shanghai" },
      { value: "Asia/Tokyo", labelKey: "tz.tokyo" },
      { value: "Asia/Seoul", labelKey: "tz.seoul" },
      { value: "Asia/Singapore", labelKey: "tz.singapore" },
      { value: "Asia/Hong_Kong", labelKey: "tz.hongkong" },
      { value: "Asia/Taipei", labelKey: "tz.taipei" },
      { value: "Asia/Kolkata", labelKey: "tz.kolkata" },
      { value: "Asia/Dubai", labelKey: "tz.dubai" },
      { value: "Asia/Bangkok", labelKey: "tz.bangkok" },
    ],
  },
  {
    groupKey: "Americas",
    zones: [
      { value: "America/New_York", labelKey: "tz.newyork" },
      { value: "America/Chicago", labelKey: "tz.chicago" },
      { value: "America/Denver", labelKey: "tz.denver" },
      { value: "America/Los_Angeles", labelKey: "tz.losangeles" },
      { value: "America/Toronto", labelKey: "tz.toronto" },
      { value: "America/Sao_Paulo", labelKey: "tz.saopaulo" },
      { value: "America/Mexico_City", labelKey: "tz.mexicocity" },
    ],
  },
  {
    groupKey: "Europe",
    zones: [
      { value: "Europe/London", labelKey: "tz.london" },
      { value: "Europe/Paris", labelKey: "tz.paris" },
      { value: "Europe/Berlin", labelKey: "tz.berlin" },
      { value: "Europe/Moscow", labelKey: "tz.moscow" },
      { value: "Europe/Istanbul", labelKey: "tz.istanbul" },
      { value: "Europe/Amsterdam", labelKey: "tz.amsterdam" },
      { value: "Europe/Madrid", labelKey: "tz.madrid" },
    ],
  },
  {
    groupKey: "Pacific",
    zones: [
      { value: "Pacific/Auckland", labelKey: "tz.auckland" },
      { value: "Australia/Sydney", labelKey: "tz.sydney" },
      { value: "Australia/Melbourne", labelKey: "tz.melbourne" },
      { value: "Pacific/Honolulu", labelKey: "tz.honolulu" },
    ],
  },
  { groupKey: "Other", zones: [{ value: "UTC", labelKey: "tz.utc" }] },
]

export const LANGUAGE_OPTIONS = [
  { code: "zh-CN", label: "\u7B80\u4F53\u4E2D\u6587" },
  { code: "zh-TW", label: "\u7E41\u9AD4\u4E2D\u6587" },
  { code: "en", label: "English" },
  { code: "ja", label: "\u65E5\u672C\u8A9E" },
  { code: "ko", label: "\uD55C\uAD6D\uC5B4" },
  { code: "es", label: "Espa\u00F1ol" },
  { code: "pt", label: "Portugu\u00EAs" },
  { code: "ru", label: "\u0420\u0443\u0441\u0441\u043A\u0438\u0439" },
  { code: "ar", label: "\u0627\u0644\u0639\u0631\u0628\u064A\u0629" },
  { code: "tr", label: "T\u00FCrk\u00E7e" },
  { code: "vi", label: "Ti\u1EBFng Vi\u1EC7t" },
  { code: "ms", label: "Bahasa Melayu" },
]

export const PRESET_STYLES = ["concise", "detailed"]

/** Props for text inputs that handle IME composition correctly */
export type TextInputProps = {
  value: string
  onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  onCompositionStart: () => void
  onCompositionEnd: (e: React.CompositionEvent<HTMLInputElement | HTMLTextAreaElement>) => void
  onBlur: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => void
}
