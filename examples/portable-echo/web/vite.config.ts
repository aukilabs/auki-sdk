import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  base: "./",
  build: {
    rollupOptions: {
      input: {
        playground: resolve(import.meta.dirname, "index.html"),
        minimal: resolve(import.meta.dirname, "minimal.html"),
      },
    },
  },
});
