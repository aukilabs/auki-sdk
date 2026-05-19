import { beforeEach, describe, expect, it } from "vitest";
import { installAukiBrowserPeer } from "./installGlobal";
import type { BrowserDomainPeer } from "./contract";

declare global {
  interface Window {
    aukiBrowserPeer?: { createPeer(): Promise<BrowserDomainPeer> };
  }
}

describe("installAukiBrowserPeer", () => {
  beforeEach(() => {
    delete window.aukiBrowserPeer;
  });

  it("installs a Park-compatible global factory", async () => {
    const peer = {} as BrowserDomainPeer;
    installAukiBrowserPeer(() => Promise.resolve(peer));

    await expect(window.aukiBrowserPeer?.createPeer()).resolves.toBe(peer);
  });
});
