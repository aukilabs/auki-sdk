import type { BrowserDomainPeer, BrowserDomainPeerFactory } from "./contract.js";

declare global {
  interface Window {
    aukiBrowserPeer?: BrowserDomainPeerFactory;
  }
}

export function installAukiBrowserPeer(createPeer: () => Promise<BrowserDomainPeer>): void {
  window.aukiBrowserPeer = { createPeer };
}
