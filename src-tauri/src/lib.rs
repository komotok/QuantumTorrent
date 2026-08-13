mod preview;
mod search;

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::path::BaseDirectory;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_deep_link::DeepLinkExt;

use librqbit::api::TorrentIdOrHash;
use librqbit::limits::LimitsConfig;
use librqbit_core::hash_id::Id20;
use librqbit_core::peer_id::{
    generate_azereus_style, try_decode_peer_id, AzureusStyleKind, PeerId,
};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions,
    SessionPersistenceConfig, TorrentStatsState,
};

struct PendingTorrent {
    torrent_bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    listen_port: Option<u16>,
    bind_ip: Option<String>,
    upnp: bool,
    units: Units,
    geoip_path: Option<String>,
    download_limit: Option<u32>,
    upload_limit: Option<u32>,
    #[serde(default)]
    disabled_plugins: Vec<String>,
    #[serde(default)]
    compact: bool,
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default)]
    seed_ratio_limit: Option<f32>,
    #[serde(default)]
    seed_time_limit: Option<u32>,
    #[serde(default)]
    max_active_downloads: Option<u32>,
    #[serde(default)]
    minimize_to_tray: bool,
}

fn default_theme() -> String {
    "default".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            listen_port: None,
            bind_ip: None,
            upnp: true,
            units: Units::default(),
            geoip_path: None,
            download_limit: None,
            upload_limit: None,
            disabled_plugins: Vec::new(),
            compact: false,
            theme: default_theme(),
            seed_ratio_limit: None,
            seed_time_limit: None,
            max_active_downloads: None,
            minimize_to_tray: false,
        }
    }
}

fn apply_limits(session: &Session, settings: &Settings) {
    let limits = settings.limits();
    session.ratelimits.set_download_bps(limits.download_bps);
    session.ratelimits.set_upload_bps(limits.upload_bps);
}

fn kib_to_bps(kib: Option<u32>) -> Option<NonZeroU32> {
    kib.filter(|v| *v > 0)
        .and_then(|v| NonZeroU32::new(v.saturating_mul(1024)))
}

impl Settings {
    fn limits(&self) -> LimitsConfig {
        LimitsConfig {
            download_bps: kib_to_bps(self.download_limit),
            upload_bps: kib_to_bps(self.upload_limit),
        }
    }

    fn parsed_bind_ip(&self) -> Option<IpAddr> {
        self.bind_ip.as_ref().and_then(|s| s.parse().ok())
    }

