import i18next from "i18next"
import { initReactI18next } from "react-i18next"

import { en } from "@/i18n/en"
import { zhCN } from "@/i18n/zh-CN"

export type StudioLocale = "en" | "zh-CN"

export const STUDIO_LOCALE_STORAGE_KEY = "graphloom.studio.locale"

export function normalizeStudioLocale(locale: string | null | undefined): StudioLocale | null {
  if (locale === null || locale === undefined) return null
  const normalized = locale.trim().replaceAll("_", "-").toLowerCase()
  if (normalized === "en" || normalized.startsWith("en-")) return "en"
  if (normalized === "zh" || normalized === "zh-cn" || normalized === "zh-sg" || normalized === "zh-hans" || normalized.startsWith("zh-hans-")) return "zh-CN"
  return null
}

export function resolveStudioLocale(storedLocale: string | null, browserLanguages: readonly string[]): StudioLocale {
  const stored = normalizeStudioLocale(storedLocale)
  if (stored !== null) return stored
  for (const language of browserLanguages) {
    const locale = normalizeStudioLocale(language)
    if (locale !== null) return locale
  }
  return "en"
}

function initialLocale(): StudioLocale {
  if (typeof window === "undefined") return "en"
  const stored = storedLocale()
  const languages = navigator.languages.length > 0 ? navigator.languages : [navigator.language]
  return resolveStudioLocale(stored, languages)
}

function storedLocale(): string | null {
  try {
    return window.localStorage.getItem(STUDIO_LOCALE_STORAGE_KEY)
  } catch {
    return null
  }
}

function syncDocumentLanguage(locale: StudioLocale): void {
  if (typeof document !== "undefined") document.documentElement.lang = locale
}

export const i18n = i18next.createInstance()

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    "zh-CN": { translation: zhCN },
  },
  lng: initialLocale(),
  fallbackLng: "en",
  supportedLngs: ["en", "zh-CN"],
  interpolation: { escapeValue: false },
  keySeparator: false,
  returnNull: false,
})

syncDocumentLanguage(normalizeStudioLocale(i18n.language) ?? "en")
i18n.on("languageChanged", (language) => syncDocumentLanguage(normalizeStudioLocale(language) ?? "en"))

export async function setStudioLocale(locale: StudioLocale, persist = true): Promise<void> {
  if (persist && typeof window !== "undefined") {
    try {
      window.localStorage.setItem(STUDIO_LOCALE_STORAGE_KEY, locale)
    } catch {
      // A blocked storage API must not prevent an in-memory language change.
    }
  }
  await i18n.changeLanguage(locale)
}

export function activeStudioLocale(): StudioLocale {
  return normalizeStudioLocale(i18n.resolvedLanguage ?? i18n.language) ?? "en"
}
