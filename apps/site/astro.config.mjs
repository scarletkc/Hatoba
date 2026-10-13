import { fileURLToPath } from "node:url";
import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://hatoba.fog.moe",
  trailingSlash: "always",
  devToolbar: { enabled: false },
  i18n: {
    locales: ["en", "zh", "ja"],
    defaultLocale: "en",
    routing: { prefixDefaultLocale: false },
  },
  integrations: [
    sitemap({
      i18n: { defaultLocale: "en", locales: { en: "en", zh: "zh-CN", ja: "ja" } },
    }),
  ],
  vite: {
    // The site uses the screenshots in docs/media, the app icon, and scripts/release from the
    // rest of the repository.
    server: { fs: { allow: [fileURLToPath(new URL("../..", import.meta.url))] } },
  },
});
