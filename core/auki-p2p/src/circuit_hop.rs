//! Live exact-route circuit hops, keyed like WSS `direct_connections`.
//!
//! One preferred hop per `(target peer, circuit multiaddr)`, plus at most one
//! old generation during explicit replacement. Unlike the relay WebSocket,
//! a circuit slot is scarce, so the last owner release tears the hop down.
//! Releases are idempotent per owner so a cancelled `close` plus `Drop` cannot
//! drop a sibling stream's hop.

use std::collections::{HashMap, HashSet};

use libp2p::{swarm::ConnectionId, Multiaddr, PeerId};

/// Identity of one p2p-circuit hop to a target through a specific circuit
/// multiaddr.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CircuitHopKey {
    pub target_peer_id: PeerId,
    pub circuit_address: Multiaddr,
}

/// One `connect_relayed` / `open_exact` owner of a shared hop.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CircuitHopOwner(u64);

impl CircuitHopOwner {
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn from_raw(id: u64) -> Self {
        Self(id)
    }
}

/// Result of trying to take a ref on a circuit hop before dialing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CircuitAcquire {
    /// Incremented an already-established hop. Skip `dial_circuit`.
    Live {
        connection_id: ConnectionId,
        owner: CircuitHopOwner,
    },
    /// Another task is already dialing this hop. Join its waiters.
    Pending,
    /// No live or in-flight hop. Caller must dial.
    Vacant,
}

#[derive(Debug, Default)]
pub(crate) struct CircuitHopTable {
    by_key: HashMap<CircuitHopKey, ConnectionId>,
    by_connection: HashMap<ConnectionId, CircuitHopEntry>,
    pending: HashSet<CircuitHopKey>,
    next_owner: u64,
}

#[derive(Debug)]
struct CircuitHopEntry {
    key: CircuitHopKey,
    owners: HashSet<CircuitHopOwner>,
}

impl CircuitHopTable {
    pub(crate) fn next_owner(&mut self) -> CircuitHopOwner {
        self.next_owner = self.next_owner.wrapping_add(1);
        if self.next_owner == 0 {
            self.next_owner = 1;
        }
        CircuitHopOwner(self.next_owner)
    }

    pub(crate) fn acquire(&mut self, key: &CircuitHopKey) -> CircuitAcquire {
        if let Some(connection_id) = self.by_key.get(key).copied() {
            if self.by_connection.contains_key(&connection_id) {
                let owner = self.next_owner();
                self.by_connection
                    .get_mut(&connection_id)
                    .expect("checked the hop entry exists")
                    .owners
                    .insert(owner);
                return CircuitAcquire::Live {
                    connection_id,
                    owner,
                };
            }
            self.by_key.remove(key);
        }
        if self.pending.contains(key) {
            CircuitAcquire::Pending
        } else {
            CircuitAcquire::Vacant
        }
    }

    pub(crate) fn mark_pending(&mut self, key: CircuitHopKey) {
        self.pending.insert(key);
    }

    /// Record a finished dial. Empty owners means every waiter cancelled; the
    /// caller closes the connection and this table stores nothing.
    pub(crate) fn established(
        &mut self,
        key: CircuitHopKey,
        connection_id: ConnectionId,
        owners: impl IntoIterator<Item = CircuitHopOwner>,
    ) {
        self.pending.remove(&key);
        let owners: HashSet<_> = owners.into_iter().collect();
        if owners.is_empty() {
            self.restore_previous(&key);
            return;
        }
        self.by_key.insert(key.clone(), connection_id);
        self.by_connection
            .insert(connection_id, CircuitHopEntry { key, owners });
    }

    pub(crate) fn fail_pending(&mut self, key: &CircuitHopKey) {
        self.pending.remove(key);
        self.restore_previous(key);
    }

    /// Drop one owner of `connection_id`. A repeated release of the same owner
    /// is a no-op and does not close a hop other owners still hold. Unknown
    /// IDs close, matching the previous CloseConnection behavior.
    pub(crate) fn release(&mut self, connection_id: ConnectionId, owner: CircuitHopOwner) -> bool {
        let Some(entry) = self.by_connection.get_mut(&connection_id) else {
            return true;
        };
        if !entry.owners.remove(&owner) {
            return false;
        }
        if !entry.owners.is_empty() {
            return false;
        }
        if self.by_key.get(&entry.key) == Some(&connection_id) {
            self.by_key.remove(&entry.key);
        }
        let key = entry.key.clone();
        self.by_connection.remove(&connection_id);
        self.restore_previous(&key);
        true
    }

