import { describe, it, expect, beforeEach } from "vitest";
import { getRoute, navigate, onRouteChange, type Route } from "./router";

// Router parses + serialises hashes round-trip. These tests exercise
// the core invariant: navigate(parse(x)) === x for every route shape,
// AND onRouteChange fires with the new route after a hash change.

describe("router", () => {
  beforeEach(() => {
    window.location.hash = "";
  });

  it("parses '' as directory", () => {
    window.location.hash = "";
    expect(getRoute()).toEqual({ view: "directory" });
  });

  it("parses '#/' as directory", () => {
    window.location.hash = "#/";
    expect(getRoute()).toEqual({ view: "directory" });
  });

  it("parses '#/cluster' as cluster", () => {
    window.location.hash = "#/cluster";
    expect(getRoute()).toEqual({ view: "cluster" });
  });

  it("parses '#/robot/<encoded>' as robot with decoded url", () => {
    const url = "http://192.168.9.72:8080";
    window.location.hash = `#/robot/${encodeURIComponent(url)}`;
    expect(getRoute()).toEqual({ view: "robot", url });
  });

  it("falls back to directory for unknown routes", () => {
    window.location.hash = "#/something-unknown";
    expect(getRoute()).toEqual({ view: "directory" });
  });

  it("navigate + getRoute round-trips every route shape", () => {
    const cases: Route[] = [
      { view: "directory" },
      { view: "cluster" },
      { view: "robot", url: "http://192.168.9.72:8080" },
    ];
    for (const route of cases) {
      navigate(route);
      expect(getRoute()).toEqual(route);
    }
  });

  it("onRouteChange fires on hash change", async () => {
    const fired: Route[] = [];
    const dispose = onRouteChange((r) => fired.push(r));
    // Initial fire on subscribe.
    expect(fired.length).toBe(1);
    expect(fired[0]).toEqual({ view: "directory" });

    navigate({ view: "cluster" });
    // hashchange is async via the browser event loop; flush microtasks.
    await new Promise((r) => setTimeout(r, 0));
    expect(fired.length).toBeGreaterThanOrEqual(2);
    expect(fired[fired.length - 1]).toEqual({ view: "cluster" });

    dispose();
  });
});
