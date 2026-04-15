use std::sync::Arc;

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

type UserId = String;

#[derive(Clone)]
struct Room {
    users: Arc<DashMap<UserId, mpsc::UnboundedSender<String>>>,
}

impl Room {
    fn new() -> Self {
        Self { users: Arc::new(DashMap::new()) }
    }
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<DashMap<String, Room>>,
}

#[derive(Serialize)]
struct RoomCreated {
    room_id: String,
}

const INDEX_HTML: &str = include_str!("../static/index.html");

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn create_room(State(state): State<AppState>) -> Json<RoomCreated> {
    let id = Uuid::new_v4().to_string()[..8].to_string();
    state.rooms.insert(id.clone(), Room::new());
    Json(RoomCreated { room_id: id })
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Path(room_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, room_id, state))
}

async fn handle_ws(socket: WebSocket, room_id: String, state: AppState) {
    let uid = Uuid::new_v4().to_string()[..8].to_string();

    // Get or create room; snapshot existing users before inserting self
    let room = {
        state.rooms.entry(room_id.clone()).or_insert_with(Room::new);
        state.rooms.get(&room_id).unwrap().clone()
    };

    let existing: Vec<String> = room.users.iter().map(|e| e.key().clone()).collect();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    room.users.insert(uid.clone(), tx);

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Greet new user
    let _ = ws_tx.send(Message::Text(
        serde_json::json!({"type": "you-are", "userId": uid}).to_string(),
    )).await;
    let _ = ws_tx.send(Message::Text(
        serde_json::json!({"type": "existing-users", "users": existing}).to_string(),
    )).await;

    // Notify others
    for entry in room.users.iter() {
        if entry.key().as_str() != uid {
            let _ = entry.value().send(
                serde_json::json!({"type": "user-joined", "userId": uid}).to_string(),
            );
        }
    }

    // Forward channel messages → WebSocket
    let mut fwd = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Relay WebSocket messages to target peers
    let room2 = room.clone();
    let uid2 = uid.clone();
    let mut recv = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if let Message::Text(text) = msg {
                if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) {
                    let to = v["to"].as_str().map(|s| s.to_owned());
                    v["from"] = serde_json::json!(uid2);
                    if let Some(target) = to {
                        if let Some(peer_tx) = room2.users.get(&target) {
                            let _ = peer_tx.send(v.to_string());
                        }
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = &mut fwd  => recv.abort(),
        _ = &mut recv => fwd.abort(),
    }

    // Cleanup
    room.users.remove(&uid);
    for entry in room.users.iter() {
        let _ = entry.value().send(
            serde_json::json!({"type": "user-left", "userId": uid}).to_string(),
        );
    }
    if room.users.is_empty() {
        state.rooms.remove(&room_id);
    }
}

#[tokio::main]
async fn main() {
    let state = AppState { rooms: Arc::new(DashMap::new()) };

    let app = Router::new()
        .route("/", get(index))
        .route("/room/:id", get(index))
        .route("/api/rooms", post(create_room))
        .route("/ws/:room_id", get(ws_upgrade))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("rchat → http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}
