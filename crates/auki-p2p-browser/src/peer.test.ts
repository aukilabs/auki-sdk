import { readFile } from "node:fs/promises";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { peerIdFromSeed } from "./identity.js";
import {
  createAukiBrowserPeer,
  type AukiBrowserPeerTraceEvent,
  type SpatialMessage,
} from "./peer.js";
import { PREVIEW_PAYLOAD_TYPE, publishPreviewOffer } from "./preview.js";
import { createSubscribeEndForPath, LatestPublishedByteSource } from "./publication.js";
import { JsonFrameReader, writeJsonFrame } from "./stream.js";
import {
  createGetRequest,
  createOfferCatalogRequest,
  createPeerBinding,
  createPeerHandshake,
  createSubscribeRequest,
  parseGetResponse,
  parseOfferCatalogResponse,
  parseSubscribeEnd,
  parseSubscribeStartResult,
  validateGetResponseForRequest,
  validatePeerHandshakeAuthority,
  validateSubscribeDataMessage,
  validateSubscribeEndForOffer,
  type JsonObject,
} from "./protocol.js";
import type {
  BrowserConnectionPath,
  BrowserProtocolStream,
  BrowserTransport,
} from "./transport.js";

describe("AukiBrowserPeer shell", () => {
  it("connects bootstrap records through an injected transport", async () => {
    const transport = new MemoryTransport("browser-peer", ["/p2p-circuit/p2p/browser-peer"]);
    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: [
        bootstrapRecord("native-peer", "/memory/native-direct"),
        bootstrapRecord("relay-peer", "/memory/relay"),
      ],
      protocolWasm: await protocolWasmInput(),
    });

    expect(peer.peerId).toBe("browser-peer");
    expect(peer.supportedTransports).toContain("webrtc_direct");
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: false,
        dialAddresses: ["/memory/native-direct"],
        observedAddresses: [],
        connectionPaths: [],
      },
      {
        peerId: "relay-peer",
        connected: false,
        dialAddresses: ["/memory/relay"],
        observedAddresses: [],
        connectionPaths: [],
      },
    ]);

    await peer.connectBootstrap([
      bootstrapRecord("native-peer", "/memory/native-direct"),
      bootstrapRecord("relay-peer", "/memory/relay"),
    ]);

    expect(transport.started).toBe(1);
    expect(transport.dials).toEqual([["/memory/native-direct"], ["/memory/relay"]]);
    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: true,
        dialAddresses: ["/memory/native-direct"],
        observedAddresses: [],
        connectionPaths: [],
      },
      {
        peerId: "relay-peer",
        connected: true,
        dialAddresses: ["/memory/relay"],
        observedAddresses: [],
        connectionPaths: [],
      },
    ]);
  });

  it("stops the underlying transport", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await peer.dial("/memory/native");
    await peer.stop();

    expect(transport.started).toBe(1);
    expect(transport.stopped).toBe(1);
  });

  it("derives observed addresses from active connection paths", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/bootstrap"),
      protocolWasm: await protocolWasmInput(),
    });
    transport.setConnectionPaths("native-peer", [
      memoryConnectionPath("/memory/observed"),
      memoryConnectionPath("/memory/observed"),
    ]);

    expect(peer.listPeers()).toEqual([
      {
        peerId: "native-peer",
        connected: false,
        dialAddresses: ["/memory/bootstrap"],
        observedAddresses: ["/memory/observed"],
        connectionPaths: [
          memoryConnectionPath("/memory/observed"),
          memoryConnectionPath("/memory/observed"),
        ],
      },
    ]);
  });

  it("exports a local browser bootstrap record once dialable", async () => {
    const transport = new MemoryTransport("browser-peer", [
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer",
      "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
    ]);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await expect(peer.localBootstrapRecord()).resolves.toEqual({
      peerId: "browser-peer",
      agentVersion: "auki-p2p-browser/0.0.0",
      directAddresses: [],
      webrtcDirectAddresses: [],
      relayAddresses: [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ],
      relayServerAddresses: [],
      bootstrapAddresses: [
        "/ip4/127.0.0.1/tcp/2/ws/p2p/relay-peer/p2p-circuit/p2p/browser-peer",
      ],
    });
    expect(transport.started).toBe(1);
  });

  it("exports local relay addresses after adding a relay bootstrap peer", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });
    await peer.connectBootstrap({
      peer_id: "relay-peer",
      direct_addresses: ["/memory/relay"],
      webrtc_direct_addresses: [],
      relay_addresses: [],
      relay_server_addresses: ["/memory/relay/p2p/relay-peer"],
      bootstrap_addresses: ["/memory/relay"],
    });

    await expect(peer.localBootstrapRecord()).resolves.toMatchObject({
      peerId: "browser-peer",
      relayAddresses: ["/memory/relay/p2p/relay-peer/p2p-circuit/p2p/browser-peer"],
      bootstrapAddresses: ["/memory/relay/p2p/relay-peer/p2p-circuit/p2p/browser-peer"],
    });
  });

  it("rejects local browser bootstrap export before the peer is dialable", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await expect(peer.localBootstrapRecord()).rejects.toThrow("not dialable yet");
  });

  it("switches a connected peer to a selected dial address", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    const record = {
      peer_id: "native-peer",
      direct_addresses: ["/memory/websocket"],
      webrtc_direct_addresses: ["/memory/webrtc-direct"],
      relay_addresses: [],
      relay_server_addresses: [],
      bootstrap_addresses: ["/memory/websocket", "/memory/webrtc-direct"],
    };

    await peer.connectBootstrap(record);
    await peer.switchPeerAddress("native-peer", "/memory/websocket");

    expect(transport.forcedDials).toEqual([["/memory/websocket"]]);
    expect(transport.closedPeers).toEqual([
      { peerId: "native-peer", keepAddresses: ["/memory/websocket"] },
    ]);
    expect(peer.listPeers()[0]?.dialAddresses[0]).toBe("/memory/websocket");
  });

  it("force dials selected peer address even when it is already reported active", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.connectBootstrap(bootstrapRecord("native-peer", "/memory/websocket"));
    transport.setConnectionPaths("native-peer", [
      memoryConnectionPath("/memory/websocket"),
    ]);

    await peer.switchPeerAddress("native-peer", "/memory/websocket");

    expect(transport.forcedDials).toEqual([["/memory/websocket"]]);
    expect(transport.closedPeers).toEqual([
      { peerId: "native-peer", keepAddresses: ["/memory/websocket"] },
    ]);
  });

  it("retries selected peer address after dropping existing paths", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.connectBootstrap(switchableBootstrapRecord());
    transport.failNextDial(new Error("first dial failed"));

    await peer.switchPeerAddress("native-peer", "/memory/webrtc-direct");

    expect(transport.forcedDials).toEqual([
      ["/memory/webrtc-direct"],
      ["/memory/webrtc-direct"],
    ]);
    expect(transport.closedPeers).toEqual([
      { peerId: "native-peer", keepAddresses: [] },
      { peerId: "native-peer", keepAddresses: ["/memory/webrtc-direct"] },
    ]);
  });

  it("accepts a selected peer address when dial rejects after the path became active", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.connectBootstrap(switchableBootstrapRecord());
    transport.failNextDial(new Error("All promises were rejected"));
    transport.setConnectionPaths("native-peer", [
      memoryConnectionPath("/memory/webrtc-direct"),
    ]);

    await peer.switchPeerAddress("native-peer", "/memory/webrtc-direct");

    expect(transport.forcedDials).toEqual([["/memory/webrtc-direct"]]);
    expect(transport.closedPeers).toEqual([
      { peerId: "native-peer", keepAddresses: ["/memory/webrtc-direct"] },
    ]);
    expect(peer.listPeers()[0]?.dialAddresses[0]).toBe("/memory/webrtc-direct");
  });

  it("reconnects previous addresses when selected peer address cannot be dialed", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.connectBootstrap(switchableBootstrapRecord());
    transport.failNextDial(new Error("first dial failed"));
    transport.failNextDial(new Error("second dial failed"));

    await expect(peer.switchPeerAddress("native-peer", "/memory/webrtc-direct")).rejects.toThrow(
      "Switch to selected address failed",
    );

    expect(transport.forcedDials).toEqual([
      ["/memory/webrtc-direct"],
      ["/memory/webrtc-direct"],
      ["/memory/websocket"],
    ]);
    expect(transport.closedPeers).toEqual([
      { peerId: "native-peer", keepAddresses: [] },
    ]);
  });

  it("rejects switching to an unknown peer address", async () => {
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.connectBootstrap(bootstrapRecord("native-peer", "/memory/websocket"));

    await expect(peer.switchPeerAddress("native-peer", "/memory/other")).rejects.toThrow(
      "Address is not known",
    );
  });

  it("exchanges lifecycle handshakes when the browser has a seed", async () => {
    const localSeed = new Uint8Array(32).fill(7);
    const remoteSeed = new Uint8Array(32).fill(9);
    const localPeerId = await peerIdFromSeed(localSeed);
    const remotePeerId = await peerIdFromSeed(remoteSeed);
    const transport = new MemoryTransport(localPeerId, []);
    transport.handleProtocol(LIFECYCLE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const localHandshake = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(localHandshake.value.type).toBe("auki.peer_handshake.v1");

      const remoteBinding = await createPeerBinding(
        remoteSeed,
        remotePeerId,
        new Date(Date.now() - 1_000).toISOString(),
        "native-peer",
      );
      await writeJsonFrame(
        stream,
        await createPeerHandshake(remoteBinding),
        DEFAULT_FRAME_BODY_LIMIT,
      );
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      protocolWasm: await protocolWasmInput(),
    });

    await peer.connectBootstrap(bootstrapRecord(remotePeerId, "/memory/native-direct"));

    expect(transport.protocolDials).toEqual([
      {
        peerId: remotePeerId,
        addresses: ["/memory/native-direct"],
        protocol: LIFECYCLE_PROTOCOL_ID,
      },
    ]);
  });

  it("declares browser local domains during outbound lifecycle", async () => {
    const localSeed = new Uint8Array(32).fill(7);
    const remoteSeed = new Uint8Array(32).fill(9);
    const localPeerId = await peerIdFromSeed(localSeed);
    const remotePeerId = await peerIdFromSeed(remoteSeed);
    const transport = new MemoryTransport(localPeerId, []);
    let declaredDomainId: string | undefined;
    transport.handleProtocol(LIFECYCLE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const localHandshake = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(localHandshake.value).toMatchObject({
        type: "auki.peer_handshake.v1",
        declared_domains: [
          expect.objectContaining({
            domain_id: declaredDomainId,
            metadata: { role: "producer" },
          }),
        ],
      });
      await expect(
        validatePeerHandshakeAuthority(
          localHandshake.value,
          localPeerId,
          true,
          new Date(Date.now() + 1_000).toISOString(),
        ),
      ).resolves.toMatchObject({
        accepted_served_domains: [
          {
            domain_id: declaredDomainId,
            authority: "direct_owner",
          },
        ],
        rejected_declared_domains: [],
      });

      const remoteBinding = await createPeerBinding(
        remoteSeed,
        remotePeerId,
        new Date(Date.now() - 1_000).toISOString(),
        "native-peer",
      );
      await writeJsonFrame(
        stream,
        await createPeerHandshake(remoteBinding),
        DEFAULT_FRAME_BODY_LIMIT,
      );
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      label: "browser-demo",
      protocolWasm: await protocolWasmInput(),
    });
    const localDomain = await peer.createLocalDomain({
      nonce: new Uint8Array(16).fill(5),
      label: "browser-demo-domain",
      metadata: { role: "producer" },
    });
    declaredDomainId = localDomain.domainId;

    await peer.connectBootstrap(bootstrapRecord(remotePeerId, "/memory/native-direct"));

    expect(transport.protocolDials).toEqual([
      {
        peerId: remotePeerId,
        addresses: ["/memory/native-direct"],
        protocol: LIFECYCLE_PROTOCOL_ID,
      },
    ]);
  });

  it("serves inbound lifecycle handshakes with declared browser local domains", async () => {
    const localSeed = new Uint8Array(32).fill(8);
    const remoteSeed = new Uint8Array(32).fill(10);
    const localPeerId = await peerIdFromSeed(localSeed);
    const remotePeerId = await peerIdFromSeed(remoteSeed);
    const transport = new MemoryTransport(localPeerId, []);
    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      label: "browser-demo",
      protocolWasm: await protocolWasmInput(),
    });
    const localDomain = await peer.createLocalDomain({
      nonce: new Uint8Array(16).fill(6),
      label: "browser-demo-domain",
    });

    const stream = await transport.openInbound(remotePeerId, LIFECYCLE_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    const localHandshake = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
    expect(localHandshake.value).toMatchObject({
      type: "auki.peer_handshake.v1",
      declared_domains: [
        expect.objectContaining({
          domain_id: localDomain.domainId,
        }),
      ],
    });
    await expect(
      validatePeerHandshakeAuthority(
        localHandshake.value,
        localPeerId,
        true,
        new Date(Date.now() + 1_000).toISOString(),
      ),
    ).resolves.toMatchObject({
      accepted_served_domains: [
        {
          domain_id: localDomain.domainId,
          authority: "direct_owner",
        },
      ],
      rejected_declared_domains: [],
    });

    const remoteBinding = await createPeerBinding(
      remoteSeed,
      remotePeerId,
      new Date(Date.now() - 1_000).toISOString(),
      "remote-browser",
    );
    await writeJsonFrame(
      stream,
      await createPeerHandshake(remoteBinding),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(peer.listPeers()).toEqual([
      {
        peerId: remotePeerId,
        connected: true,
        dialAddresses: [],
        observedAddresses: [],
        connectionPaths: [],
      },
    ]);
  });

  it("rejects inbound lifecycle when the signed peer binding does not match the authenticated peer id", async () => {
    const localSeed = new Uint8Array(32).fill(8);
    const remoteSeed = new Uint8Array(32).fill(10);
    const wrongRemoteSeed = new Uint8Array(32).fill(11);
    const localPeerId = await peerIdFromSeed(localSeed);
    const remotePeerId = await peerIdFromSeed(remoteSeed);
    const wrongRemotePeerId = await peerIdFromSeed(wrongRemoteSeed);
    const transport = new MemoryTransport(localPeerId, []);
    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      protocolWasm: await protocolWasmInput(),
    });
    await peer.createLocalDomain({
      nonce: new Uint8Array(16).fill(6),
      label: "browser-demo-domain",
    });

    const stream = await transport.openInbound(remotePeerId, LIFECYCLE_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    await reader.read(DEFAULT_FRAME_BODY_LIMIT);
    const mismatchedBinding = await createPeerBinding(
      wrongRemoteSeed,
      wrongRemotePeerId,
      new Date(Date.now() - 1_000).toISOString(),
      "wrong-remote-browser",
    );
    await writeJsonFrame(
      stream,
      await createPeerHandshake(mismatchedBinding),
      DEFAULT_FRAME_BODY_LIMIT,
    );

    await expect(reader.read(DEFAULT_FRAME_BODY_LIMIT)).rejects.toThrow();
    expect(peer.listPeers()).toEqual([]);
  });

  it("unregisters inbound lifecycle handlers when stopped", async () => {
    const localSeed = new Uint8Array(32).fill(8);
    const localPeerId = await peerIdFromSeed(localSeed);
    const transport = new MemoryTransport(localPeerId, []);
    const peer = await createAukiBrowserPeer({
      seed: localSeed,
      transport,
      protocolWasm: await protocolWasmInput(),
    });

    await peer.dial("/memory/native");
    await peer.stop();

    await expect(transport.openInbound("remote-peer", LIFECYCLE_PROTOCOL_ID)).rejects.toThrow(
      "No inbound handler registered",
    );
  });

  it("loads offer catalogs over an RFC protocol stream", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const catalog = fixture.positive.response_with_offer.object as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual({
        type: "auki.offer_catalog_request.v1",
      });
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    await expect(peer.listOffers("native-peer")).resolves.toEqual([
      {
        peerId: "native-peer",
        domainId: fixture.inputs.domain_id,
        offerId: "camera-main",
        kind: "sensor.frame",
        payloadType: "auki.frame",
        accessModes: ["get", "subscribe"],
      },
    ]);
    expect(transport.protocolDials).toEqual([
      {
        peerId: "native-peer",
        addresses: ["/memory/native-direct"],
        protocol: OFFER_CATALOG_PROTOCOL_ID,
      },
    ]);
  });

  it("keeps cached offers when switching transport addresses", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const catalog = fixture.positive.response_with_offer.object as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: switchableBootstrapRecord(),
      protocolWasm: await protocolWasmInput(),
    });

    await peer.listOffers("native-peer");
    await peer.switchPeerAddress("native-peer", "/memory/webrtc-direct");
    await peer.listOffers("native-peer");

    expect(transport.protocolDials.filter((dial) => dial.protocol === OFFER_CATALOG_PROTOCOL_ID))
      .toHaveLength(1);
  });

  it("subscribes to spatial messages over an RFC protocol stream", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const data = subscribeFixture.positive.data_message.object as JsonObject;
    const end = subscribeFixture.positive.end_message.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual(subscribeFixture.positive.request.object);
      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, data, DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, end, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    const messages: SpatialMessage[] = [];
    for await (const message of peer.subscribe({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      params: { frame: "latest", stream: "live" },
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxMessageBytes: inputs.max_message_bytes as number,
    })) {
      messages.push(message);
    }

    expect(messages).toEqual([data]);
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      SUBSCRIBE_PROTOCOL_ID,
    ]);
  });

  it("retries a reset Subscribe start stream before returning a subscription", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    const traces: AukiBrowserPeerTraceEvent[] = [];
    let subscribeAttempts = 0;
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      subscribeAttempts += 1;
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual(subscribeFixture.positive.request.object);
      if (subscribeAttempts === 1) {
        if (!stream.abort) {
          throw new Error("test stream missing abort");
        }
        stream.abort(new Error("The stream has been reset"));
        return;
      }

      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      await parseSubscribeEnd((await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
      trace: (event) => traces.push(event),
    });

    const subscription = await peer.openSubscription({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      params: { frame: "latest", stream: "live" },
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxMessageBytes: inputs.max_message_bytes as number,
    });
    await subscription.stop();

    expect(subscribeAttempts).toBe(2);
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      SUBSCRIBE_PROTOCOL_ID,
      SUBSCRIBE_PROTOCOL_ID,
    ]);
    expect(traces.map((event) => `${event.operation}:${event.phase}:${event.attempt}`)).toEqual([
      "subscribe:dialing:1",
      "subscribe:opened:1",
      "subscribe:request_sent:1",
      "subscribe:stream_closed:1",
      "subscribe:retrying:1",
      "subscribe:dialing:2",
      "subscribe:opened:2",
      "subscribe:request_sent:2",
      "subscribe:start_received:2",
      "subscribe:accepted:2",
    ]);
    expect(traces[4]).toMatchObject({
      error: "The stream has been reset",
      retryable: true,
      nextAttempt: 2,
    });
  });

  it("aborts an active Subscribe stream when the caller aborts the signal", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    let requestSeen!: () => void;
    const requestReceived = new Promise<void>((resolve) => {
      requestSeen = resolve;
    });
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      requestSeen();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });
    const abort = new AbortController();
    const iterator = peer
      .subscribe({
        peerId: "native-peer",
        domainId: inputs.domain_id as string,
        offerId: inputs.offer_id as string,
        acceptedPayloadTypes: [inputs.selected_payload_type as string],
        maxMessageBytes: inputs.max_message_bytes as number,
        signal: abort.signal,
      })
      [Symbol.asyncIterator]();

    const next = iterator.next();
    await requestReceived;
    abort.abort(new Error("Stopped by user"));

    await expect(next).resolves.toEqual({ done: true, value: undefined });
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      SUBSCRIBE_PROTOCOL_ID,
    ]);
  });

  it("stops a Subscribe handle by sending SubscribeEnd cancelled", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    let endSeen!: (end: JsonObject) => void;
    const endReceived = new Promise<JsonObject>((resolve) => {
      endSeen = resolve;
    });
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      const end = await parseSubscribeEnd((await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value);
      endSeen(end);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    const subscription = await peer.openSubscription({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxMessageBytes: inputs.max_message_bytes as number,
    });
    await subscription.stop();

    await expect(endReceived).resolves.toMatchObject({
      domain_id: inputs.domain_id,
      offer_id: inputs.offer_id,
      reason: "cancelled",
    });
  });

  it("gets one spatial message over an RFC protocol stream", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const getFixture = await fixtureJson("v1_get.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const response = getFixture.positive.success_response.object as JsonObject;
    const inputs = getFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(GET_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual(getFixture.positive.request.object);
      await writeJsonFrame(stream, response, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    await expect(
      peer.get({
        peerId: "native-peer",
        domainId: inputs.domain_id as string,
        offerId: inputs.offer_id as string,
        params: { frame: "latest" },
        acceptedPayloadTypes: [inputs.selected_payload_type as string],
        maxPayloadBytes: inputs.max_payload_bytes as number,
      }),
    ).resolves.toEqual(response.message);
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      GET_PROTOCOL_ID,
    ]);
  });

  it("returns Get response before slow stream close finishes", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const getFixture = await fixtureJson("v1_get.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const response = getFixture.positive.success_response.object as JsonObject;
    const inputs = getFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    let releaseGetClose: (() => void) | undefined;
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(GET_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, response, DEFAULT_FRAME_BODY_LIMIT);
      if (stream instanceof QueueStream) {
        stream.delayCloseUntil(
          new Promise<void>((resolve) => {
            releaseGetClose = resolve;
          }),
        );
      }
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    const result = await peer.get({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      params: { frame: "latest" },
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxPayloadBytes: inputs.max_payload_bytes as number,
    });

    expect(result).toEqual(response.message);
    releaseGetClose?.();
  });

  it("retries a reset Get stream before returning a spatial message", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const getFixture = await fixtureJson("v1_get.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const response = getFixture.positive.success_response.object as JsonObject;
    const inputs = getFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    const traces: AukiBrowserPeerTraceEvent[] = [];
    let getAttempts = 0;
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(GET_PROTOCOL_ID, async (stream) => {
      getAttempts += 1;
      const reader = new JsonFrameReader(stream);
      const request = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(request.value).toEqual(getFixture.positive.request.object);
      if (getAttempts === 1) {
        if (!stream.abort) {
          throw new Error("test stream missing abort");
        }
        stream.abort(new Error("The stream has been reset"));
        return;
      }

      await writeJsonFrame(stream, response, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
      trace: (event) => traces.push(event),
    });

    await expect(
      peer.get({
        peerId: "native-peer",
        domainId: inputs.domain_id as string,
        offerId: inputs.offer_id as string,
        params: { frame: "latest" },
        acceptedPayloadTypes: [inputs.selected_payload_type as string],
        maxPayloadBytes: inputs.max_payload_bytes as number,
      }),
    ).resolves.toEqual(response.message);
    expect(getAttempts).toBe(2);
    expect(transport.protocolDials.map((dial) => dial.protocol)).toEqual([
      OFFER_CATALOG_PROTOCOL_ID,
      GET_PROTOCOL_ID,
      GET_PROTOCOL_ID,
    ]);
    const phases = traces.map((event) => `${event.operation}:${event.phase}:${event.attempt}`);
    expect(phases.slice(0, 3)).toEqual([
      "get:dialing:1",
      "get:opened:1",
      "get:request_sent:1",
    ]);
    expect(phases).toEqual(
      expect.arrayContaining([
        "get:stream_closed:1",
        "get:retrying:1",
        "get:dialing:2",
        "get:opened:2",
        "get:request_sent:2",
        "get:response_received:2",
        "get:completed:2",
        "get:stream_closed:2",
      ]),
    );
    expect(phases.indexOf("get:retrying:1")).toBeLessThan(phases.indexOf("get:dialing:2"));
    expect(traces.find((event) => event.phase === "retrying")).toMatchObject({
      error: "The stream has been reset",
      retryable: true,
      nextAttempt: 2,
    });
  });

  it("keeps maxMessageBytes scoped to Subscribe data messages", async () => {
    const offerFixture = await fixtureJson("v1_offer_catalogs.json");
    const subscribeFixture = await fixtureJson("v1_subscribe.json");
    const catalog = offerFixture.positive.response_with_offer.object as JsonObject;
    const accept = subscribeFixture.positive.accept_start_result.object as JsonObject;
    const end = subscribeFixture.positive.end_message.object as JsonObject;
    const inputs = subscribeFixture.inputs as JsonObject;
    const transport = new MemoryTransport("browser-peer", []);
    transport.handleProtocol(OFFER_CATALOG_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, catalog, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });
    transport.handleProtocol(SUBSCRIBE_PROTOCOL_ID, async (stream) => {
      const reader = new JsonFrameReader(stream);
      await reader.read(DEFAULT_FRAME_BODY_LIMIT);
      expect(JSON.stringify(accept).length).toBeGreaterThan(32);
      expect(JSON.stringify(end).length).toBeGreaterThan(32);
      await writeJsonFrame(stream, accept, DEFAULT_FRAME_BODY_LIMIT);
      await writeJsonFrame(stream, end, DEFAULT_FRAME_BODY_LIMIT);
      await stream.close();
    });

    const peer = await createAukiBrowserPeer({
      transport,
      bootstrap: bootstrapRecord("native-peer", "/memory/native-direct"),
      protocolWasm: await protocolWasmInput(),
    });

    const messages: SpatialMessage[] = [];
    for await (const message of peer.subscribe({
      peerId: "native-peer",
      domainId: inputs.domain_id as string,
      offerId: inputs.offer_id as string,
      acceptedPayloadTypes: [inputs.selected_payload_type as string],
      maxMessageBytes: 32,
    })) {
      messages.push(message);
    }

    expect(messages).toEqual([]);
  });

  it("publishes local offers through inbound offer catalog streams", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    const handle = await peer.publishOffer({
      source: [],
      domainId: fixture.inputs.domain_id as string,
      offerId: "browser-bytes",
      kind: "example.bytes",
      payload: {
        type: "example.bytes.v1",
        encoding: "binary",
        media_type: "application/octet-stream",
        schema_version: "1",
      },
      displayName: "Browser Bytes",
    });

    await expect(peer.listOffers("browser-peer")).resolves.toEqual([
      {
        peerId: "browser-peer",
        domainId: fixture.inputs.domain_id,
        offerId: "browser-bytes",
        kind: "example.bytes",
        payloadType: "example.bytes.v1",
        accessModes: ["subscribe"],
      },
    ]);

    const stream = await transport.openInbound("remote-peer", OFFER_CATALOG_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    await writeJsonFrame(
      stream,
      await createOfferCatalogRequest([fixture.inputs.domain_id as string]),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    const response = await parseOfferCatalogResponse(
      (await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
    );

    expect(response.offers).toEqual([
      expect.objectContaining({
        domain_id: fixture.inputs.domain_id,
        offer_id: "browser-bytes",
        display_name: "Browser Bytes",
        kind: "example.bytes",
        status: "available",
        access_modes: ["subscribe"],
        payload: {
          type: "example.bytes.v1",
          encoding: "binary",
          media_type: "application/octet-stream",
          schema_version: "1",
        },
        registry_refs: [],
      }),
    ]);

    await handle.stop();
    const emptyStream = await transport.openInbound("remote-peer", OFFER_CATALOG_PROTOCOL_ID);
    const emptyReader = new JsonFrameReader(emptyStream);
    await writeJsonFrame(
      emptyStream,
      await createOfferCatalogRequest([fixture.inputs.domain_id as string]),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    await expect(emptyReader.read(DEFAULT_FRAME_BODY_LIMIT)).resolves.toMatchObject({
      value: {
        type: "auki.offer_catalog_response.v1",
        offers: [],
      },
    });
  });

  it("serves published offer bytes through inbound Subscribe streams", async () => {
    const fixture = await fixtureJson("v1_subscribe.json");
    const inputs = fixture.inputs as JsonObject;
    const domainId = inputs.domain_id as string;
    const offerId = "browser-bytes";
    const payloadType = "example.bytes.v1";
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });
    await peer.publishOffer({
      source: () => [new Uint8Array([1, 2, 3])],
      domainId,
      offerId,
      kind: "example.bytes",
      payload: {
        type: payloadType,
        encoding: "binary",
        media_type: "application/octet-stream",
        schema_version: "1",
      },
    });

    const stream = await transport.openInbound("remote-peer", SUBSCRIBE_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    const request = await createSubscribeRequest(
      domainId,
      offerId,
      undefined,
      [payloadType],
      4096,
    );
    await writeJsonFrame(stream, request, DEFAULT_FRAME_BODY_LIMIT);

    const accept = await parseSubscribeStartResult(
      (await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
    );
    expect(accept).toEqual(
      expect.objectContaining({
        type: "auki.subscribe_accept.v1",
        domain_id: domainId,
        offer_id: offerId,
        payload: {
          type: payloadType,
          encoding: "binary",
          media_type: "application/octet-stream",
          schema_version: "1",
        },
      }),
    );

    const dataFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
    const data = await validateSubscribeDataMessage(
      accept,
      dataFrame.value,
      dataFrame.bodyLength,
      4096,
    );
    expect(data).toEqual(
      expect.objectContaining({
        type: "auki.spatial_message.v1",
        domain_id: domainId,
        offer_id: offerId,
        sequence: "0",
        payload: {
          type: payloadType,
          encoding: "binary",
          media_type: "application/octet-stream",
          schema_version: "1",
          bytes: "AQID",
        },
      }),
    );

    const end = await validateSubscribeEndForOffer(
      (await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
      domainId,
      offerId,
    );
    expect(end.reason).toBe("complete");
  });

  it("stops serving published Subscribe streams when the consumer cancels", async () => {
    const fixture = await fixtureJson("v1_subscribe.json");
    const inputs = fixture.inputs as JsonObject;
    const domainId = inputs.domain_id as string;
    const offerId = "browser-bytes";
    const payloadType = "example.bytes.v1";
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });
    let releaseSecondChunk!: () => void;
    const secondChunk = new Promise<void>((resolve) => {
      releaseSecondChunk = resolve;
    });
    await peer.publishOffer({
      source: async function* () {
        yield new Uint8Array([1, 2, 3]);
        await secondChunk;
        yield new Uint8Array([4, 5, 6]);
      },
      domainId,
      offerId,
      kind: "example.bytes",
      payload: {
        type: payloadType,
        encoding: "binary",
        media_type: "application/octet-stream",
        schema_version: "1",
      },
    });

    const stream = await transport.openInbound("remote-peer", SUBSCRIBE_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    await writeJsonFrame(
      stream,
      await createSubscribeRequest(domainId, offerId, undefined, [payloadType], 4096),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    const accept = await parseSubscribeStartResult(
      (await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
    );
    const firstDataFrame = await reader.read(DEFAULT_FRAME_BODY_LIMIT);
    await expect(
      validateSubscribeDataMessage(accept, firstDataFrame.value, firstDataFrame.bodyLength, 4096),
    ).resolves.toMatchObject({
      sequence: "0",
      payload: expect.objectContaining({ bytes: "AQID" }),
    });

    await writeJsonFrame(
      stream,
      await createSubscribeEndForPath(domainId, offerId, "cancelled"),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    releaseSecondChunk();

    await expect(reader.read(DEFAULT_FRAME_BODY_LIMIT)).rejects.toThrow(
      "protocol stream closed before a complete frame arrived",
    );
  });

  it("serves published offer bytes through inbound Get streams", async () => {
    const fixture = await fixtureJson("v1_get.json");
    const inputs = fixture.inputs as JsonObject;
    const domainId = inputs.domain_id as string;
    const offerId = "browser-bytes";
    const payloadType = inputs.selected_payload_type as string;
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });
    await peer.publishOffer({
      source: () => [new Uint8Array([1, 2, 3])],
      domainId,
      offerId,
      kind: "example.bytes",
      payload: {
        type: payloadType,
        encoding: "binary",
        media_type: "application/octet-stream",
        schema_version: "1",
      },
      accessModes: ["get", "subscribe"],
    });

    const stream = await transport.openInbound("remote-peer", GET_PROTOCOL_ID);
    const reader = new JsonFrameReader(stream);
    const request = await createGetRequest(domainId, offerId, undefined, [payloadType], 4096);
    await writeJsonFrame(stream, request, DEFAULT_FRAME_BODY_LIMIT);

    const response = await parseGetResponse((await reader.read(DEFAULT_FRAME_BODY_LIMIT)).value);
    const message = await validateGetResponseForRequest(request, response, payloadType);

    expect(message).toEqual(
      expect.objectContaining({
        type: "auki.spatial_message.v1",
        domain_id: domainId,
        offer_id: offerId,
        sequence: "0",
        payload: {
          type: payloadType,
          encoding: "binary",
          media_type: "application/octet-stream",
          schema_version: "1",
          bytes: "AQID",
        },
      }),
    );
  });

  it("serves Get and Subscribe from one latest published source", async () => {
    const fixture = await fixtureJson("v1_get.json");
    const inputs = fixture.inputs as JsonObject;
    const domainId = inputs.domain_id as string;
    const offerId = "browser-bytes";
    const payloadType = inputs.selected_payload_type as string;
    const source = new LatestPublishedByteSource();
    source.publish({
      bytes: new Uint8Array([7, 8, 9]),
      sequence: 42,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });
    await peer.publishOffer({
      source,
      domainId,
      offerId,
      kind: "example.bytes",
      payload: {
        type: payloadType,
        encoding: "binary",
        media_type: "application/octet-stream",
        schema_version: "1",
      },
      accessModes: ["get", "subscribe"],
    });

    const getStream = await transport.openInbound("remote-peer", GET_PROTOCOL_ID);
    const getReader = new JsonFrameReader(getStream);
    const getRequest = await createGetRequest(domainId, offerId, undefined, [payloadType], 4096);
    await writeJsonFrame(getStream, getRequest, DEFAULT_FRAME_BODY_LIMIT);
    const getResponse = await parseGetResponse(
      (await getReader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
    );
    const getMessage = await validateGetResponseForRequest(
      getRequest,
      getResponse,
      payloadType,
    );

    const subscribeStream = await transport.openInbound("remote-peer", SUBSCRIBE_PROTOCOL_ID);
    const subscribeReader = new JsonFrameReader(subscribeStream);
    await writeJsonFrame(
      subscribeStream,
      await createSubscribeRequest(domainId, offerId, undefined, [payloadType], 4096),
      DEFAULT_FRAME_BODY_LIMIT,
    );
    const accept = await parseSubscribeStartResult(
      (await subscribeReader.read(DEFAULT_FRAME_BODY_LIMIT)).value,
    );
    const subscribeDataFrame = await subscribeReader.read(DEFAULT_FRAME_BODY_LIMIT);
    const subscribeData = await validateSubscribeDataMessage(
      accept,
      subscribeDataFrame.value,
      subscribeDataFrame.bodyLength,
      4096,
    );

    expect(getMessage).toMatchObject({
      sequence: "42",
      generated_at: "2026-05-29T00:00:00Z",
      payload: expect.objectContaining({ bytes: "BwgJ" }),
    });
    expect(subscribeData).toMatchObject({
      sequence: "42",
      generated_at: "2026-05-29T00:00:00Z",
      payload: expect.objectContaining({ bytes: "BwgJ" }),
    });
    source.close();
  });

  it("loads, gets, subscribes, and stops a browser-published preview from another browser peer", async () => {
    const publisherSeed = new Uint8Array(32).fill(21);
    const subscriberSeed = new Uint8Array(32).fill(22);
    const publisherPeerId = await peerIdFromSeed(publisherSeed);
    const subscriberPeerId = await peerIdFromSeed(subscriberSeed);
    const publisherTransport = new MemoryTransport(publisherPeerId, [
      `/memory/browser-a/p2p/${publisherPeerId}`,
    ]);
    const subscriberTransport = new MemoryTransport(subscriberPeerId, []);
    subscriberTransport.connectPeer(publisherTransport);
    const publisher = await createAukiBrowserPeer({
      seed: publisherSeed,
      transport: publisherTransport,
      label: "browser-a",
      protocolWasm: await protocolWasmInput(),
    });
    const subscriber = await createAukiBrowserPeer({
      seed: subscriberSeed,
      transport: subscriberTransport,
      label: "browser-b",
      protocolWasm: await protocolWasmInput(),
    });
    const domain = await publisher.createLocalDomain({
      nonce: new Uint8Array(16).fill(3),
      label: "browser-a-domain",
    });
    const source = new LatestPublishedByteSource();
    source.publish({
      bytes: new Uint8Array([1, 2, 3]),
      sequence: 10,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    await publishPreviewOffer(publisher, source, {
      domainId: domain.domainId,
      offerId: "browser-preview",
    });

    await subscriber.connectBootstrap(await publisher.localBootstrapRecord());
    const offers = await subscriber.listOffers(publisherPeerId);

    expect(subscriber.listPeers()).toEqual([
      expect.objectContaining({
        peerId: publisherPeerId,
        connected: true,
      }),
    ]);
    expect(offers).toEqual([
      {
        peerId: publisherPeerId,
        domainId: domain.domainId,
        offerId: "browser-preview",
        kind: "auki.sensor.rgb_camera.preview",
        payloadType: PREVIEW_PAYLOAD_TYPE,
        accessModes: ["get", "subscribe"],
      },
    ]);

    const firstGet = await subscriber.get({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxPayloadBytes: 4096,
    });
    expect(firstGet).toMatchObject({
      sequence: "10",
      generated_at: "2026-05-29T00:00:00Z",
      payload: expect.objectContaining({ bytes: "AQID" }),
    });

    const subscription = await subscriber.openSubscription({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxMessageBytes: 4096,
    });
    const iterator = subscription.messages[Symbol.asyncIterator]();
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "10",
        payload: expect.objectContaining({ bytes: "AQID" }),
      }),
    });

    source.publish({
      bytes: new Uint8Array([4, 5, 6]),
      sequence: 11,
      generatedAt: "2026-05-29T00:00:01Z",
    });
    const secondGet = await subscriber.get({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxPayloadBytes: 4096,
    });
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "11",
        generated_at: "2026-05-29T00:00:01Z",
        payload: expect.objectContaining({ bytes: "BAUG" }),
      }),
    });
    expect(secondGet).toMatchObject({
      sequence: "11",
      generated_at: "2026-05-29T00:00:01Z",
      payload: expect.objectContaining({ bytes: "BAUG" }),
    });

    await subscription.stop();
    source.close();
  });

  it("fans out one browser-published source to two browser subscribers", async () => {
    const publisherSeed = new Uint8Array(32).fill(31);
    const subscriberOneSeed = new Uint8Array(32).fill(32);
    const subscriberTwoSeed = new Uint8Array(32).fill(33);
    const publisherPeerId = await peerIdFromSeed(publisherSeed);
    const subscriberOnePeerId = await peerIdFromSeed(subscriberOneSeed);
    const subscriberTwoPeerId = await peerIdFromSeed(subscriberTwoSeed);
    const publisherTransport = new MemoryTransport(publisherPeerId, [
      `/memory/browser-a/p2p/${publisherPeerId}`,
    ]);
    const subscriberOneTransport = new MemoryTransport(subscriberOnePeerId, []);
    const subscriberTwoTransport = new MemoryTransport(subscriberTwoPeerId, []);
    subscriberOneTransport.connectPeer(publisherTransport);
    subscriberTwoTransport.connectPeer(publisherTransport);
    const publisher = await createAukiBrowserPeer({
      seed: publisherSeed,
      transport: publisherTransport,
      label: "browser-a",
      protocolWasm: await protocolWasmInput(),
    });
    const subscriberOne = await createAukiBrowserPeer({
      seed: subscriberOneSeed,
      transport: subscriberOneTransport,
      label: "browser-b",
      protocolWasm: await protocolWasmInput(),
    });
    const subscriberTwo = await createAukiBrowserPeer({
      seed: subscriberTwoSeed,
      transport: subscriberTwoTransport,
      label: "browser-c",
      protocolWasm: await protocolWasmInput(),
    });
    const domain = await publisher.createLocalDomain({
      nonce: new Uint8Array(16).fill(4),
      label: "browser-a-domain",
    });
    const source = new LatestPublishedByteSource();
    source.publish({
      bytes: new Uint8Array([1, 2, 3]),
      sequence: 1,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    await publishPreviewOffer(publisher, source, {
      domainId: domain.domainId,
      offerId: "browser-preview",
    });

    const bootstrap = await publisher.localBootstrapRecord();
    await subscriberOne.connectBootstrap(bootstrap);
    await subscriberTwo.connectBootstrap(bootstrap);
    await subscriberOne.listOffers(publisherPeerId);
    await subscriberTwo.listOffers(publisherPeerId);

    const subscriptionOne = await subscriberOne.openSubscription({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxMessageBytes: 4096,
    });
    const subscriptionTwo = await subscriberTwo.openSubscription({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxMessageBytes: 4096,
    });
    const iteratorOne = subscriptionOne.messages[Symbol.asyncIterator]();
    const iteratorTwo = subscriptionTwo.messages[Symbol.asyncIterator]();

    await expect(iteratorOne.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "1",
        generated_at: "2026-05-29T00:00:00Z",
        payload: expect.objectContaining({ bytes: "AQID" }),
      }),
    });
    await expect(iteratorTwo.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "1",
        generated_at: "2026-05-29T00:00:00Z",
        payload: expect.objectContaining({ bytes: "AQID" }),
      }),
    });

    source.publish({
      bytes: new Uint8Array([4, 5, 6]),
      sequence: 2,
      generatedAt: "2026-05-29T00:00:01Z",
    });
    await expect(iteratorOne.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "2",
        generated_at: "2026-05-29T00:00:01Z",
        payload: expect.objectContaining({ bytes: "BAUG" }),
      }),
    });
    await expect(iteratorTwo.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "2",
        generated_at: "2026-05-29T00:00:01Z",
        payload: expect.objectContaining({ bytes: "BAUG" }),
      }),
    });

    await subscriptionOne.stop();
    await subscriptionTwo.stop();
    source.close();
  });

  it("ends active browser Subscribe streams when the shared source closes", async () => {
    const publisherSeed = new Uint8Array(32).fill(41);
    const subscriberSeed = new Uint8Array(32).fill(42);
    const publisherPeerId = await peerIdFromSeed(publisherSeed);
    const subscriberPeerId = await peerIdFromSeed(subscriberSeed);
    const publisherTransport = new MemoryTransport(publisherPeerId, [
      `/memory/browser-a/p2p/${publisherPeerId}`,
    ]);
    const subscriberTransport = new MemoryTransport(subscriberPeerId, []);
    subscriberTransport.connectPeer(publisherTransport);
    const publisher = await createAukiBrowserPeer({
      seed: publisherSeed,
      transport: publisherTransport,
      label: "browser-a",
      protocolWasm: await protocolWasmInput(),
    });
    const subscriber = await createAukiBrowserPeer({
      seed: subscriberSeed,
      transport: subscriberTransport,
      label: "browser-b",
      protocolWasm: await protocolWasmInput(),
    });
    const domain = await publisher.createLocalDomain({
      nonce: new Uint8Array(16).fill(5),
      label: "browser-a-domain",
    });
    const source = new LatestPublishedByteSource();
    source.publish({
      bytes: new Uint8Array([1, 2, 3]),
      sequence: 1,
      generatedAt: "2026-05-29T00:00:00Z",
    });
    await publishPreviewOffer(publisher, source, {
      domainId: domain.domainId,
      offerId: "browser-preview",
    });

    await subscriber.connectBootstrap(await publisher.localBootstrapRecord());
    await subscriber.listOffers(publisherPeerId);
    const subscription = await subscriber.openSubscription({
      peerId: publisherPeerId,
      domainId: domain.domainId,
      offerId: "browser-preview",
      acceptedPayloadTypes: [PREVIEW_PAYLOAD_TYPE],
      maxMessageBytes: 4096,
    });
    const iterator = subscription.messages[Symbol.asyncIterator]();
    await expect(iterator.next()).resolves.toEqual({
      done: false,
      value: expect.objectContaining({
        sequence: "1",
        payload: expect.objectContaining({ bytes: "AQID" }),
      }),
    });

    source.close();

    await expect(iterator.next()).resolves.toEqual({
      done: true,
      value: undefined,
    });
    await subscription.stop();
  });

  it("keeps preview publishing as a helper over generic offer publishing", async () => {
    const fixture = await fixtureJson("v1_offer_catalogs.json");
    const transport = new MemoryTransport("browser-peer", []);
    const peer = await createAukiBrowserPeer({ transport, protocolWasm: await protocolWasmInput() });

    await publishPreviewOffer(peer, [], {
      domainId: fixture.inputs.domain_id as string,
      offerId: "browser-preview",
    });

    await expect(peer.listOffers("browser-peer")).resolves.toEqual([
      {
        peerId: "browser-peer",
        domainId: fixture.inputs.domain_id,
        offerId: "browser-preview",
        kind: "auki.sensor.rgb_camera.preview",
        payloadType: "auki.camera.jpeg_frame.v1",
        accessModes: ["get", "subscribe"],
      },
    ]);
  });
});

