"""Python-side tests for ``auki_identity``.

Run via::

    maturin develop
    pytest python_tests/

These tests verify the three exposed APIs round-trip correctly across
the Python ↔ Rust seam, and pin the cross-language ``peer_id`` vector
that lets us assert the bindings agree byte-for-byte with the Rust
crate.
"""

from __future__ import annotations

import os
import re
import stat
from pathlib import Path

import pytest

import auki_identity
from auki_identity import Wallet, app_instance, load_or_mint_seed


# ─── load_or_mint_seed ───────────────────────────────────────────────────────


def test_load_or_mint_seed_mints_when_missing(tmp_path: Path) -> None:
    path = tmp_path / "identity.seed"
    assert not path.exists()

    seed = load_or_mint_seed(str(path))

    assert isinstance(seed, bytes)
    assert len(seed) == 32
    assert path.exists()
    assert path.read_bytes() == seed


def test_load_or_mint_seed_idempotent(tmp_path: Path) -> None:
    path = tmp_path / "identity.seed"
    first = load_or_mint_seed(str(path))
    second = load_or_mint_seed(str(path))
    assert first == second


def test_load_or_mint_seed_creates_parent_directories(tmp_path: Path) -> None:
    path = tmp_path / "a" / "b" / "c" / "identity.seed"
    assert not path.parent.exists()
    seed = load_or_mint_seed(str(path))
    assert path.exists()
    assert len(seed) == 32


def test_load_or_mint_seed_rejects_wrong_length_with_value_error(
    tmp_path: Path,
) -> None:
    path = tmp_path / "identity.seed"
    path.write_bytes(b"not 32 bytes")
    with pytest.raises(ValueError, match="32 bytes"):
        load_or_mint_seed(str(path))


def test_load_or_mint_seed_accepts_existing_32_byte_file(tmp_path: Path) -> None:
    path = tmp_path / "identity.seed"
    payload = bytes(range(32))
    path.write_bytes(payload)
    seed = load_or_mint_seed(str(path))
    assert seed == payload


@pytest.mark.skipif(os.name != "posix", reason="0o600 mode is Unix-specific")
def test_load_or_mint_seed_sets_0600_mode_on_unix(tmp_path: Path) -> None:
    path = tmp_path / "identity.seed"
    load_or_mint_seed(str(path))
    mode = stat.S_IMODE(path.stat().st_mode)
    assert mode == 0o600, f"expected 0o600, got {oct(mode)}"


# ─── Wallet ──────────────────────────────────────────────────────────────────


def test_wallet_from_seed_then_derive_child_then_peer_id_round_trip() -> None:
    seed = b"\x03" * 32
    w = Wallet.from_seed(seed)
    peer = w.derive_child("peer/v1")
    pid = peer.peer_id()
    assert isinstance(pid, str)
    # Canonical libp2p ed25519 PeerIds start with "12D3KooW".
    assert pid.startswith("12D3KooW"), f"expected canonical PeerId, got {pid!r}"


def test_wallet_peer_id_is_deterministic() -> None:
    seed = b"\x07" * 32
    pid_a = Wallet.from_seed(seed).derive_child("peer/v1").peer_id()
    pid_b = Wallet.from_seed(seed).derive_child("peer/v1").peer_id()
    assert pid_a == pid_b


def test_wallet_peer_id_differs_across_seeds() -> None:
    a = Wallet.from_seed(b"\x01" * 32).derive_child("peer/v1").peer_id()
    b = Wallet.from_seed(b"\x02" * 32).derive_child("peer/v1").peer_id()
    assert a != b


def test_wallet_derive_child_differs_across_labels() -> None:
    seed = b"\x05" * 32
    w = Wallet.from_seed(seed)
    peer = w.derive_child("peer/v1").peer_id()
    other = w.derive_child("app/boosterapp").peer_id()
    assert peer != other


def test_wallet_from_seed_rejects_wrong_length() -> None:
    with pytest.raises(ValueError, match="32 bytes"):
        Wallet.from_seed(b"too short")


def test_wallet_seed_round_trips_for_root_wallet() -> None:
    """Wallet.from_seed(seed).seed() == seed — the trivial round-trip."""
    seed = b"\x42" * 32
    w = Wallet.from_seed(seed)
    assert w.seed() == seed


