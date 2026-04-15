mod peer;
mod room;
mod signal;

use std::sync::Arc;

use axum::{
    Router,
    extract::{Path, State, WebSocketUpgrade},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
};
use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    peer::setup_peer,
    room::Room,
    signal::{ClientMsg, ServerMsg},
};

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    rooms: Arc<DashMap<String, Room>>,
}

impl AppState {
    fn get_or_create(&self, id: &str) -> Room {
        self.rooms
            .entry(id.to_string())
            .or_insert_with(Room::new)
            .clone()
    }
}

// ── HTTP handlers ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct RoomCreated { room_id: String }

const INDEX_HTML: &str = include_str!("../static/index.html");

async fn index() -> Html<&'static str> { Html(INDEX_HTML) }

async fn create_room(State(s): State<AppState>) -> Json<RoomCreated> {
    let id = Uuid::new_v4().to_string()[..8].to_string();
    s.rooms.insert(id.clone(), Room::new());
    Json(RoomCreated { room_id: id })
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, room_id, state))
}

// ── WebSocket / signaling handler ─────────────────────────────────────────────

async fn handle_ws(socket: WebSocket, room_id: String, state: AppState) {
    let uid  = Uuid::new_v4().to_string()[..8].to_string();
    let room = state.get_or_create(&room_id);

    let (mut ws_sink, mut ws_stream) = socket.split();
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<String>();

    // Pump outgoing channel → WebSocket
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if ws_sink.send(Message::Text(msg)).await.is_err() { break; }
        }
    });

    // Tell browser its assigned ID
    ws_tx.send(
        serde_json::to_string(&ServerMsg::YouAre { user_id: uid.clone() }).unwrap()
    ).ok();

    // Wait for the browser's initial offer
    let offer_sdp = loop {
        match ws_stream.next().await {
            Some(Ok(Message::Text(txt))) => {
                if let Ok(ClientMsg::Offer { sdp }) = serde_json::from_str(&txt) {
                    break sdp;
                }
            }
            _ => return,
        }
    };

    // Build server-side PeerConnection + send answer
    match setup_peer(uid.clone(), offer_sdp, room.clone(), ws_tx.clone()).await {
        Ok(answer_sdp) => {
            ws_tx.send(
                serde_json::to_string(&ServerMsg::Answer { sdp: answer_sdp }).unwrap()
            ).ok();
        }
        Err(e) => { eprintln!("peer setup error for {uid}: {e:#}"); return; }
    }

    // Process ICE candidates and renegotiation answers
    while let Some(Ok(Message::Text(txt))) = ws_stream.next().await {
        let Ok(msg) = serde_json::from_str::<ClientMsg>(&txt) else { continue };

        match msg {
            ClientMsg::IceCandidate { candidate } => {
                if let Some(peer) = room.peers.get(&uid) {
                    peer.pc.add_ice_candidate(candidate.into()).await.ok();
                }
            }
            ClientMsg::Answer { sdp } => {
                if let Some(peer) = room.peers.get(&uid) {
                    peer.answer_tx.send(sdp).await.ok();
                }
            }
            ClientMsg::Offer { .. } => {} // unexpected after setup
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────────
    // Notify remaining peers before removing
    for entry in room.peers.iter() {
        if entry.key().as_str() != uid {
            let msg = serde_json::to_string(
                &ServerMsg::UserLeft { user_id: uid.clone() }
            ).unwrap();
            entry.value().ws_tx.send(msg).ok();
        }
    }

    room.remove(&uid);

    if room.peers.is_empty() {
        state.rooms.remove(&room_id);
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let state = AppState { rooms: Arc::new(DashMap::new()) };

    let app = Router::new()
        .route("/",               get(index))
        .route("/room/:id",       get(index))
        .route("/api/rooms",      post(create_room))
        .route("/ws/:room_id",    get(ws_upgrade))
        .with_state(state);

    let port     = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await.unwrap();
    println!("rchat → http://localhost:{port}");
    axum::serve(listener, app).await.unwrap();
}
