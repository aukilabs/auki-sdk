import assert from "node:assert/strict";
export async function checkDiscovery(page) {
  const result = await page.evaluate(async () => {
    const { DomainDiscovery } = await import("/src/discovery.ts");
    const tick = () => new Promise((r) => setTimeout(r, 2));
    async function until(fn) {
      for (let i = 0; i < 300; i++) {
        if (fn()) return;
        await tick();
      }
      throw Error("Discovery timed out");
    }
    const candidate = (peerId) => ({
      peerId,
      routes: [`/dns4/relay.test/tcp/443/wss/p2p/r/p2p-circuit/p2p/${peerId}`],
      expiresAt: new Date(Date.now() + 60000).toISOString(),
    });
    const visible = ["a"],
      active = new Map(),
      followed = [],
      scanned = [],
      tasks = [];
    function host(id) {
      const releases = new Map();
      const controller = new DomainDiscovery(
        {
          localPeer: id,
          discover: async () => {
            scanned.push(id);
            return [
              ...visible.map(candidate),
              { ...candidate("expired"), expiresAt: "2000-01-01" },
              { ...candidate("tcp"), routes: ["/ip4/127.0.0.1/tcp/1"] },
            ];
          },
          inspect: async (peer) => `${peer}-product`,
          follow: async (selection, peer) => {
            followed.push(`${id}:${peer}`);
            active.set(`${id}:${peer}`, selection);
            await new Promise((r) => releases.set(peer, r));
            active.delete(`${id}:${peer}`);
          },
          unfollow: async (peer) => {
            releases.get(peer)?.();
          },
          status() {},
        },
        2,
      );
      tasks.push(controller.run());
      return { controller, releases };
    }
    const a = host("a");
    await tick();
    await tick();
    const solo = followed.length === 0;
    visible.push("b");
    const b = host("b");
    await until(() => active.size === 2);
    visible.push("c");
    const c = host("c");
    await until(() => active.size === 6);
    const allDirections = [...active.keys()].sort();
    // A departed stream must not end discovery; a replacement peer joins the survivors.
    c.controller.cancel();
    a.releases.get("c")();
    b.releases.get("c")();
    visible.splice(visible.indexOf("c"), 1);
    await until(() => !active.has("c:a") && !active.has("c:b"));
    visible.push("d");
    const d = host("d");
    await until(
      () =>
        active.has("a:d") &&
        active.has("b:d") &&
        active.has("d:a") &&
        active.has("d:b"),
    );
    const rejoined = true;
    a.controller.cancel();
    b.controller.cancel();
    d.controller.cancel();
    await Promise.all(tasks);
    // DDS may fail longer than the previous three-attempt limit, then recover.
    let attempts = 0,
      recovered = false,
      release;
    const retry = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => {
          if (++attempts < 5) throw Error("DDS down");
          return [candidate("b")];
        },
        inspect: async () => "map",
        follow: async () => {
          recovered = true;
          await new Promise((r) => (release = r));
        },
        unfollow: async () => release?.(),
        status() {},
      },
      1,
    );
    const retryTask = retry.run();
    await until(() => recovered);
    retry.cancel();
    await retryTask;
    // Cancellation with an in-flight inspection cannot create a late receive task.
    let inspectDone,
      late = false;
    const cancel = new DomainDiscovery(
      {
        localPeer: "a",
        discover: async () => [candidate("b")],
        inspect: async () => new Promise((r) => (inspectDone = r)),
        follow: async () => {
          late = true;
        },
        unfollow: async () => {},
        status() {},
      },
      1,
    );
    const cancelTask = cancel.run();
    await until(() => inspectDone);
    cancel.cancel();
    inspectDone("map");
    await cancelTask;
    return {
      solo,
      allDirections,
      rejoined,
      attempts,
      recovered,
      late,
      remaining: active.size,
      filtered: followed.every(
        (p) => !p.includes("expired") && !p.includes("tcp"),
      ),
    };
  });
  assert.equal(result.solo, true);
  assert.deepEqual(result.allDirections, [
    "a:b",
    "a:c",
    "b:a",
    "b:c",
    "c:a",
    "c:b",
  ]);
  assert.equal(result.rejoined, true);
  assert.equal(result.recovered, true);
  assert.ok(result.attempts >= 5);
  assert.equal(result.late, false);
  assert.equal(result.remaining, 0);
  assert.equal(result.filtered, true);
  console.log(
    "Passed: solo start, three-peer full mesh, late joins, departures/rejoins, sustained DDS recovery, filtering and cancellation.",
  );
}
