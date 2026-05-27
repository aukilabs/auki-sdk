// Motion language — durations + easings centralised so every animation
// reads from the same set, regardless of whether it's driven by CSS
// transitions, View Transitions, or Motion One.
//
// Numeric values mirror the CSS custom properties in style.css. Update
// both together if these ever change.

import { animate, type AnimationPlaybackControls } from "motion";

export const dur = {
  micro: 0.18, // button presses, hovers, micro-interactions
  panel: 0.26, // modals, drawers, tooltips
  page: 0.42, // route transitions, large surface morphs
} as const;

export const ease = {
  emphasized: [0.22, 1, 0.36, 1] as [number, number, number, number],
  standard: [0.4, 0, 0.2, 1] as [number, number, number, number],
} as const;

// ─── ready-made entrances / exits ────────────────────────────────────────────

/** Fade + slight upward slide. The default modal/dialog entrance. */
export function fadeUpIn(
  el: HTMLElement,
  opts?: { duration?: number; y?: number },
): AnimationPlaybackControls {
  return animate(
    el,
    { opacity: [0, 1], transform: [`translateY(${opts?.y ?? 8}px)`, "translateY(0)"] },
    { duration: opts?.duration ?? dur.panel, ease: ease.emphasized },
  );
}

/** Fade out + slide down slightly. Pair with fadeUpIn for symmetric exits. */
export function fadeDownOut(
  el: HTMLElement,
  opts?: { duration?: number; y?: number },
): AnimationPlaybackControls {
  return animate(
    el,
    { opacity: [1, 0], transform: ["translateY(0)", `translateY(${opts?.y ?? 8}px)`] },
    { duration: opts?.duration ?? dur.micro, ease: ease.standard },
  );
}

/** Plain opacity fade. Backdrops, dim layers. */
export function fade(
  el: HTMLElement,
  to: number,
  opts?: { duration?: number },
): AnimationPlaybackControls {
  return animate(
    el,
    { opacity: to },
    { duration: opts?.duration ?? dur.panel, ease: ease.standard },
  );
}

// ─── route transitions ───────────────────────────────────────────────────────

/**
 * Run route DOM updates independently of the View Transitions API.
 * Park's route swap can lazy-load views, and Chromium may skip/abort
 * async view-transition callbacks before the new view is appended.
 * Rendering directly preserves the final Park pixels and avoids a
 * blank content pane when Overwatch joins a domain.
 */
export function withViewTransition(update: () => void | Promise<void>): void {
  runUpdate(update);
}

function runUpdate(update: () => void | Promise<void>): void {
  try {
    const result = update();
    if (isPromiseLike(result)) {
      void result.catch(reportAsyncError);
    }
  } catch (error) {
    reportAsyncError(error);
  }
}

function isPromiseLike(value: unknown): value is Promise<void> {
  return Boolean(value && typeof value === "object" && "catch" in value);
}

function reportAsyncError(error: unknown): void {
  window.setTimeout(() => {
    throw error;
  }, 0);
}
