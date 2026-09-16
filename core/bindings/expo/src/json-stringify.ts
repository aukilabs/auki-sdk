/**
 * Wasm protocol records use i64/u64 (catalog ns windows, stream timestamps).
 * serde_wasm_bindgen emits those as BigInt; JSON.stringify otherwise throws.
 */
export function jsonStringify(
  value: unknown,
  space?: string | number,
): string {
  return JSON.stringify(
    value,
    (_key, field: unknown) =>
      typeof field === "bigint" ? field.toString() : field,
    space,
  );
}
