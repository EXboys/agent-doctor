import { defineConfig } from "vite";
import { resolve } from "node:path";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Lock UI to the same edition as the Rust binary (`AGENT_DOCTOR_EDITION`).
// @ts-expect-error process is a nodejs global
const editionRaw = (
  process.env.AGENT_DOCTOR_EDITION ||
  process.env.VITE_AGENT_DOCTOR_EDITION ||
  "personal"
)
  .trim()
  .toLowerCase();
const edition =
  editionRaw === "team" || editionRaw === "enterprise" || editionRaw === "evotown"
    ? "team"
    : "personal";
// @ts-expect-error process is a nodejs global
process.env.VITE_AGENT_DOCTOR_EDITION = edition;

// https://vite.dev/config/
export default defineConfig(async () => ({
  // Relative asset URLs are required for Tauri's custom protocol (absolute
  // `/assets/...` paths white-screen the webview when loading frontendDist).
  base: "./",

  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        chat: resolve(__dirname, "chat.html"),
        resources: resolve(__dirname, "resources.html"),
        diagnose: resolve(__dirname, "diagnose.html"),
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    // Bind IPv4 explicitly — macOS `localhost` often prefers ::1, which leaves
    // the Tauri webview white when Vite only listens on 127.0.0.1.
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
