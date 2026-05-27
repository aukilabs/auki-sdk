import { describe, it, expect, beforeEach } from "vitest";
import {
  getDomainName,
  setDomainName,
  clearDomainName,
  DOMAIN_STORAGE_KEY,
} from "./domain";

// Same shape as src/data/seenDaemons.test.ts — Node 22+ ships an
// experimental `localStorage` that lacks the Storage API, so we
// install a per-test in-memory polyfill.
class TestLocalStorage implements Storage {
  private store = new Map<string, string>();
  get length() {
    return this.store.size;
  }
  clear() {
    this.store.clear();
  }
  getItem(k: string) {
    return this.store.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.store.set(k, v);
  }
  removeItem(k: string) {
    this.store.delete(k);
  }
  key(i: number) {
    return Array.from(this.store.keys())[i] ?? null;
  }
}

beforeEach(() => {
  Object.defineProperty(globalThis, "localStorage", {
    value: new TestLocalStorage(),
    writable: true,
    configurable: true,
  });
});

describe("getDomainName", () => {
  it("returns null when nothing is stored (first boot)", () => {
    expect(getDomainName()).toBeNull();
  });

  it("returns null for empty-string value (treated as unset)", () => {
    localStorage.setItem(DOMAIN_STORAGE_KEY, "");
    expect(getDomainName()).toBeNull();
  });

  it("returns the stored value when present", () => {
    localStorage.setItem(DOMAIN_STORAGE_KEY, "atlanta-warehouse");
    expect(getDomainName()).toBe("atlanta-warehouse");
  });
});

describe("setDomainName", () => {
  it("persists the canonical form", () => {
    setDomainName("atlanta-warehouse");
    expect(getDomainName()).toBe("atlanta-warehouse");
    expect(localStorage.getItem(DOMAIN_STORAGE_KEY)).toBe("atlanta-warehouse");
  });

  it("overwrites previous value", () => {
    setDomainName("first");
    setDomainName("second");
    expect(getDomainName()).toBe("second");
  });
});

describe("clearDomainName", () => {
  it("removes the persisted value", () => {
    setDomainName("atlanta-warehouse");
    clearDomainName();
    expect(getDomainName()).toBeNull();
  });

  it("is a no-op when nothing is stored", () => {
    clearDomainName();
    expect(getDomainName()).toBeNull();
  });
});

describe("storage key", () => {
  it("uses the auki.park.*.v<N> namespace", () => {
    expect(DOMAIN_STORAGE_KEY).toMatch(/^auki\.park\..*\.v\d+$/);
  });
});
