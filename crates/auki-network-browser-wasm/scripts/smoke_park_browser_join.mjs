import { access } from "node:fs/promises";
import http from "node:http";
import { chromium } from "playwright-core";

const address = process.argv[2];
const parkUrl = process.argv[3] ?? "http://127.0.0.1:7881";

if (!address) {
  throw new Error(
    "usage: node scripts/smoke_park_browser_join.mjs <manager-webrtc-direct-multiaddr> [park-url]",
  );
}

const managerPeerId = address.match(/\/p2p\/([^/]+)$/)?.[1];
if (!managerPeerId) {
  throw new Error("manager multiaddr must end with /p2p/<peer-id>");
}

const domain = "browser-join-smoke";

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
  const page = await browser.newPage();
  await page.goto(parkUrl);
  await page.waitForFunction(() => window.aukiBrowserPeer?.createPeer);
  const result = await page.evaluate(
    async ({ discoveryUrl, domain }) => {
      const peer = await window.aukiBrowserPeer.createPeer();
      const snapshots = [];
      peer.observeParticipants((snapshot) => snapshots.push(snapshot));
      await peer.setParticipantMetadata({ appId: "park", displayName: "Park Smoke" });
      await peer.declareLocalSensors([
        {
          id: "audio",
          kind: "audio",
          label: "Microphone",
          publishable: true,
          subscribable: false,
        },
      ]);
      const joinResult = await peer.joinDomain(discoveryUrl, domain);
      return { joinResult, snapshot: snapshots.at(-1) };
    },
    { discoveryUrl, domain },
  );

  if (!result.joinResult.ok) {
    throw new Error(
      result.joinResult.error?.message ?? result.joinResult.error ?? "Park browser join failed",
    );
  }
  if (result.snapshot?.domainName !== domain) {
    throw new Error(`bad snapshot domain: ${result.snapshot?.domainName}`);
  }
  if (result.snapshot?.managerPeerId !== managerPeerId) {
    throw new Error(`bad snapshot manager: ${result.snapshot?.managerPeerId}`);
  }
  if (!result.snapshot?.participants?.some((participant) => participant.isSelf)) {
    throw new Error("snapshot missing self participant");
  }

  console.log(`ok Park browser peer joined and emitted snapshot for ${domain} via ${managerPeerId}`);
} finally {
  await browser.close();
  discovery.close();
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
