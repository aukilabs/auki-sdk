import { onRouteChange, type Route } from "./router";

let currentInspectFocus: string | null = null;
let audioListenTarget: string | null = null;
let started = false;

function focusFor(route: Route): string | null {
  if (route.view === "robot") return route.url;
  return null;
}

export function startInspectFocusSync(): void {
  if (started) return;
  started = true;
  onRouteChange((route) => {
    currentInspectFocus = focusFor(route);
  });
}

export function setAudioListenTarget(peerId: string | null): void {
  audioListenTarget = peerId;
}

export function getInspectFocus(): string | null {
  return currentInspectFocus;
}

export function getAudioListenTarget(): string | null {
  return audioListenTarget;
}
