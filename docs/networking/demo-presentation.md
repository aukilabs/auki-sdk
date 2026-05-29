# P2P Preview Demo Presentation Draft

Audience: internal demo peers, mostly non-technical.
Length: 5-10 minutes.
Tone: warm, short, and concrete. Use this as speaker notes, not a deep technical deck.

## One-line story

We moved the networking work from "interesting code" toward an SDK story: a clear RFC baseline, a Rust runtime, and a browser peer that can use the same rules to receive preview data directly from a native Sentinel-style peer.

## Presenter guardrails

- Keep the meeting about what the demo proves, not about file trees or implementation details.
- Say "preview" and "first slice" often; do not imply production or merge readiness.
- If time is tight, show Get first and treat Subscribe as optional.
- Do not raise PR conflict status unless someone asks.

## Slide 1 - Warm opening

Speaker script:

> Thanks everyone. I want to show a small but important step in the SDK networking work. The goal today is not to walk through every crate or every protocol detail. The goal is to show the story from RFCs, to SDK code, to a simple peer-to-peer preview demo.

Bullets:

- This is a short demo of the new RFC-first networking path.
- We will focus on the story: why it matters, what exists now, what is still preview.
- The live demo is intentionally narrow so we can learn from it.

## Slide 2 - Why we did this

Speaker script:

> The problem we were running into is that networking code can get ahead of the shared language. If people use different words for peer, domain, cluster, authority, and data exchange, then bugs become hard to classify and features are hard to review.

Bullets:

- The RFCs make the first peer-to-peer path small, explicit, and reviewable.
- The baseline defines how peers identify each other, decide what they can serve, and exchange domain-scoped data.
- The SDK implementation now has a clearer target instead of relying on implementation folklore.

## Slide 3 - What exists in this branch

Speaker script:

> The branch is more than documentation now. It has the protocol rules, a native Rust peer runtime, a browser package, and two demo apps that exercise the first preview path.

Bullets:

- `auki-protocol`: the shared rules for handshakes, offers, Get, Subscribe, status, and validation.
- `auki-p2p`: the native Rust runtime that connects peers and serves or consumes offers.
- `auki-p2p-browser`: a browser peer package that uses the Rust protocol rules through WASM instead of reimplementing them in TypeScript.

## Slide 4 - The simple picture

Speaker script:

> The mental model is: one native Sentinel-style peer publishes a small preview offer, and the browser becomes an Auki peer that can connect, discover that offer, and request or subscribe to preview frames.

Bullets:

- Native Sentinel preview producer: generates JPEG preview frames and publishes an offer.
- Browser receiver: starts its own peer, loads bootstrap JSON, connects, and shows offers.
- Demo path: connect -> load offers -> Get one frame -> optionally Subscribe to a stream.

## Slide 5 - Why Rust plus browser libp2p matters

Speaker script:

> The important point is not that we used a specific library. The important point is that the SDK can have a native runtime and a browser runtime speaking the same peer-to-peer language.

Bullets:

- Rust gives us one careful implementation of the protocol rules and native peer behavior.
- The browser package brings those same rules into a web app without a second protocol interpretation.
- libp2p is the connectivity layer that lets peers talk directly when the network path allows it.

Optional plain-language analogy:

> Think of the RFC as the conversation rules, Rust as the careful reference speaker, and the browser package as a web speaker using the same phrasebook.

## Slide 6 - Live demo talk track

Before clicking:

> This is a development preview. I am going to show the smallest useful path first, then only go further if the connection is stable.

Steps:

1. Start the native preview Sentinel and point out the generated bootstrap JSON.
2. Open the browser P2P Preview page and click **Start Peer**.
3. Add or load the Sentinel bootstrap JSON with **Add Peer**.
4. Point to the peer and offer panels: the browser now sees a remote peer and its preview offer.
5. Click **Get** first if available: "This proves the browser can request one preview frame."
6. Click **Subscribe** only if preflight was green: "This proves the browser can keep receiving preview frames over the peer-to-peer path."

Keep the narration light:

- "This browser tab is acting as an SDK peer, not just a static web page."
- "The preview offer is a small stand-in for richer spatial data later."
- "The diagnostics are here to help us debug the preview, not to define product UX."

## Slide 7 - What this does not prove yet

Speaker script:

> This is useful, but it is not the finish line. It proves the first narrow path and gives us something concrete to improve.

Bullets:

- Camera capture is not part of this first slice; generated preview frames are used for repeatability.
- Browser publishing, browser-to-browser preview, multi-browser and multi-Sentinel demos are still follow-up work.
- Transport details, relay behavior, Playwright smoke coverage, and production hardening still need more evidence.

## Slide 8 - If the live demo is flaky

Use this if preflight is not green or Subscribe flakes:

> I am going to switch to the fallback walkthrough. The useful result is still the same: we now have a concrete RFC-to-SDK path and a demo surface that shows where the next bugs are. Rather than spend the meeting debugging connectivity, I will show the intended flow and call out exactly what needs more preflight time.

Fallback flow:

- Show the Sentinel and browser README commands instead of live debugging.
- Show the browser UI panels: Start Peer, Add Peer, Peers, Offers, Diagnostics.
- Explain the sequence: bootstrap, lifecycle connection, offer loading, Get, Subscribe.
- If only Get works, stop there and say Subscribe is the next preflight target.

## Slide 9 - Feedback ask

Speaker script:

> The feedback I want is not whether every detail is final. I want to know whether this is the right first story for SDK networking: small baseline, shared language, native and browser peers, and a visible preview path.

Bullets:

- Is this the right demo shape for explaining the SDK networking direction?
- Is the Rust + browser peer story clear without going too deep?
- Which next proof matters most: browser publishing, multi-peer demo, camera source, or stronger smoke tests?

## Slide 10 - Closing

Speaker script:

> The headline is: the RFC work is now connected to an SDK implementation path and a visible demo. It is still a preview, but it is no longer abstract. We can run it, see peers and offers, request frames, and use the gaps to guide the next slice.

Bullets:

- We have a grounded RFC-to-runtime-to-browser story.
- The demo is intentionally small and honest about unfinished work.
- Next step: preflight the live path, gather feedback, then harden the highest-value follow-up.

## Quick 5-minute cut

If the meeting runs short, use only these sections:

1. Warm opening: this is the RFC-to-SDK-to-demo story.
2. Why: shared language before protocol code grows too far.
3. What exists: `auki-protocol`, `auki-p2p`, `auki-p2p-browser`, preview examples.
4. Demo: Start Peer, Add Peer, Get; Subscribe only if stable.
5. Caveat: preview slice, not production readiness.
6. Ask: which next proof matters most?
