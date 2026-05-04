//! Per-machine identifier — the `app_instance` field that distinguishes
//! two daemons of the same `app` running on different hardware.
//!
//! ## Recipe (locked per ansuz D4)
//!
//! 1. Enumerate the host's network interfaces.
//! 2. Skip any interface that has no MAC (tunnels, virtual links).
//! 3. Skip the loopback MAC (`00:00:00:00:00:00`).
//! 4. Skip locally-administered MACs — the U/L bit (second-least-significant
//!    bit of the first octet, mask `0x02`) is `1`. These are typically
//!    randomized / generated MACs (macOS private-Wi-Fi, Docker bridges,
//!    VMs, etc.); they're not stable across reboots, so they're useless as
//!    machine identifiers.
//! 5. Sort the remaining MACs lexicographically (by raw bytes) and pick
//!    the first one. This is the deterministic ordering — same machine,
//!    same MAC selection, regardless of OS-level interface enumeration
//!    quirks.
//! 6. Render as 12 lowercase hex characters with no separators
//!    (`aabbccddeeff`).
//!
//! ## Stability caveats
//!
//! This is fragile in some environments — by design, ansuz accepts the
//! tradeoff:
//!
//! - **Containers / Docker.** Each container typically gets a fresh
//!   locally-administered MAC, which we skip. If the container has no
//!   IEEE-administered NIC visible (common), `derive()` returns
//!   [`DeriveError::NoSuitableMac`].
//! - **VMs.** Hypervisors that mint locally-administered MACs (most do)
//!   produce instances that change MAC across host migrations.
//! - **Multi-NIC machines.** Adding/removing a NIC can change which MAC
//!   sorts first. The ordering is stable for a fixed hardware set, not
//!   stable across hardware changes.
//! - **MAC-randomization features.** macOS "Private Wi-Fi Address" and
//!   Linux's `MACAddressPolicy=random` produce locally-administered MACs
//!   we'll skip — but a wired NIC on the same machine will still resolve.
//!
//! A future stable-id alternative (wallet-derived, persisted on first
//! boot) is in the parking lot. For ansuz, MAC-by-convention is the
//! agreed shape.

use std::io;

/// 12 lowercase hex chars, no separators (e.g. `"00163eabcdef"`).
///
/// See module docs for the recipe and stability caveats.
pub fn derive() -> Result<String, DeriveError> {
    let macs = collect_macs().map_err(DeriveError::Io)?;
    if macs.is_empty() {
        return Err(DeriveError::NoNetworkInterfaces);
    }
    derive_from(&macs)
}

/// Pure variant for tests. `derive()` calls this with the host's real
/// MACs; tests call it with fixtures.
///
/// Empty input → [`DeriveError::NoNetworkInterfaces`]. All inputs filtered
/// out (loopback / locally-administered) → [`DeriveError::NoSuitableMac`].
pub fn derive_from(macs: &[[u8; 6]]) -> Result<String, DeriveError> {
    if macs.is_empty() {
        return Err(DeriveError::NoNetworkInterfaces);
    }
    let mut candidates: Vec<&[u8; 6]> = macs
        .iter()
        .filter(|m| !is_loopback(m))
        .filter(|m| !is_locally_administered(m))
        .collect();
    if candidates.is_empty() {
        return Err(DeriveError::NoSuitableMac);
    }
    candidates.sort();
    Ok(format_mac(candidates[0]))
}

/// Why [`derive`] couldn't produce a value.
#[derive(Debug)]
pub enum DeriveError {
    /// The host has no enumerable network interfaces at all. Vanishingly
    /// rare on real hardware; a sandboxed environment with no networking
    /// stack can hit it.
    NoNetworkInterfaces,
    /// Interfaces exist, but every one is either loopback or has a
    /// locally-administered (random / generated) MAC. Common in
    /// containers; common on a laptop with only Private Wi-Fi enabled and
    /// no wired NIC.
    NoSuitableMac,
    /// Underlying syscall (e.g. `getifaddrs`, `GetAdaptersAddresses`)
    /// failed.
    Io(io::Error),
}

impl std::fmt::Display for DeriveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeriveError::NoNetworkInterfaces => {
                f.write_str("no network interfaces enumerable on this host")
            }
            DeriveError::NoSuitableMac => f.write_str(
                "no IEEE-administered MAC found — every interface is loopback or has a locally-administered (random) MAC",
            ),
            DeriveError::Io(e) => write!(f, "interface enumeration failed: {e}"),
        }
    }
}

impl std::error::Error for DeriveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DeriveError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// All-zero MAC. Loopback interfaces (`lo`, `lo0`) report this on every
/// platform we target.
fn is_loopback(mac: &[u8; 6]) -> bool {
    *mac == [0, 0, 0, 0, 0, 0]
}

/// U/L bit set — second-least-significant bit of the first octet.
/// Per IEEE 802, `0x02` masked into the first byte means the MAC was
/// locally administered (i.e. assigned by software, not by an IEEE
/// OUI-holder). Random / generated MACs all set this bit.
fn is_locally_administered(mac: &[u8; 6]) -> bool {
    (mac[0] & 0x02) != 0
}

