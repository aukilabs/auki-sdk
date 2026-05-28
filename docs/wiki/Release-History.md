# Release History

*Coming soon — content to be drafted in a follow-up card.*

This page will index the SDK's tags with prose summaries of what changed, who's affected, and migration notes for each.

Until then, the canonical source for each release is the annotated tag message:

```bash
git tag --list --sort=-v:refname  # list versions
git show v0.0.53                   # detailed annotated-tag message for one version
```

Recent highlights:

- **v0.0.53** (2026-05-27) — [#216](https://github.com/aukilabs/auki-sdk/issues/216) the SDK becomes the robot data plane. Schema migration touching registries (peer_id), manifests (source/writer split), wire protocols (bumped to 0.2.0), catalog row reshape, deletion of legacy resource types, new `auki-session` crate as the declarative app surface, new `auki-session-py` Python binding.
- **v0.0.52** — [#217](https://github.com/aukilabs/auki-sdk/issues/217) enforce closed sensor kinds.

[← Back to: Design + Architecture](Design-and-Architecture)
