import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri serves the dev build from a fixed port and expects a static bundle in
// ../dist for release builds.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    // Matches the WebKit version shipped with the minimum supported macOS.
    target: "safari14",
    sourcemap: false,
    chunkSizeWarningLimit: 700,
  },
});
