import { cp, mkdir, rm, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const exampleRoot = path.resolve(here, "..");
const repoRoot = path.resolve(exampleRoot, "../..");

const packages = [
  {
    name: "auki-network",
    source: path.join(repoRoot, "bindings/javascript/auki-network"),
    required: "index.js",
  },
  {
    name: "auki-domain",
    source: path.join(repoRoot, "bindings/javascript/auki-domain"),
    required: "index.js",
  },
];

for (const pkg of packages) {
  const requiredPath = path.join(pkg.source, pkg.required);
  if (!(await exists(requiredPath))) {
    console.error(`Missing generated SDK package file: ${requiredPath}`);
    console.error("Run:\n  just generate-javascript-bindings auki-network\n  just generate-javascript-bindings auki-domain");
    process.exit(1);
  }
}

await mkdir(path.join(exampleRoot, "sdk-generated"), { recursive: true });

for (const pkg of packages) {
  const target = path.join(exampleRoot, "sdk-generated", pkg.name);
  await rm(target, { recursive: true, force: true });
  await cp(pkg.source, target, {
    recursive: true,
    filter: (source) => {
      const base = path.basename(source);
      return base !== "node_modules" && !base.startsWith(".auki-");
    },
  });
}

console.log("Staged generated SDK packages for Overwatch.");

async function exists(file) {
  try {
    await stat(file);
    return true;
  } catch (error) {
    if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}
