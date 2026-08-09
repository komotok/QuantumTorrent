import { useState, useEffect, useRef, useCallback, useLayoutEffect, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import "./torrent-theme.css";
import "./TorrentClient.css";

import addIcon from "@material-symbols/svg-400/rounded/add.svg?raw";
import deleteIcon from "@material-symbols/svg-400/rounded/delete.svg?raw";
import pauseIcon from "@material-symbols/svg-400/rounded/pause.svg?raw";
import playIcon from "@material-symbols/svg-400/rounded/play_arrow.svg?raw";
import filesIcon from "@material-symbols/svg-400/rounded/description.svg?raw";
import folderIcon from "@material-symbols/svg-400/rounded/folder_open.svg?raw";
import copyIcon from "@material-symbols/svg-400/rounded/content_copy.svg?raw";
import settingsIcon from "@material-symbols/svg-400/rounded/settings.svg?raw";
import searchIcon from "@material-symbols/svg-400/rounded/search.svg?raw";
import chevronLeftIcon from "@material-symbols/svg-400/rounded/chevron_left.svg?raw";
import chevronRightIcon from "@material-symbols/svg-400/rounded/chevron_right.svg?raw";
import expandMoreIcon from "@material-symbols/svg-400/rounded/keyboard_arrow_down.svg?raw";
import videoIcon from "@material-symbols/svg-400/rounded/movie.svg?raw";
import audioIcon from "@material-symbols/svg-400/rounded/music_note.svg?raw";
import imageIcon from "@material-symbols/svg-400/rounded/image.svg?raw";
import archiveIcon from "@material-symbols/svg-400/rounded/folder_zip.svg?raw";
import pdfIcon from "@material-symbols/svg-400/rounded/picture_as_pdf.svg?raw";
import codeIcon from "@material-symbols/svg-400/rounded/code.svg?raw";
import subtitleIcon from "@material-symbols/svg-400/rounded/subtitles.svg?raw";
import execIcon from "@material-symbols/svg-400/rounded/terminal.svg?raw";
import fontIcon from "@material-symbols/svg-400/rounded/font_download.svg?raw";
import discIcon from "@material-symbols/svg-400/rounded/album.svg?raw";
import recheckIcon from "@material-symbols/svg-400/rounded/refresh.svg?raw";
import moveIcon from "@material-symbols/svg-400/rounded/drive_file_move.svg?raw";
import announceIcon from "@material-symbols/svg-400/rounded/campaign.svg?raw";
import fileIcon from "@material-symbols/svg-400/rounded/draft.svg?raw";

type TorrentStatus =
  | "downloading"
  | "seeding"
  | "paused"
  | "checking"
  | "error"
  | "queued";

interface Torrent {
  id: number;
  name: string;
  size: string;
  progress: number;
  status: TorrentStatus;
  down: string;
  up: string;
  eta: string;
  error: string | null;
  sizeBytes: number;
  downBps: number;
  upBps: number;
  etaSecs: number | null;
}

type SortKey = "id" | "name" | "sizeBytes" | "progress" | "status" | "downBps" | "upBps" | "etaSecs";

const COLUMNS: Array<{ key: SortKey; label: string; numeric?: boolean }> = [
  { key: "name", label: "Name" },
  { key: "sizeBytes", label: "Size", numeric: true },
  { key: "progress", label: "Progress", numeric: true },
  { key: "status", label: "Status" },
  { key: "downBps", label: "Down", numeric: true },
  { key: "upBps", label: "Up", numeric: true },
  { key: "etaSecs", label: "ETA", numeric: true },
];

const STATUS_RANK: Record<TorrentStatus, number> = {
  error: 0,
  downloading: 1,
  checking: 2,
  queued: 3,
  seeding: 4,
  paused: 5,
};

function compareTorrents(a: Torrent, b: Torrent, key: SortKey): number {
  switch (key) {
    case "name":
      return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
    case "status":
      return STATUS_RANK[a.status] - STATUS_RANK[b.status];
    case "etaSecs": {
      if (a.etaSecs == null && b.etaSecs == null) return 0;
      if (a.etaSecs == null) return 1;
      if (b.etaSecs == null) return -1;
      return a.etaSecs - b.etaSecs;
    }
    default:
      return (a[key] as number) - (b[key] as number);
  }
}

interface FileEntry {
  index: number;
  path: string;
  size: number;
  sizeStr: string;
  selected: boolean;
  padding: boolean;
}

type ConnState = "online" | "firewalled" | "connecting" | "offline";

interface SessionStatus {
  state: ConnState;
  detail: string;
  listenPort: number | null;
  dhtNodes: number | null;
  peers: number;
  incoming: number;
  downSpeed: string;
  upSpeed: string;
  downloaded: string;
  uploaded: string;
  notice: string | null;
  downLimit: number | null;
  upLimit: number | null;
}

const CONN_LABELS: Record<ConnState, string> = {
  online: "Online",
  firewalled: "Firewalled",
  connecting: "Connecting",
  offline: "Offline",
};

interface TorrentPreview {
  infoHash: string;
  name: string;
  totalSize: number;
  totalSizeStr: string;
  outputFolder: string;
  files: FileEntry[];
}

const STATUS_LABELS: Record<TorrentStatus, string> = {
  downloading: "Downloading",
  seeding: "Seeding",
  paused: "Paused",
  checking: "Checking",
  error: "Error",
  queued: "Queued",
};

type FilterState = "all" | TorrentStatus;
const FILTERS: FilterState[] = ["all", "downloading", "seeding", "paused", "checking", "error"];

type Units = "binary" | "decimal";

const SIZE_DECIMALS = [0, 1, 1, 2, 2];

/** Mirrors format_size in the backend, including the unit-boundary step-up:
 *  1048575 bytes is 1023.999 KiB and must not render as "1024.0 KiB". */
function formatSize(bytes: number, units: Units): string {
  const k = units === "binary" ? 1024 : 1000;
  const n = units === "binary"
    ? ["B", "KiB", "MiB", "GiB", "TiB"]
    : ["B", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = 0;
  while (v >= k && i + 1 < n.length) {
    v /= k;
    i += 1;
  }
  const f = 10 ** SIZE_DECIMALS[i];
  if (Math.round(v * f) / f >= k && i + 1 < n.length) {
    v /= k;
    i += 1;
  }
  return `${v.toFixed(SIZE_DECIMALS[i])} ${n[i]}`;
}

const ICONS = {
  add: addIcon,
  remove: deleteIcon,
  pause: pauseIcon,
  play: playIcon,
  files: filesIcon,
  folder: folderIcon,
  copy: copyIcon,
  settings: settingsIcon,
  search: searchIcon,
  chevronLeft: chevronLeftIcon,
  chevronRight: chevronRightIcon,
  expandMore: expandMoreIcon,
  video: videoIcon,
  audio: audioIcon,
  image: imageIcon,
  archive: archiveIcon,
  pdf: pdfIcon,
  code: codeIcon,
  subtitle: subtitleIcon,
  exec: execIcon,
  font: fontIcon,
  disc: discIcon,
  recheck: recheckIcon,
  move: moveIcon,
  announce: announceIcon,
  file: fileIcon,
} as const;

const FILE_TYPES: Array<[keyof typeof ICONS, string[]]> = [
  ["video", ["mkv", "mp4", "avi", "mov", "wmv", "flv", "m4v", "webm", "mpg", "mpeg", "ts", "m2ts", "vob", "ogv", "rmvb", "divx"]],
  ["audio", ["mp3", "flac", "wav", "aac", "ogg", "opus", "m4a", "wma", "alac", "aiff", "ape", "mid", "midi"]],
  ["image", ["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "tiff", "tif", "ico", "heic", "avif", "psd"]],
  ["archive", ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst", "cab", "arj", "z01", "r00"]],
  ["disc", ["iso", "img", "bin", "cue", "nrg", "mdf", "mds", "ccd", "toast", "vcd", "daa"]],
  ["pdf", ["pdf", "epub", "mobi", "azw3", "djvu", "cbz", "cbr"]],
  ["subtitle", ["srt", "sub", "ass", "ssa", "vtt", "idx", "sup"]],
  ["exec", ["exe", "msi", "bat", "cmd", "sh", "app", "deb", "rpm", "apk", "dmg", "pkg"]],
  ["code", ["js", "ts", "tsx", "jsx", "py", "rs", "go", "c", "h", "cpp", "cs", "java", "rb", "php", "html", "css", "json", "xml", "yml", "yaml", "toml", "ini", "cfg", "sql"]],
  ["font", ["ttf", "otf", "woff", "woff2", "fon"]],
];

const EXT_TO_ICON: Record<string, keyof typeof ICONS> = Object.fromEntries(
  FILE_TYPES.flatMap(([icon, exts]) => exts.map(e => [e, icon] as const)),
);

function iconForFile(path: string): keyof typeof ICONS {
  const base = path.slice(Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/")) + 1);
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "file";
  return EXT_TO_ICON[base.slice(dot + 1).toLowerCase()] ?? "file";
}

function Icon({
  name,
  className = "btn-icon",
  size = 18,
}: {
  name: keyof typeof ICONS;
  className?: string;
  size?: number;
}) {
  return (
    <span
      className={className}
      style={{ width: size, height: size }}
      aria-hidden="true"
      dangerouslySetInnerHTML={{ __html: ICONS[name] }}
    />
  );
}

function useExitTransition(items: Torrent[], durationMs: number) {
  const [exiting, setExiting] = useState<Array<{ t: Torrent; index: number }>>([]);
  const prevRef = useRef<Torrent[]>([]);
  const key = items.map(i => i.id).join(",");

  useEffect(() => {
    const ids = new Set(items.map(i => i.id));
    const removed = prevRef.current
      .map((t, index) => ({ t, index }))
      .filter(({ t }) => !ids.has(t.id));

    if (removed.length) {
      setExiting(cur => [
        ...cur.filter(c => !ids.has(c.t.id)),
        ...removed.filter(r => !cur.some(c => c.t.id === r.t.id)),
      ]);
      const timers = removed.map(({ t }) =>
        window.setTimeout(
          () => setExiting(cur => cur.filter(c => c.t.id !== t.id)),
          durationMs,
        ),
      );
      prevRef.current = items;
      return () => timers.forEach(clearTimeout);
    }

    setExiting(cur => (cur.some(c => ids.has(c.t.id)) ? cur.filter(c => !ids.has(c.t.id)) : cur));
    prevRef.current = items;
  }, [key]);

  const rows: Array<{ t: Torrent; exiting: boolean }> = items.map(t => ({ t, exiting: false }));
  exiting.forEach(({ t, index }) =>
    rows.splice(Math.min(index, rows.length), 0, { t, exiting: true }),
  );
  return rows;
}

/** Never round up to 100: a file at 99.6% is not finished, and showing "100%"
 *  next to an active download rate is a contradiction. */
function formatPercent(v: number): string {
  if (v >= 100) return "100";
  if (v <= 0) return "0";
  return Math.min(99, Math.floor(v)).toString();
}

function splitPath(path: string): [string, string] {
  const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return i >= 0 ? [path.slice(0, i + 1), path.slice(i + 1)] : ["", path];
}

function ProgressBar({ progress, status }: { progress: number; status: TorrentStatus }) {
  const indeterminate = status === "checking" && progress < 1;
  if (indeterminate) {
    return (
      <div className="progress">
        <div className="progress-indeterminate" />
      </div>
    );
  }
  return (
    <div
      className="progress"
      role="progressbar"
      aria-valuenow={Math.round(progress)}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div style={{ width: `${progress}%`, background: `var(--status-${status})` }} />
    </div>
  );
}

function StatusPill({ status }: { status: TorrentStatus }) {
  return (
    <span className={`status-pill status-${status}`}>
      {STATUS_LABELS[status]}
    </span>
  );
}

function FilePicker({
  files,
  selected,
  onChange,
  units,
}: {
  files: FileEntry[];
  selected: Set<number>;
  onChange: (next: Set<number>) => void;
  units: Units;
}) {
  const pickable = files.filter(f => !f.padding);
  const selectedSize = pickable
    .filter(f => selected.has(f.index))
    .reduce((sum, f) => sum + f.size, 0);

  const toggle = (index: number) => {
    const next = new Set(selected);
    if (next.has(index)) next.delete(index);
    else next.add(index);
    onChange(next);
  };

  return (
    <div className="filepicker">
      <div className="filepicker-toolbar">
        <button
          type="button"
          className="linkbtn"
          onClick={() => onChange(new Set(pickable.map(f => f.index)))}
        >
          Select all
        </button>
        <button type="button" className="linkbtn" onClick={() => onChange(new Set())}>
          Select none
        </button>
        <span className="filepicker-summary">
          {selected.size} of {pickable.length} files · {formatSize(selectedSize, units)}
        </span>
      </div>
      <div className="filepicker-list">
        {pickable.map(f => {
          const [dir, base] = splitPath(f.path);
          return (
            <label key={f.index} className="filerow">
              <input
                type="checkbox"
                checked={selected.has(f.index)}
                onChange={() => toggle(f.index)}
              />
              <Icon name={iconForFile(f.path)} className="filerow-icon" size={18} />
              <span className="filerow-path" title={f.path}>
                {dir && <span className="filerow-dir">{dir}</span>}
                {base}
              </span>
              <span className="filerow-size">{f.sizeStr}</span>
            </label>
          );
        })}
      </div>
    </div>
  );
}

interface Settings {
  listenPort: number | null;
  bindIp: string | null;
  upnp: boolean;
  units: Units;
  geoipPath: string | null;
  downloadLimit: number | null;
  uploadLimit: number | null;
}

interface GeoipStatus {
  loaded: boolean;
  bundled: boolean;
  path: string | null;
  attribution: string | null;
  databaseType: string | null;
  built: string | null;
}

interface NetworkInterface {
  name: string;
  ip: string;
  isLoopback: boolean;
}

interface SelectOption {
  value: string;
  label: string;
}

function Select({
  value,
  options,
  onChange,
  width,
  ariaLabel,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  width?: number;
  ariaLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const current = options.find(o => o.value === value);

  useEffect(() => {
    if (!open) return;
    setActive(Math.max(0, options.findIndex(o => o.value === value)));
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const commit = (i: number) => {
    const opt = options[i];
    if (opt) onChange(opt.value);
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        setOpen(true);
      }
      return;
    }
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); setOpen(false); }
    else if (e.key === "ArrowDown") { e.preventDefault(); setActive(i => Math.min(i + 1, options.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setActive(i => Math.max(i - 1, 0)); }
    else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); commit(active); }
    else if (e.key === "Home") { e.preventDefault(); setActive(0); }
    else if (e.key === "End") { e.preventDefault(); setActive(options.length - 1); }
  };

  return (
    <div className="select" ref={ref} style={width ? { width } : undefined}>
      <button
        type="button"
        className={`select-btn${open ? " open" : ""}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => setOpen(o => !o)}
        onKeyDown={onKeyDown}
      >
        <span className="select-value">{current?.label ?? ""}</span>
        <Icon name="expandMore" className="select-chevron" />
      </button>
      {open && (
        <ul className="select-menu" role="listbox" tabIndex={-1}>
          {options.map((o, i) => (
            <li
              key={o.value}
              role="option"
              aria-selected={o.value === value}
              className={`select-option${i === active ? " active" : ""}${o.value === value ? " selected" : ""}`}
              onMouseEnter={() => setActive(i)}
              onClick={() => commit(i)}
            >
              {o.label}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const MENU_EXIT_MS = 170;

interface MenuItem {
  label: string;
  icon?: keyof typeof ICONS;
  onSelect: () => void;
  disabled?: boolean;
  danger?: boolean;
  separatorBefore?: boolean;
}

function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y, ready: false });
  const [closing, setClosing] = useState(false);
  const closeTimer = useRef<number | undefined>(undefined);

  const requestClose = useCallback(() => {
    setClosing(prev => {
      if (prev) return prev;
      closeTimer.current = window.setTimeout(onClose, MENU_EXIT_MS);
      return true;
    });
  }, [onClose]);

  useEffect(() => () => window.clearTimeout(closeTimer.current), []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      x: x + width > window.innerWidth ? Math.max(4, x - width) : x,
      y: y + height > window.innerHeight ? Math.max(4, y - height) : y,
      ready: true,
    });
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) requestClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); requestClose(); }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", requestClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", requestClose);
    };
  }, [requestClose]);

  return (
    <div
      ref={ref}
      className={`context-menu${closing ? " closing" : ""}`}
      role="menu"
      style={{ left: pos.x, top: pos.y, visibility: pos.ready ? "visible" : "hidden" }}
    >
      {items.map((item, i) => (
        <div key={i} className={item.separatorBefore ? "menu-group" : undefined}>
          <button
            type="button"
            role="menuitem"
            className={`menu-item${item.danger ? " danger" : ""}`}
            disabled={item.disabled}
            onClick={() => { item.onSelect(); requestClose(); }}
          >
            {item.icon && <Icon name={item.icon} className="menu-icon" />}
            {item.label}
          </button>
        </div>
      ))}
    </div>
  );
}

function AutoHeight({ children }: { children: ReactNode }) {
  const inner = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState<number | null>(null);
  const settled = useRef(false);

  useLayoutEffect(() => {
    const el = inner.current;
    if (!el) return;
    const measure = () => setHeight(el.offsetHeight);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    const t = window.setTimeout(() => { settled.current = true; }, 0);
    return () => { ro.disconnect(); clearTimeout(t); };
  }, []);

  return (
    <div
      className="auto-height"
      style={{
        height: height ?? undefined,
        transition: settled.current ? undefined : "none",
      }}
    >
      <div ref={inner}>{children}</div>
    </div>
  );
}

interface SearchResult {
  name: string;
  source: string;
  size: number | null;
  seeders: number | null;
  link: string;
}

interface SearchResponse {
  results: SearchResult[];
  errors: string[];
}

interface SearchPlugin {
  id: string;
  name: string;
  site: string | null;
  builtin: boolean;
  enabled: boolean;
}

interface PeerInfo {
  addr: string;
  state: string;
  country: string | null;
  client: string;
  progress: number;
  downloaded: string;
  downBytes: number;
  uploaded: string;
  upBytes: number;
  pieces: number;
  inflight: number;
  interested: boolean;
  errors: number;
}

interface DetailFile {
  index: number;
  path: string;
  sizeStr: string;
  progress: number;
  doneBytes: number;
  selected: boolean;
}

interface TorrentDetail {
  name: string;
  infoHash: string;
  savePath: string;
  size: string;
  downloaded: string;
  uploaded: string;
  ratio: string;
  pieces: number;
  pieceSize: string;
  peersLive: number;
  peersConnecting: number;
  peersQueued: number;
  peersSeen: number;
  trackers: string[];
  files: DetailFile[];
  peers: PeerInfo[];
}

type DetailTab = "general" | "files" | "peers" | "trackers";
const DETAIL_TABS: Array<{ key: DetailTab; label: string }> = [
  { key: "general", label: "General" },
  { key: "files", label: "Files" },
  { key: "peers", label: "Peers" },
  { key: "trackers", label: "Trackers" },
];

type AddStage = "input" | "resolving" | "review";

export default function TorrentClient() {
  const [torrents, setTorrents] = useState<Torrent[]>([]);
  const [conn, setConn] = useState<SessionStatus | null>(null);
  const [units, setUnits] = useState<Units>("binary");
  const [filter, setFilter] = useState<FilterState>("all");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [sortKey, setSortKey] = useState<SortKey>(() => {
    try { return (localStorage.getItem("sortKey") as SortKey) || "id"; } catch { return "id"; }
  });
  const [sortAsc, setSortAsc] = useState<boolean>(() => {
    try { return localStorage.getItem("sortAsc") !== "0"; } catch { return true; }
  });

  useEffect(() => {
    try {
      localStorage.setItem("sortKey", sortKey);
      localStorage.setItem("sortAsc", sortAsc ? "1" : "0");
    } catch {}
  }, [sortKey, sortAsc]);

  function toggleSort(key: SortKey) {
    if (key === sortKey) setSortAsc(v => !v);
    else { setSortKey(key); setSortAsc(true); }
  }

  const [navCollapsed, setNavCollapsed] = useState<boolean>(() => {
    try { return localStorage.getItem("navCollapsed") === "1"; } catch { return false; }
  });

  useEffect(() => {
    try { localStorage.setItem("navCollapsed", navCollapsed ? "1" : "0"); } catch {}
  }, [navCollapsed]);

  useEffect(() => {
    invoke<Settings>("get_settings")
      .then(s => setUnits(s.units))
      .catch(e => console.error("get_settings failed:", e));
  }, []);

  useEffect(() => {
    const block = (e: Event) => e.preventDefault();
    document.addEventListener("dragstart", block);
    document.addEventListener("contextmenu", block);
    return () => {
      document.removeEventListener("dragstart", block);
      document.removeEventListener("contextmenu", block);
    };
  }, []);

  const [addStage, setAddStage] = useState<AddStage | null>(null);
  const [magnetInput, setMagnetInput] = useState("");
  const [preview, setPreview] = useState<TorrentPreview | null>(null);
  const [resolvedSource, setResolvedSource] = useState<string | null>(null);
  const [pickedFiles, setPickedFiles] = useState<Set<number>>(new Set());
  const [outputFolder, setOutputFolder] = useState("");
  const [addError, setAddError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const magnetRef = useRef<HTMLInputElement>(null);

  const [filesModal, setFilesModal] = useState<{ id: number; name: string; files: FileEntry[] } | null>(null);
  const [filesPicked, setFilesPicked] = useState<Set<number>>(new Set());

  const [removeTarget, setRemoveTarget] = useState<Torrent | null>(null);
  const [deleteFiles, setDeleteFiles] = useState(false);

  const [view, setView] = useState<"torrents" | "search">("torrents");
  const [toolbarExpanded, setToolbarExpanded] = useState(true);
  const toolbarTimer = useRef<number | undefined>(undefined);

  function leaveSearch() {
    setView("torrents");
    window.clearTimeout(toolbarTimer.current);
    toolbarTimer.current = window.setTimeout(() => setToolbarExpanded(true), 375);
  }

  useEffect(() => () => window.clearTimeout(toolbarTimer.current), []);

  const [displayedView, setDisplayedView] = useState(view);
  const [viewLeaving, setViewLeaving] = useState(false);

  useEffect(() => {
    if (view === displayedView) return;
    setViewLeaving(true);
    const t = window.setTimeout(() => {
      setDisplayedView(view);
      setViewLeaving(false);
    }, 130);
    return () => window.clearTimeout(t);
  }, [view, displayedView]);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SearchResult[] | null>(null);
  const [searchErrors, setSearchErrors] = useState<string[]>([]);
  const [searching, setSearching] = useState(false);
  const [plugins, setPlugins] = useState<SearchPlugin[]>([]);
  const searchRef = useRef<HTMLInputElement>(null);

  const [menu, setMenu] = useState<{ x: number; y: number; torrent: Torrent } | null>(null);
  const [moving, setMoving] = useState<number | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [detail, setDetail] = useState<TorrentDetail | null>(null);
  const [peerRates, setPeerRates] = useState<Record<string, { down: number; up: number }>>({});
  const peerPrev = useRef<{
    at: number;
    bytes: Record<string, { down: number; up: number }>;
  } | null>(null);
  const [fileRates, setFileRates] = useState<Record<number, number>>({});
  const filePrev = useRef<{ at: number; bytes: Record<number, number> } | null>(null);
  const [detailTab, setDetailTab] = useState<DetailTab>("general");
  const [showDetail, setShowDetail] = useState<boolean>(() => {
    try { return localStorage.getItem("showDetail") !== "0"; } catch { return true; }
  });

  useEffect(() => {
    try { localStorage.setItem("showDetail", showDetail ? "1" : "0"); } catch { /* ignore */ }
  }, [showDetail]);
  const toastTimer = useRef<number | undefined>(undefined);

  function notify(message: string) {
    window.clearTimeout(toastTimer.current);
    setToast(message);
    toastTimer.current = window.setTimeout(() => setToast(null), 3000);
  }

  useEffect(() => () => window.clearTimeout(toastTimer.current), []);

  const [showSettings, setShowSettings] = useState(false);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [interfaces, setInterfaces] = useState<NetworkInterface[]>([]);
  const [geoip, setGeoip] = useState<GeoipStatus | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [portMode, setPortMode] = useState<"auto" | "fixed">("auto");

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const [list, status] = await Promise.all([
          invoke<Torrent[]>("get_torrents"),
          invoke<SessionStatus>("get_session_status"),
        ]);
        if (cancelled) return;
        setTorrents(list);
        setConn(status);
      } catch (e) {
        console.error("poll failed:", e);
        if (!cancelled) setConn(null);
      }
    };
    poll();
    const timer = setInterval(poll, 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, []);

  const resolveSource = useCallback(async (source: string) => {
    setAddStage("resolving");
    setAddError(null);
    try {
      const p = await invoke<TorrentPreview>("preview_torrent", { source });
      setPreview(p);
      setResolvedSource(source);
      setPickedFiles(new Set(p.files.filter(f => !f.padding).map(f => f.index)));
      setOutputFolder(p.outputFolder);
      setAddStage("review");
    } catch (e) {
      setAddError(String(e));
      setAddStage("input");
    }
  }, []);

  function goToReview() {
    const source = magnetInput.trim();
    if (!source) return;
    if (preview && resolvedSource === source) {
      setAddError(null);
      setAddStage("review");
      return;
    }
    resolveSource(source);
  }

  useEffect(() => {
    const drain = async () => {
      try {
        const sources = await invoke<string[]>("take_pending_opens");
        if (sources.length > 0) {
          setMagnetInput(sources[0]);
          setAddStage("input");
          resolveSource(sources[0]);
        }
      } catch (e) {
        console.error("take_pending_opens failed:", e);
      }
    };
    drain();
    const unlisten = listen("torrent-open-request", drain);
    return () => { unlisten.then(fn => fn()); };
  }, [resolveSource]);

  useEffect(() => {
    if (addStage === "input") {
      const t = setTimeout(() => magnetRef.current?.focus(), 50);
      return () => clearTimeout(t);
    }
  }, [addStage]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (removeTarget) setRemoveTarget(null);
      else if (view === "search") leaveSearch();
      else if (showSettings) setShowSettings(false);
      else if (filesModal) setFilesModal(null);
      else if (addStage) closeAddDialog();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const ROW_EXIT_MS = 370;
  // Below this the interval is too short to divide by without inflating rates.
  const MIN_SAMPLE_MS = 200;

  const visible = torrents
    .filter(t => filter === "all" || t.status === filter)
    .filter(t => search === "" || t.name.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => {
      const c = compareTorrents(a, b, sortKey);
      return (c !== 0 ? (sortAsc ? c : -c) : a.id - b.id);
    });

  const rows = useExitTransition(visible, ROW_EXIT_MS);

  useEffect(() => {
    if (!showDetail || view !== "torrents" || selectedId === null) {
      setDetail(null);
      peerPrev.current = null;
      filePrev.current = null;
      setPeerRates({});
      setFileRates({});
      return;
    }
    let cancelled = false;
    const load = async () => {
      try {
        const d = await invoke<TorrentDetail>("get_torrent_detail", { id: selectedId });
        if (cancelled) return;
        setDetail(d);
        const now = performance.now();
        const bytes: Record<string, { down: number; up: number }> = {};
        for (const p of d.peers) bytes[p.addr] = { down: p.downBytes, up: p.upBytes };
        const prev = peerPrev.current;
        if (prev && now - prev.at >= MIN_SAMPLE_MS) {
          const dt = (now - prev.at) / 1000;
          const rates: Record<string, { down: number; up: number }> = {};
          for (const [addr, b] of Object.entries(bytes)) {
            const was = prev.bytes[addr];
            if (!was) continue;
            rates[addr] = {
              down: b.down >= was.down ? (b.down - was.down) / dt : 0,
              up: b.up >= was.up ? (b.up - was.up) / dt : 0,
            };
          }
          setPeerRates(rates);
        }
        peerPrev.current = { at: now, bytes };

        const fbytes: Record<number, number> = {};
        for (const f of d.files) fbytes[f.index] = f.doneBytes;
        const fprev = filePrev.current;
        if (fprev && now - fprev.at >= MIN_SAMPLE_MS) {
          const dt = (now - fprev.at) / 1000;
          const frates: Record<number, number> = {};
          for (const [idx, b] of Object.entries(fbytes)) {
            const was = fprev.bytes[Number(idx)];
            if (was !== undefined && b > was) frates[Number(idx)] = (b - was) / dt;
          }
          setFileRates(frates);
        }
        filePrev.current = { at: now, bytes: fbytes };
      } catch {
        if (!cancelled) setDetail(null);
      }
    };
    load();
    const timer = setInterval(load, 1000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [showDetail, view, selectedId]);

  function closeAddDialog() {
    if (preview) invoke("cancel_preview", { infoHash: preview.infoHash }).catch(() => {});
    setAddStage(null);
    setPreview(null);
    setResolvedSource(null);
    setPickedFiles(new Set());
    setMagnetInput("");
    setAddError(null);
  }

  async function handleBrowseTorrentFile() {
    const path = await open({
      multiple: false,
      filters: [{ name: "Torrent", extensions: ["torrent"] }],
    });
    if (typeof path === "string") { setMagnetInput(path); resolveSource(path); }
  }

  async function handleChooseFolder() {
    const dir = await open({ directory: true, defaultPath: outputFolder || undefined });
    if (typeof dir === "string") setOutputFolder(dir);
  }

  async function handleConfirmAdd() {
    if (!preview) return;
    setBusy(true);
    setAddError(null);
    try {
      await invoke("add_prepared_torrent", {
        infoHash: preview.infoHash,
        onlyFiles: [...pickedFiles],
        outputFolder,
      });
      setPreview(null);
      setResolvedSource(null);
      setAddStage(null);
      setMagnetInput("");
      setPickedFiles(new Set());
    } catch (e) {
      setAddError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function openFilesModal(forId?: number) {
    const id = forId ?? selectedId;
    if (id === null || id === undefined) return;
    const t = torrents.find(x => x.id === id);
    if (!t) return;
    try {
      const files = await invoke<FileEntry[]>("get_torrent_files", { id });
      setFilesModal({ id, name: t.name, files });
      setFilesPicked(new Set(files.filter(f => f.selected && !f.padding).map(f => f.index)));
    } catch (e) {
      setAddError(String(e));
    }
  }

  async function handleSaveFiles() {
    if (!filesModal) return;
    setBusy(true);
    try {
      await invoke("update_torrent_files", {
        id: filesModal.id,
        onlyFiles: [...filesPicked],
      });
      setFilesModal(null);
    } catch (e) {
      alert(`Failed to update files: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function recheckTorrent(t: Torrent) {
    const ok = window.confirm(
      `Re-verify "${t.name}" against the files on disk?

` +
        `This re-hashes ${t.size} and can take a while. Downloading resumes automatically.`,
    );
    if (!ok) return;
    try {
      await invoke("recheck_torrent", { id: t.id });
    } catch (e) {
      alert(`Could not recheck: ${e}`);
    }
  }

  async function reannounce(id: number) {
    try {
      await invoke("reannounce_torrent", { id });
      notify("Asked trackers to reannounce");
    } catch (e) {
      alert(`Could not reannounce: ${e}`);
    }
  }

  async function moveTorrent(t: Torrent) {
    const dir = await open({ directory: true, title: "Move data to" });
    if (typeof dir !== "string") return;
    setMoving(t.id);
    try {
      await invoke("move_torrent", { id: t.id, newFolder: dir });
      notify(`Moved to ${dir}`);
    } catch (e) {
      alert(`Could not move: ${e}`);
    } finally {
      setMoving(null);
    }
  }

  async function openFolder(id: number) {
    try {
      await invoke("open_torrent_folder", { id });
    } catch (e) {
      alert(`Could not open folder: ${e}`);
    }
  }

  async function copyMagnet(id: number) {
    try {
      const magnet = await invoke<string>("get_magnet_link", { id });
      await navigator.clipboard.writeText(magnet);
      notify("Magnet link copied");
    } catch (e) {
      alert(`Could not copy magnet link: ${e}`);
    }
  }

  function menuItems(t: Torrent): MenuItem[] {
    const running = t.status === "downloading" || t.status === "seeding";
    const stopped = t.status === "paused" || t.status === "error";
    return [
      { label: "Pause", icon: "pause", onSelect: () => pauseTorrent(t.id), disabled: !running },
      { label: "Resume", icon: "play", onSelect: () => resumeTorrent(t.id), disabled: !stopped },
      { label: "Force reannounce", icon: "announce", onSelect: () => reannounce(t.id), separatorBefore: true },
      { label: "Force recheck", icon: "recheck", onSelect: () => recheckTorrent(t) },
      { label: "Move data", icon: "move", onSelect: () => moveTorrent(t), disabled: moving !== null },
      { label: "Open containing folder", icon: "folder", onSelect: () => openFolder(t.id) },
      { label: "Files", icon: "files", onSelect: () => { setSelectedId(t.id); openFilesModal(t.id); } },
      { label: "Copy magnet link", icon: "copy", onSelect: () => copyMagnet(t.id) },
      { label: "Remove", icon: "remove", onSelect: () => setRemoveTarget(t), danger: true, separatorBefore: true },
    ];
  }

  async function openSearch() {
    window.clearTimeout(toolbarTimer.current);
    setToolbarExpanded(false);
    setView("search");
    setTimeout(() => searchRef.current?.focus(), 50);
    try {
      setPlugins(await invoke<SearchPlugin[]>("list_search_plugins"));
    } catch (e) {
      console.error("list_search_plugins failed:", e);
    }
    if (searchResults === null) runSearch("");
  }

  async function runSearch(override?: string) {
    const q = override ?? searchQuery.trim();
    setSearching(true);
    setSearchErrors([]);
    try {
      const r = await invoke<SearchResponse>("search_torrents", { query: q });
      setSearchResults(r.results);
      setSearchErrors(r.errors);
    } catch (e) {
      setSearchResults([]);
      setSearchErrors([String(e)]);
    } finally {
      setSearching(false);
    }
  }

  async function togglePlugin(id: string, enabled: boolean) {
    try {
      setPlugins(await invoke<SearchPlugin[]>("set_search_plugin_enabled", { id, enabled }));
    } catch (e) {
      alert(`Could not update source: ${e}`);
    }
  }

  function addFromSearch(result: SearchResult) {
    setMagnetInput(result.link);
    setAddStage("input");
    resolveSource(result.link);
  }

  async function openSettings() {
    setSettingsError(null);
    try {
      const [s, ifaces, geo] = await Promise.all([
        invoke<Settings>("get_settings"),
        invoke<NetworkInterface[]>("list_network_interfaces"),
        invoke<GeoipStatus>("geoip_status"),
      ]);
      setGeoip(geo);
      setDraft(s);
      setPortMode(s.listenPort == null ? "auto" : "fixed");
      setInterfaces(ifaces);
      setShowSettings(true);
    } catch (e) {
      alert(`Could not load settings: ${e}`);
    }
  }

  async function handleSaveSettings() {
    if (!draft) return;
    setBusy(true);
    setSettingsError(null);
    try {
      const payload: Settings = {
        ...draft,
        listenPort: portMode === "auto" ? null : draft.listenPort,
      };
      const status = await invoke<SessionStatus>("set_settings", { settings: payload });
      setConn(status);
      setUnits(payload.units);
      setShowSettings(false);
    } catch (e) {
      setSettingsError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handleConfirmRemove() {
    if (!removeTarget) return;
    setBusy(true);
    try {
      await invoke("remove_torrent", { id: removeTarget.id, deleteFiles });
      setSelectedId(null);
      setRemoveTarget(null);
      setDeleteFiles(false);
    } catch (e) {
      alert(`Failed to remove: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function pauseTorrent(id: number) {
    try {
      await invoke("pause_torrent", { id });
    } catch (e) {
      alert(`Failed to pause: ${e}`);
    }
  }

  async function resumeTorrent(id: number) {
    try {
      await invoke("resume_torrent", { id });
    } catch (e) {
      alert(`Failed to resume: ${e}`);
    }
  }

  function handlePause() { if (selectedId !== null) pauseTorrent(selectedId); }
  function handleResume() { if (selectedId !== null) resumeTorrent(selectedId); }

  const sel = selectedId !== null ? torrents.find(t => t.id === selectedId) ?? null : null;
  const canPause = sel?.status === "downloading" || sel?.status === "seeding";
  const canResume = sel?.status === "paused" || sel?.status === "error";

  const hasSeeders = (searchResults ?? []).some(r => r.seeders != null);


  const renderDetail = () => {
    if (!detail) {
      return <div className="detail-empty">Select a torrent to see its details.</div>;
    }
    if (detailTab === "general") {
      return (
        <dl className="detail-grid">
          <dt>Name</dt><dd title={detail.name}>{detail.name}</dd>
          <dt>Save path</dt><dd title={detail.savePath}>{detail.savePath}</dd>
          <dt>Info hash</dt><dd className="num">{detail.infoHash}</dd>
          <dt>Size</dt><dd className="num">{detail.size}</dd>
          <dt>Downloaded</dt><dd className="num">{detail.downloaded}</dd>
          <dt>Uploaded</dt><dd className="num">{detail.uploaded}</dd>
          <dt>Ratio</dt><dd className="num">{detail.ratio}</dd>
          <dt>Pieces</dt><dd className="num">{detail.pieces} x {detail.pieceSize}</dd>
          <dt>Peers</dt>
          <dd className="num">
            {detail.peersLive} connected, {detail.peersConnecting} connecting,{" "}
            {detail.peersQueued} queued, {detail.peersSeen} seen
          </dd>
        </dl>
      );
    }
    if (detailTab === "files") {
      return (
        <div className="detail-list">
          {detail.files.map(f => (
            <div className="detail-file" key={f.index}>
              <Icon name={iconForFile(f.path)} className="filerow-icon" size={16} />
              <span className="detail-file-name" title={f.path}>{f.path}</span>
              <span className="detail-file-bar">
                <span style={{ width: `${f.progress}%` }} />
              </span>
              <span className="num detail-file-pct">{formatPercent(f.progress)}%</span>
              <span className="num detail-file-rate">
                {fileRates[f.index] ? `${formatSize(fileRates[f.index], units)}/s` : ""}
              </span>
              <span className="num detail-file-size">{f.sizeStr}</span>
              {!f.selected && <span className="detail-skip">skipped</span>}
            </div>
          ))}
        </div>
      );
    }
    if (detailTab === "peers") {
      if (detail.peers.length === 0) {
        return <div className="detail-empty">No connected peers.</div>;
      }
      return (
        <div className="detail-list">
          <div className="detail-peer detail-peer-head">
            <span></span>
            <span>Address</span>
            <span>Client</span>
            <span className="right">Progress</span>
            <span className="right">↓ Speed</span>
            <span className="right">↑ Speed</span>
            <span className="right">↓ Total</span>
            <span className="right">↑ Total</span>
            <span>State</span>
          </div>
          {detail.peers.map(p => (
            <div className="detail-peer" key={p.addr}>
              <span className="peer-flag">{p.country ?? ""}</span>
              <span className="num" title={p.addr}>{p.addr}</span>
              <span title={p.client}>{p.client}</span>
              <span className="num right">{formatPercent(p.progress)}%</span>
              <span className="num right">
                {peerRates[p.addr]?.down ? `${formatSize(peerRates[p.addr].down, units)}/s` : "—"}
              </span>
              <span className="num right">
                {peerRates[p.addr]?.up ? `${formatSize(peerRates[p.addr].up, units)}/s` : "—"}
              </span>
              <span className="num right">{p.downloaded}</span>
              <span className="num right">{p.uploaded}</span>
              <span
                title={`${p.pieces} pieces, ${p.inflight} in flight${
                  p.interested ? ", interested" : ""
                }${p.errors ? `, ${p.errors} errors` : ""}`}
              >
                {p.state}
              </span>
            </div>
          ))}
        </div>
      );
    }
    return (
      <div className="detail-list">
        {detail.trackers.length === 0 ? (
          <div className="detail-empty">No trackers - this torrent relies on DHT.</div>
        ) : (
          detail.trackers.map(t => (
            <div className="detail-tracker" key={t} title={t}>{t}</div>
          ))
        )}
      </div>
    );
  };

  const renderSearchPage = () => (
    <div className="search-page">
      {searchErrors.length > 0 && (
        <div className="search-notice modal-error">{searchErrors.join(" · ")}</div>
      )}

      {searching ? (
        <div className="empty-state">
          <div className="resolving">
            <div className="spinner" />
            <div className="resolving-title">Searching sources</div>
          </div>
        </div>
      ) : !searchResults || searchResults.length === 0 ? (
        <div className="empty-state"><p>No results.</p></div>
      ) : (
        <>

          <div className="result-list">
            <div className={`result-header${hasSeeders ? "" : " no-seeders"}`}>
              <div>Name</div>
              <div className="right">Size</div>
              {hasSeeders && <div className="right">Seeders</div>}
              <div>Source</div>
            </div>
            {searchResults.map((r, i) => (
              <button
                key={`${r.link}-${i}`}
                type="button"
                className={`result-row${hasSeeders ? "" : " no-seeders"}`}
                onClick={() => addFromSearch(r)}
                title={r.name}
              >
                <span className="result-name">{r.name}</span>
                <span className="result-meta num">
                  {r.size != null ? formatSize(r.size, units) : "—"}
                </span>
                {hasSeeders && (
                  <span className="result-meta num">
                    {r.seeders != null ? r.seeders : "—"}
                  </span>
                )}
                <span className="result-source">{r.source}</span>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );

  return (
    <div className={`app${navCollapsed ? " nav-collapsed" : ""}`}>

      {addStage && (
        <div className="modal-overlay" role="dialog" aria-modal="true" onClick={closeAddDialog}>
          <div
            className={`modal${addStage === "review" ? " modal-wide" : ""}`}
            onClick={e => e.stopPropagation()}
          >
            <div className="modal-title">
              {addStage === "review" ? "Confirm torrent" : "Add torrent"}
            </div>

            <AutoHeight>
            <div className="modal-body">
              {addError && <div className="modal-error">{addError}</div>}

              {addStage === "input" && (
                <>
                  <input
                    ref={magnetRef}
                    type="text"
                    className="filter-input modal-input"
                    placeholder="Paste magnet link or torrent URL"
                    value={magnetInput}
                    onChange={e => setMagnetInput(e.target.value)}
                    onKeyDown={e => {
                      if (e.key === "Enter") goToReview();
                    }}
                  />
                  <div className="modal-actions">
                    <button className="btn spread" type="button" onClick={handleBrowseTorrentFile}>
                      Browse .torrent
                    </button>
                    <button className="btn" type="button" onClick={closeAddDialog}>
                      Cancel
                    </button>
                    <button
                      className="btn btn-primary"
                      type="button"
                      onClick={goToReview}
                      disabled={!magnetInput.trim()}
                    >
                      Next
                    </button>
                  </div>
                </>
              )}

              {addStage === "resolving" && (
                <div className="resolving">
                  <div className="spinner" />
                  <div>
                    <div className="resolving-title">Fetching torrent metadata</div>
                    <div className="resolving-sub">
                      Magnet links need to run a peer lookup first. This may take a few seconds.
                    </div>
                  </div>
                  <button className="btn" type="button" onClick={closeAddDialog}>
                    Cancel
                  </button>
                </div>
              )}

              {addStage === "review" && preview && (
                <>
                  <div className="dialog-subject">
                    <div className="dialog-subject-name" title={preview.name}>{preview.name}</div>
                    <div className="dialog-subject-meta">
                      {preview.totalSizeStr} · {preview.files.filter(f => !f.padding).length} files
                    </div>
                  </div>

                  <div className="field">
                    <label className="field-label" htmlFor="save-to">Save to</label>
                    <div className="field-row">
                      <input
                        id="save-to"
                        type="text"
                        className="filter-input"
                        value={outputFolder}
                        onChange={e => setOutputFolder(e.target.value)}
                      />
                      <button className="btn" type="button" onClick={handleChooseFolder}>
                        Browse
                      </button>
                    </div>
                  </div>

                  <div className="field">
                    <div className="field-label">Files</div>
                    <FilePicker
                      files={preview.files}
                      selected={pickedFiles}
                      onChange={setPickedFiles}
                      units={units}
                    />
                  </div>

                  <div className="modal-actions">
                    <button
                      className="btn spread"
                      type="button"
                      onClick={() => setAddStage("input")}
                      disabled={busy}
                    >
                      Back
                    </button>
                    <button className="btn" type="button" onClick={closeAddDialog} disabled={busy}>
                      Cancel
                    </button>
                    <button
                      className="btn btn-primary"
                      type="button"
                      onClick={handleConfirmAdd}
                      disabled={busy || pickedFiles.size === 0}
                    >
                      {busy ? "Adding" : "Download"}
                    </button>
                  </div>
                </>
              )}
            </div>
            </AutoHeight>
          </div>
        </div>
      )}

      {filesModal && (
        <div className="modal-overlay" role="dialog" aria-modal="true" onClick={() => setFilesModal(null)}>
          <div className="modal modal-wide" onClick={e => e.stopPropagation()}>
            <div className="modal-title">Files</div>
            <div className="modal-body">
              <div className="dialog-subject">
                <div className="dialog-subject-name" title={filesModal.name}>{filesModal.name}</div>
                <div className="dialog-subject-meta">
                  Unselected files are skipped automatically and won't be downloaded.
                </div>
              </div>

              <FilePicker
                files={filesModal.files}
                selected={filesPicked}
                onChange={setFilesPicked}
                units={units}
              />
              <div className="modal-actions">
                <button className="btn" type="button" onClick={() => setFilesModal(null)}>
                  Cancel
                </button>
                <button
                  className="btn btn-primary"
                  type="button"
                  onClick={handleSaveFiles}
                  disabled={busy || filesPicked.size === 0}
                >
                  {busy ? "Saving" : "Save"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {showSettings && draft && (
        <div className="modal-overlay" role="dialog" aria-modal="true" onClick={() => setShowSettings(false)}>
          <div className="modal modal-settings" onClick={e => e.stopPropagation()}>
            <div className="modal-title">Network</div>
            <div className="modal-body">
              {settingsError && <div className="modal-error">{settingsError}</div>}

              <div className="settings-list">
                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">Incoming port</div>
                    <div className="setting-desc">
                      For VPNs, select manual and enter the port provided by the VPN client.
                    </div>
                  </div>
                  <div className="setting-control">
                    <Select
                      ariaLabel="Port mode"
                      width={132}
                      value={portMode}
                      onChange={v => setPortMode(v as "auto" | "fixed")}
                      options={[
                        { value: "auto", label: "Automatic" },
                        { value: "fixed", label: "Manual" },
                      ]}
                    />
                    {portMode === "fixed" && (
                      <input
                        type="number"
                        className="filter-input port-input"
                        min={1}
                        max={65535}
                        placeholder="6881"
                        value={draft.listenPort ?? ""}
                        onChange={e =>
                          setDraft({
                            ...draft,
                            listenPort: e.target.value ? Number(e.target.value) : null,
                          })
                        }
                      />
                    )}
                  </div>
                </div>

                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">Network interface</div>
                    <div className="setting-desc">
                      Connects all trackers, DHT, and peers to the bound interface. Leave on Any to use all interfaces. Recommended to prevent IP leaks.
                    </div>
                  </div>
                  <div className="setting-control">
                    <Select
                      ariaLabel="Network interface"
                      width={230}
                      value={draft.bindIp ?? ""}
                      onChange={v => setDraft({ ...draft, bindIp: v || null })}
                      options={[
                        { value: "", label: "Any" },
                        ...interfaces.map(i => ({
                          value: i.ip,
                          label: `${i.name} — ${i.ip}${i.isLoopback ? " (loopback)" : ""}`,
                        })),
                      ]}
                    />
                  </div>
                </div>

                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">Speed limits</div>
                    <div className="setting-desc">
                      Provide KiB/s. Leave empty for unlimited. 
                    </div>
                  </div>
                  <div className="setting-control">
                    <label className="limit-field">
                      <span>Down</span>
                      <input
                        type="number"
                        className="filter-input port-input"
                        min={0}
                        placeholder="∞"
                        value={draft.downloadLimit ?? ""}
                        onChange={e =>
                          setDraft({
                            ...draft,
                            downloadLimit: e.target.value ? Number(e.target.value) : null,
                          })
                        }
                      />
                    </label>
                    <label className="limit-field">
                      <span>Up</span>
                      <input
                        type="number"
                        className="filter-input port-input"
                        min={0}
                        placeholder="∞"
                        value={draft.uploadLimit ?? ""}
                        onChange={e =>
                          setDraft({
                            ...draft,
                            uploadLimit: e.target.value ? Number(e.target.value) : null,
                          })
                        }
                      />
                    </label>
                  </div>
                </div>

                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">Peer country database</div>
                    <div className="setting-desc truncate">
                      {draft.geoipPath
                        ? draft.geoipPath
                        : geoip?.loaded
                          ? `${geoip.databaseType ?? "Unknown"} built ${geoip.built ?? "?"} · ${
                              geoip.attribution ?? ""
                            }`
                          : "No database loaded - peer flags are hidden."}
                    </div>
                  </div>
                  <div className="setting-control">
                    {draft.geoipPath && (
                      <button
                        className="btn"
                        type="button"
                        onClick={() => setDraft({ ...draft, geoipPath: null })}
                      >
                        Use bundled
                      </button>
                    )}
                    <button
                      className="btn"
                      type="button"
                      onClick={async () => {
                        const f = await open({
                          multiple: false,
                          filters: [{ name: "MaxMind DB", extensions: ["mmdb"] }],
                        });
                        if (typeof f === "string") setDraft({ ...draft, geoipPath: f });
                      }}
                    >
                      Choose
                    </button>
                  </div>
                </div>

                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">Size units</div>
                    <div className="setting-desc">
                      Binary uses KiB, MiB, GiB. Decimal uses KB, MB, GB. This only affects display.
                    </div>
                  </div>
                  <div className="setting-control">
                    <Select
                      ariaLabel="Size units"
                      width={190}
                      value={draft.units}
                      onChange={v => setDraft({ ...draft, units: v as Units })}
                      options={[
                        { value: "binary", label: "Binary (KiB, MiB, GiB)" },
                        { value: "decimal", label: "Decimal (KB, MB, GB)" },
                      ]}
                    />
                  </div>
                </div>

                <div className="setting-row">
                  <div className="setting-text">
                    <div className="setting-label">UPnP forwarding</div>
                    <div className="setting-desc">
                     VPNs mostly use NAT-PMP, please manually select port for these. Routers can use UPnP.
                    </div>
                  </div>
                  <div className="setting-control">
                    <label className="switch">
                      <input
                        type="checkbox"
                        checked={draft.upnp}
                        onChange={e => setDraft({ ...draft, upnp: e.target.checked })}
                      />
                      <span className="switch-track" aria-hidden="true" />
                    </label>
                  </div>
                </div>
              </div>

              <div className="modal-actions">
                <span className="modal-actions-note">
                  Applying restarts QuantumTorrent. Any existing streams will be restarted automatically.
                </span>
                <button className="btn" type="button" onClick={() => setShowSettings(false)} disabled={busy}>
                  Cancel
                </button>
                <button className="btn btn-primary" type="button" onClick={handleSaveSettings} disabled={busy}>
                  {busy ? "Applying" : "Apply"}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {removeTarget && (
        <div className="modal-overlay" role="dialog" aria-modal="true" onClick={() => setRemoveTarget(null)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <div className="modal-title">Remove torrent</div>
            <AutoHeight>
            <div className="modal-body">
              <div className="dialog-subject">
                <div className="dialog-subject-name" title={removeTarget.name}>
                  {removeTarget.name}
                </div>
                <div className="dialog-subject-meta">
                  {removeTarget.size} · {STATUS_LABELS[removeTarget.status]}
                </div>
              </div>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={deleteFiles}
                  onChange={e => setDeleteFiles(e.target.checked)}
                />
                <span>Also delete downloaded files from disk</span>
              </label>
              {deleteFiles && (
                <p className="field-warning">
                  {removeTarget.size} will be permanently deleted from your disk. This is irreversible.
                </p>
              )}
              <div className="modal-actions">
                <button
                  className="btn"
                  type="button"
                  onClick={() => { setRemoveTarget(null); setDeleteFiles(false); }}
                >
                  Cancel
                </button>
                <button
                  className={`btn ${deleteFiles ? "btn-danger" : "btn-primary"}`}
                  type="button"
                  onClick={handleConfirmRemove}
                  disabled={busy}
                >
                  {deleteFiles ? "Remove and delete" : "Remove"}
                </button>
              </div>
            </div>
            </AutoHeight>
          </div>
        </div>
      )}

      <div className="toolbar">

        <div className="toolbar-group" key="toolbar-left">
          {view === "search" ? (
            <div className="toolbar-actions" key="search">
              <button className="btn" type="button" onClick={leaveSearch}>
                <Icon name="chevronLeft" />
                Back
              </button>
              <span className="toolbar-divider" />
            </div>
          ) : !toolbarExpanded ? null : (
            <div className="toolbar-actions" key="torrents">
              <button className="btn btn-primary" type="button" onClick={() => setAddStage("input")}>
                <Icon name="add" />
                Add torrent
              </button>
              <button className="btn" type="button" onClick={openSearch}>
                <Icon name="search" />
                Search
              </button>
              <span className="toolbar-divider" />
              <button className="btn" type="button" onClick={handlePause} disabled={!canPause}>
                <Icon name="pause" />
                Pause
              </button>
              <button className="btn" type="button" onClick={handleResume} disabled={!canResume}>
                <Icon name="play" />
                Resume
              </button>
              <button
                className="btn"
                type="button"
                onClick={() => openFilesModal()}
                disabled={selectedId === null}
              >
                <Icon name="files" />
                Files
              </button>
              <button
                className="btn"
                type="button"
                onClick={() => sel && setRemoveTarget(sel)}
                disabled={selectedId === null}
              >
                <Icon name="remove" />
                Remove
              </button>
            </div>
          )}
        </div>

        <div
          className={`search-field${view === "search" ? " search-field-grow" : ""}`}
          key="toolbar-search"
        >
          <Icon name="search" className="search-icon" size={18} />
          <input
            ref={searchRef}
            className="filter-input"
            type="text"
            placeholder={view === "search" ? "Search torrents" : "Filter torrents"}
            value={view === "search" ? searchQuery : search}
            onChange={e =>
              view === "search" ? setSearchQuery(e.target.value) : setSearch(e.target.value)
            }
            onKeyDown={e => {
              if (view === "search" && e.key === "Enter") runSearch();
            }}
          />

          {view === "search" && (
            <button
              className="icon-btn icon-btn-sm search-submit"
              type="button"
              onClick={() => runSearch()}
              disabled={searching}
              aria-label="Search"
              title="Search"
            >
              {searching ? <span className="spinner spinner-sm" /> : <Icon name="chevronRight" size={18} />}
            </button>
          )}
        </div>
      </div>

      <div className="sidebar">
        <div className="section-label">{displayedView === "search" ? "Sources" : "State"}</div>
        <div
          className={`view-swap${viewLeaving ? " leaving" : ""}`}
          key={`sidebar-${displayedView}`}
        >
        {displayedView === "search" ? (
          <>
            {plugins.map(p => (
              <button
                key={p.id}
                type="button"
                className={`sidebar-item${p.enabled ? " active" : ""}`}
                onClick={() => togglePlugin(p.id, !p.enabled)}
                title={p.site ?? (p.builtin ? undefined : "User plugin")}
                aria-pressed={p.enabled}
              >
                <span>{p.name}</span>
                <span className="count">{p.enabled ? "On" : "Off"}</span>
              </button>
            ))}
            <button
              type="button"
              className="sidebar-item sidebar-action"
              onClick={() => invoke("open_plugins_folder").catch(e => alert(String(e)))}
            >
              <Icon name="folder" className="btn-icon" size={16} />
              <span>Add sources</span>
            </button>
          </>
        ) : (
          FILTERS.map(f => (
            <button
              key={f}
              type="button"
              className={`sidebar-item${filter === f ? " active" : ""}`}
              onClick={() => setFilter(f)}
            >
              <span>{f === "all" ? "All" : STATUS_LABELS[f]}</span>
              <span className="count">
                {f === "all" ? torrents.length : torrents.filter(t => t.status === f).length}
              </span>
            </button>
          ))
        )}
        </div>
      </div>

      <div className="main">
        <div className="main-body">
        <div
          className={`view-swap view-swap-fill${viewLeaving ? " leaving" : ""}`}
          key={`main-${displayedView}`}
        >
        {displayedView === "search" ? (
          renderSearchPage()
        ) : torrents.length === 0 ? (
          <div className="empty-state">
            <p>No torrents exist — click <strong>Add torrent</strong> to get started.</p>
          </div>
        ) : visible.length === 0 ? (
          <div className="empty-state">
            <p>No torrents exist for this filter.</p>
          </div>
        ) : (
          <div className="torrents" role="table">
            <div className="torrents-header" role="row">
              {COLUMNS.map(col => (
                <div
                  key={col.key}
                  className="cell"
                  role="columnheader"
                  aria-sort={
                    sortKey === col.key ? (sortAsc ? "ascending" : "descending") : "none"
                  }
                >
                  <button
                    type="button"
                    className={`col-sort${sortKey === col.key ? " active" : ""}${col.numeric ? " right" : ""}`}
                    onClick={() => toggleSort(col.key)}
                  >
                    <span>{col.label}</span>
                    {sortKey === col.key && (
                      <Icon
                        name="expandMore"
                        className={`sort-arrow${sortAsc ? " asc" : ""}`}
                        size={16}
                      />
                    )}
                  </button>
                </div>
              ))}
            </div>
            <div className="torrents-body" role="rowgroup">
              {rows.map(({ t, exiting }) => (
                <div
                  key={t.id}
                  role="row"
                  tabIndex={exiting ? -1 : 0}
                  aria-selected={selectedId === t.id}
                  className={`torrent-row${selectedId === t.id ? " selected" : ""}${exiting ? " exiting" : ""}`}
                  title={t.error ?? undefined}
                  onClick={() => setSelectedId(t.id)}
                  onDoubleClick={() => { setSelectedId(t.id); openFilesModal(t.id); }}
                  onContextMenu={e => {
                    e.preventDefault();
                    setSelectedId(t.id);
                    setMenu({ x: e.clientX, y: e.clientY, torrent: t });
                  }}
                  onKeyDown={e => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelectedId(t.id);
                    }
                  }}
                >
                  <div className="cell name" role="cell">{t.name}</div>
                  <div className="cell num" role="cell">{t.size}</div>
                  <div className="cell" role="cell"><ProgressBar progress={t.progress} status={t.status} /></div>
                  <div className="cell" role="cell"><StatusPill status={t.status} /></div>
                  <div className="cell num" role="cell">{t.down}</div>
                  <div className="cell num" role="cell">{t.up}</div>
                  <div className="cell num" role="cell">{t.eta}</div>
                </div>
              ))}
            </div>
          </div>
        )}
        </div>
        </div>

        {displayedView === "torrents" && (
          <div
            className={`detail-panel${showDetail ? "" : " collapsed"}`}
            aria-hidden={!showDetail}
          >
            <div className="detail-tabs">
              {DETAIL_TABS.map(t => (
                <button
                  key={t.key}
                  type="button"
                  className={`detail-tab${detailTab === t.key ? " active" : ""}`}
                  onClick={() => setDetailTab(t.key)}
                >
                  {t.label}
                </button>
              ))}
              <button
                className="icon-btn icon-btn-sm detail-close"
                type="button"
                onClick={() => setShowDetail(false)}
                aria-label="Hide details"
                title="Hide details"
              >
                <Icon name="expandMore" size={18} />
              </button>
            </div>
            <div className="detail-body">{renderDetail()}</div>
          </div>
        )}
      </div>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.torrent)}
          onClose={() => setMenu(null)}
        />
      )}

      {toast && <div className="toast" role="status">{toast}</div>}

      <div className="statusbar">
        <button
          className="icon-btn icon-btn-sm"
          type="button"
          onClick={() => setNavCollapsed(c => !c)}
          aria-label={navCollapsed ? "Show filters" : "Hide filters"}
          aria-expanded={!navCollapsed}
          title={navCollapsed ? "Show filters" : "Hide filters"}
        >
          <Icon name={navCollapsed ? "chevronRight" : "chevronLeft"} />
        </button>
        <button
          className={`icon-btn icon-btn-sm${showDetail ? " active" : ""}`}
          type="button"
          onClick={() => setShowDetail(v => !v)}
          aria-label={showDetail ? "Hide details" : "Show details"}
          title={showDetail ? "Hide details" : "Show details"}
        >
          <Icon name="files" size={18} />
        </button>
        <button
          className="icon-btn icon-btn-sm"
          type="button"
          onClick={openSettings}
          aria-label="Network settings"
          title="Network settings"
        >
          <Icon name="settings" />
        </button>
        <span title="Torrents in the list">{torrents.length} torrents</span>

        {conn && (
          <span title={`${conn.peers} peers connected across all torrents`}>
            {conn.peers} peers
          </span>
        )}
        {conn?.dhtNodes != null && (
          <span title="Nodes in the DHT routing table">DHT {conn.dhtNodes}</span>
        )}
        {conn && (
          <span className="statusbar-rates">
            <span title={`Downloaded this session: ${conn.downloaded}`}>
              ↓ {conn.downSpeed}
            </span>
            <span title={`Uploaded this session: ${conn.uploaded}`}>
              ↑ {conn.upSpeed}
            </span>
          </span>
        )}
        {conn && (conn.downLimit || conn.upLimit) && (
          <span
            className="statusbar-limit"
            title={`Speed limit: ${conn.downLimit ? `${conn.downLimit} KiB/s down` : "no down limit"}, ${
              conn.upLimit ? `${conn.upLimit} KiB/s up` : "no up limit"
            } - change in Network settings`}
          >
            limited
          </span>
        )}

        <span
          className={`conn conn-${conn?.state ?? "offline"}`}
          title={conn?.detail ?? "Not connected to the engine."}
        >
          <span className="dot" />
          {conn ? CONN_LABELS[conn.state] : "Offline"}
        </span>
      </div>

    </div>
  );
}
