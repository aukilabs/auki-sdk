import { describe, it, expect } from "vitest";
import { formatAge } from "./info";

// `formatAge` is the load-bearing duration helper for "running 12m" and
// "in cluster · 8m" labels. Minute precision is intentional (operators
// don't care about the seconds for session-age UX).

describe("formatAge", () => {
  it("clamps negative to 0s", () => {
    expect(formatAge(-1)).toBe("0s");
    expect(formatAge(-1000)).toBe("0s");
  });

  it("renders sub-minute durations with second precision", () => {
    expect(formatAge(0)).toBe("0s");
    expect(formatAge(999)).toBe("0s");
    expect(formatAge(1_000)).toBe("1s");
    expect(formatAge(45_000)).toBe("45s");
    expect(formatAge(59_999)).toBe("59s");
  });

  it("renders sub-hour durations as round minutes", () => {
    expect(formatAge(60_000)).toBe("1m");
    expect(formatAge(60_999)).toBe("1m");
    expect(formatAge(120_000)).toBe("2m");
    expect(formatAge(59 * 60_000)).toBe("59m");
  });

  it("renders durations >= 1h with hours + remaining minutes", () => {
    expect(formatAge(60 * 60_000)).toBe("1h");
    expect(formatAge(74 * 60_000)).toBe("1h 14m");
    expect(formatAge(2 * 60 * 60_000 + 14 * 60_000)).toBe("2h 14m");
    expect(formatAge(3 * 60 * 60_000)).toBe("3h");
  });
});
