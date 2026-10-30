import { fileURLToPath } from "node:url";
import sitemap from "@astrojs/sitemap";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://hatoba.fog.moe",
  trailingSlash: "always",
  devToolbar: { enabled: false },
  markdown: {
    // Code takes its colours from the tokens (--astro-code-*), in both schemes.
    shikiConfig: { theme: "css-variables" },
  },
  i18n: {
    locales: ["en", "zh-cn", "zh-tw", "ja"],
    defaultLocale: "en",
    routing: { prefixDefaultLocale: false },
  },
  // Simplified Chinese lived at /zh/ before Traditional Chinese came at /zh-tw/.
  redirects: {
    "/zh/": "/zh-cn/",
    "/zh/download/": "/zh-cn/download/",
  },
  integrations: [
    sitemap({
      i18n: { defaultLocale: "en", locales: { en: "en", "zh-cn": "zh-CN", "zh-tw": "zh-TW", ja: "ja" } },
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
            if (request.url && /^\/demo\/(\?|$)/.test(request.url))
              request.url = request.url.replace("/demo/", "/demo/index.html");
            next();
          });
        },
      },
    ],
  },
});
