# Parking lot — auki-datatypes-py

Open questions for the `auki-datatypes-py` package. Cross-cutting questions live in the [root `parking_lot.md`](../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../CLAUDE.md) for the workflow.

---

## Regen-check test

Generated Python files in `auki_datatypes/auki/` are committed alongside the `.proto` source. They can drift if a contributor edits a `.proto` without re-running `regen.sh`. Worth adding a CI check that re-runs codegen and asserts no diff against the committed files. Defer until CI grows that capability; a contributor who skips regen will trip the locked-vector tests anyway (the simplest backstop).

## `betterproto` 1.x vs 2.x

We pin `betterproto==1.2.5`. Version 2.x exists in beta and has different generated-code shape; bumping requires re-running `regen.sh`, re-locking the cross-language vectors, and updating any consumer code that relies on 1.x specifics. Defer until 2.x stabilizes or a real consumer needs a 2.x feature.

## PyPI distribution policy

Same question as every other `*-py` crate. Today consumers `pip install -e .` from the repo. Defer until a non-source-build consumer needs the wheel.

## `auki_datatypes` vs `auki_datatypes.auki` import path

Generated files live at `auki_datatypes/auki/<name>.py` because the proto package paths are `auki.<name>`. The top-level `auki_datatypes/__init__.py` re-exports the submodules so consumers write `from auki_datatypes import detection` rather than `from auki_datatypes.auki import detection`. The double-`auki` is a betterproto codegen artifact, not a chosen API.

If `regen.sh` ever changes to flatten the layout (so generated files live at `auki_datatypes/<name>.py` directly), the `__init__.py` re-exports become trivial passthroughs and could be deleted. Open question: is the current re-export layer worth keeping for stability, or should we restructure the codegen output? Lean: keep the current layer; restructuring is a churn-for-aesthetics move.

## Type stubs

`betterproto` generates dataclasses with type annotations baked in, so IDEs already get type hints from the source. Type stubs aren't critical here in the way they would be for PyO3 wrappers. Track the parallel `*-py` discussion if a consumer asks.
