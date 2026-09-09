const { test } = require('node:test');
const assert = require('node:assert/strict');
const { prepareMetroWasm } = require('./prepare-metro-wasm.cjs');
const fallback = "module_or_path = new URL('auki_sdk_web_bg.wasm', import.meta.url);";
test('requires explicit URL without leaving unparseable import.meta syntax', () => {
  const result = prepareMetroWasm(`if (module_or_path === undefined) { ${fallback} }`);
  assert.ok(result.includes('requires an explicit Wasm asset URL'));
  assert.ok(!result.includes('import.meta'));
  new Function('module_or_path', result)('http://127.0.0.1/module.wasm');
  assert.throws(() => new Function('module_or_path', result)(undefined), /explicit/);
});
test('generator drift fails loudly rather than silently corrupting output', () => {
  assert.throws(() => prepareMetroWasm('different generator'));
  assert.throws(() => prepareMetroWasm(fallback + fallback));
  assert.throws(() => prepareMetroWasm(fallback + ' import.meta.other'));
});
