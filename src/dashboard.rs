use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use webrtc::track::track_local::TrackLocal;

use crate::room::AppState;

const R: &str = "\x1B[0m";
const B: &str = "\x1B[1m";
const D: &str = "\x1B[2m";
const G: &str = "\x1B[32m";
const Y: &str = "\x1B[33m";
const C: &str = "\x1B[36m";
const SEP: &str = "────────────────────────────────────────────────";

fn fmt_rate(bps: u64) -> String {
    let kbps = bps * 8 / 1000;
    if kbps >= 1000 { format!("{:.1} Mbps", kbps as f64 / 1000.0) }
    else             { format!("{kbps} kbps") }
}

fn redraw(state: &AppState, port: &str, prev: &mut HashMap<String, u64>) {
    let total_peers: usize = state.rooms.iter().map(|r| r.peers.len()).sum();
    let mut out = "\x1B[2J\x1B[H".to_string(); // clear + home

    out += &format!("{B}rchat{R}  {D}http://localhost:{port}{R}\n");
    out += &format!("{D}{SEP}{R}\n");
    out += &format!("  rooms {B}{}{R}   peers {B}{total_peers}{R}\n", state.rooms.len());
    out += &format!("{D}{SEP}{R}\n");

    if state.rooms.is_empty() {
        out += &format!("{D}  (no active rooms){R}\n");
    }

    for re in state.rooms.iter() {
        let (rid, room) = (re.key(), re.value());

        let mut peers: Vec<_> = room.peers.iter()
            .map(|pe| (pe.key().clone(), Arc::clone(pe.value())))
            .collect();
        peers.sort_by(|a, b| a.0.cmp(&b.0));

        // Compute per-peer rates and room total
        let mut room_bps: u64 = 0;
        let mut peer_rates: Vec<u64> = Vec::new();
        for (uid, peer) in &peers {
            let key      = format!("{rid}/{uid}");
            let now      = peer.load_bytes_up();
            let prev_val = prev.get(&key).copied().unwrap_or(now);
            let bps      = now.saturating_sub(prev_val);
            prev.insert(key, now);
            room_bps += bps;
            peer_rates.push(bps);
        }

        let room_rate = if room_bps > 0 {
            format!("{G}↑ {}{R}", fmt_rate(room_bps))
        } else {
            format!("{D}↑ 0 kbps{R}")
        };

        out += &format!("\n  {C}{B}{rid}{R}  {D}({} peer)  {R}{room_rate}\n", peers.len());

        for ((uid, peer), bps) in peers.iter().zip(peer_rates.iter()) {
            let cs     = format!("{:?}", peer.pc.connection_state()).to_lowercase();
            let cs_col = if cs == "connected" { G } else { D };

            let (mut has_a, mut has_v) = (false, false);
            if let Some(tracks) = room.tracks.get(uid.as_str()) {
                for t in tracks.iter() {
                    if t.id().starts_with("audio") { has_a = true; }
                    if t.id().starts_with("video") { has_v = true; }
                }
            }
            let media = match (has_a, has_v) {
                (true,  true)  => format!("{G}a+v{R}"),
                (true,  false) => format!("{Y}aud{R}"),
                (false, true)  => format!("{Y}vid{R}"),
                _              => format!("{D} — {R}"),
            };

            let rate_col = if *bps > 0 { G } else { D };

            out += &format!(
                "    {D}·{R} {B}{uid}{R}  {media}  {rate_col}↑ {:<12}{R}  {cs_col}{cs}{R}\n",
                fmt_rate(*bps),
            );
        }
    }

    out += &format!("\n{D}{SEP}\n  logs → stderr  ·  Ctrl-C to quit{R}\n");

    print!("{out}");
    let _ = std::io::stdout().flush();
}

pub fn spawn(state: AppState, port: String) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(1));
        let mut prev: HashMap<String, u64> = HashMap::new();
        loop {
            tick.tick().await;
            redraw(&state, &port, &mut prev);
        }
    });
}
