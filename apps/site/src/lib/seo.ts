import { REPO_URL, SITE_URL } from "../consts";
import { LANG, localePath, messages, type Locale } from "../i18n";
import type { SiteRelease } from "./release";

/** The <title> of a page: its own title, then Hatoba and its tagline on the home page. */
export function pageTitle(locale: Locale, title?: string): string {
  return title ? `${title} – Hatoba` : `Hatoba – ${messages(locale).meta.tagline}`;
}

/** schema.org data for search engines, describing Hatoba as the app people download. */
export function softwareApplication(locale: Locale, release: SiteRelease): Record<string, unknown> {
  return {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Hatoba",
    description: messages(locale).meta.description,
    url: new URL(localePath(locale), SITE_URL).href,
    image: new URL(localePath(locale, "/og.png"), SITE_URL).href,
    inLanguage: LANG[locale],
    applicationCategory: "DeveloperApplication",
    operatingSystem: "Windows 10, Windows 11, Linux, macOS 13",
    softwareVersion: release.version,
    datePublished: release.publishedAt.slice(0, 10),
    downloadUrl: new URL(localePath(locale, "/download/"), SITE_URL).href,
    license: "https://opensource.org/licenses/MIT",
    isAccessibleForFree: true,
    offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
    sameAs: [REPO_URL],
  };
}
