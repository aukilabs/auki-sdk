use auki_network::PeerIdentity;
use auki_network::browser_session_protocol::{
    BrowserMediaPresence, BrowserSessionParticipant, BrowserSessionSensor,
};
use libp2p::Multiaddr;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct BrowserPeerDebugState {
    pub uses_browser_session: bool,
    pub advertised_multiaddrs: Vec<String>,
    pub membership_peer_count: usize,
}

#[derive(Debug, Clone)]
pub struct BrowserPeerMember {
    pub peer_id: String,
    pub multiaddrs: Vec<String>,
    pub participant: Option<BrowserSessionParticipant>,
}

pub struct BrowserFullPeer {
    identity: PeerIdentity,
    advertised_multiaddrs: RefCell<Vec<Multiaddr>>,
    members: RefCell<BTreeMap<String, BrowserPeerMember>>,
    local_participant: RefCell<BrowserSessionParticipant>,
}

impl BrowserFullPeer {
    pub fn new(identity: PeerIdentity, participant: BrowserSessionParticipant) -> Rc<Self> {
        Rc::new(Self {
            identity,
            advertised_multiaddrs: RefCell::new(Vec::new()),
            members: RefCell::new(BTreeMap::new()),
            local_participant: RefCell::new(participant),
        })
    }

    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    pub fn set_advertised_multiaddrs(&self, addrs: Vec<Multiaddr>) {
        self.advertised_multiaddrs.replace(addrs);
    }

    pub fn advertised_multiaddrs(&self) -> Vec<Multiaddr> {
        self.advertised_multiaddrs.borrow().clone()
    }

    pub fn debug_state(&self) -> BrowserPeerDebugState {
        BrowserPeerDebugState {
            uses_browser_session: false,
            advertised_multiaddrs: self
                .advertised_multiaddrs
                .borrow()
                .iter()
                .map(ToString::to_string)
                .collect(),
            membership_peer_count: self.members.borrow().len(),
        }
    }

    pub fn update_local_participant(&self, participant: BrowserSessionParticipant) {
        self.local_participant.replace(participant);
    }

    pub fn set_local_sensors(&self, sensors: Vec<BrowserSessionSensor>) {
        self.local_participant.borrow_mut().sensors = sensors;
    }

    pub fn set_local_media(&self, media: BrowserMediaPresence) {
        self.local_participant.borrow_mut().media_presence = media;
    }
}
