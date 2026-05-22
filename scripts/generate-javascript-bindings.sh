#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <crate>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

crate="$1"
crate_dir="crates/${crate}"
crate_module="${crate//-/_}"
package_parent="bindings/javascript"
out_dir="bindings/javascript/${crate}"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack is required; run just install-toolchain" >&2
  exit 1
fi

if [[ ! -d "$crate_dir" ]]; then
  echo "crate not found: $crate_dir" >&2
  exit 1
fi

mkdir -p "$package_parent"
tmp_dir="$(mktemp -d "${package_parent}/.${crate}.tmp.XXXXXX")"
backup_dir=""
swapped=0

cleanup() {
  status=$?
  rm -rf "$tmp_dir"
  if [[ "$status" -ne 0 && "$swapped" -eq 1 ]]; then
    rm -rf "$out_dir"
    if [[ -n "$backup_dir" && -d "$backup_dir" ]]; then
      mv "$backup_dir" "$out_dir"
    fi
  elif [[ -n "$backup_dir" && -d "$backup_dir" && ! -e "$out_dir" ]]; then
    mv "$backup_dir" "$out_dir"
  fi
}
trap cleanup EXIT

(
  cd "$crate_dir"
  wasm-pack build . \
    --target web \
    --out-dir "../../${tmp_dir}" \
    --no-default-features \
    --features wasm
)

rm -f "$tmp_dir/.gitignore"

node - "$tmp_dir/package.json" "$crate" "$crate_module" <<'NODE'
const fs = require("node:fs");

const [packagePath, crate, crateModule] = process.argv.slice(2);
const packageJson = JSON.parse(fs.readFileSync(packagePath, "utf8"));

packageJson.description = `wasm-bindgen JavaScript bindings for the ${crate} proving crate.`;
packageJson.files = [
  `${crateModule}_bg.wasm`,
  `${crateModule}.js`,
  `${crateModule}.d.ts`,
  `${crateModule}_bg.wasm.d.ts`,
  "smoke.mjs",
];
packageJson.main = `${crateModule}.js`;
packageJson.types = `${crateModule}.d.ts`;

fs.writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
NODE

cat > "$tmp_dir/README.md" <<'README'
# __CRATE__ JavaScript bindings

Generated wasm-bindgen JavaScript package for the `__CRATE__` proving crate.

Generate the package from the repo root:

```bash
just generate-javascript-bindings __CRATE__
```

The generated package contains:

- `package.json` — npm package metadata for the web-targeted ESM package.
- `__CRATE_MODULE__.js` — wasm-bindgen JavaScript glue.
- `__CRATE_MODULE__.d.ts` — TypeScript declarations for the JavaScript glue.
- `__CRATE_MODULE___bg.wasm` — compiled WebAssembly module.
- `__CRATE_MODULE___bg.wasm.d.ts` — wasm-bindgen WebAssembly declarations.
- `smoke.mjs` — Node-compatible smoke test for the generated web-target package.
README
perl -0pi -e "s/__CRATE_MODULE__/${crate_module}/g; s/__CRATE__/${crate}/g" "$tmp_dir/README.md"

cat > "$tmp_dir/smoke.mjs" <<'SMOKE'
import { readFile } from "node:fs/promises";
import init, {
  add,
  hello,
  makeGreeting,
  delayedGreeting,
  Counter,
  GreetingStyle,
} from "./__CRATE_MODULE__.js";

const wasmBytes = await readFile(new URL("./__CRATE_MODULE___bg.wasm", import.meta.url));
await init({ module_or_path: wasmBytes });

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

assert(add(2, 3) === 5, "add failed");
assert(hello("JavaScript") === "Hello, JavaScript.", "hello failed");

const greeting = makeGreeting("JavaScript", GreetingStyle.Formal);
assert(greeting.message === "Good day, JavaScript.", "formal greeting failed");
assert(greeting.nameLength === 10, "nameLength failed");
assert(greeting.style === GreetingStyle.Formal, "greeting style failed");
if (typeof greeting.free === "function") {
  greeting.free();
}

const delayed = await delayedGreeting("JavaScript", 0);
assert(delayed.message === "Hello, JavaScript.", "delayed greeting failed");
assert(delayed.style === GreetingStyle.Casual, "delayed greeting style failed");
if (typeof delayed.free === "function") {
  delayed.free();
}

const counter = new Counter(10);
assert(counter.value() === 10, "counter initial failed");
const updated = await counter.addAfter(7, 0);
assert(updated === 17, "counter update failed");
assert(counter.value() === 17, "counter final failed");
if (typeof counter.free === "function") {
  counter.free();
}

const releasedCounter = new Counter(20);
const pendingUpdate = releasedCounter.addAfter(4, 1);
if (typeof releasedCounter.free === "function") {
  releasedCounter.free();
}
assert(await pendingUpdate === 24, "counter pending update after free failed");

console.log("javascript wasm smoke ok");
SMOKE
perl -0pi -e "s/__CRATE_MODULE__/${crate_module}/g" "$tmp_dir/smoke.mjs"

if [[ -e "$out_dir" ]]; then
  backup_dir="$(mktemp -d "${package_parent}/.${crate}.old.XXXXXX")"
  rmdir "$backup_dir"
  mv "$out_dir" "$backup_dir"
fi

mv "$tmp_dir" "$out_dir"
swapped=1

echo "Generated JavaScript bindings in $out_dir"
node "${out_dir}/smoke.mjs"

if [[ -n "$backup_dir" ]]; then
  rm -rf "$backup_dir"
fi
trap - EXIT
