# Turning The RFC Into SDK Code

Audience: internal weekly update.
Length: 5-10 minutes.
Tone: short, personal, and concrete.

## One-line story

I started turning the P2P RFC work into SDK code. The useful result this week is not a finished networking stack. It is that the RFC is now being forced through a real implementation path, and the gaps are becoming easier to see.

## Slide 1 - Where I am

Speaker script:

> Last week I talked about keeping the protocol boring: write the rules first, then let implementation expose what is missing. This week I started doing the second part. I began implementing the RFC into the SDK.

Bullets:

- The RFC is no longer only a document.
- I started mapping it into SDK code and demo surfaces.
- The goal is a small first slice, not the whole future SDK.

## Slide 2 - Why this matters

Speaker script:

> The reason to do this now is simple: protocol text gets much better when it has to survive contact with code. If the SDK cannot implement a rule cleanly, then either the rule is unclear, the code is wrong, or the RFC is missing something.

Bullets:

- Implementation turns vague protocol language into concrete decisions.
- It separates baseline behavior from product behavior.
- It gives us real feedback instead of more abstract design discussion.

## Slide 3 - The shape of the first slice

Speaker script:

> I am keeping the first slice intentionally small. A peer should be able to connect, identify itself, describe what it can serve, and exchange data through the basic Offer / Get / Subscribe path.

Bullets:

- Connect peers.
- Check identity and lifecycle behavior.
- Work with served domains and offers.
- Fetch or stream a small piece of data.
- Keep everything narrow enough to test and reason about.

## Slide 4 - What changed this week

Speaker script:

> The main change is that I moved from writing the baseline toward implementing it. I started putting the RFC concepts into SDK structure, and I started using demo code to check whether the shape makes sense.

Bullets:

- RFC concepts are being translated into SDK code.
- The implementation is starting to show which parts are clear.
- The demo path is becoming a way to test the protocol shape, not just show a feature.

## Slide 5 - What I am learning

Speaker script:

> The useful part is that the implementation is already making the protocol less theoretical. Some things are straightforward. Some things need sharper language. Some things probably belong outside the baseline.

Bullets:

- Clear rules are easy to turn into code.
- Ambiguous rules become visible quickly.
- Product decisions should not quietly become protocol requirements.

## Slide 6 - What this makes easier

Speaker script:

> Having a real SDK path makes the next decisions smaller. Instead of asking whether the whole networking design is right, I can ask whether this handshake shape is clear, whether this offer shape is enough, and whether this demo path proves the right thing.

Bullets:

- Turn protocol language into API names and message shapes.
- Write examples against the behavior we actually want to support.
- Find the next missing compatibility notes, expected results, and validation cases.

## Slide 7 - What this does not prove yet

Speaker script:

> This is still a first implementation slice. It does not mean the networking stack is done, and it does not mean every transport or browser case is ready. It means we now have a concrete path to test, criticize, and improve.

Bullets:

- The first slice is for learning and alignment, not production readiness.
- Browser, relay, multi-peer, and smoke-test coverage still need more evidence.
- The RFC and SDK should keep changing together as implementation exposes gaps.

## Slide 8 - What I am doing next

Speaker script:

> Next I want to keep the loop tight: implement a small piece, check it against the RFC, update the language where it is wrong or vague, and then use the demo surface to show the current state honestly.

Bullets:

- Keep the first SDK path narrow.
- Use implementation feedback to sharpen the RFC and backlog.
- Demo only the behavior that has actually been preflighted.

## Quick 5-minute cut

If the meeting runs short, use only these sections:

1. Where I am: I started implementing the RFC into the SDK.
2. Why it matters: implementation exposes unclear protocol language.
3. First slice: connect, identify, serve offers, Get / Subscribe small data.
4. Learning: clear rules become code; unclear rules become visible.
5. Next: keep the RFC and SDK moving together through small, testable slices.
