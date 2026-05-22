import { access } from "node:fs/promises";
import http from "node:http";
import { chromium } from "playwright-core";

const addressArg = process.argv[2];
const parkUrl = process.argv[3] ?? "http://127.0.0.1:7880";

if (!addressArg) {
  throw new Error(
    "usage: node scripts/smoke_park_two_browser_acceptance.mjs <manager-webrtc-direct-multiaddr>[|relay-ws-multiaddr] [park-url]",
  );
}

const [address, relayAddress] = addressArg.split("|").filter(Boolean);
const managerPeerId = address.match(/\/p2p\/([^/]+)$/)?.[1];
if (!managerPeerId) {
  throw new Error("manager multiaddr must end with /p2p/<peer-id>");
}

const domain = "browser-two-peer-smoke";

const discovery = http.createServer((req, res) => {
  res.setHeader("access-control-allow-origin", "*");
  res.setHeader("access-control-allow-methods", "GET, OPTIONS");
  res.setHeader("access-control-allow-headers", "content-type");
  if (req.method === "OPTIONS") {
    res.statusCode = 204;
    res.end();
    return;
  }

  const url = new URL(req.url, "http://127.0.0.1");
  if (url.pathname !== "/clusters") {
    res.statusCode = 404;
    res.end("not found");
    return;
  }

  res.setHeader("content-type", "application/json; charset=utf-8");
  res.end(
    JSON.stringify({
      clusters: [
        {
          name: domain,
          manager_peer_id: managerPeerId,
          manager_multiaddrs: [address],
          relay_multiaddrs: relayAddress ? [relayAddress] : [],
          peer_count: 1,
          created_ns: 1,
          last_liveness_check_ns: 1,
        },
      ],
    }),
  );
});

await new Promise((resolve) => discovery.listen(0, "127.0.0.1", resolve));
const discoveryUrl = `http://127.0.0.1:${discovery.address().port}`;

const executablePath = await chromeExecutable();
const browser = await chromium.launch({
  headless: true,
  ...(executablePath ? { executablePath } : {}),
});

try {
  const [peerA, peerB] = await Promise.all([
    createParkPeer(browser, parkUrl, "Park Smoke A"),
    createParkPeer(browser, parkUrl, "Park Smoke B"),
  ]);

  const joinA = await joinDomain(peerA, discoveryUrl, domain);
  if (!joinA.joinResult.ok) {
    throw new Error(`peer A join failed: ${JSON.stringify(joinA.joinResult)}`);
  }

  const joinB = await joinDomain(peerB, discoveryUrl, domain);
  if (!joinB.joinResult.ok) {
    throw new Error(`peer B join failed: ${JSON.stringify(joinB.joinResult)}`);
  }

  await delay(1500);

  const [snapshotA, snapshotB] = await Promise.all([
    latestSnapshot(peerA),
    latestSnapshot(peerB),
  ]);
  const publishA = await publishAudio(peerA);
  const publishB = await publishAudio(peerB);
  const listenAtoB = await listenTo(peerA, peerB.selfPeerId);
  const listenBtoA = await listenTo(peerB, peerA.selfPeerId);
  const [postAudioSnapshotA, postAudioSnapshotB] = await Promise.all([
    latestSnapshot(peerA),
    latestSnapshot(peerB),
  ]);

  const report = {
    domain,
    managerPeerId,
    peers: {
      A: {
        selfPeerId: peerA.selfPeerId,
        snapshot: summarize(snapshotA),
        postAudioSnapshot: summarize(postAudioSnapshotA),
      },
      B: {
        selfPeerId: peerB.selfPeerId,
        snapshot: summarize(snapshotB),
        postAudioSnapshot: summarize(postAudioSnapshotB),
      },
    },
    media: {
      publishA,
      publishB,
      listenAtoB,
      listenBtoA,
    },
  };

  const failures = [];
  if (!hasParticipant(snapshotA, peerA.selfPeerId, true)) {
    failures.push("peer A snapshot does not contain itself");
  }
  if (!hasParticipant(snapshotB, peerB.selfPeerId, true)) {
    failures.push("peer B snapshot does not contain itself");
  }
  if (!hasParticipant(snapshotA, peerB.selfPeerId, false)) {
    failures.push("peer A snapshot does not contain peer B");
  }
  if (!hasParticipant(snapshotB, peerA.selfPeerId, false)) {
    failures.push("peer B snapshot does not contain peer A");
  }
  if (!hasDisplayName(snapshotA, peerB.selfPeerId, "Park Smoke B")) {
    failures.push("peer A does not see peer B's display name from /auki/info/0.0.1");
  }
  if (!hasDisplayName(snapshotB, peerA.selfPeerId, "Park Smoke A")) {
    failures.push("peer B does not see peer A's display name from /auki/info/0.0.1");
  }
  if (!hasAudioSensor(snapshotA, peerB.selfPeerId)) {
    failures.push("peer A does not see peer B's audio sensor from /auki/sensors/0.0.1");
  }
  if (!hasAudioSensor(snapshotB, peerA.selfPeerId)) {
    failures.push("peer B does not see peer A's audio sensor from /auki/sensors/0.0.1");
  }
  for (const [name, result] of Object.entries({ publishA, publishB, listenAtoB, listenBtoA })) {
    if (!result?.ok) failures.push(`${name} failed: ${result?.error?.code ?? "unknown"}`);
  }
  if (!hasConnectedAudioHealth(postAudioSnapshotA, peerA.selfPeerId, peerB.selfPeerId)) {
    failures.push("peer A did not report connected playback health after listening to peer B");
  }
  if (!hasConnectedAudioHealth(postAudioSnapshotB, peerB.selfPeerId, peerA.selfPeerId)) {
    failures.push("peer B did not report connected playback health after listening to peer A");
  }

  if (failures.length > 0) {
    throw new Error(
      `Park two-browser acceptance failed: ${failures.join("; ")}\n${JSON.stringify(report, null, 2)}`,
    );
  }

  console.log(
    `ok Park two-browser acceptance passed for ${domain}: ${peerA.selfPeerId} <-> ${peerB.selfPeerId}`,
  );
} finally {
  await browser.close();
  discovery.close();
}

