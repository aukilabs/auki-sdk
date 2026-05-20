# Parking lot — auki-layout-py

Open questions for the `auki-layout-py` crate. Cross-cutting questions live in the [root `parking_lot.md`](../../../parking_lot.md).

When a question is answered inline, an agent will replace the item with a "Propagate: …" task — see [CLAUDE.md](../../../CLAUDE.md) for the workflow.

---

## Type stubs (`auki_layout.pyi`)

The surface is ten functions, all `str → str`. IDE support without stubs is OK but not great. Track [`auki-network-py`](../auki-network-py/parking_lot.md)'s parallel discussion; when the team picks a stub-generation pattern, follow it here.

## PyPI distribution policy

Same question as `auki-identity-py` / `auki-network-py` / `auki-logs-py`. Today every consumer builds from source via `maturin develop`. Defer until a non-source-build consumer needs the wheel.