    /// Drop a hop after a remote RST even if owners remain, so the next
    /// `open_exact` redials.
    pub(crate) fn invalidate(&mut self, connection_id: ConnectionId) {
        if let Some(entry) = self.by_connection.remove(&connection_id) {
            if self.by_key.get(&entry.key) == Some(&connection_id) {
                self.by_key.remove(&entry.key);
            }
            self.restore_previous(&entry.key);
        }
    }

    /// Atomically stop selecting the old generation before acquiring its replacement.
    /// Concurrent requests against the same old generation share one candidate.
    /// A second rotation must wait for the previous generation's owners to close.
    pub(crate) fn prepare_replacement(
        &mut self,
        key: &CircuitHopKey,
        old: ConnectionId,
    ) -> Result<(), &'static str> {
        if self
            .by_connection
            .get(&old)
            .is_some_and(|entry| &entry.key != key)
        {
            return Err("replacement route does not match the old circuit");
        }
        if self.by_key.get(key) != Some(&old) {
            return Ok(());
        }
        if self
            .by_connection
            .values()
            .filter(|entry| &entry.key == key)
            .count()
            >= 2
        {
            return Err("close the previous circuit generation before replacing again");
        }
        self.by_key.remove(key);
        Ok(())
    }

    /// Failed or abandoned candidates restore selection of the still-live old hop.
    fn restore_previous(&mut self, key: &CircuitHopKey) {
        if !self.by_key.contains_key(key) && !self.pending.contains(key) {
            if let Some((&id, _)) = self
                .by_connection
                .iter()
                .find(|(_, entry)| &entry.key == key)
            {
                self.by_key.insert(key.clone(), id);
            }
        }
    }

    #[cfg(test)]
    fn live_connection(&self, key: &CircuitHopKey) -> Option<ConnectionId> {
        self.by_key.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p_identity::PeerId;

    fn key(port: u16) -> CircuitHopKey {
        CircuitHopKey {
            target_peer_id: PeerId::random(),
            circuit_address: format!("/ip4/127.0.0.1/tcp/{port}/p2p-circuit")
                .parse()
                .expect("test circuit address"),
        }
    }

    fn connection(id: usize) -> ConnectionId {
        ConnectionId::new_unchecked(id)
    }

    #[test]
    fn acquire_is_vacant_then_pending_until_established() {
        let mut table = CircuitHopTable::default();
        let hop = key(1);
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
        table.mark_pending(hop.clone());
        assert_eq!(table.acquire(&hop), CircuitAcquire::Pending);
        let owners = [table.next_owner(), table.next_owner()];
        table.established(hop.clone(), connection(7), owners);
        match table.acquire(&hop) {
            CircuitAcquire::Live { connection_id, .. } => {
                assert_eq!(connection_id, connection(7));
            }
            other => panic!("expected a live hop, got {other:?}"),
        }
    }

    #[test]
    fn first_of_two_releases_keeps_the_hop() {
        let mut table = CircuitHopTable::default();
        let hop = key(2);
        let first = table.next_owner();
        let second = table.next_owner();
        table.established(hop.clone(), connection(8), [first, second]);
        assert!(!table.release(connection(8), first));
        assert_eq!(table.live_connection(&hop), Some(connection(8)));
        assert!(table.release(connection(8), second));
        assert_eq!(table.live_connection(&hop), None);
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn release_is_idempotent_per_owner() {
        let mut table = CircuitHopTable::default();
        let hop = key(6);
        let first = table.next_owner();
        let second = table.next_owner();
        table.established(hop.clone(), connection(11), [first, second]);
        assert!(!table.release(connection(11), first));
        assert!(!table.release(connection(11), first));
        assert!(table.release(connection(11), second));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn failed_live_delivery_rolls_back() {
        let mut table = CircuitHopTable::default();
        let hop = key(7);
        let original = table.next_owner();
        table.established(hop.clone(), connection(12), [original]);
        let CircuitAcquire::Live {
            connection_id,
            owner,
        } = table.acquire(&hop)
        else {
            panic!("expected a live hop");
        };
        assert_eq!(connection_id, connection(12));
        assert_ne!(owner, original);
        assert!(!table.release(connection_id, owner));
        assert!(table.release(connection(12), original));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn invalidate_drops_live_refs_so_the_next_open_redials() {
        let mut table = CircuitHopTable::default();
        let hop = key(3);
        let owner = table.next_owner();
        let other = table.next_owner();
        table.established(hop.clone(), connection(9), [owner, other]);
        table.invalidate(connection(9));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
        assert!(table.release(connection(9), owner));
    }

    #[test]
    fn established_with_zero_refs_does_not_cache() {
        let mut table = CircuitHopTable::default();
        let hop = key(4);
        table.mark_pending(hop.clone());
        table.established(hop.clone(), connection(10), []);
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn fail_pending_clears_the_inflight_marker() {
        let mut table = CircuitHopTable::default();
        let hop = key(5);
        table.mark_pending(hop.clone());
        table.fail_pending(&hop);
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn retiring_keeps_old_owners_and_old_release_preserves_replacement() {
        let mut table = CircuitHopTable::default();
        let hop = key(8);
        let first = table.next_owner();
        let sibling = table.next_owner();
        table.established(hop.clone(), connection(20), [first, sibling]);
        table.prepare_replacement(&hop, connection(20)).unwrap();
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
        table.mark_pending(hop.clone());
        assert_eq!(table.acquire(&hop), CircuitAcquire::Pending);
        let replacement = table.next_owner();
        table.established(hop.clone(), connection(21), [replacement]);
        table.prepare_replacement(&hop, connection(20)).unwrap(); // A stale retirement cannot retire the new hop.
        assert_eq!(table.live_connection(&hop), Some(connection(21)));
        assert!(!table.release(connection(20), first));
        assert!(table.release(connection(20), sibling));
        assert_eq!(table.live_connection(&hop), Some(connection(21)));
        assert!(table.release(connection(21), replacement));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn retired_remote_reset_cannot_remove_replacement() {
        let mut table = CircuitHopTable::default();
        let hop = key(9);
        let old = table.next_owner();
        table.established(hop.clone(), connection(22), [old]);
        table.prepare_replacement(&hop, connection(22)).unwrap();
        let new = table.next_owner();
        table.established(hop.clone(), connection(23), [new]);
        table.invalidate(connection(22));
        assert_eq!(table.live_connection(&hop), Some(connection(23)));
        assert!(table.release(connection(23), new));
    }

    #[test]
    fn replacement_is_bounded_and_failed_candidate_restores_old_selection() {
        let mut table = CircuitHopTable::default();
        let hop = key(11);
        let old = table.next_owner();
        table.established(hop.clone(), connection(30), [old]);
        table.prepare_replacement(&hop, connection(30)).unwrap();
        let new = table.next_owner();
        table.established(hop.clone(), connection(31), [new]);
        assert!(table.prepare_replacement(&hop, connection(31)).is_err());
        assert_eq!(table.live_connection(&hop), Some(connection(31)));
        assert!(table.release(connection(31), new));
        assert_eq!(table.live_connection(&hop), Some(connection(30)));
        table.prepare_replacement(&hop, connection(30)).unwrap();
        table.mark_pending(hop.clone());
        table.fail_pending(&hop);
        assert_eq!(table.live_connection(&hop), Some(connection(30)));
    }

    #[test]
    fn failed_or_cancelled_replacement_preserves_retired_owners() {
        let mut table = CircuitHopTable::default();
        let hop = key(10);
        let old = table.next_owner();
        table.established(hop.clone(), connection(24), [old]);
        table.prepare_replacement(&hop, connection(24)).unwrap();
        table.mark_pending(hop.clone());
        table.fail_pending(&hop);
        assert!(table.by_connection.contains_key(&connection(24)));
        table.mark_pending(hop.clone());
        table.established(hop.clone(), connection(25), []);
        assert_eq!(table.live_connection(&hop), Some(connection(24)));
        assert!(table.by_connection.contains_key(&connection(24)));
        assert!(!table.by_connection.contains_key(&connection(25)));
        assert!(table.release(connection(24), old));
    }
}
