import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "happy-dom",
    exclude: ["node_modules/**", "dist/**", "sdk-generated/**"],
    setupFiles: ["./src/test/setup.ts"],
  },
});
