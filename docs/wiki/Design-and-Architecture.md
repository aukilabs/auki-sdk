# Design + Architecture

This section is for engineers reading or contributing to the Auki SDK source. It frames the in-repo documentation for a wider audience.

## Where to start

- [The Five Questions](The-Five-Questions) — the architectural backbone (Identity / Spatial / Temporal / Networking / Tokenomics)
- [Glossary](Glossary) — long-form companion to `GLOSSARY.md`, with code refs and common confusions per term
- [Crate map](Crate-Map) — what each Rust crate does, in narrative form

## In-repo documentation

The repo holds frozen-in-time design records for major changes:

- [`docs/superpowers/specs/`](https://github.com/aukilabs/auki-sdk/tree/develop/docs/superpowers/specs) — design specifications
- [`docs/superpowers/plans/`](https://github.com/aukilabs/auki-sdk/tree/develop/docs/superpowers/plans) — implementation plans
- [`dataproducts.md`](https://github.com/aukilabs/auki-sdk/blob/develop/dataproducts.md) — the peer-discovery / resource catalog reference
- [`docs/control-api.md`](https://github.com/aukilabs/auki-sdk/blob/develop/docs/control-api.md) — HTTP control API for SDK-session daemons

## Release / tag history

- [Release history](Release-History) — one entry per shipped tag from v0.0.50 onward

`git tag --list --sort=-v:refname` shows the current set; `git show vX.Y.Z` is the authoritative annotated-tag message.

## Authoritative spec

- [VISION.md](https://github.com/aukilabs/auki-sdk/blob/develop/VISION.md) — the aspirational protocol spec
- [GLOSSARY.md](https://github.com/aukilabs/auki-sdk/blob/develop/GLOSSARY.md) — domain terms (this wiki's [Glossary](Glossary) page expands on it)

## Contributing

- [CONTRIBUTING.md](https://github.com/aukilabs/auki-sdk/blob/develop/CONTRIBUTING.md) — folder convention, board flow, git hygiene
- [CLAUDE.md](https://github.com/aukilabs/auki-sdk/blob/develop/CLAUDE.md) — the equivalent for AI agents
