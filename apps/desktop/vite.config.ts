import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

const host = process.env.TAURI_DEV_HOST;

/** The website's demo (`--mode demo`, src/ipc/mock/visitor.ts) stays out of search results, which show the site. */
const demoNoindex: Plugin = {
  name: "hatoba-demo-noindex",
  apply: (_, env) => env.mode === "demo",
  transformIndexHtml: () => [{ tag: "meta", attrs: { name: "robots", content: "noindex" }, injectTo: "head" }],
};

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react(), demoNoindex],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2022",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    environment: "jsdom",
  },
});
