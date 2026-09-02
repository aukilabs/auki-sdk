const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const src = path.join(root, "src", "web", "generated");
const dest = path.join(root, "build", "web", "generated");

if (!fs.existsSync(src)) {
  console.error(`missing wasm output at ${src}; run npm run build:wasm first`);
  process.exit(1);
}

fs.mkdirSync(dest, { recursive: true });
for (const name of fs.readdirSync(src)) {
  if (name === ".gitkeep") continue;
  fs.copyFileSync(path.join(src, name), path.join(dest, name));
}

console.log(`copied web artifacts → ${dest}`);
