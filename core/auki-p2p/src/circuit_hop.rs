//! Live exact-route circuit hops, keyed like WSS `direct_connections`.
//!
//! One hop per `(target peer, circuit multiaddr)`. Unlike the relay WebSocket,
//! a circuit slot is scarce, so the last release tears the hop down.

use std::collections::{HashMap, HashSet};

use libp2p::{swarm::ConnectionId, Multiaddr, PeerId};

/// Identity of one p2p-circuit hop to a target through a specific circuit
/// multiaddr.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CircuitHopKey {
    pub target_peer_id: PeerId,
    pub circuit_address: Multiaddr,
}

/// Result of trying to take a ref on a circuit hop before dialing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CircuitAcquire {
    /// Incremented an already-established hop. Skip `dial_circuit`.
    Live(ConnectionId),
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
}

#[derive(Debug)]
struct CircuitHopEntry {
    key: CircuitHopKey,
    refs: usize,
}

impl CircuitHopTable {
    pub(crate) fn acquire(&mut self, key: &CircuitHopKey) -> CircuitAcquire {
        if let Some(connection_id) = self.by_key.get(key).copied() {
            if let Some(entry) = self.by_connection.get_mut(&connection_id) {
                entry.refs = entry.refs.saturating_add(1);
                return CircuitAcquire::Live(connection_id);
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

    /// Record a finished dial. `refs` is the number of waiters that still want
    /// the hop. Zero refs means every waiter cancelled; the caller closes the
    /// connection and this table stores nothing.
    pub(crate) fn established(
        &mut self,
        key: CircuitHopKey,
        connection_id: ConnectionId,
        refs: usize,
    ) {
        self.pending.remove(&key);
        if refs == 0 {
            return;
        }
        self.by_key.insert(key.clone(), connection_id);
        self.by_connection
            .insert(connection_id, CircuitHopEntry { key, refs });
    }

    pub(crate) fn fail_pending(&mut self, key: &CircuitHopKey) {
        self.pending.remove(key);
    }

    /// Decrement one `connect_relayed` / `open_exact` owner. Returns whether
    /// the swarm should `close_connection`. Unknown IDs close, matching the
    /// previous CloseConnection behavior.
    pub(crate) fn release(&mut self, connection_id: ConnectionId) -> bool {
        let Some(entry) = self.by_connection.get_mut(&connection_id) else {
            return true;
        };
        entry.refs = entry.refs.saturating_sub(1);
        if entry.refs > 0 {
            return false;
        }
        self.by_key.remove(&entry.key);
        self.by_connection.remove(&connection_id);
        true
    }

    /// Drop a hop after a remote RST even if refs remain, so the next
    /// `open_exact` redials.
    pub(crate) fn invalidate(&mut self, connection_id: ConnectionId) {
        if let Some(entry) = self.by_connection.remove(&connection_id) {
            self.by_key.remove(&entry.key);
        }
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
        table.established(hop.clone(), connection(7), 2);
        assert_eq!(table.acquire(&hop), CircuitAcquire::Live(connection(7)));
    }

    #[test]
    fn first_of_two_releases_keeps_the_hop() {
        let mut table = CircuitHopTable::default();
        let hop = key(2);
        table.established(hop.clone(), connection(8), 2);
        assert!(!table.release(connection(8)));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Live(connection(8)));
        assert!(!table.release(connection(8)));
        assert!(table.release(connection(8)));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
    }

    #[test]
    fn invalidate_drops_live_refs_so_the_next_open_redials() {
        let mut table = CircuitHopTable::default();
        let hop = key(3);
        table.established(hop.clone(), connection(9), 3);
        table.invalidate(connection(9));
        assert_eq!(table.acquire(&hop), CircuitAcquire::Vacant);
        assert!(table.release(connection(9)));
    }

    #[test]
    fn established_with_zero_refs_does_not_cache() {
        let mut table = CircuitHopTable::default();
        let hop = key(4);
        table.mark_pending(hop.clone());
        table.established(hop.clone(), connection(10), 0);
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
}
