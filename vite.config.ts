import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and no screen clearing, so its own logs stay visible.
//
// `--mode demo` (npm run build:demo) builds the UI with the simulated backend into dist-demo/,
// with relative asset paths so the result can be served from any sub-path or inside an iframe.
// The app itself is always built in the default mode, into dist/, without the simulated backend.
export default defineConfig(({ mode }) => {
  const demo = mode === "demo";
  return {
    plugins: [react()],
    clearScreen: false,
    base: demo ? "./" : "/",
    build: { outDir: demo ? "dist-demo" : "dist" },
    server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**", "**/engine/**"] } },
  };
});
