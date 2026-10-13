import { en, type Messages } from "./en";
import { ja } from "./ja";
import { zh } from "./zh";

export const LOCALES = ["en", "zh", "ja"] as const;
export type Locale = (typeof LOCALES)[number];

const MESSAGES: Record<Locale, Messages> = { en, zh, ja };

/** The language tag of each locale, for `lang`, hreflang, and the sitemap. */
export const LANG: Record<Locale, string> = { en: "en", zh: "zh-CN", ja: "ja" };
export const OG_LOCALE: Record<Locale, string> = { en: "en_US", zh: "zh_CN", ja: "ja_JP" };
export const LANGUAGE_NAME: Record<Locale, string> = { en: "English", zh: "简体中文", ja: "日本語" };

export function messages(locale: Locale): Messages {
  return MESSAGES[locale];
}

/** The path of `path` (which starts and ends with a slash) in `locale`: English has no prefix. */
export function localePath(locale: Locale, path = "/"): string {
  return locale === "en" ? path : `/${locale}${path}`;
}

/** The static paths of a page under `[...lang]`, one per locale. */
export function localeStaticPaths() {
  return LOCALES.map((locale) => ({ params: { lang: locale === "en" ? undefined : locale }, props: { locale } }));
}

export function formatDate(locale: Locale, iso: string): string {
  return new Intl.DateTimeFormat(LANG[locale], { dateStyle: "long", timeZone: "UTC" }).format(new Date(iso));
}

export function formatMegabytes(locale: Locale, bytes: number): string {
  return new Intl.NumberFormat(LANG[locale], { maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(bytes / 1024 / 1024);
}
