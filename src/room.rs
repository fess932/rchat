use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use dashmap::DashMap;
use tokio::sync::{mpsc, Mutex};
use webrtc::{
    peer_connection::RTCPeerConnection,
    track::track_local::track_local_static_rtp::TrackLocalStaticRTP,
};

pub type UserId = String;

/// Global app state — shared between HTTP handlers and the dashboard.
#[derive(Clone)]
pub struct AppState {
    pub rooms: Arc<DashMap<String, Room>>,
}

impl AppState {
    pub fn new() -> Self {
        Self { rooms: Arc::new(DashMap::new()) }
    }

    pub fn get_or_create(&self, id: &str) -> Room {
        self.rooms
            .entry(id.to_string())
            .or_insert_with(Room::new)
            .clone()
    }
}

/// Per-peer server-side state
pub struct PeerState {
    pub pc:        Arc<RTCPeerConnection>,
    /// Channel to push encoded JSON messages toward the browser
    pub ws_tx:     mpsc::UnboundedSender<String>,
    /// Send renegotiation answers here (from WS handler → renegotiate())
    pub answer_tx: mpsc::Sender<String>,
    /// Receive renegotiation answers (used inside renegotiate())
    pub answer_rx: Arc<Mutex<mpsc::Receiver<String>>>,
    /// Total RTP payload bytes received from this peer (monotonically increasing)
    pub bytes_up:  Arc<AtomicU64>,
}

impl PeerState {
    pub fn load_bytes_up(&self) -> u64 {
        self.bytes_up.load(Ordering::Relaxed)
    }
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
