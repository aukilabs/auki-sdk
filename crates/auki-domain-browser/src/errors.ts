import type { PeerError, Result } from "./contract.js";

export function ok<T>(value: T): Result<T> {
  return { ok: true, value };
}

export function fail<T>(code: PeerError["code"], message: string): Result<T> {
  return { ok: false, error: { code, message } };
}

export function transportUnavailable<T>(): Result<T> {
  return fail("transport_unavailable", "Browser SDK transport is not implemented yet.");
}