async function createParkPeer(browser, url, displayName) {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(url);
  await page.waitForFunction(() => window.aukiBrowserPeer?.createPeer);
  const selfPeerId = await page.evaluate(async (displayName) => {
    const peer = await window.aukiBrowserPeer.createPeer();
    const snapshots = [];
    peer.observeParticipants((snapshot) => {
      snapshots.push(JSON.parse(JSON.stringify(snapshot)));
    });
    await peer.setParticipantMetadata({ appId: "park", displayName });
    await peer.declareLocalSensors([
      {
        id: "audio",
        kind: "audio",
        label: "Microphone",
        publishable: true,
        subscribable: false,
      },
    ]);
    window.__parkAcceptance = { peer, snapshots };
    return peer.getSelfPeerId();
  }, displayName);
  return { context, page, selfPeerId };
}

async function joinDomain(target, discoveryUrl, domainName) {
  return target.page.evaluate(
    async ({ discoveryUrl, domainName }) => {
      const { peer, snapshots } = window.__parkAcceptance;
      const joinResult = await peer.joinDomain(discoveryUrl, domainName);
      return { joinResult, snapshot: snapshots.at(-1) ?? null };
    },
    { discoveryUrl, domainName },
  );
}

async function latestSnapshot(target) {
  return target.page.evaluate(() => window.__parkAcceptance.snapshots.at(-1) ?? null);
}

async function publishAudio(target) {
  return target.page.evaluate(() => window.__parkAcceptance.peer.setSensorPublication("audio", true));
}

async function listenTo(target, peerId) {
  return target.page.evaluate(
    (peerId) => window.__parkAcceptance.peer.subscribeToSensor(peerId, "audio"),
    peerId,
  );
}

function summarize(snapshot) {
  return {
    domainName: snapshot?.domainName ?? null,
    managerPeerId: snapshot?.managerPeerId ?? null,
    participants:
      snapshot?.participants?.map((participant) => ({
        peerId: participant.peerId,
        appId: participant.appId,
        displayName: participant.displayName,
        isSelf: participant.isSelf,
        sensors: participant.sensors?.map((sensor) => ({
          id: sensor.id,
          kind: sensor.kind,
          publishable: sensor.publishable,
          subscribable: sensor.subscribable,
        })),
      })) ?? [],
  };
}

function hasParticipant(snapshot, peerId, isSelf) {
  return Boolean(
    snapshot?.participants?.some(
      (participant) => participant.peerId === peerId && participant.isSelf === isSelf,
    ),
  );
}

function hasDisplayName(snapshot, peerId, displayName) {
  const participant = snapshot?.participants?.find((entry) => entry.peerId === peerId);
  return participant?.displayName === displayName;
}

function hasAudioSensor(snapshot, peerId) {
  const participant = snapshot?.participants?.find((entry) => entry.peerId === peerId);
  return Boolean(participant?.sensors?.some((sensor) => sensor.kind === "audio"));
}

function hasConnectedAudioHealth(snapshot, selfPeerId, remotePeerId) {
  const participant = snapshot?.participants?.find((entry) => entry.peerId === selfPeerId);
  const media = participant?.mediaPresence;
  return Boolean(
    media?.listeningToPeerId === remotePeerId &&
      media?.listeningToSensorId === "audio" &&
      media?.playbackHealthy === true &&
      media?.selectedRemoteStreamState === "connected" &&
      typeof media?.lastFrameUnixMs === "number" &&
      Date.now() - media.lastFrameUnixMs < 10_000 &&
      typeof media?.outputLevel === "number" &&
      media.outputLevel > 0,
  );
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function chromeExecutable() {
  const candidates = [
    process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ].filter(Boolean);

  for (const candidate of candidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      // Try the next common install path.
    }
  }

  return undefined;
}
