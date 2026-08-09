use std::sync::atomic::AtomicU64;

use crate::torrent_state::live::peers::stats::atomic::AggregatePeerStatsAtomic;

#[derive(Default, Debug)]
pub struct AtomicSessionStats {
    pub fetched_bytes: AtomicU64,
    pub uploaded_bytes: AtomicU64,
    /// LOCAL PATCH: inbound peer connections successfully handed to a torrent
    /// since session start. Non-zero means our listen port is reachable from
    /// the internet, i.e. we are not firewalled. Equivalent to libtorrent's
    /// `session_status::has_incoming_connections`, which upstream librqbit
    /// tracks internally but does not expose.
    pub incoming_connections: AtomicU64,
    pub(crate) peers: AggregatePeerStatsAtomic,
}
