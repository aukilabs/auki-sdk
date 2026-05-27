import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const workspaceRoot = resolve(root, "../..");

export default {
  resolve: {
    alias: {
      "@aukilabs/auki-p2p-browser": resolve(
        workspaceRoot,
        "crates/auki-p2p-browser/src/index.ts",
      ),
    },
  },
  server: {
    fs: {
      allow: [workspaceRoot],
    },
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
  },
};