/// 6 bytes → 12 lowercase hex chars.
fn format_mac(mac: &[u8; 6]) -> String {
    let mut s = String::with_capacity(12);
    for b in mac {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Gather every MAC address visible on the host. Returns the raw 6-byte
/// arrays in `mac_address`'s native iteration order; filtering and
/// ordering are [`derive_from`]'s job.
fn collect_macs() -> io::Result<Vec<[u8; 6]>> {
    let iter = mac_address::MacAddressIterator::new()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(iter.map(|m| m.bytes()).collect())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_from_locked_mac_renders_lowercase_no_separators() {
        // Locked test (per the ansuz brief): a fixed IEEE-administered MAC
        // must render as exactly "00163eabcdef". This is the cross-language
        // contract. If anything in the formatting drifts, every consumer's
        // app_instance would change.
        let macs = [[0x00, 0x16, 0x3e, 0xab, 0xcd, 0xef]];
        assert_eq!(derive_from(&macs).unwrap(), "00163eabcdef");
    }

    #[test]
    fn derive_from_returns_no_network_interfaces_on_empty_input() {
        let result = derive_from(&[]);
        assert!(matches!(result, Err(DeriveError::NoNetworkInterfaces)));
    }

    #[test]
    fn derive_from_returns_no_suitable_mac_when_only_loopback() {
        let macs = [[0u8; 6]];
        assert!(matches!(
            derive_from(&macs),
            Err(DeriveError::NoSuitableMac)
        ));
    }

    #[test]
    fn derive_from_returns_no_suitable_mac_when_only_locally_administered() {
        // First octet 0x02, 0x06, 0x0a, 0xae all have the U/L bit set
        // (bit 1 of the first octet). Random Docker / VM MACs typically
        // start with 0x02 or 0xae.
        let macs = [
            [0x02, 0x42, 0xac, 0x11, 0x00, 0x02], // typical Docker
            [0xae, 0x12, 0x34, 0x56, 0x78, 0x9a],
            [0x0a, 0x00, 0x27, 0x00, 0x00, 0x00],
        ];
        assert!(matches!(
            derive_from(&macs),
            Err(DeriveError::NoSuitableMac)
        ));
    }

    #[test]
    fn derive_from_skips_loopback_and_picks_remaining_ieee_mac() {
        let macs = [
            [0u8; 6],                               // loopback
            [0x00, 0x16, 0x3e, 0xab, 0xcd, 0xef],   // IEEE
        ];
        assert_eq!(derive_from(&macs).unwrap(), "00163eabcdef");
    }

    #[test]
    fn derive_from_skips_locally_administered_mac() {
        let macs = [
            [0x02, 0x42, 0xac, 0x11, 0x00, 0x02],   // locally administered
            [0x3c, 0x22, 0xfb, 0x12, 0x34, 0x56],   // IEEE
        ];
        assert_eq!(derive_from(&macs).unwrap(), "3c22fb123456");
    }

    #[test]
    fn derive_from_picks_lexicographically_first_when_multiple_ieee_macs() {
        // Provide IEEE MACs out of order. The function must sort by raw
        // bytes and pick the smallest — that's the deterministic
        // ordering documented in the recipe.
        let macs = [
            [0xf0, 0x18, 0x98, 0x11, 0x22, 0x33],
            [0x00, 0x16, 0x3e, 0xab, 0xcd, 0xef],
            [0x3c, 0x22, 0xfb, 0x12, 0x34, 0x56],
        ];
        assert_eq!(derive_from(&macs).unwrap(), "00163eabcdef");
    }

    #[test]
    fn derive_from_output_is_exactly_twelve_lowercase_hex_chars() {
        // Exhaustive shape check: any successful return must satisfy the
        // schema. This is what consumers parsing the value rely on.
        // First byte 0xa0 = 10100000 — U/L bit (bit 1) is 0 → IEEE-administered.
        let macs = [[0xa0, 0xcd, 0xef, 0x01, 0x23, 0x45]];
        let s = derive_from(&macs).unwrap();
        assert_eq!(s.len(), 12);
        assert!(
            s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "expected 12 lowercase hex chars, got {s:?}"
        );
    }

    #[test]
    fn ul_bit_logic_isolates_first_octet_bit_one() {
        // Sanity check the bit math directly so the filter intent is
        // legible from tests alone. 0x02 in the first octet means
        // "locally administered"; 0x00 means IEEE-administered;
        // 0x01 (multicast bit) is unrelated to U/L.
        assert!(is_locally_administered(&[0x02, 0, 0, 0, 0, 0]));
        assert!(!is_locally_administered(&[0x00, 0, 0, 0, 0, 0]));
        assert!(!is_locally_administered(&[0x01, 0, 0, 0, 0, 0]));
        assert!(is_locally_administered(&[0x03, 0, 0, 0, 0, 0]));
        assert!(is_locally_administered(&[0xae, 0, 0, 0, 0, 0]));
        assert!(!is_locally_administered(&[0x3c, 0x22, 0xfb, 0, 0, 0]));
    }
}