const LIFECYCLE_PROTOCOL_ID = "/auki/cluster-lifecycle/0.0.1";
const OFFER_CATALOG_PROTOCOL_ID = "/auki/offer-catalog/0.0.1";
const GET_PROTOCOL_ID = "/auki/get/0.0.1";
const SUBSCRIBE_PROTOCOL_ID = "/auki/subscribe/0.0.1";
const DEFAULT_FRAME_BODY_LIMIT = 1_048_576;

function bootstrapRecord(peerId: string, address: string): unknown {
  return {
    peer_id: peerId,
    direct_addresses: [address],
    webrtc_direct_addresses: [],
    relay_addresses: [],
    relay_server_addresses: [],
    bootstrap_addresses: [address],
  };
}

function switchableBootstrapRecord(): unknown {
  return {
    peer_id: "native-peer",
    direct_addresses: ["/memory/websocket"],
    webrtc_direct_addresses: ["/memory/webrtc-direct"],
    relay_addresses: [],
    relay_server_addresses: [],
    bootstrap_addresses: ["/memory/websocket", "/memory/webrtc-direct"],
  };
}

function memoryConnectionPath(address: string): BrowserConnectionPath {
  return {
    connectionId: `memory:${address}`,
    direction: "dialer",
    transport: "unknown",
    relayInvolved: false,
    remoteAddress: address,
    status: "open",
    direct: true,
  };
}

