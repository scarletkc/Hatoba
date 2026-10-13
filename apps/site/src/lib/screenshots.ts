import type { ImageMetadata } from "astro";
import type { Locale } from "../i18n";

/**
 * The app's screenshots, which scripts/media/screenshots.py captures: the English ones in docs/media
 * (shared with the README), and the others in src/assets/screenshots/<app locale>.
 */
const english = import.meta.glob<{ default: ImageMetadata }>("../../../../docs/media/*.png", { eager: true });
const translated = import.meta.glob<{ default: ImageMetadata }>("../assets/screenshots/*/*.png", { eager: true });

/** The folder of each locale's screenshots, named after the app's locale. */
const FOLDER: Record<Exclude<Locale, "en">, string> = { zh: "zh-CN", ja: "ja" };

export type Screenshot = "ai-assistant" | "ai-approval" | "dashboard" | "sync" | "sync-deploy";

export function screenshot(locale: Locale, name: Screenshot): ImageMetadata {
  const image =
    locale === "en" ? english[`../../../../docs/media/${name}.png`] : translated[`../assets/screenshots/${FOLDER[locale]}/${name}.png`];
  if (!image) throw new Error(`There is no ${name}.png screenshot for ${locale}. Capture it with scripts/media/screenshots.py.`);
  return image.default;
}
