import { defineConfig } from "vite";

// A hot reload destroys in-memory map state and skips awaited relay cleanup.
// Apply changes by stopping the peer and manually reloading instead.
export default defineConfig({ server: { hmr: false } });
