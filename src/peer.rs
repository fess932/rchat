use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use webrtc::{
    api::{media_engine::MediaEngine, APIBuilder},
    ice_transport::ice_candidate::RTCIceCandidate,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
    },
    rtp_transceiver::rtp_codec::RTPCodecType,
    track::track_local::{track_local_static_rtp::TrackLocalStaticRTP, TrackLocalWriter},
};

use crate::{
    room::{PeerState, Room, UserId},
    signal::{IceCandidateInit, ServerMsg},
};

fn build_api() -> Result<webrtc::api::API> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    Ok(APIBuilder::new().with_media_engine(m).build())
}

/// Set up a server-side RTCPeerConnection for a new participant.
///
/// 1. Creates a PeerConnection on the server.
/// 2. Adds existing relay tracks (other users' audio/video) to it.
/// 3. Accepts the client's offer and returns an SDP answer.
/// 4. Registers the peer in the room and starts forwarding incoming tracks.
pub async fn setup_peer(
    uid: UserId,
    offer_sdp: String,
    room: Room,
    ws_tx: mpsc::UnboundedSender<String>,
) -> Result<String> {
    let pc = Arc::new(
        build_api()?
            .new_peer_connection(RTCConfiguration::default())
            .await?,
    );

    // Send existing participants' relay tracks to the new peer
    for track in room.relay_tracks_except(&uid) {
        pc.add_track(Arc::clone(&track) as Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync>)
            .await?;
    }

    // Accept the client's offer
    pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?).await?;

    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;

    // Trickle ICE: forward server candidates to the browser
    {
        let ws = ws_tx.clone();
        pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
            let ws = ws.clone();
            Box::pin(async move {
                if let Some(c) = c {
                    let Ok(json) = c.to_json() else { return };
                    let init: IceCandidateInit = json.into();
                    let msg =
                        serde_json::to_string(&ServerMsg::IceCandidate { candidate: init })
                            .unwrap();
                    ws.send(msg).ok();
                }
            })
        }));
    }

    // When the browser sends us a media track — create a relay and fan it out
    {
        let room = room.clone();
        let uid = uid.clone();

        pc.on_track(Box::new(move |track, _, _| {
            let room = room.clone();
            let uid  = uid.clone();

            Box::pin(async move {
                let kind_str = match track.kind() {
                    RTPCodecType::Audio => "audio",
                    RTPCodecType::Video => "video",
                    _ => return,
                };

                // Relay track: same codec, ID encodes owner for the browser
                let relay = Arc::new(TrackLocalStaticRTP::new(
                    track.codec().capability.clone(),
                    format!("{kind_str}-{uid}"),   // track id
                    format!("user-{uid}"),          // stream id (browser uses this)
                ));

                room.tracks
                    .entry(uid.clone())
                    .or_insert_with(Vec::new)
                    .push(Arc::clone(&relay));

                // Add relay to every other peer and renegotiate
                for entry in room.peers.iter() {
                    if entry.key().as_str() == uid { continue; }
                    let peer  = Arc::clone(entry.value());
                    let relay = Arc::clone(&relay);
                    tokio::spawn(async move {
                        if peer
                            .pc
                            .add_track(
                                relay as Arc<dyn webrtc::track::track_local::TrackLocal + Send + Sync>,
                            )
                            .await
                            .is_ok()
                        {
                            if let Err(e) = renegotiate(&peer).await {
                                eprintln!("renegotiate error: {e:#}");
                            }
                        }
                    });
                }

                // RTP forwarding loop — copy packets verbatim to relay
                while let Ok((pkt, _)) = track.read_rtp().await {
                    if relay.write_rtp(&pkt).await.is_err() {
                        break;
                    }
                }
            })
        }));
    }

    // Register peer state (answer channel used for later renegotiations)
    let (answer_tx, answer_rx) = mpsc::channel::<String>(4);
    room.peers.insert(
        uid,
        Arc::new(PeerState {
            pc: Arc::clone(&pc),
            ws_tx,
            answer_tx,
            answer_rx: Arc::new(tokio::sync::Mutex::new(answer_rx)),
        }),
    );

    Ok(answer.sdp)
}

/// Server-initiated renegotiation (called when a new track is added to an existing PC).
pub async fn renegotiate(peer: &PeerState) -> Result<()> {
    let offer = peer.pc.create_offer(None).await?;
    peer.pc.set_local_description(offer.clone()).await?;

    peer.ws_tx.send(
        serde_json::to_string(&ServerMsg::Offer { sdp: offer.sdp })?,
    )?;

    // Wait for the browser's answer (serialised by the Mutex)
    let mut rx = peer.answer_rx.lock().await;
    let sdp = rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("answer channel closed"))?;

    peer.pc
        .set_remote_description(RTCSessionDescription::answer(sdp)?)
        .await?;

    Ok(())
}