    fn network_differs(&self, other: &Settings) -> bool {
        self.listen_port != other.listen_port
            || self.bind_ip != other.bind_ip
            || self.upnp != other.upnp
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NetworkInterface {
    name: String,
    ip: String,
    is_loopback: bool,
}

struct AppState {
    session: RwLock<Arc<Session>>,
    geoip: RwLock<Option<Arc<maxminddb::Reader<Vec<u8>>>>>,
    geoip_bundled: RwLock<bool>,
    bundled_geoip_path: RwLock<Option<String>>,
    settings: RwLock<Settings>,
    notice: RwLock<Option<String>>,
    bind_lost_since: RwLock<Option<Instant>>,
    settings_path: PathBuf,
    download_dir: PathBuf,
    pending: Mutex<HashMap<String, PendingTorrent>>,
    inbox: Mutex<Vec<String>>,
    queued: Mutex<HashSet<usize>>,
    seed_done: Mutex<HashSet<usize>>,
    seed_since: Mutex<HashMap<usize, Instant>>,
    quitting: AtomicBool,
    preview: tokio::sync::Mutex<Option<preview::PreviewServer>>,
}

impl AppState {
    fn session(&self) -> Arc<Session> {
        self.session.read().unwrap().clone()
    }

    fn units(&self) -> Units {
        self.settings.read().unwrap().units
    }

    fn notice(&self) -> Option<String> {
        self.notice.read().unwrap().clone()
    }

    fn limits(&self) -> (Option<u32>, Option<u32>) {
        let s = self.settings.read().unwrap();
        (
            s.download_limit.filter(|v| *v > 0),
            s.upload_limit.filter(|v| *v > 0),
        )
    }

    /// Reloads only when the configured path changes.
    fn load_geoip(&self, path: Option<&str>) {
        let mut slot = self.geoip.write().unwrap();
        match path {
            None => *slot = None,
            Some(p) => match maxminddb::Reader::open_readfile(p) {
                Ok(r) => *slot = Some(Arc::new(r)),
                Err(e) => {
                    eprintln!("[qtorrent] could not open GeoIP database {p}: {e}");
                    *slot = None;
                }
            },
        }
    }

    fn geoip(&self) -> Option<Arc<maxminddb::Reader<Vec<u8>>>> {
        self.geoip.read().unwrap().clone()
    }

    fn bind_lost_for(&self) -> Option<Duration> {
        self.bind_lost_since.read().unwrap().map(|t| t.elapsed())
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TorrentInfo {
    id: usize,
    name: String,
    size: String,
    progress: f64,
    status: String,
    down: String,
    up: String,
    eta: String,
    error: Option<String>,
    size_bytes: u64,
    down_bps: f64,
    up_bps: f64,
    eta_secs: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    index: usize,
    path: String,
    size: u64,
    size_str: String,
    selected: bool,
    padding: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SessionStatus {
    state: String,
    detail: String,
    listen_port: Option<u16>,
    dht_nodes: Option<usize>,
    peers: usize,
    incoming: u64,
    down_speed: String,
    up_speed: String,
    downloaded: String,
    uploaded: String,
    notice: Option<String>,
    down_limit: Option<u32>,
    up_limit: Option<u32>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TorrentPreview {
    info_hash: String,
    name: String,
    total_size: u64,
    total_size_str: String,
    output_folder: String,
    files: Vec<FileEntry>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
enum Units {
    #[default]
    Binary,
    Decimal,
}

impl Units {
    fn scale(self) -> (f64, [&'static str; 5]) {
        match self {
            Units::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB"]),
            Units::Decimal => (1000.0, ["B", "KB", "MB", "GB", "TB"]),
        }
    }
}

/// Decimal places per unit index, preserving the existing conventions:
/// bytes whole, KiB/MiB to one place, GiB/TiB to two.
const SIZE_DECIMALS: [usize; 5] = [0, 1, 1, 2, 2];

fn format_size(bytes: u64, units: Units) -> String {
    let (k, n) = units.scale();
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= k && i + 1 < n.len() {
        v /= k;
        i += 1;
    }
    // Rounding can push a value up into the next unit: 1048575 bytes is
    // 1023.999 KiB, which renders as "1024.0 KiB" unless we step up here.
    let f = 10f64.powi(SIZE_DECIMALS[i] as i32);
    if (v * f).round() / f >= k && i + 1 < n.len() {
        v /= k;
        i += 1;
    }
    format!("{:.*} {}", SIZE_DECIMALS[i], v, n[i])
}

fn format_speed(mbps: f64, units: Units) -> String {
    let (k, n) = units.scale();
    let mut v = mbps * 1_048_576.0;
    let mut i = 0;
    while v >= k && i + 2 < n.len() {
        v /= k;
        i += 1;
    }
    let f = 10f64.powi(SIZE_DECIMALS[i] as i32);
    if (v * f).round() / f >= k && i + 2 < n.len() {
        v /= k;
        i += 1;
    }
    format!("{:.*} {}/s", SIZE_DECIMALS[i], v, n[i])
}

fn torrent_to_info(
    id: usize,
    handle: &Arc<ManagedTorrent>,
    units: Units,
    queued: &HashSet<usize>,
) -> TorrentInfo {
    let stats = handle.stats();
    let name = handle.name().unwrap_or_else(|| format!("Torrent #{}", id));

    let status = match stats.state {
        TorrentStatsState::Initializing => "checking",
        TorrentStatsState::Live => {
            if stats.finished { "seeding" } else { "downloading" }
        }
        TorrentStatsState::Paused => {
            if queued.contains(&id) { "queued" } else { "paused" }
        }
        TorrentStatsState::Error => "error",
    };

    let progress = if stats.total_bytes > 0 {
        (stats.progress_bytes as f64 / stats.total_bytes as f64) * 100.0
    } else {
        0.0
    };

    let down = format_speed(stats.live.as_ref().map(|l| l.download_speed.mbps).unwrap_or(0.0), units);
    let up = format_speed(stats.live.as_ref().map(|l| l.upload_speed.mbps).unwrap_or(0.0), units);
    let eta = stats
        .live
        .as_ref()
        .and_then(|l| l.time_remaining.as_ref())
        .map(|d| d.to_string())
        .unwrap_or_else(|| "—".to_string());

    let down_bps = stats.live.as_ref().map(|l| l.download_speed.mbps).unwrap_or(0.0) * 1_048_576.0;
    let up_bps = stats.live.as_ref().map(|l| l.upload_speed.mbps).unwrap_or(0.0) * 1_048_576.0;
    let eta_secs = if down_bps > 0.0 && stats.total_bytes > stats.progress_bytes {
        Some(((stats.total_bytes - stats.progress_bytes) as f64 / down_bps) as u64)
    } else {
        None
    };

    TorrentInfo {
        id,
        name,
        size: format_size(stats.total_bytes, units),
        progress,
        status: status.to_string(),
        down,
        up,
        eta,
        error: stats.error,
        size_bytes: stats.total_bytes,
        down_bps,
        up_bps,
        eta_secs,
    }
}

fn source_to_add_torrent(source: &str) -> Result<AddTorrent<'static>, String> {
    if source.starts_with("magnet:")
        || source.starts_with("http://")
        || source.starts_with("https://")
    {
        Ok(AddTorrent::from_url(source.to_string()))
    } else {
        AddTorrent::from_local_filename(source)
            .map_err(|e| format!("Could not read torrent file: {e}"))
    }
}

#[tauri::command]
async fn preview_torrent(
    state: State<'_, AppState>,
    source: String,
) -> Result<TorrentPreview, String> {
    let units = state.units();
    let add = source_to_add_torrent(&source)?;
    let opts = AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };

    let response = state
        .session()
        .add_torrent(add, Some(opts))
        .await
        .map_err(|e| format!("{e:#}"))?;

    let lo = match response {
        AddTorrentResponse::ListOnly(lo) => lo,
        AddTorrentResponse::AlreadyManaged(_, _) => {
            return Err("This torrent is already in the list.".to_string());
        }
        AddTorrentResponse::Added(_, _) => {
            return Err("Unexpected: torrent was started during preview.".to_string());
        }
    };

    let mut files = Vec::new();
    let mut total_size = 0u64;
    let details = lo
        .info
        .iter_file_details()
        .map_err(|e| format!("Could not read file list: {e}"))?;
    for (index, fd) in details.enumerate() {
        let path = fd
            .filename
            .to_string()
            .unwrap_or_else(|_| format!("file_{index}"));
        let padding = fd.attrs().padding;
        total_size += fd.len;
        files.push(FileEntry {
            index,
            path,
            size: fd.len,
            size_str: format_size(fd.len, units),
            selected: !padding,
            padding,
        });
    }

    let name = lo
        .info
        .name
        .as_ref()
        .map(|n| String::from_utf8_lossy(n.as_ref()).to_string())
        .unwrap_or_else(|| "Unnamed torrent".to_string());

    let info_hash = lo.info_hash.as_string();

    state.pending.lock().unwrap().insert(
        info_hash.clone(),
        PendingTorrent {
            torrent_bytes: lo.torrent_bytes.to_vec(),
        },
    );

    Ok(TorrentPreview {
        info_hash,
        name,
        total_size,
        total_size_str: format_size(total_size, units),
        output_folder: lo.output_folder.display().to_string(),
        files,
    })
}

#[tauri::command]
async fn add_prepared_torrent(
    state: State<'_, AppState>,
    info_hash: String,
    only_files: Vec<usize>,
    output_folder: String,
) -> Result<usize, String> {
    if only_files.is_empty() {
        return Err("Select at least one file.".to_string());
    }

    let bytes = {
        let mut pending = state.pending.lock().unwrap();
        pending
            .remove(&info_hash)
            .ok_or_else(|| "This torrent is no longer staged — please add it again.".to_string())?
            .torrent_bytes
    };

    let opts = AddTorrentOptions {
        only_files: Some(only_files),
        output_folder: Some(output_folder),
        overwrite: true,
        ..Default::default()
    };

    let response = state
        .session()
        .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
        .await
        .map_err(|e| format!("{e:#}"))?;

    match response {
        AddTorrentResponse::Added(id, _) => Ok(id),
        AddTorrentResponse::AlreadyManaged(id, _) => Ok(id),
        AddTorrentResponse::ListOnly(_) => Err("Unexpected list-only response.".to_string()),
    }
}

#[tauri::command]
async fn cancel_preview(state: State<'_, AppState>, info_hash: String) -> Result<(), String> {
    state.pending.lock().unwrap().remove(&info_hash);
    Ok(())
}

#[tauri::command]
async fn get_torrent_files(
    state: State<'_, AppState>,
    id: usize,
) -> Result<Vec<FileEntry>, String> {
    let units = state.units();
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let metadata = handle.metadata.load();
    let metadata = metadata
        .as_ref()
        .ok_or_else(|| "Metadata not resolved yet — still checking.".to_string())?;

    let only: Option<HashSet<usize>> = handle
        .only_files()
        .map(|v| v.into_iter().collect());

    Ok(metadata
        .file_infos
        .iter()
        .enumerate()
        .map(|(index, fi)| FileEntry {
            index,
            path: fi.relative_filename.display().to_string(),
            size: fi.len,
            size_str: format_size(fi.len, units),
            selected: only.as_ref().map(|s| s.contains(&index)).unwrap_or(true),
            padding: fi.attrs.padding,
        })
        .collect())
}

#[tauri::command]
async fn preview_file(
    app: AppHandle,
    state: State<'_, AppState>,
    id: usize,
    file_index: usize,
) -> Result<String, String> {
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let name = {
        let metadata = handle.metadata.load();
        let metadata = metadata
            .as_ref()
            .ok_or_else(|| "Metadata not resolved yet — still checking.".to_string())?;
        let fi = metadata
            .file_infos
            .get(file_index)
            .ok_or_else(|| format!("File {file_index} not found"))?;
        fi.relative_filename
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string())
    };

    let mut guard = state.preview.lock().await;
    if guard.is_none() {
        let resolver: preview::Resolver = Arc::new(move |id| {
            app.try_state::<AppState>()
                .and_then(|s| s.session().get(TorrentIdOrHash::Id(id)))
        });
        *guard = Some(
            preview::start(resolver)
                .await
                .map_err(|e| format!("Could not start preview server: {e:#}"))?,
        );
    }
    let server = guard.as_ref().unwrap();

    Ok(format!(
        "http://127.0.0.1:{}/{}/{}/{}/{}",
        server.port,
        server.token,
        id,
        file_index,
        urlencoding_path(&name)
    ))
}

/// Percent-encodes anything that would break the path segment. The filename is
/// there so players can see an extension and pick a demuxer.
fn urlencoding_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[tauri::command]
async fn update_torrent_files(
    state: State<'_, AppState>,
    id: usize,
    only_files: Vec<usize>,
) -> Result<(), String> {
    if only_files.is_empty() {
        return Err("Select at least one file.".to_string());
    }
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let set: HashSet<usize> = only_files.into_iter().collect();
    state
        .session()
        .update_only_files(&handle, &set)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn get_torrents(state: State<'_, AppState>) -> Result<Vec<TorrentInfo>, String> {
    let units = state.units();
    let queued = state.queued.lock().unwrap().clone();
    Ok(state.session().with_torrents(|iter| {
        iter.map(|(id, handle)| torrent_to_info(id, handle, units, &queued))
            .collect::<Vec<_>>()
    }))
}

const FIREWALL_GRACE_SECS: u64 = 300;

const CONNECTING_GRACE_SECS: u64 = 60;

const BIND_DROPOUT_GRACE_SECS: u64 = 25;

const BIND_POLL: Duration = Duration::from_secs(3);

const MANAGE_POLL: Duration = Duration::from_secs(5);

const LISTEN_PORTS: std::ops::Range<u16> = 6881..6891;

/// Azureus-style version digits for the peer id. "qT" is unregistered - BEP-20
/// has no registry, clients just pick an unused pair.
const CLIENT_VERSION: (u8, u8, u8, u8) = (
    parse_u8(env!("CARGO_PKG_VERSION_MAJOR")),
    parse_u8(env!("CARGO_PKG_VERSION_MINOR")),
    parse_u8(env!("CARGO_PKG_VERSION_PATCH")),
    0,
);

const fn parse_u8(s: &str) -> u8 {
    let b = s.as_bytes();
    let mut n = 0u8;
    let mut i = 0;
    while i < b.len() {
        n = n.saturating_mul(10).saturating_add(b[i] - b'0');
        i += 1;
    }
    n
}

fn session_options(settings: &Settings, listen: bool) -> SessionOptions {
    let range = match settings.listen_port {
        Some(p) => p..p.saturating_add(1),
        None => LISTEN_PORTS,
    };
    SessionOptions {
        persistence: Some(SessionPersistenceConfig::Json { folder: None }),
        fastresume: true,
        listen_port_range: if listen { Some(range) } else { None },
        enable_upnp_port_forwarding: listen && settings.upnp,
        bind_ip: settings.parsed_bind_ip(),
        // Identify as this app rather than inheriting librqbit's "-rQ" id.
        // The version comes from OUR Cargo.toml: crate_version! inside
        // librqbit-core would report librqbit's version instead.
        peer_id: Some(generate_azereus_style(*b"qT", CLIENT_VERSION)),
        user_agent: Some(format!("quantum-torrent/{}", env!("CARGO_PKG_VERSION"))),
        ratelimits: settings.limits(),
        ..Default::default()
    }
}

fn reveal_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show QuantumTorrent", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("QuantumTorrent")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_window(app),
            "quit" => {
                if let Some(state) = app.try_state::<AppState>() {
                    state.quitting.store(true, Ordering::SeqCst);
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

struct TorrentSnap {
    id: usize,
    handle: Arc<ManagedTorrent>,
    finished: bool,
    paused: bool,
    live: bool,
    uploaded: u64,
    total: u64,
}

fn spawn_torrent_manager<R: Runtime>(handle: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(MANAGE_POLL);
        loop {
            tick.tick().await;
            let Some(state) = handle.try_state::<AppState>() else { continue };

            let settings = state.settings.read().unwrap().clone();
            let session = state.session();

            let snaps: Vec<TorrentSnap> = session.with_torrents(|iter| {
                iter.map(|(id, h)| {
                    let s = h.stats();
                    TorrentSnap {
                        id,
                        handle: h.clone(),
                        finished: s.finished,
                        paused: matches!(s.state, TorrentStatsState::Paused),
                        live: matches!(s.state, TorrentStatsState::Live),
                        uploaded: s.uploaded_bytes,
                        total: s.total_bytes,
                    }
                })
                .collect::<Vec<_>>()
            });

            let alive: HashSet<usize> = snaps.iter().map(|s| s.id).collect();
            state.queued.lock().unwrap().retain(|id| alive.contains(id));
            state.seed_done.lock().unwrap().retain(|id| alive.contains(id));
            state.seed_since.lock().unwrap().retain(|id, _| alive.contains(id));

            for s in &snaps {
                if !s.live || !s.finished {
                    continue;
                }
                // A torrent that already hit its limit stays hit. Without this a
                // manual resume would just be undone on the next tick.
                if state.seed_done.lock().unwrap().contains(&s.id) {
                    continue;
                }

                let started = {
                    let mut m = state.seed_since.lock().unwrap();
                    *m.entry(s.id).or_insert_with(Instant::now)
                };

                let ratio_hit = settings
                    .seed_ratio_limit
                    .filter(|l| *l > 0.0)
                    .is_some_and(|l| s.total > 0 && s.uploaded as f64 / s.total as f64 >= l as f64);
                let time_hit = settings
                    .seed_time_limit
                    .filter(|m| *m > 0)
                    .is_some_and(|m| started.elapsed() >= Duration::from_secs(m as u64 * 60));

                if (ratio_hit || time_hit) && session.pause(&s.handle).await.is_ok() {
                    state.seed_done.lock().unwrap().insert(s.id);
                    state.queued.lock().unwrap().remove(&s.id);
                }
            }

            match settings.max_active_downloads.filter(|m| *m > 0) {
                Some(max) => {
                    let max = max as usize;
                    let mut active: Vec<usize> = snaps
                        .iter()
                        .filter(|s| s.live && !s.finished)
                        .map(|s| s.id)
                        .collect();
                    active.sort_unstable();

                    if active.len() > max {
                        // Oldest first, so the ones nearest completion keep their slots.
                        for id in &active[max..] {
                            let Some(s) = snaps.iter().find(|s| s.id == *id) else { continue };
                            if session.pause(&s.handle).await.is_ok() {
                                state.queued.lock().unwrap().insert(*id);
                            }
                        }
                    } else {
                        let mut slots = max - active.len();
                        let mut waiting: Vec<usize> =
                            state.queued.lock().unwrap().iter().copied().collect();
                        waiting.sort_unstable();
                        for id in waiting {
                            if slots == 0 {
                                break;
                            }
                            let Some(s) = snaps.iter().find(|s| s.id == id) else { continue };
                            if !s.paused || s.finished {
                                state.queued.lock().unwrap().remove(&id);
                                continue;
                            }
                            if session.unpause(&s.handle).await.is_ok() {
                                state.queued.lock().unwrap().remove(&id);
                                slots -= 1;
                            }
                        }
                    }
                }
                None => {
                    // Limit switched off - let everything the queue paused go.
                    let waiting: Vec<usize> =
                        state.queued.lock().unwrap().iter().copied().collect();
                    for id in waiting {
                        let Some(s) = snaps.iter().find(|s| s.id == id) else { continue };
                        if !s.paused || session.unpause(&s.handle).await.is_ok() {
                            state.queued.lock().unwrap().remove(&id);
                        }
                    }
                }
            }
        }
    });
}

fn spawn_bind_monitor<R: Runtime>(handle: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(BIND_POLL);
        loop {
            tick.tick().await;
            let Some(state) = handle.try_state::<AppState>() else { continue };

            let settings = state.settings.read().unwrap().clone();
            let Some(ip) = settings.parsed_bind_ip() else {
                *state.bind_lost_since.write().unwrap() = None;
                continue;
            };

            let available = bind_ip_available(ip);
            let was_lost = state.bind_lost_since.read().unwrap().is_some();
            let blocked = state.notice.read().unwrap().is_some();

            if available && (was_lost || blocked) {
                eprintln!("[qtorrent] {ip} is back; rebinding session");
                state.session().cancellation_token().cancel();
                match build_session(state.download_dir.clone(), &settings).await {
                    Ok((new, notice)) => {
                        *state.session.write().unwrap() = new;
                        *state.notice.write().unwrap() = notice;
                        *state.bind_lost_since.write().unwrap() = None;
                    }
                    Err(e) => {
                        eprintln!("[qtorrent] rebind failed: {e}; retrying");
                        *state.notice.write().unwrap() = Some(format!(
                            "Could not rebind to {ip}: {e}"
                        ));
                    }
                }
            } else if !available && !was_lost {
                eprintln!("[qtorrent] bound interface {ip} disappeared");
                *state.bind_lost_since.write().unwrap() = Some(Instant::now());
            }
        }
    });
}

fn bind_ip_available(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }
    if_addrs::get_if_addrs()
        .map(|list| list.iter().any(|i| i.ip() == ip))
        .unwrap_or(false)
}

async fn build_session(
    download_dir: PathBuf,
    settings: &Settings,
) -> Result<(Arc<Session>, Option<String>), String> {
    let mut effective = settings.clone();
    let mut notice = None;

    if let Some(ip) = settings.parsed_bind_ip() {
        if !bind_ip_available(ip) {
            notice = Some(format!(
                "Set to bind to {ip}, but no interface has that address right now — \
                 is the VPN connected? Networking stays off until it returns."
            ));
            // Fail closed. Falling back to "any interface" would push peer,
            // DHT and tracker traffic out of the very address the user bound
            // away from, which is worse than not running at all.
            effective.bind_ip = Some(Ipv4Addr::LOCALHOST.to_string());
        }
    }

    match Session::new_with_opts(download_dir.clone(), session_options(&effective, true)).await {
        Ok(s) => Ok((s, notice)),
        Err(e) => {
            eprintln!("[qtorrent] no usable listen port ({e:#}); continuing without one");
            let s = Session::new_with_opts(download_dir, session_options(&effective, false))
                .await
                .map_err(|e| format!("{e:#}"))?;
            Ok((s, notice))
        }
    }
}

fn load_settings(path: &PathBuf) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating config dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("writing settings: {e}"))
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.read().unwrap().clone())
}

