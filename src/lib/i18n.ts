// The pages speak the language the app does: the app decides it (the
// settings, or the system's preferred languages; src-tauri/src/i18n.rs),
// tells a page as it loads and announces changes.
// Text lives in src/locales/<locale>.json; `common` is the vocabulary the
// pages share with the bar and menus, from the app's own catalogs. Both use
// `%{name}` placeholders, and `_windows` for what differs on Windows.

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "@/locales/en.json";
import ja from "@/locales/ja.json";
import zhCN from "@/locales/zh-CN.json";
import { api } from "@/lib/api";
import { common as enCommon } from "../../src-tauri/locales/en.json";
import { common as jaCommon } from "../../src-tauri/locales/ja.json";
import { common as zhCNCommon } from "../../src-tauri/locales/zh-CN.json";

/** The languages the app has text in (`i18n::Locale`), each by its own name. */
export const LOCALES = {
  en: "English",
  "zh-CN": "简体中文",
  ja: "日本語",
} as const;

export type Locale = keyof typeof LOCALES;

/** What the settings may pick: a locale, or the system's (`i18n::Language`). */
export type Language = "system" | Locale;

export const resources = {
  en: { translation: { ...en, common: enCommon } },
  "zh-CN": { translation: { ...zhCN, common: zhCNCommon } },
  ja: { translation: { ...ja, common: jaCommon } },
} as const satisfies Record<Locale, unknown>;

/** Picks a message's Windows form where it has one (`showSymbol_windows`). */
export const PLATFORM = __WINDOWS__ ? ({ context: "windows" } as const) : {};

/** The document's `lang` follows, which picks the fonts for Chinese and Japanese. */
async function speak(locale: Locale): Promise<void> {
  document.documentElement.lang = locale;
  await i18n.changeLanguage(locale);
}

/**
 * Speaks the app's language before the first render, and whatever it switches
 * to later. The page asks rather than being handed it with the window, so one
 * that loads again speaks what the app speaks then. Every catalog is bundled:
 * a switch is instant.
 */
export async function setupI18n(): Promise<void> {
  void i18n.use(initReactI18next).init({
    resources,
    lng: "en",
    fallbackLng: "en",
    supportedLngs: Object.keys(LOCALES),
    initAsync: false,
    interpolation: { prefix: "%{", suffix: "}", escapeValue: false },
    react: { useSuspense: false },
  });
  // Listening first, for the page's lifetime: an announcement is always newer
  // than the answer below.
  let announced = false;
  void api.onLocale((locale) => {
    announced = true;
    void speak(locale);
  });
  const locale = await api.getLocale().catch(() => null);
  if (locale && !announced) await speak(locale);
}
