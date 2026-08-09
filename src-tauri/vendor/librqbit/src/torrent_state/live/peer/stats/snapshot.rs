use std::{collections::HashMap, sync::atomic::Ordering};

use serde::{Deserialize, Serialize};

use crate::torrent_state::live::peer::{Peer, PeerState};

#[derive(Serialize, Deserialize)]
pub struct PeerCounters {
    pub incoming_connections: u32,
    pub fetched_bytes: u64,
    /// LOCAL PATCH: see `PeerCountersAtomic::uploaded_bytes`.
    pub uploaded_bytes: u64,
    pub total_time_connecting_ms: u64,
    pub connection_attempts: u32,
    pub connections: u32,
    pub errors: u32,
    pub fetched_chunks: u32,
    pub downloaded_and_checked_pieces: u32,
    pub total_piece_download_ms: u64,
    pub times_stolen_from_me: u32,
    pub times_i_stole: u32,
}

#[derive(Serialize, Deserialize)]
pub struct PeerStats {
    pub counters: PeerCounters,
    pub state: &'static str,
    /// LOCAL PATCH: everything below is tracked by `LivePeerState` but never
    /// surfaced upstream — `peer_id` is even marked `#[allow(dead_code)]`.
    /// Without it a peer list can only show an address and a state name.
    ///
    /// Raw 20-byte peer id, hex encoded. Decode with
    /// `librqbit_core::peer_id::try_decode_peer_id` for a client name.
    pub peer_id: Option<String>,
    /// Fraction of the torrent this peer holds, 0.0..=1.0.
    pub progress: Option<f64>,
    /// Whether the peer is interested in what we have.
    pub interested: Option<bool>,
    /// Requests we're currently waiting on from this peer.
    pub inflight: Option<usize>,
}

impl From<&super::atomic::PeerCountersAtomic> for PeerCounters {
    fn from(counters: &super::atomic::PeerCountersAtomic) -> Self {
        Self {
            incoming_connections: counters.incoming_connections.load(Ordering::Relaxed),
            fetched_bytes: counters.fetched_bytes.load(Ordering::Relaxed),
            uploaded_bytes: counters.uploaded_bytes.load(Ordering::Relaxed),
            total_time_connecting_ms: counters.total_time_connecting_ms.load(Ordering::Relaxed),
            connection_attempts: counters
                .outgoing_connection_attempts
                .load(Ordering::Relaxed),
            connections: counters.outgoing_connections.load(Ordering::Relaxed),
            errors: counters.errors.load(Ordering::Relaxed),
            fetched_chunks: counters.fetched_chunks.load(Ordering::Relaxed),
            downloaded_and_checked_pieces: counters
                .downloaded_and_checked_pieces
                .load(Ordering::Relaxed),
            total_piece_download_ms: counters.total_piece_download_ms.load(Ordering::Relaxed),
            times_i_stole: counters.times_i_stole.load(Ordering::Relaxed),
            times_stolen_from_me: counters.times_stolen_from_me.load(Ordering::Relaxed),
        }
    }
}

impl From<&Peer> for PeerStats {
    fn from(peer: &Peer) -> Self {
        // LOCAL PATCH: pull the live-only detail out when the peer is live.
        let live = match peer.get_state() {
            PeerState::Live(l) => Some(l),
            _ => None,
        };
        Self {
            counters: peer.stats.counters.as_ref().into(),
            state: peer.get_state().name(),
            peer_id: live.map(|l| l.peer_id_hex()),
            progress: live.map(|l| {
                let total = l.bitfield.len();
                if total == 0 {
                    0.0
                } else {
                    l.bitfield.count_ones() as f64 / total as f64
                }
            }),
            interested: live.map(|l| l.peer_interested),
            inflight: live.map(|l| l.inflight_requests.len()),
        }
    }
}

#[derive(Serialize)]
pub struct PeerStatsSnapshot {
    pub peers: HashMap<String, PeerStats>,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub enum PeerStatsFilterState {
    All,
    #[default]
    Live,
}

impl PeerStatsFilterState {
    pub(crate) fn matches(&self, s: &PeerState) -> bool {
        matches!((self, s), (Self::All, _) | (Self::Live, PeerState::Live(_)))
    }
}

#[derive(Default, Deserialize)]
pub struct PeerStatsFilter {
    pub state: PeerStatsFilterState,
}