/// Which country database is active, and the attribution CC BY 4.0 requires.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GeoipStatus {
    loaded: bool,
    bundled: bool,
    path: Option<String>,
    attribution: Option<String>,
    /// Read from the database's own metadata, so the pinned version is
    /// self-describing rather than a comment that drifts from the file.
    database_type: Option<String>,
    built: Option<String>,
}

/// Unix epoch -> YYYY-MM-DD, without pulling in a date crate.
fn epoch_to_date(epoch: u64) -> String {
    let days = (epoch / 86_400) as i64;
    let (mut y, mut d) = (1970i64, days);
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut m = 0usize;
    while m < 12 && d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!("{y:04}-{:02}-{:02}", m + 1, d + 1)
}

#[tauri::command]
async fn geoip_status(state: State<'_, AppState>) -> Result<GeoipStatus, String> {
    let bundled = *state.geoip_bundled.read().unwrap();
    let loaded = state.geoip.read().unwrap().is_some();
    let path = if bundled {
        state.bundled_geoip_path.read().unwrap().clone()
    } else {
        state.settings.read().unwrap().geoip_path.clone()
    };
    let (database_type, built) = match state.geoip.read().unwrap().as_ref() {
        Some(r) => (
            Some(r.metadata().database_type.clone()),
            Some(epoch_to_date(r.metadata().build_epoch)),
        ),
        None => (None, None),
    };

    Ok(GeoipStatus {
        loaded,
        bundled,
        path,
        attribution: (loaded && bundled)
            .then(|| "IP geolocation by DB-IP (db-ip.com), CC BY 4.0".to_string()),
        database_type,
        built,
    })
}

