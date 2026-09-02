import wasmAsset from "./generated/auki_sdk_web_bg.wasm";

export type AukiSdkWasm = typeof import("./generated/auki_sdk_web.js");

let wasmPromise: Promise<AukiSdkWasm> | null = null;

function resolveWasmModuleOrPath(asset: unknown): string | URL | Request {
  if (typeof asset === "string") {
    if (/^https?:\/\//i.test(asset) || asset.startsWith("/")) {
      return asset;
    }
    const base = globalThis.location?.href;
    if (!base) {
      throw new Error("auki-sdk-expo: cannot resolve relative wasm path without location.href");
    }
    return new URL(asset, base);
  }
  if (typeof asset === "object" && asset !== null && "default" in asset) {
    return resolveWasmModuleOrPath((asset as { default: unknown }).default);
  }
  throw new Error(
    `auki-sdk-expo: expected Metro web wasm URL string, got ${typeof asset}: ${String(asset)}`,
  );
}

/**
 * Lazy-load the wasm-pack output under `./generated`.
 * Run `npm run build:wasm` before typecheck/build.
 *
 * Passes the .wasm asset explicitly — `--target web` defaults to
 * `new URL(..., import.meta.url)`, which Metro does not provide.
 */
export async function loadAukiSdkWasm(): Promise<AukiSdkWasm> {
  if (!wasmPromise) {
    wasmPromise = (async () => {
      const mod = await import("./generated/auki_sdk_web.js");
      const moduleOrPath = resolveWasmModuleOrPath(wasmAsset);
      console.log("[auki-sdk-expo] init wasm from", moduleOrPath);
      if (typeof mod.default === "function") {
        await mod.default({ module_or_path: moduleOrPath });
      } else {
        const init = (mod as { init?: (arg?: unknown) => Promise<unknown> }).init;
        if (typeof init === "function") {
          await init({ module_or_path: moduleOrPath });
        }
      }
      return mod as AukiSdkWasm;
    })();
  }
  return wasmPromise;
}
