import assert from "node:assert/strict";

export async function checkDiscovery(page) {
  const results = await page.evaluate(async () => {
    const { DomainDiscovery } = await import("/src/discovery.ts");
    const fresh = new Date(Date.now() + 60000).toISOString();
    const route = (peer) =>
      `/dns4/relay.test/tcp/443/wss/p2p/relay/p2p-circuit/p2p/${peer}`;
    const candidate = (peerId) => ({
      peerId,
      routes: [route(peerId)],
      expiresAt: fresh,
    });
    const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
    async function until(predicate) {
      for (let i = 0; i < 100; i++) {
        if (predicate()) return;
        await tick();
      }
      throw Error("Discovery test timed out");
    }
    // Staggered starts, self/stale/non-WSS filtering, and two independent receive directions.
    const visible = [candidate("a")],
      followed = [],
      inspected = [],
      releases = [];
    function create(localPeer) {
      return new DomainDiscovery(
        {
          localPeer,
          discover: async () => [
            ...visible,
            { ...candidate("expired"), expiresAt: "2000-01-01T00:00:00Z" },
            { ...candidate("tcp"), routes: ["/ip4/127.0.0.1/tcp/1"] },
          ],
          inspect: async (peer) => {
            inspected.push(`${localPeer}:${peer}`);
            return `map-${peer}`;
          },
          follow: async (selection) => {
            followed.push(`${localPeer}:${selection}`);
            await new Promise((resolve) => releases.push(resolve));
          },
          matches() {},
          status() {},
        },
        1,
        100,
      );
    }
    const a = create("a");
    const aa = a.run();
    await tick();
    visible.push(candidate("b"));
    const b = create("b");
    const bb = b.run();
    await until(() => followed.length === 2);
    a.cancel();
    b.cancel();
    releases.forEach((resolve) => resolve());
    await Promise.all([aa, bb]);
    const bidirectional = [...followed].sort();
    const filtered = inspected.every(
      (value) => value === "a:b" || value === "b:a",
    );

    // An unreachable candidate must not hide a reachable matching-session peer.
    const probes = [];
    let match;
    let release;
    const routes = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => [
          candidate("stale"),
          candidate("other-session"),
          { ...candidate("b"), routes: [route("bad"), route("b")] },
        ],
        inspect: async (peer, address) => {
          probes.push(address);
          if (peer === "stale" || address === route("bad"))
            throw Error("unreachable");
          return peer === "other-session" ? undefined : "b-map";
        },
        follow: async (value) => {
          match = value;
          await new Promise((resolve) => {
            release = resolve;
          });
        },
        matches() {},
        status() {},
      },
      1,
      10,
    );
    const routeTask = routes.run();
    await until(() => !!release);
    routes.cancel();
    release();
    await routeTask;

    // Do not pick an arbitrary peer from several peers in the same session.
    let choices, chosen, chooseRelease;
    const many = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => [candidate("b"), candidate("c")],
        inspect: async (peer) => peer,
        follow: async (peer) => {
          chosen = peer;
          await new Promise((resolve) => {
            chooseRelease = resolve;
          });
        },
        matches: (value) => {
          choices = value.map((v) => v.peerId);
        },
        status() {},
      },
      1,
      100,
    );
    const manyTask = many.run();
    await until(() => !!choices);
    const noArbitraryChoice = chosen === undefined;
    many.choose("c");
    await until(() => !!chooseRelease);
    many.cancel();
    chooseRelease();
    await manyTask;

    // Stop while a lookup is in flight: late results must never subscribe or update UI.
    let finishInspect,
      lateFollow = false,
      lateUi = false;
    const cancel = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => [candidate("b")],
        inspect: async () =>
          new Promise((resolve) => {
            finishInspect = resolve;
          }),
        follow: async () => {
          lateFollow = true;
        },
        matches: () => {
          lateUi = true;
        },
        status() {},
      },
      1,
      5,
    );
    const cancelTask = cancel.run();
    await until(() => !!finishInspect);
    cancel.cancel();
    finishInspect("b-map");
    await cancelTask;

    // Rediscover a fresh publication/peer after the first subscription ends.
    let publication = "old",
      reconnected,
      restartRelease;
    const restart = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => [candidate(publication)],
        inspect: async (peer) => peer,
        follow: async (peer) => {
          if (peer === "old") {
            publication = "new";
            throw Error("publisher stopped");
          }
          reconnected = peer;
          await new Promise((resolve) => {
            restartRelease = resolve;
          });
        },
        matches() {},
        status() {},
      },
      1,
      10,
    );
    const restartTask = restart.run();
    await until(() => !!restartRelease);
    restart.cancel();
    restartRelease();
    await restartTask;

    // Permanent DDS errors have a finite retry budget.
    let attempts = 0,
      boundedError = "";
    const failing = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => {
          attempts++;
          throw Error("DDS unavailable");
        },
        inspect: async () => undefined,
        follow: async () => {},
        matches() {},
        status() {},
      },
      1,
      100,
    );
    try {
      await failing.run();
    } catch (error) {
      boundedError = error.message;
    }
    return {
      bidirectional,
      filtered,
      match,
      fallback: probes.includes(route("b")),
      choices,
      chosen,
      noArbitraryChoice,
      lateFollow,
      lateUi,
      reconnected,
      attempts,
      boundedError,
    };
  });
  assert.deepEqual(results.bidirectional, ["a:map-b", "b:map-a"]);
  assert.equal(results.filtered, true);
  assert.equal(results.match, "b-map");
  assert.equal(results.fallback, true);
  assert.deepEqual(results.choices, ["b", "c"]);
  assert.equal(results.chosen, "c");
  assert.equal(results.noArbitraryChoice, true);
  assert.equal(results.lateFollow, false);
  assert.equal(results.lateUi, false);
  assert.equal(results.reconnected, "new");
  assert.equal(results.attempts, 3);
  assert.match(results.boundedError, /paused after three failures/);
  console.log(
    "Passed: automatic bidirectional discovery, filtering, route fallback, ambiguity, cancellation, restart and retry bounds.",
  );
}
