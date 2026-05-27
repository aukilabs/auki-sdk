import { describe, it, expect } from "vitest";
import {
  canonicalize,
  validate,
  reasonLabel,
  DOMAIN_NAME_MAX,
  RESERVED_NAMES,
} from "./domainName";

describe("canonicalize", () => {
  it("lowercases", () => {
    expect(canonicalize("Atlanta")).toBe("atlanta");
  });

  it("replaces whitespace with hyphens", () => {
    expect(canonicalize("Atlanta Warehouse")).toBe("atlanta-warehouse");
  });

  it("collapses repeated whitespace", () => {
    expect(canonicalize("Atlanta   Warehouse")).toBe("atlanta-warehouse");
  });

  it("collapses repeated hyphens", () => {
    expect(canonicalize("atlanta---warehouse")).toBe("atlanta-warehouse");
  });

  it("trims leading and trailing hyphens", () => {
    expect(canonicalize("---atlanta-warehouse---")).toBe("atlanta-warehouse");
  });

  it("trims surrounding whitespace", () => {
    expect(canonicalize("   atlanta   ")).toBe("atlanta");
  });

  it("strips punctuation", () => {
    expect(canonicalize("atlanta_warehouse.com!")).toBe("atlantawarehousecom");
  });

  it("strips unicode beyond ascii alphanumerics", () => {
    expect(canonicalize("北京-warehouse")).toBe("warehouse");
  });

  it("preserves embedded digits", () => {
    expect(canonicalize("Q3 pilot 2026")).toBe("q3-pilot-2026");
  });

  it("returns empty string for whitespace-only input", () => {
    expect(canonicalize("   ")).toBe("");
  });

  it("returns empty string for punctuation-only input", () => {
    expect(canonicalize("!!!@@@")).toBe("");
  });

  it("truncates at DOMAIN_NAME_MAX characters", () => {
    const input = "a".repeat(DOMAIN_NAME_MAX + 10);
    expect(canonicalize(input).length).toBe(DOMAIN_NAME_MAX);
  });

  it("idempotent — canonicalize(canonicalize(x)) === canonicalize(x)", () => {
    const samples = ["Atlanta Warehouse", "  q3   pilot 2026  ", "AT&T-Q3"];
    for (const s of samples) {
      const once = canonicalize(s);
      expect(canonicalize(once)).toBe(once);
    }
  });
});

describe("validate", () => {
  it("accepts a normal name", () => {
    expect(validate("atlanta-warehouse")).toEqual({
      ok: true,
      canonical: "atlanta-warehouse",
    });
  });

  it("accepts after normalization", () => {
    expect(validate("Atlanta Warehouse")).toEqual({
      ok: true,
      canonical: "atlanta-warehouse",
    });
  });

  it("rejects empty input", () => {
    expect(validate("")).toEqual({
      ok: false,
      reason: "empty",
      canonical: "",
    });
  });

  it("rejects whitespace-only input as empty", () => {
    expect(validate("   ")).toEqual({
      ok: false,
      reason: "empty",
      canonical: "",
    });
  });

  it("rejects punctuation-only input as empty", () => {
    expect(validate("!!!")).toEqual({
      ok: false,
      reason: "empty",
      canonical: "",
    });
  });

  it("does not reject by length when the canonicalized form fits", () => {
    // canonicalize truncates, so any input becomes acceptable on length.
    // This pins the current behaviour — if we ever relax canonicalize's
    // truncation, validate must catch it.
    const input = "a".repeat(DOMAIN_NAME_MAX + 10);
    expect(validate(input)).toEqual({
      ok: true,
      canonical: "a".repeat(DOMAIN_NAME_MAX),
    });
  });

  it("RESERVED_NAMES is empty in v1 (Q12 lean)", () => {
    expect(RESERVED_NAMES.size).toBe(0);
  });
});

describe("reasonLabel", () => {
  it("has a label for every reason", () => {
    expect(reasonLabel("empty")).toMatch(/Enter/);
    expect(reasonLabel("too_long")).toMatch(new RegExp(String(DOMAIN_NAME_MAX)));
    expect(reasonLabel("reserved")).toMatch(/reserved/);
  });
});
