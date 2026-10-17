import { defineConfig } from "vite";

// La raiz del proyecto Vite es `src/` (contiene index.html) y el bundle se
// emite en `dist/`, que es lo que Tauri publica como frontendDist.
export default defineConfig({
  root: "src",
  publicDir: false,
  clearScreen: false,
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "chrome110",
    sourcemap: false,
    minify: "esbuild",
  },
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
});
