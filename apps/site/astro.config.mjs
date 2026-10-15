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
    plugins: [
      {
        // The dev server serves public/demo (scripts/demo.mjs) at /demo/ as GitHub Pages does,
        // which it would otherwise answer with the 404 page.
        name: "hatoba-demo-index",
        configureServer(server) {
          server.middlewares.use((request, _response, next) => {
            if (request.url && /^\/demo\/(\?|$)/.test(request.url)) request.url = request.url.replace("/demo/", "/demo/index.html");
            next();
          });
        },
      },
    ],
  },
});
