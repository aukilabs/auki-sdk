import { afterEach, describe, expect, it } from "vitest";

import { withViewTransition } from "./anim";

describe("withViewTransition", () => {
  const originalStartViewTransition = (
    document as unknown as { startViewTransition?: unknown }
  ).startViewTransition;

  afterEach(() => {
    (
      document as unknown as { startViewTransition?: unknown }
    ).startViewTransition = originalStartViewTransition;
  });

  it("renders route work without depending on the View Transitions API", async () => {
    let updated = false;
    (
      document as unknown as {
        startViewTransition?: (cb: () => unknown) => unknown;
      }
    ).startViewTransition = (cb) => {
      void cb;
      throw new Error("startViewTransition should not be used for route rendering");
    };

    withViewTransition(async () => {
      await Promise.resolve();
      updated = true;
    });

    await Promise.resolve();
    await Promise.resolve();
    expect(updated).toBe(true);
  });
});