async function protocolWasmInput(): Promise<{ module_or_path: Uint8Array }> {
  return {
    module_or_path: await readFile(
      path.resolve(process.cwd(), "../auki-protocol-wasm/pkg-web/auki_protocol_wasm_bg.wasm"),
    ),
  };
}

async function fixtureJson(name: string): Promise<JsonObject> {
  const content = await readFile(
    path.resolve(process.cwd(), "../auki-protocol/vectors", name),
    "utf8",
  );
  return JSON.parse(content) as JsonObject;
}

class MemoryTransport implements BrowserTransport {
  readonly dials: string[][] = [];
  readonly forcedDials: string[][] = [];
  readonly closedPeers: Array<{ peerId: string; keepAddresses: string[] }> = [];
  readonly protocolDials: Array<{ peerId: string; addresses: string[]; protocol: string }> = [];
  private readonly relayServerAddresses: string[] = [];
  private readonly paths = new Map<string, BrowserConnectionPath[]>();
  private readonly dialFailures: Error[] = [];
  private readonly protocolHandlers = new Map<
    string,
    (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void
  >();
  private readonly remoteTransports = new Map<string, MemoryTransport>();
  private readonly inboundHandlers = new Map<
    string,
    (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void
  >();
  started = 0;
  stopped = 0;

  constructor(
    readonly peerId: string,
    private readonly addresses: string[],
  ) {}

  async start(): Promise<void> {
    this.started += 1;
  }

  async stop(): Promise<void> {
    this.stopped += 1;
  }

  multiaddrs(): string[] {
    return [
      ...this.addresses,
      ...this.relayServerAddresses.map(
        (address) => `${address}/p2p-circuit/p2p/${this.peerId}`,
      ),
    ];
  }

  addRelayServerAddresses(addresses: string[]): void {
    for (const address of addresses) {
      if (!this.relayServerAddresses.includes(address)) {
        this.relayServerAddresses.push(address);
      }
    }
  }

  async dial(addresses: string[], options: { force?: boolean } = {}): Promise<void> {
    this.dials.push(addresses.slice());
    if (options.force) {
      this.forcedDials.push(addresses.slice());
    }
    const failure = this.dialFailures.shift();
    if (failure) {
      throw failure;
    }
  }

  async closePeerConnections(peerId: string, keepAddresses: string[] = []): Promise<void> {
    this.closedPeers.push({ peerId, keepAddresses: keepAddresses.slice() });
  }

  connectPeer(peer: MemoryTransport): void {
    this.remoteTransports.set(peer.peerId, peer);
  }

  setConnectionPaths(peerId: string, paths: BrowserConnectionPath[]): void {
    this.paths.set(peerId, paths.slice());
  }

  failNextDial(error: Error): void {
    this.dialFailures.push(error);
  }

  connectionPaths(peerId: string): BrowserConnectionPath[] {
    return this.paths.get(peerId)?.slice() ?? [];
  }

  handleProtocol(
    protocol: string,
    handler: (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void,
  ): void {
    this.protocolHandlers.set(protocol, handler);
  }

  async registerProtocolHandler(
    protocol: string,
    handler: (stream: BrowserProtocolStream, peerId: string) => Promise<void> | void,
  ): Promise<void> {
    this.inboundHandlers.set(protocol, handler);
  }

  async unregisterProtocolHandler(protocol: string): Promise<void> {
    this.inboundHandlers.delete(protocol);
  }

  async dialProtocol(
    peerId: string,
    addresses: string[],
    protocol: string,
  ): Promise<BrowserProtocolStream> {
    this.protocolDials.push({ peerId, addresses: addresses.slice(), protocol });
    const handler = this.protocolHandlers.get(protocol);
    if (handler) {
      const [local, remote] = linkedStreams();
      Promise.resolve(handler(remote, peerId)).catch((error: unknown) => {
        remote.abort(error instanceof Error ? error : new Error(String(error)));
      });
      return local;
    }
    const remoteTransport = this.remoteTransports.get(peerId);
    const remoteHandler = remoteTransport?.inboundHandlers.get(protocol);
    if (!remoteHandler) {
      throw new Error(`No handler registered for ${protocol}`);
    }
    const [local, remote] = linkedStreams();
    Promise.resolve(remoteHandler(remote, this.peerId)).catch((error: unknown) => {
      remote.abort(error instanceof Error ? error : new Error(String(error)));
    });
    return local;
  }

  async openInbound(peerId: string, protocol: string): Promise<BrowserProtocolStream> {
    const handler = this.inboundHandlers.get(protocol);
    if (!handler) {
      throw new Error(`No inbound handler registered for ${protocol}`);
    }
    const [remote, local] = linkedStreams();
    Promise.resolve(handler(local, peerId)).catch((error: unknown) => {
      local.abort(error instanceof Error ? error : new Error(String(error)));
    });
    return remote;
  }
}

function linkedStreams(): [QueueStream, QueueStream] {
  const left = new QueueStream();
  const right = new QueueStream();
  left.link(right);
  right.link(left);
  return [left, right];
}

class QueueStream implements BrowserProtocolStream {
  private peer?: QueueStream;
  private readonly chunks: Uint8Array[] = [];
  private readonly waiters: Array<() => void> = [];
  private closeDelay?: Promise<void>;
  private closed = false;
  private aborted?: Error;

  link(peer: QueueStream): void {
    this.peer = peer;
  }

  send(data: Uint8Array): boolean {
    if (this.closed || this.aborted) {
      return false;
    }
    this.peer?.push(data);
    return true;
  }

  async close(): Promise<void> {
    await this.closeDelay;
    this.finish();
    this.peer?.finish();
  }

  delayCloseUntil(delay: Promise<void>): void {
    this.closeDelay = delay;
  }

  abort(error: Error): void {
    this.fail(error);
    this.peer?.fail(error);
  }

  async onDrain(): Promise<void> {}

  async *[Symbol.asyncIterator](): AsyncIterator<Uint8Array> {
    for (;;) {
      const chunk = this.chunks.shift();
      if (chunk) {
        yield chunk;
        continue;
      }
      if (this.aborted) {
        throw this.aborted;
      }
      if (this.closed) {
        return;
      }
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
  }

  private push(data: Uint8Array): void {
    if (this.closed || this.aborted) {
      return;
    }
    this.chunks.push(data.slice());
    this.wake();
  }

  private finish(): void {
    this.closed = true;
    this.wake();
  }

  private fail(error: Error): void {
    this.aborted = error;
    this.closed = true;
    this.wake();
  }

  private wake(): void {
    for (const resolve of this.waiters.splice(0)) {
      resolve();
    }
  }
}
