import { resolve } from "node:path";
import { defineConfig } from "vite";

// 多页应用：4 个窗口各一个 HTML 入口（main/popup/snip/pin）
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2021",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        popup: resolve(__dirname, "popup.html"),
        snip: resolve(__dirname, "snip.html"),
        pin: resolve(__dirname, "pin.html"),
      },
    },
  },
});