#[tauri::command]
async fn list_network_interfaces() -> Result<Vec<NetworkInterface>, String> {
    let addrs = if_addrs::get_if_addrs().map_err(|e| format!("enumerating interfaces: {e}"))?;
    let mut out: Vec<NetworkInterface> = addrs
        .into_iter()
        .map(|i| NetworkInterface {
            is_loopback: i.is_loopback(),
            ip: i.ip().to_string(),
            name: i.name,
        })
        .collect();
    out.sort_by(|a, b| {
        a.is_loopback
            .cmp(&b.is_loopback)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.ip.cmp(&b.ip))
    });
    Ok(out)
}

#[tauri::command]
async fn set_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<SessionStatus, String> {
    if let Some(ip) = settings.bind_ip.as_ref() {
        if ip.parse::<IpAddr>().is_err() {
            return Err(format!("'{ip}' is not a valid IP address."));
        }
    }
    if settings.listen_port == Some(0) {
        return Err("Port 0 means 'any port' — leave it on automatic instead.".to_string());
    }

    let previous = state.settings.read().unwrap().clone();
    let needs_rebuild = previous.network_differs(&settings);
    if previous.geoip_path != settings.geoip_path {
        let bundled = state.bundled_geoip_path.read().unwrap().clone();
        let path = settings.geoip_path.clone().or(bundled);
        state.load_geoip(path.as_deref());
        *state.geoip_bundled.write().unwrap() = settings.geoip_path.is_none();
    }
    // Rate limits are swapped live (librqbit holds them in an ArcSwap), so they
    // deliberately don't count as a network change - no engine restart.
    apply_limits(&state.session(), &settings);
    save_settings(&state.settings_path, &settings)?;
    *state.settings.write().unwrap() = settings.clone();

    if needs_rebuild {
        state.session().cancellation_token().cancel();

        match build_session(state.download_dir.clone(), &settings).await {
            Ok((new, notice)) => {
                *state.session.write().unwrap() = new;
                *state.notice.write().unwrap() = notice;
            }
            Err(e) => {
                // The old session is already cancelled, so returning here would
                // strand the app holding a dead engine. Roll back instead.
                let (restored, notice) = build_session(state.download_dir.clone(), &previous)
                    .await
                    .map_err(|e2| format!("{e}

Could not restore previous settings: {e2}"))?;
                *state.session.write().unwrap() = restored;
                *state.notice.write().unwrap() = notice;
                *state.settings.write().unwrap() = previous.clone();
                save_settings(&state.settings_path, &previous).ok();
                return Err(e);
            }
        }
    }

    session_status(
        &state.session(),
        state.units(),
        state.notice(),
        state.bind_lost_for(),
        state.limits(),
    )
}

#[tauri::command]
async fn get_session_status(state: State<'_, AppState>) -> Result<SessionStatus, String> {
    session_status(
        &state.session(),
        state.units(),
        state.notice(),
        state.bind_lost_for(),
        state.limits(),
    )
}

fn session_status(
    session: &Arc<Session>,
    units: Units,
    notice: Option<String>,
    bind_lost_for: Option<Duration>,
    limits: (Option<u32>, Option<u32>),
) -> Result<SessionStatus, String> {
    let snap = session.stats_snapshot();

    let listen_port = session.tcp_listen_port();
    let dht_nodes = session.get_dht().map(|d| d.stats().routing_table_size);
    let peers = snap.peers.live;
    let incoming = snap.incoming_connections;
    let uptime = snap.uptime_seconds;

    let dht_desc = match dht_nodes {
        Some(n) => format!("{n} DHT nodes"),
        None => "DHT disabled".to_string(),
    };

    let connected = dht_nodes.unwrap_or(0) > 0 || peers > 0;

    let dropout = bind_lost_for.map(|d| d.as_secs());
    let blocked = match dropout {
        Some(secs) if secs < BIND_DROPOUT_GRACE_SECS => Some((
            "connecting".to_string(),
            format!(
                "The bound network interface just dropped. Waiting {}s for it to come back                  before giving up.",
                BIND_DROPOUT_GRACE_SECS - secs
            ),
            None,
        )),
        Some(_) => Some((
            "offline".to_string(),
            "The bound network interface is gone — is the VPN connected? Nothing is sent              while it's missing."
                .to_string(),
            Some(
                "The bound network interface is gone — is the VPN connected? Nothing is sent                  while it's missing."
                    .to_string(),
            ),
        )),
        None => notice
            .clone()
            .map(|n| ("offline".to_string(), n.clone(), Some(n))),
    };

    if let Some((state_str, detail, bar)) = blocked {
        return Ok(SessionStatus {
            state: state_str,
            detail,
            listen_port,
            dht_nodes,
            peers,
            incoming,
            down_speed: format_speed(snap.download_speed.mbps, units),
            up_speed: format_speed(snap.upload_speed.mbps, units),
            downloaded: format_size(snap.fetched_bytes, units),
            uploaded: format_size(snap.uploaded_bytes, units),
            notice: bar,
            down_limit: limits.0,
            up_limit: limits.1,
        });
    }

    let (state_str, detail) = if !connected && uptime < CONNECTING_GRACE_SECS {
        (
            "connecting",
            "Connecting — bootstrapping DHT and looking for peers.".to_string(),
        )
    } else if !connected {
        (
            "offline",
            "Offline. No DHT nodes and no connected peers — check your network connection."
                .to_string(),
        )
    } else if let Some(port) = listen_port {
        if incoming == 0 && uptime >= FIREWALL_GRACE_SECS {
            (
                "firewalled",
                format!(
                    "Firewalled — this is fine, just slower. Nothing has reached port {port} \
                     from outside, so you only connect outward. Downloads work normally; you \
                     just reach fewer of the swarm (never other firewalled peers), and seeding \
                     is limited. Forwarding port {port} would speed things up."
                ),
            )
        } else {
            (
                "online",
                format!(
                    "Online. Listening on port {port} · {dht_desc} · {peers} peers · \
                     {incoming} incoming connections"
                ),
            )
        }
    } else {
        (
            "firewalled",
            format!(
                "Firewalled — this is fine, just slower. Not listening for incoming \
                 connections, so you only connect outward. {dht_desc} · {peers} peers."
            ),
        )
    };

    Ok(SessionStatus {
        state: state_str.to_string(),
        detail,
        listen_port,
        dht_nodes,
        peers,
        incoming,
        down_speed: format_speed(snap.download_speed.mbps, units),
        up_speed: format_speed(snap.upload_speed.mbps, units),
        downloaded: format_size(snap.fetched_bytes, units),
        uploaded: format_size(snap.uploaded_bytes, units),
        notice,
        down_limit: limits.0,
        up_limit: limits.1,
    })
}

#[tauri::command]
async fn take_pending_opens(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let mut inbox = state.inbox.lock().unwrap();
    Ok(std::mem::take(&mut *inbox))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    results: Vec<search::SearchResult>,
    errors: Vec<String>,
}

fn plugins_dir(state: &AppState) -> PathBuf {
    let config = state
        .settings_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    search::plugins_dir(&config)
}

#[tauri::command]
async fn list_search_plugins(
    state: State<'_, AppState>,
) -> Result<Vec<search::SearchPlugin>, String> {
    let disabled = state.settings.read().unwrap().disabled_plugins.clone();
    Ok(search::load_plugins(&plugins_dir(&state), &disabled).await)
}

#[tauri::command]
async fn set_search_plugin_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Vec<search::SearchPlugin>, String> {
    {
        let mut s = state.settings.write().unwrap();
        s.disabled_plugins.retain(|d| d != &id);
        if !enabled {
            s.disabled_plugins.push(id);
        }
    }
    let settings = state.settings.read().unwrap().clone();
    save_settings(&state.settings_path, &settings)?;
    Ok(search::load_plugins(&plugins_dir(&state), &settings.disabled_plugins).await)
}

#[tauri::command]
async fn search_torrents(
    state: State<'_, AppState>,
    query: String,
) -> Result<SearchResponse, String> {
    let query = query.trim().to_string();
    let disabled = state.settings.read().unwrap().disabled_plugins.clone();
    let plugins = search::load_plugins(&plugins_dir(&state), &disabled).await;
    if !plugins.iter().any(|p| p.enabled) {
        return Err("No search sources are enabled.".to_string());
    }
    let (results, errors) = search::search(plugins, query).await;
    Ok(SearchResponse { results, errors })
}

#[tauri::command]
async fn open_plugins_folder(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let dir = plugins_dir(&state);
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating plugins folder: {e}"))?;
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("{e:#}"))
}

/// `fs::rename` fails across volumes, which is the common case when moving data
/// off a full drive. Fall back to copy-then-delete.
fn move_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

/// Relocate a torrent's files, then re-add it pointing at the new folder.
///
/// librqbit can't repoint a live torrent's storage, so this removes it (keeping
/// the data), moves the files, and re-adds from the torrent bytes it already
/// holds. Files are moved individually rather than by moving `output_folder`:
/// single-file torrents share the session's default folder, so moving the whole
/// directory would take unrelated torrents with it.
#[tauri::command]
async fn move_torrent(
    state: State<'_, AppState>,
    id: usize,
    new_folder: String,
) -> Result<(), String> {
    let session = state.session();
    let handle = session
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let (torrent_bytes, relatives) = {
        let md = handle.metadata.load();
        let md = md
            .as_ref()
            .ok_or_else(|| "Metadata isn't resolved yet.".to_string())?;
        (
            md.torrent_bytes.to_vec(),
            md.file_infos
                .iter()
                .map(|f| f.relative_filename.clone())
                .collect::<Vec<_>>(),
        )
    };
    let only_files = handle.only_files();
    let old_folder = handle.output_folder().to_path_buf();
    let new_folder = PathBuf::from(&new_folder);

    if old_folder == new_folder {
        return Ok(());
    }
    std::fs::create_dir_all(&new_folder).map_err(|e| format!("creating {new_folder:?}: {e}"))?;

    // Stop managing it first so the files aren't held open while they move.
    session
        .delete(TorrentIdOrHash::Id(id), false)
        .await
        .map_err(|e| format!("{e:#}"))?;

    let from_root = old_folder.clone();
    let to_root = new_folder.clone();
    let rels = relatives.clone();
    let moved = tauri::async_runtime::spawn_blocking(move || {
        for rel in &rels {
            let from = from_root.join(rel);
            if !from.exists() {
                continue;
            }
            let to = to_root.join(rel);
            move_file(&from, &to).map_err(|e| format!("moving {rel:?}: {e}"))?;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("move task failed: {e}"))?;

    // Re-add wherever the data actually ended up, so a partial move still
    // leaves a working torrent rather than an orphaned one.
    let target = if moved.is_ok() { &new_folder } else { &old_folder };
    let opts = AddTorrentOptions {
        only_files,
        output_folder: Some(target.to_string_lossy().to_string()),
        overwrite: true,
        ..Default::default()
    };
    session
        .add_torrent(AddTorrent::from_bytes(torrent_bytes), Some(opts))
        .await
        .map_err(|e| format!("re-adding after move: {e:#}"))?;

    moved
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PeerInfo {
    addr: String,
    state: String,
    country: Option<String>,
    client: String,
    progress: f64,
    downloaded: String,
    down_bytes: u64,
    uploaded: String,
    up_bytes: u64,
    pieces: u32,
    inflight: usize,
    interested: bool,
    errors: u32,
}

/// ISO 3166-1 alpha-2 -> regional indicator pair, which renders as a flag.
fn iso_to_flag(code: &str) -> Option<String> {
    let c = code.as_bytes();
    if c.len() != 2 || !c[0].is_ascii_alphabetic() || !c[1].is_ascii_alphabetic() {
        return None;
    }
    let ch = |b: u8| char::from_u32(0x1F1E6 + (b.to_ascii_uppercase() - b'A') as u32);
    Some(format!("{}{}", ch(c[0])?, ch(c[1])?))
}

/// Country lookup is strictly local - resolving peer IPs through a web service
/// would leak the swarm to a third party, which defeats the interface binding.
fn lookup_country(
    reader: Option<&Arc<maxminddb::Reader<Vec<u8>>>>,
    addr: &str,
) -> Option<String> {
    let reader = reader?;
    let ip: IpAddr = addr.rsplit_once(':')?.0.trim_matches(['[', ']']).parse().ok()?;
    let found = reader.lookup(ip).ok()?;
    let country: maxminddb::geoip2::Country = found.decode().ok()??;
    iso_to_flag(country.country.iso_code?)
}

/// Turn a hex peer id into something like "qBittorrent 4.6.0.0". Falls back to
/// the printable ASCII prefix, which is what most non-Azureus clients use.
fn client_name(peer_id_hex: Option<&String>) -> String {
    let Some(hex) = peer_id_hex else {
        return "-".to_string();
    };
    let mut bytes = [0u8; 20];
    for i in 0..20 {
        match hex.get(i * 2..i * 2 + 2).and_then(|b| u8::from_str_radix(b, 16).ok()) {
            Some(v) => bytes[i] = v,
            None => return "-".to_string(),
        }
    }

    if let Some(PeerId::AzureusStyle(a)) = Id20::from_bytes(&bytes).ok().and_then(try_decode_peer_id)
    {
        let name = match a.kind {
            AzureusStyleKind::Deluge => "Deluge".to_string(),
            AzureusStyleKind::LibTorrent => "libtorrent".to_string(),
            AzureusStyleKind::Transmission => "Transmission".to_string(),
            AzureusStyleKind::QBittorrent => "qBittorrent".to_string(),
            AzureusStyleKind::UTorrent => "uTorrent".to_string(),
            AzureusStyleKind::RQBit => "rqbit".to_string(),
            AzureusStyleKind::Other(o) => String::from_utf8_lossy(&o).to_string(),
        };
        // Trailing zeros are padding, not meaningful version components:
        // qBittorrent reports 5.2.3.0 but calls itself 5.2.3.
        let mut parts: Vec<String> = a.version.iter().map(|d| d.to_string()).collect();
        while parts.len() > 2 && parts.last().map(|p| p == "0").unwrap_or(false) {
            parts.pop();
        }
        return format!("{name} {}", parts.join("."));
    }

    // Non-Azureus ids (Shad0w style, or random). Show the printable prefix if
    // it looks like a name, otherwise don't pretend to know.
    let printable: String = bytes
        .iter()
        .take(8)
        .take_while(|b| b.is_ascii_graphic())
        .map(|b| *b as char)
        .collect();
    if printable.chars().filter(|c| c.is_ascii_alphanumeric()).count() >= 4 {
        printable
    } else {
        "Unknown".to_string()
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DetailFile {
    index: usize,
    path: String,
    size_str: String,
    progress: f64,
    /// Raw bytes complete; the UI diffs this across polls for a per-file rate.
    done_bytes: u64,
    selected: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TorrentDetail {
    name: String,
    info_hash: String,
    save_path: String,
    size: String,
    downloaded: String,
    uploaded: String,
    ratio: String,
    pieces: u32,
    piece_size: String,
    peers_live: usize,
    peers_connecting: usize,
    peers_queued: usize,
    peers_seen: usize,
    trackers: Vec<String>,
    files: Vec<DetailFile>,
    peers: Vec<PeerInfo>,
}

#[tauri::command]
async fn get_torrent_detail(
    state: State<'_, AppState>,
    id: usize,
) -> Result<TorrentDetail, String> {
    let units = state.units();
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let stats = handle.stats();
    let metadata = handle.metadata.load();
    let metadata = metadata
        .as_ref()
        .ok_or_else(|| "Metadata isn't resolved yet.".to_string())?;

    let only: Option<HashSet<usize>> = handle.only_files().map(|v| v.into_iter().collect());
    let files = metadata
        .file_infos
        .iter()
        .enumerate()
        .map(|(index, fi)| {
            let done = stats.file_progress.get(index).copied().unwrap_or(0);
            DetailFile {
                index,
                path: fi.relative_filename.display().to_string(),
                size_str: format_size(fi.len, units),
                done_bytes: done,
                progress: if fi.len > 0 {
                    (done as f64 / fi.len as f64) * 100.0
                } else {
                    100.0
                },
                selected: only.as_ref().map(|s| s.contains(&index)).unwrap_or(true),
            }
        })
        .collect();

    // Peer detail only exists while the torrent is live.
    let geoip = state.geoip();
    let peers = handle
        .live()
        .map(|live| {
            let snap = live.per_peer_stats_snapshot(Default::default());
            let mut v: Vec<PeerInfo> = snap
                .peers
                .into_iter()
                .map(|(addr, p)| PeerInfo {
                    country: lookup_country(geoip.as_ref(), &addr),
                    addr,
                    state: p.state.to_string(),
                    client: client_name(p.peer_id.as_ref()),
                    progress: p.progress.unwrap_or(0.0) * 100.0,
                    downloaded: format_size(p.counters.fetched_bytes, units),
                    down_bytes: p.counters.fetched_bytes,
                    uploaded: format_size(p.counters.uploaded_bytes, units),
                    up_bytes: p.counters.uploaded_bytes,
                    pieces: p.counters.downloaded_and_checked_pieces,
                    inflight: p.inflight.unwrap_or(0),
                    interested: p.interested.unwrap_or(false),
                    errors: p.counters.errors,
                })
                .collect();
            v.sort_by(|a, b| {
                (b.down_bytes + b.up_bytes)
                    .cmp(&(a.down_bytes + a.up_bytes))
                    .then_with(|| a.addr.cmp(&b.addr))
            });
            v
        })
        .unwrap_or_default();

    let agg = stats.live.as_ref().map(|l| &l.snapshot.peer_stats);
    // Ratio against what's actually been verified, so a fresh torrent doesn't
    // read as an infinite ratio.
    let ratio = if stats.progress_bytes > 0 {
        format!(
            "{:.2}",
            stats.uploaded_bytes as f64 / stats.progress_bytes as f64
        )
    } else {
        "—".to_string()
    };

    let mut trackers: Vec<String> = handle
        .shared()
        .trackers
        .iter()
        .map(|t| t.to_string())
        .collect();
    trackers.sort();

    Ok(TorrentDetail {
        name: handle.name().unwrap_or_else(|| format!("Torrent #{id}")),
        info_hash: handle.info_hash().as_string(),
        save_path: handle.output_folder().display().to_string(),
        size: format_size(stats.total_bytes, units),
        downloaded: format_size(stats.progress_bytes, units),
        uploaded: format_size(stats.uploaded_bytes, units),
        ratio,
        pieces: metadata.lengths.total_pieces(),
        piece_size: format_size(metadata.lengths.default_piece_length() as u64, units),
        peers_live: agg.map(|a| a.live).unwrap_or(0),
        peers_connecting: agg.map(|a| a.connecting).unwrap_or(0),
        peers_queued: agg.map(|a| a.queued).unwrap_or(0),
        peers_seen: agg.map(|a| a.seen).unwrap_or(0),
        trackers,
        files,
        peers,
    })
}

#[tauri::command]
async fn reannounce_torrent(state: State<'_, AppState>, id: usize) -> Result<(), String> {
    let session = state.session();
    let handle = session
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;
    session.force_reannounce(&handle);
    Ok(())
}

#[tauri::command]
async fn recheck_torrent(state: State<'_, AppState>, id: usize) -> Result<(), String> {
    let session = state.session();
    let handle = session
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;
    session
        .force_recheck(&handle)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn open_torrent_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    id: usize,
) -> Result<(), String> {
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;
    let folder = handle.output_folder().to_path_buf();
    if !folder.exists() {
        return Err(format!("{} doesn't exist yet.", folder.display()));
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(folder.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn get_magnet_link(state: State<'_, AppState>, id: usize) -> Result<String, String> {
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;

    let mut magnet = format!("magnet:?xt=urn:btih:{}", handle.shared.info_hash.as_string());
    if let Some(name) = handle.name() {
        magnet.push_str(&format!("&dn={}", urlencode(&name)));
    }
    for tracker in handle.shared.trackers.iter() {
        magnet.push_str(&format!("&tr={}", urlencode(tracker.as_str())));
    }
    Ok(magnet)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[tauri::command]
async fn remove_torrent(
    state: State<'_, AppState>,
    id: usize,
    delete_files: bool,
) -> Result<(), String> {
    state
        .session()
        .delete(TorrentIdOrHash::Id(id), delete_files)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn pause_torrent(state: State<'_, AppState>, id: usize) -> Result<(), String> {
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;
    state.queued.lock().unwrap().remove(&id);
    state.session().pause(&handle).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn resume_torrent(state: State<'_, AppState>, id: usize) -> Result<(), String> {
    let handle = state
        .session()
        .get(TorrentIdOrHash::Id(id))
        .ok_or_else(|| format!("Torrent {id} not found"))?;
    state.queued.lock().unwrap().remove(&id);
    // Resuming an already-finished torrent is a deliberate "seed past the
    // limit", so exempt it. Resuming one that is still downloading is not.
    if handle.stats().finished {
        state.seed_done.lock().unwrap().insert(id);
    }
    state.seed_since.lock().unwrap().remove(&id);
    state.session().unpause(&handle).await.map_err(|e| format!("{e:#}"))
}

fn queue_open<R: Runtime>(handle: &AppHandle<R>, s: String) {
    let is_magnet = s.starts_with("magnet:");
    let is_file = s.to_lowercase().ends_with(".torrent");
    if !is_magnet && !is_file {
        return;
    }
    eprintln!("[qtorrent] queueing open request: {:.100}", s);

    if let Some(state) = handle.try_state::<AppState>() {
        state.inbox.lock().unwrap().push(s);
    }
    let _ = handle.emit("torrent-open-request", ());

    if let Some(win) = handle.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            eprintln!("[qtorrent] single-instance argv: {argv:?}");
            for arg in argv.into_iter().skip(1) {
                queue_open(app, arg);
            }
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else { return };
            let Some(state) = window.app_handle().try_state::<AppState>() else { return };
            if state.quitting.load(Ordering::SeqCst) {
                return;
            }
            if state.settings.read().unwrap().minimize_to_tray {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let download_dir = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| PathBuf::from("."));

            let settings_path = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("settings.json");
            let settings = load_settings(&settings_path);

            type InitResult = Result<(Arc<Session>, Option<String>), String>;
            let (tx, rx) = std::sync::mpsc::channel::<InitResult>();
            let init_dir = download_dir.clone();
            let init_settings = settings.clone();
            tauri::async_runtime::spawn(async move {
                let mut result = build_session(init_dir.clone(), &init_settings).await;
                if let Err(e) = &result {
                    // Last resort: come up with default networking so the app
                    // is usable and the settings can be corrected, rather than
                    // panicking on launch.
                    eprintln!("[qtorrent] session init failed ({e}); falling back to defaults");
                    let fallback = Settings { units: init_settings.units, ..Settings::default() };
                    result = build_session(init_dir, &fallback).await.map(|(s, _)| {
                        (
                            s,
                            Some(format!(
                                "Could not start with your network settings ({e}). \
                                 Running with defaults — check Network settings."
                            )),
                        )
                    });
                }
                tx.send(result).ok();
            });

            let (session, notice) = rx
                .recv()
                .expect("Session init channel closed")
                .expect("Failed to create torrent session");

            app.manage(AppState {
                session: RwLock::new(session),
                settings: RwLock::new(settings),
                notice: RwLock::new(notice),
                geoip: RwLock::new(None),
                geoip_bundled: RwLock::new(true),
                bundled_geoip_path: RwLock::new(None),
                bind_lost_since: RwLock::new(None),
                settings_path,
                download_dir,
                pending: Mutex::new(HashMap::new()),
                inbox: Mutex::new(Vec::new()),
                queued: Mutex::new(HashSet::new()),
                seed_done: Mutex::new(HashSet::new()),
                seed_since: Mutex::new(HashMap::new()),
                quitting: AtomicBool::new(false),
                preview: tokio::sync::Mutex::new(None),
            });

            #[cfg(any(windows, target_os = "linux"))]
            app.deep_link().register_all()?;

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let urls = event.urls();
                eprintln!("[qtorrent] on_open_url fired: {urls:?}");
                for url in urls {
                    queue_open(&handle, url.to_string());
                }
            });

            let startup_args: Vec<String> = std::env::args().skip(1).collect();
            eprintln!("[qtorrent] startup argv: {startup_args:?}");
            for arg in startup_args {
                queue_open(app.handle(), arg);
            }

            spawn_bind_monitor(app.handle().clone());
            spawn_torrent_manager(app.handle().clone());
            build_tray(app.handle())?;

            // Fall back to the bundled DB-IP Lite database when the user
            // hasn't pointed at their own.
            let bundled_geoip = app
                .path()
                .resolve("resources/dbip-country-lite.mmdb", BaseDirectory::Resource)
                .ok()
                .filter(|p| p.exists())
                .map(|p| p.to_string_lossy().to_string());

            if let Some(state) = app.try_state::<AppState>() {
                *state.bundled_geoip_path.write().unwrap() = bundled_geoip.clone();
                let configured = state.settings.read().unwrap().geoip_path.clone();
                let path = configured.or(bundled_geoip);
                state.load_geoip(path.as_deref());
                *state.geoip_bundled.write().unwrap() =
                    state.settings.read().unwrap().geoip_path.is_none();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_torrents,
            get_session_status,
            get_settings,
            set_settings,
            list_network_interfaces,
            geoip_status,
            preview_torrent,
            add_prepared_torrent,
            cancel_preview,
            get_torrent_files,
            update_torrent_files,
            preview_file,
            take_pending_opens,
            remove_torrent,
            pause_torrent,
            resume_torrent,
            open_torrent_folder,
            recheck_torrent,
            reannounce_torrent,
            get_torrent_detail,
            move_torrent,
            get_magnet_link,
            search_torrents,
            list_search_plugins,
            set_search_plugin_enabled,
            open_plugins_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
