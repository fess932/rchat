use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{mpsc, Mutex};
use webrtc::{
    peer_connection::RTCPeerConnection,
    track::track_local::track_local_static_rtp::TrackLocalStaticRTP,
};

pub type UserId = String;

/// Per-peer server-side state
pub struct PeerState {
    pub pc:        Arc<RTCPeerConnection>,
    /// Channel to push encoded JSON messages toward the browser
    pub ws_tx:     mpsc::UnboundedSender<String>,
    /// Send renegotiation answers here (from WS handler → renegotiate())
    pub answer_tx: mpsc::Sender<String>,
    /// Receive renegotiation answers (used inside renegotiate())
    pub answer_rx: Arc<Mutex<mpsc::Receiver<String>>>,
}

/// Shared state for one room
#[derive(Clone, Default)]
pub struct Room {
    /// Active WebRTC peers
    pub peers:  Arc<DashMap<UserId, Arc<PeerState>>>,
    /// Relay tracks published by each user (forwarded to all other peers)
    pub tracks: Arc<DashMap<UserId, Vec<Arc<TrackLocalStaticRTP>>>>,
}

impl Room {
    pub fn new() -> Self {
        Self::default()
    }

    /// All relay tracks except those belonging to `exclude`
    pub fn relay_tracks_except(&self, exclude: &str) -> Vec<Arc<TrackLocalStaticRTP>> {
        self.tracks
            .iter()
            .filter(|e| e.key().as_str() != exclude)
            .flat_map(|e| e.value().clone())
            .collect()
    }

    pub fn remove(&self, uid: &str) {
        self.peers.remove(uid);
        self.tracks.remove(uid);
    }
}
