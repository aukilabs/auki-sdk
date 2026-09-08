// Generated-output adaptation, not a second Wasm loader. Metro client bundles
// are classic scripts and cannot parse import.meta, even in an unused branch.
const fs = require('node:fs');
const path = require('node:path');

function prepareMetroWasm(source) {
  const fallback = "module_or_path = new URL('auki_sdk_web_bg.wasm', import.meta.url);";
  if (source.split(fallback).length !== 2) throw new Error('wasm-bindgen default URL shape changed; inspect the generated loader');
  const result = source.replace(fallback, "throw new Error('auki-sdk-expo requires an explicit Wasm asset URL');");
  if (result.includes('import.meta')) throw new Error('unexpected import.meta outside the default Wasm URL');
  return result;
}
if (require.main === module) {
  const file = path.resolve(__dirname, '../src/web/generated/auki_sdk_web.js');
  fs.writeFileSync(file, prepareMetroWasm(fs.readFileSync(file, 'utf8')));
}
module.exports = { prepareMetroWasm };
