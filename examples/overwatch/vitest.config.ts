import { defineConfig } from "vitest/config";

// Vitest config — kept separate from `vite.config.ts` so production
// build stays free of test-specific concerns. Vitest auto-discovers
// `*.test.ts` files under `src/`. happy-dom (lighter/faster than jsdom)
// covers tests that touch DOM APIs (e.g. localStorage, URL); pure-logic
// tests still run in the default node environment when they don't
// reference DOM globals.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "happy-dom",
    globals: false,
  },
});