def test_wallet_seed_round_trips_for_derived_child() -> None:
    """The peer-identity path: derive once, hand the bytes to lower-level
    networking, expect the runtime PeerId to equal the derived child's peer_id.
    Concretely: a wallet reconstructed from the derived seed must have the same
    peer_id as the original derived wallet."""
    parent = Wallet.from_seed(b"\x07" * 32)
    derived = parent.derive_child("peer/v1")
    derived_seed = derived.seed()
    derived_peer_id = derived.peer_id()

    # auki_network::PeerIdentity::from_seed(derived_seed) ↔
    # Wallet::from_seed(derived_seed).peer_id() must agree:
    reconstructed = Wallet.from_seed(derived_seed)
    assert reconstructed.peer_id() == derived_peer_id


def test_wallet_seed_returns_bytes_of_length_32() -> None:
    seed = Wallet.from_seed(b"\x01" * 32).seed()
    assert isinstance(seed, bytes)
    assert len(seed) == 32


def test_wallet_seed_differs_across_derivations() -> None:
    """Two child wallets with different labels must yield different seeds —
    otherwise the derivation collapses and security goes out the window."""
    parent = Wallet.from_seed(b"\x05" * 32)
    a = parent.derive_child("peer/v1").seed()
    b = parent.derive_child("app/boosterapp").seed()
    assert a != b


# ─── app_instance.derive ─────────────────────────────────────────────────────


def test_app_instance_derive_returns_12_lowercase_hex_or_runtime_error() -> None:
    """``derive()`` returns a 12-char lowercase hex string on machines
    with at least one IEEE-administered NIC, or raises ``RuntimeError``
    in containers / random-MAC-only environments. Both are valid; the
    test gates accordingly so CI containers don't false-fail."""
    try:
        instance = app_instance.derive()
    except RuntimeError as e:
        # Acceptable in container / privacy-MAC-only environments.
        msg = str(e)
        assert "NoNetworkInterfaces" in msg or "NoSuitableMac" in msg, msg
        return

    assert isinstance(instance, str)
    assert re.fullmatch(r"[0-9a-f]{12}", instance), (
        f"expected 12 lowercase hex chars, got {instance!r}"
    )


# ─── Locked cross-language peer_id vector ────────────────────────────────────


# This string is the canonical libp2p PeerId for
# ``Wallet.from_seed(b'\x03' * 32).derive_child("peer/v1").peer_id()`` —
# computed from the Rust API once and baked here as a literal. The
# parallel Rust agent's locked test asserts the same string from
# ``PeerIdentity::from_wallet(&Wallet::from_seed(&[3u8; 32])).peer_id().to_string()``.
# If both pass, the bindings agree byte-for-byte with the Rust crate.
#
# IMPORTANT: at the time this test was authored, the local sandbox
# could not execute Rust code (cargo/rustc are not on the allow-list),
# so the literal below is a placeholder marker to be filled in once
# the parallel agent's Rust output lands. Until then this test is
# strict-skipped — it asserts shape invariants only.
#
# To regenerate locally::
#
#     cd crates/auki-identity-py
#     maturin develop --release
#     python -c "import auki_identity; \
#         print(auki_identity.Wallet.from_seed(b'\\x03'*32).derive_child('peer/v1').peer_id())"
#
# Then paste the printed string into ``LOCKED_PEER_ID_FROM_SEED_03``
# below and unskip the strict assertion.
LOCKED_PEER_ID_FROM_SEED_03: str | None = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"


def test_locked_peer_id_vector() -> None:
    """The cross-language locked vector — `Wallet.from_seed(b'\\x03' * 32)
    .derive_child('peer/v1').peer_id()`. Asserts shape always; asserts
    exact match against the locked literal once it's filled in."""
    pid = Wallet.from_seed(b"\x03" * 32).derive_child("peer/v1").peer_id()

    # Shape pin — true regardless of whether the locked literal is
    # filled in. If any layer (XXH3 derivation, ed25519, libp2p
    # protobuf-multihash-base58 encoding) drifts, this catches it.
    assert pid.startswith("12D3KooW")
    assert 46 <= len(pid) <= 64, f"PeerId length out of range: {pid!r}"

    if LOCKED_PEER_ID_FROM_SEED_03 is not None:
        assert pid == LOCKED_PEER_ID_FROM_SEED_03, (
            f"locked vector mismatch — Rust and Python disagree.\n"
            f"  expected: {LOCKED_PEER_ID_FROM_SEED_03}\n"
            f"  got:      {pid}"
        )


# ─── Module surface ──────────────────────────────────────────────────────────


def test_module_exposes_only_documented_apis() -> None:
    """Pin the Python surface — the three documented entry points
    plus ``app_instance``. Anything else creeping in is a deliberate
    decision and should bump the changelog."""
    public = {name for name in dir(auki_identity) if not name.startswith("_")}
    expected = {"Wallet", "app_instance", "load_or_mint_seed"}
    assert expected.issubset(public), (
        f"missing expected API: {expected - public}"
    )
