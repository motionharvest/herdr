//! Compact herdplay player at the bottom of the spaces sidebar.
//!
//! Collapsed: a 3-row rounded box matching `+ new` (border, bar, border).
//! Expanded: full-mode sidebar view (header, paste/Add, playlist, embed,
//! now-playing, transport, scrub, volume + status). Transport, Add, volume,
//! playlist, and seek talk to the localhost daemon. The daemon is polled on a
//! background thread so a down or slow localhost never stalls the UI.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use serde::Deserialize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Palette;
use crate::app::AppState;

/// Collapsed box: top border, bar, bottom border — same height as `+ new`.
pub(crate) const PLAYER_COLLAPSED_ROWS: u16 = 3;
/// Minimum full-mode height: header, paste, playlist, embed remnant, now-playing, transport, scrub, volume/status.
pub(crate) const PLAYER_FULL_MIN_ROWS: u16 = 18;
/// Minimum full-mode width. Default sidebar is 26 (player box ~25 after the
/// divider); design target is 32 at 122×45. Below this, paste/queue/scrub
/// collide; show a hint instead of cramming the full chrome.
pub(crate) const PLAYER_FULL_MIN_WIDTH: u16 = 22;
/// Rows kept above the full player for the spaces header, a remnant list, and `+ new`.
const FULL_KEEP_ABOVE: u16 = 9;

const DAEMON_HOST: &str = "127.0.0.1";
const DAEMON_PORT: u16 = 8737;
const POLL_MS: u64 = 750;
const IO_TIMEOUT: Duration = Duration::from_millis(80);

const COVER: &str = "♪";
const PREV: &str = "⏮";
const PLAY: &str = "▶";
const PAUSE: &str = "⏸";
const NEXT: &str = "⏭";
const LOOP: &str = "⟲";
const SHUFFLE: &str = "⇄";
const LINK_PLACEHOLDER: &str = "paste a link…";
const SAVE_NAME_PLACEHOLDER: &str = "playlist name…";
const ADD_LABEL: &str = "Add";
const CANCEL_LABEL: &str = "Cancel";
const SAVE_LABEL: &str = "save";
const SAVE_CONFIRM_LABEL: &str = "Save";
const LOAD_LABEL: &str = "load";
const VOL_LABEL: &str = "vol";
const VOLUME_STEP: f64 = 0.1;
const PLAYLIST_EMPTY: &str = "paste a link, then Add";
const SAVED_EMPTY: &str = "no saved playlists";
const QUEUE_LABEL: &str = "queue";
const CLEAR_LABEL: &str = "clear";
const OVERWRITE_NO: &str = "no";
const OVERWRITE_YES: &str = "yes";
const REMOVE_GLYPH: &str = "×";
const SCRUB_TIME_W: u16 = 6;
const PLAYLIST_MAX_ROWS: u16 = 8;
const TOO_SMALL_HINT: &str = "widen this pane for full player view";
const HEADER_LABEL: &str = "♪ PLAYER";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlayerSnapshot {
    Offline,
    Online {
        title: Option<String>,
        artist: Option<String>,
        url: Option<String>,
        playing: bool,
        looping: bool,
        shuffle: bool,
        elapsed_sec: u64,
        duration_sec: u64,
        volume: f64,
        seekable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PlaylistSnapshot {
    pub items: Vec<PlaylistItem>,
    pub index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlaylistItem {
    pub title: String,
    pub url: String,
}

/// Queue chrome: save/load sit on the `queue … save load [clear]` row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum PlayerQueueMode {
    #[default]
    Queue,
    SaveName,
    SaveOverwrite { name: String },
    LoadPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PlayerDensity {
    Compact,
    #[default]
    Comfortable,
    LargeText,
}

impl PlayerDensity {
    fn from_settings(value: &str) -> Self {
        match value {
            "compact" => Self::Compact,
            "large-text" => Self::LargeText,
            _ => Self::Comfortable,
        }
    }

    fn playlist_rows(self) -> u16 {
        match self {
            Self::Compact => 10,
            Self::Comfortable => PLAYLIST_MAX_ROWS,
            Self::LargeText => 4,
        }
    }

    fn hide_embed(self) -> bool {
        !matches!(self, Self::Comfortable)
    }
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self::Offline
    }
}

impl PlayerSnapshot {
    fn playing(&self) -> bool {
        matches!(
            self,
            Self::Online {
                playing: true,
                ..
            }
        )
    }

    fn looping(&self) -> bool {
        matches!(self, Self::Online { looping: true, .. })
    }

    fn shuffle(&self) -> bool {
        matches!(self, Self::Online { shuffle: true, .. })
    }

    pub(crate) fn volume(&self) -> f64 {
        match self {
            Self::Online { volume, .. } => *volume,
            Self::Offline => 1.0,
        }
    }

    fn duration_sec(&self) -> u64 {
        match self {
            Self::Online { duration_sec, .. } => *duration_sec,
            Self::Offline => 0,
        }
    }

    fn seekable(&self) -> bool {
        matches!(self, Self::Online { seekable: true, .. }) && self.duration_sec() > 0
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonState {
    title: Option<String>,
    artist: Option<String>,
    playing: Option<bool>,
    elapsed_sec: Option<f64>,
    duration_sec: Option<f64>,
    url: Option<String>,
    path: Option<String>,
    volume: Option<f64>,
    seekable: Option<bool>,
    queue: Option<DaemonQueue>,
}

#[derive(Debug, Deserialize)]
struct DaemonPlaylist {
    items: Option<Vec<DaemonPlaylistItem>>,
    index: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DaemonPlaylistItem {
    url: Option<String>,
    path: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DaemonQueue {
    #[serde(rename = "loop")]
    looping: Option<bool>,
    shuffle: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct SettingsField {
    key: String,
    #[serde(rename = "type")]
    field_type: String,
    value: serde_json::Value,
    #[serde(default)]
    allowed: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SettingsBody {
    fields: Option<Vec<SettingsField>>,
}

#[derive(Debug, Deserialize)]
struct SavedPlaylistsBody {
    names: Option<Vec<String>>,
}

static SNAPSHOT: Mutex<PlayerSnapshot> = Mutex::new(PlayerSnapshot::Offline);
static PLAYLIST: Mutex<PlaylistSnapshot> = Mutex::new(PlaylistSnapshot {
    items: Vec::new(),
    index: None,
});
static SAVED_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SETTINGS_FIELDS: Mutex<Vec<SettingsField>> = Mutex::new(Vec::new());
static DENSITY: Mutex<PlayerDensity> = Mutex::new(PlayerDensity::Comfortable);
static DENSITY_KEY: Mutex<Option<String>> = Mutex::new(None);
static DENSITY_VALUE: Mutex<String> = Mutex::new(String::new());
static DENSITY_ALLOWED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static POLLER: OnceLock<()> = OnceLock::new();
#[cfg(test)]
static TEST_POSTS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
#[cfg(test)]
static TEST_REPLIES: Mutex<Vec<(u16, String)>> = Mutex::new(Vec::new());
#[cfg(test)]
static TEST_SAVED: Mutex<Vec<String>> = Mutex::new(Vec::new());
#[cfg(test)]
static TEST_SETTINGS: Mutex<Vec<SettingsField>> = Mutex::new(Vec::new());
#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_test_player() -> std::sync::MutexGuard<'static, ()> {
    let guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TEST_REPLIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    TEST_SAVED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    TEST_SETTINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    *density_lock() = PlayerDensity::Comfortable;
    *density_key_lock() = None;
    *density_value_lock() = String::new();
    *density_allowed_lock() = Vec::new();
    *saved_lock() = Vec::new();
    *settings_lock() = Vec::new();
    guard
}

fn snapshot_lock() -> std::sync::MutexGuard<'static, PlayerSnapshot> {
    SNAPSHOT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn playlist_lock() -> std::sync::MutexGuard<'static, PlaylistSnapshot> {
    PLAYLIST
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn saved_lock() -> std::sync::MutexGuard<'static, Vec<String>> {
    SAVED_NAMES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn settings_lock() -> std::sync::MutexGuard<'static, Vec<SettingsField>> {
    SETTINGS_FIELDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn density_lock() -> std::sync::MutexGuard<'static, PlayerDensity> {
    DENSITY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn density_key_lock() -> std::sync::MutexGuard<'static, Option<String>> {
    DENSITY_KEY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn density_value_lock() -> std::sync::MutexGuard<'static, String> {
    DENSITY_VALUE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn density_allowed_lock() -> std::sync::MutexGuard<'static, Vec<String>> {
    DENSITY_ALLOWED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn current_density() -> PlayerDensity {
    *density_lock()
}

fn current_density_value() -> String {
    let value = density_value_lock().clone();
    if value.is_empty() {
        "comfortable".into()
    } else {
        value
    }
}

fn density_header_label() -> String {
    match current_density_value().as_str() {
        "compact" => "compact".into(),
        "comfortable" => "comfy".into(),
        "large-text" => "large".into(),
        other => other.to_string(),
    }
}

pub(crate) fn current_saved_names() -> Vec<String> {
    saved_lock().clone()
}

pub(crate) fn density_changed(last: &mut PlayerDensity) -> bool {
    let next = current_density();
    if next == *last {
        false
    } else {
        *last = next;
        true
    }
}

pub(crate) fn current_snapshot() -> PlayerSnapshot {
    #[cfg(not(test))]
    ensure_poller();
    snapshot_lock().clone()
}

pub(crate) fn current_playlist() -> PlaylistSnapshot {
    #[cfg(not(test))]
    ensure_poller();
    playlist_lock().clone()
}

pub(crate) fn snapshot_changed(last: &mut PlayerSnapshot) -> bool {
    let next = current_snapshot();
    if next == *last {
        false
    } else {
        *last = next;
        true
    }
}

pub(crate) fn playlist_changed(last: &mut PlaylistSnapshot) -> bool {
    let next = current_playlist();
    if next == *last {
        false
    } else {
        *last = next;
        true
    }
}

pub(crate) fn ensure_poller() {
    POLLER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("herdplay-player".into())
            .spawn(|| loop {
                refresh_from_daemon();
                std::thread::sleep(Duration::from_millis(POLL_MS));
            });
    });
}

fn refresh_from_daemon() {
    *snapshot_lock() = fetch_snapshot();
    *playlist_lock() = fetch_playlist();
    apply_settings(fetch_settings_fields());
    *saved_lock() = fetch_saved_names();
}

fn fetch_snapshot() -> PlayerSnapshot {
    match fetch_daemon_state() {
        Some(state) => PlayerSnapshot::Online {
            title: nonempty(state.title),
            artist: nonempty(state.artist),
            url: nonempty(state.url)
                .or_else(|| nonempty(state.path)),
            playing: state.playing.unwrap_or(false),
            looping: state
                .queue
                .as_ref()
                .and_then(|queue| queue.looping)
                .unwrap_or(false),
            shuffle: state
                .queue
                .as_ref()
                .and_then(|queue| queue.shuffle)
                .unwrap_or(false),
            elapsed_sec: state.elapsed_sec.unwrap_or(0.0).max(0.0) as u64,
            duration_sec: state.duration_sec.unwrap_or(0.0).max(0.0) as u64,
            volume: quantize_volume(state.volume.unwrap_or(1.0)),
            seekable: state.seekable.unwrap_or(false),
        },
        None => PlayerSnapshot::Offline,
    }
}

fn fetch_playlist() -> PlaylistSnapshot {
    let Some((status, body)) = daemon_exchange("GET", "/playlist", "") else {
        return PlaylistSnapshot::default();
    };
    if status != 200 {
        return PlaylistSnapshot::default();
    }
    parse_playlist_body(&body)
}

fn parse_playlist_body(body: &str) -> PlaylistSnapshot {
    let Ok(parsed) = serde_json::from_str::<DaemonPlaylist>(body) else {
        return PlaylistSnapshot::default();
    };
    let items = parsed
        .items
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            let url = nonempty(item.url)
                .or_else(|| nonempty(item.path))
                .unwrap_or_default();
            let title = playlist_display_title(item.title, &url);
            let title = if title.is_empty() {
                "(untitled)".into()
            } else {
                title
            };
            if url.is_empty() {
                None
            } else {
                Some(PlaylistItem { title, url })
            }
        })
        .collect::<Vec<_>>();
    let index = parsed
        .index
        .filter(|idx| *idx >= 0)
        .map(|idx| idx as usize)
        .filter(|idx| *idx < items.len());
    PlaylistSnapshot { items, index }
}

fn fetch_settings_fields() -> Vec<SettingsField> {
    #[cfg(test)]
    {
        return TEST_SETTINGS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
    }
    #[cfg(not(test))]
    {
        let Some((status, body)) = daemon_exchange("GET", "/settings", "") else {
            return Vec::new();
        };
        if status != 200 {
            return Vec::new();
        }
        parse_settings_body(&body)
    }
}

fn parse_settings_body(body: &str) -> Vec<SettingsField> {
    serde_json::from_str::<SettingsBody>(body)
        .ok()
        .and_then(|parsed| parsed.fields)
        .unwrap_or_default()
}

fn apply_settings(fields: Vec<SettingsField>) {
    let found = fields
        .iter()
        .find(|field| field.key == "density" && field.field_type == "string")
        .cloned();
    *settings_lock() = fields;
    match found {
        Some(field) => {
            let value = field
                .value
                .as_str()
                .unwrap_or("comfortable")
                .to_string();
            *density_key_lock() = Some(field.key);
            *density_value_lock() = value.clone();
            *density_allowed_lock() = field.allowed.unwrap_or_default();
            *density_lock() = PlayerDensity::from_settings(&value);
        }
        None => {
            *density_key_lock() = None;
            *density_allowed_lock() = Vec::new();
        }
    }
}

/// Bind only `key === "density"` (string). Cycle the field's `allowed` list;
/// never invent values or POST unknown keys.
fn density_field(fields: &[SettingsField]) -> Option<&SettingsField> {
    fields
        .iter()
        .find(|field| field.key == "density" && field.field_type == "string")
}

fn fetch_saved_names() -> Vec<String> {
    #[cfg(test)]
    {
        return TEST_SAVED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
    }
    #[cfg(not(test))]
    {
        let Some((status, body)) = daemon_exchange("GET", "/playlist/saved", "") else {
            return Vec::new();
        };
        if status != 200 {
            return Vec::new();
        }
        parse_saved_names(&body)
    }
}

fn parse_saved_names(body: &str) -> Vec<String> {
    serde_json::from_str::<SavedPlaylistsBody>(body)
        .ok()
        .and_then(|parsed| parsed.names)
        .unwrap_or_default()
}

pub(crate) fn valid_playlist_name(name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

fn playlist_name_char_ok(ch: char, current: &str) -> bool {
    if ch == '.' && current.ends_with('.') {
        return false;
    }
    ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-'
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn fetch_daemon_state() -> Option<DaemonState> {
    daemon_http("GET", "/state", "")
        .and_then(|body| serde_json::from_str(&body).ok())
}

fn daemon_http(method: &str, path: &str, body: &str) -> Option<String> {
    daemon_exchange(method, path, body).map(|(_, body)| body)
}

fn daemon_exchange(method: &str, path: &str, body: &str) -> Option<(u16, String)> {
    if method != "GET" {
        if let Ok(log_path) = std::env::var("HERDPLAY_POST_LOG") {
            let line = format!("{method} {path} {body}\n");
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
                .and_then(|mut file| file.write_all(line.as_bytes()));
        }
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DAEMON_PORT);
    let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    let extra = if method == "GET" && body.is_empty() {
        format!(
            "{method} {path} HTTP/1.0\r\nHost: {DAEMON_HOST}:{DAEMON_PORT}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "{method} {path} HTTP/1.0\r\nHost: {DAEMON_HOST}:{DAEMON_PORT}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    stream.write_all(extra.as_bytes()).ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    parse_http_response(&buf)
}

fn parse_http_response(buf: &str) -> Option<(u16, String)> {
    let (head, body) = buf
        .split_once("\r\n\r\n")
        .or_else(|| buf.split_once("\n\n"))?;
    let status = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((status, body.to_string()))
}

fn post_daemon(path: &'static str, body: String) {
    #[cfg(test)]
    {
        TEST_POSTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((path.to_string(), body.clone()));
        return;
    }
    #[cfg(not(test))]
    {
        let _ = std::thread::Builder::new()
            .name("herdplay-post".into())
            .spawn(move || {
                let _ = daemon_http("POST", path, &body);
                refresh_from_daemon();
            });
    }
}

/// Synchronous POST for save/load (needs 409/404/body). Transport stays async.
fn post_daemon_response(path: &'static str, body: String) -> (u16, String) {
    #[cfg(test)]
    {
        TEST_POSTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((path.to_string(), body));
        let mut replies = TEST_REPLIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if replies.is_empty() {
            (200, "{}".into())
        } else {
            replies.remove(0)
        }
    }
    #[cfg(not(test))]
    {
        let result = daemon_exchange("POST", path, &body).unwrap_or((0, String::new()));
        refresh_from_daemon();
        result
    }
}

#[cfg(test)]
pub(crate) fn take_test_posts() -> Vec<(String, String)> {
    std::mem::take(
        &mut *TEST_POSTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

pub(crate) fn play_pause() {
    if current_snapshot().playing() {
        post_daemon("/pause", String::new());
    } else {
        post_daemon("/play", String::new());
    }
}

pub(crate) fn post_prev() {
    post_daemon("/prev", String::new());
}

pub(crate) fn post_next() {
    post_daemon("/next", String::new());
}

pub(crate) fn post_loop() {
    post_daemon("/loop", String::new());
}

pub(crate) fn post_shuffle() {
    post_daemon("/shuffle", String::new());
}

fn quantize_volume(level: f64) -> f64 {
    ((level.clamp(0.0, 1.0) * 100.0).round() / 100.0).clamp(0.0, 1.0)
}

fn post_volume(level: f64) {
    let level = quantize_volume(level);
    if let PlayerSnapshot::Online { volume, .. } = &mut *snapshot_lock() {
        *volume = level;
    }
    let body = serde_json::json!({ "level": level }).to_string();
    post_daemon("/volume", body);
}

pub(crate) fn nudge_volume(delta: f64) {
    post_volume(current_snapshot().volume() + delta);
}

pub(crate) fn post_volume_at_bar(bar: Rect, col: u16) {
    if bar.width == 0 {
        return;
    }
    let t = (col.saturating_sub(bar.x) as f64 + 0.5) / bar.width as f64;
    post_volume(t);
}

pub(crate) fn post_seek_at_bar(bar: Rect, col: u16) {
    let snapshot = current_snapshot();
    if !snapshot.seekable() || bar.width == 0 {
        return;
    }
    let t = ((col.saturating_sub(bar.x) as f64 + 0.5) / bar.width as f64).clamp(0.0, 1.0);
    let seconds = t * snapshot.duration_sec() as f64;
    if let PlayerSnapshot::Online { elapsed_sec, .. } = &mut *snapshot_lock() {
        *elapsed_sec = seconds.max(0.0) as u64;
    }
    let body = serde_json::json!({ "seconds": seconds }).to_string();
    post_daemon("/seek", body);
}

pub(crate) fn post_playlist_load(index: usize) {
    let Some(item) = current_playlist().items.get(index).cloned() else {
        return;
    };
    let body = serde_json::json!({ "url": item.url }).to_string();
    post_daemon("/load", body);
}

pub(crate) fn post_playlist_remove(index: usize) {
    let body = serde_json::json!({ "index": index }).to_string();
    post_daemon("/playlist/remove", body);
    let mut playlist = playlist_lock();
    if index < playlist.items.len() {
        playlist.items.remove(index);
        if let Some(current) = playlist.index {
            if current == index {
                playlist.index = None;
            } else if current > index {
                playlist.index = Some(current - 1);
            }
        }
    }
}

pub(crate) fn post_playlist_clear() {
    post_daemon("/playlist/clear", "{}".to_string());
    *playlist_lock() = PlaylistSnapshot::default();
}

pub(crate) fn enter_save_name(app: &mut AppState) {
    app.player_queue_mode = PlayerQueueMode::SaveName;
    app.player_save_name.clear();
    app.player_expanded = true;
    app.player_input_focused = true;
    app.mode = crate::app::state::Mode::PlayerInput;
}

pub(crate) fn enter_load_picker(app: &mut AppState) {
    *saved_lock() = fetch_saved_names();
    app.player_queue_mode = PlayerQueueMode::LoadPicker;
    app.player_playlist_scroll = 0;
    unfocus_player_input(app);
}

pub(crate) fn cancel_player_queue_mode(app: &mut AppState) {
    let was_save = matches!(
        app.player_queue_mode,
        PlayerQueueMode::SaveName | PlayerQueueMode::SaveOverwrite { .. }
    );
    app.player_queue_mode = PlayerQueueMode::Queue;
    if was_save {
        app.player_save_name.clear();
        unfocus_player_input(app);
    }
}

pub(crate) fn handle_player_queue_esc(app: &mut AppState) -> bool {
    if matches!(app.player_queue_mode, PlayerQueueMode::Queue) {
        return false;
    }
    cancel_player_queue_mode(app);
    true
}

pub(crate) fn submit_player_save(app: &mut AppState, overwrite: bool) {
    let name = if overwrite {
        match &app.player_queue_mode {
            PlayerQueueMode::SaveOverwrite { name } => name.clone(),
            _ => return,
        }
    } else {
        let name = app.player_save_name.text();
        let name = name.trim().to_string();
        if !valid_playlist_name(&name) {
            return;
        }
        name
    };
    let body = if overwrite {
        serde_json::json!({ "name": name, "overwrite": true }).to_string()
    } else {
        serde_json::json!({ "name": name }).to_string()
    };
    let (status, _) = post_daemon_response("/playlist/save", body);
    if status == 409 {
        app.player_queue_mode = PlayerQueueMode::SaveOverwrite { name };
        unfocus_player_input(app);
        return;
    }
    if status == 200 {
        {
            let mut saved = saved_lock();
            if !saved.iter().any(|existing| existing == &name) {
                saved.push(name);
                saved.sort();
            }
        }
        cancel_player_queue_mode(app);
    }
}

pub(crate) fn submit_load_named(app: &mut AppState, index: usize) {
    let Some(name) = current_saved_names().get(index).cloned() else {
        return;
    };
    let body = serde_json::json!({ "name": name }).to_string();
    let (status, resp) = post_daemon_response("/playlist/load", body);
    if status != 200 {
        return;
    }
    *playlist_lock() = parse_playlist_body(&resp);
    app.player_queue_mode = PlayerQueueMode::Queue;
    app.player_playlist_scroll = 0;
}

pub(crate) fn cycle_player_density() {
    let allowed = density_allowed_lock().clone();
    if allowed.is_empty() {
        return;
    }
    let current = current_density_value();
    let idx = allowed
        .iter()
        .position(|value| value == &current)
        .unwrap_or(0);
    let next = allowed[(idx + 1) % allowed.len()].clone();
    let previous = current;
    *density_value_lock() = next.clone();
    *density_lock() = PlayerDensity::from_settings(&next);
    let Some(key) = density_key_lock().clone() else {
        return;
    };
    let body = serde_json::json!({
        "key": key,
        "value": next,
    })
    .to_string();
    let (status, resp) = post_daemon_response("/settings", body);
    if status == 200 {
        let fields = parse_settings_body(&resp);
        if density_field(&fields).is_some() {
            apply_settings(fields);
        }
        return;
    }
    *density_value_lock() = previous.clone();
    *density_lock() = PlayerDensity::from_settings(&previous);
}

pub(crate) fn save_name_char_allowed(app: &AppState, ch: char) -> bool {
    playlist_name_char_ok(ch, &app.player_save_name.text())
}

/// Scroll-wheel on the volume row. Returns true when the event was consumed
/// so the workspace list does not steal it.
pub(crate) fn player_scroll_volume(app: &AppState, col: u16, row: u16, up: bool) -> bool {
    if !app.player_expanded {
        return false;
    }
    let hits = player_hit_areas(app);
    if !contains(hits.volume, col, row) {
        return false;
    }
    nudge_volume(if up { VOLUME_STEP } else { -VOLUME_STEP });
    true
}

/// Scroll-wheel on the playlist list. Hooked above the spaces-list arm in
/// `mouse.rs` the same way volume is.
pub(crate) fn player_scroll_playlist(app: &mut AppState, col: u16, row: u16, up: bool) -> bool {
    if !app.player_expanded {
        return false;
    }
    let hits = player_hit_areas(app);
    if !contains(hits.playlist, col, row) {
        return false;
    }
    let visible = hits.playlist.height as usize;
    let len = if matches!(app.player_queue_mode, PlayerQueueMode::LoadPicker) {
        current_saved_names().len()
    } else {
        current_playlist().items.len()
    };
    let max_scroll = len.saturating_sub(visible);
    if up {
        app.player_playlist_scroll = app.player_playlist_scroll.saturating_sub(1);
    } else {
        app.player_playlist_scroll = (app.player_playlist_scroll + 1).min(max_scroll);
    }
    true
}

#[cfg(test)]
pub(crate) fn set_test_snapshot(snapshot: PlayerSnapshot) {
    *snapshot_lock() = snapshot;
}

#[cfg(test)]
pub(crate) fn set_test_playlist(playlist: PlaylistSnapshot) {
    *playlist_lock() = playlist;
}

#[cfg(test)]
fn set_test_saved_names(names: Vec<String>) {
    TEST_SAVED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone_from(&names);
    *saved_lock() = names;
}

#[cfg(test)]
fn push_test_reply(status: u16, body: impl Into<String>) {
    TEST_REPLIES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((status, body.into()));
}

#[cfg(test)]
fn set_test_settings(fields: Vec<SettingsField>) {
    TEST_SETTINGS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone_from(&fields);
    apply_settings(fields);
}

pub(crate) fn submit_player_add(app: &mut AppState) {
    let Some(url) = take_player_url(app) else {
        return;
    };
    let body = serde_json::json!({ "url": url }).to_string();
    post_daemon("/playlist/add", body);
    let title = playlist_title_from_url(&url);
    playlist_lock().items.push(PlaylistItem { title, url });
    app.player_link.clear();
}

fn take_player_url(app: &AppState) -> Option<String> {
    let url = app.player_link.text();
    let url = url.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

fn playlist_title_from_url(url: &str) -> String {
    url.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(url)
        .to_string()
}

fn looks_like_locator(text: &str) -> bool {
    let text = text.trim();
    text.starts_with('/')
        || text.starts_with('\\')
        || text.contains("://")
        || text
            .as_bytes()
            .get(1)
            .copied()
            .is_some_and(|b| b == b':' )
            && text
                .as_bytes()
                .get(2)
                .copied()
                .is_some_and(|b| b == b'\\' || b == b'/')
}

/// Shared by playlist rows and the now-playing footer: real title wins,
/// otherwise basename of url/path, never the raw locator.
fn playlist_display_title(title: Option<String>, url: &str) -> String {
    let raw = nonempty(title).unwrap_or_else(|| url.trim().to_string());
    if raw.is_empty() {
        String::new()
    } else if looks_like_locator(&raw) {
        playlist_title_from_url(&raw)
    } else {
        raw
    }
}

fn snapshot_locator(snapshot: &PlayerSnapshot) -> String {
    match snapshot {
        PlayerSnapshot::Online { url: Some(url), .. } => url.clone(),
        _ => {
            let playlist = current_playlist();
            playlist
                .index
                .and_then(|idx| playlist.items.get(idx))
                .map(|item| item.url.clone())
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
pub(crate) fn player_rows(app: &AppState) -> u16 {
    player_rows_for_sidebar(app, app.view.sidebar_rect.height)
}

pub(crate) fn player_rows_for_sidebar(app: &AppState, sidebar_h: u16) -> u16 {
    if app.player_expanded {
        full_player_rows(sidebar_h)
    } else {
        PLAYER_COLLAPSED_ROWS
    }
}

fn full_player_rows(sidebar_h: u16) -> u16 {
    if sidebar_h == 0 {
        return PLAYER_FULL_MIN_ROWS;
    }
    let keep = FULL_KEEP_ABOVE.min(sidebar_h.saturating_sub(PLAYER_FULL_MIN_ROWS));
    sidebar_h.saturating_sub(keep).max(PLAYER_COLLAPSED_ROWS)
}

pub(crate) fn focus_player_input(app: &mut AppState, col: u16) {
    app.player_expanded = true;
    app.player_input_focused = true;
    app.mode = crate::app::state::Mode::PlayerInput;
    let hits = player_hit_areas(app);
    let col_in_field = col.saturating_sub(hits.input.x) as usize;
    app.player_link.set_width(hits.input.width.max(1) as usize);
    app.player_link.click(0, col_in_field);
}

pub(crate) fn unfocus_player_input(app: &mut AppState) {
    app.player_input_focused = false;
    if app.mode == crate::app::state::Mode::PlayerInput {
        app.mode = crate::app::state::Mode::Terminal;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Toggle,
    Prev,
    PlayPause,
    Next,
    Loop,
    Shuffle,
    FocusInput,
    Add,
    VolumeDown,
    VolumeUp,
    VolumeSet,
    VolumeIdle,
    Seek,
    ScrubIdle,
    PlaylistLoad(usize),
    PlaylistRemove(usize),
    PlaylistClear,
    Save,
    LoadSaved,
    CancelSave,
    ConfirmSave,
    OverwriteNo,
    OverwriteYes,
    PickSaved(usize),
    CycleDensity,
    Background,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PlayerHitAreas {
    pub player: Rect,
    pub title: Rect,
    pub prev: Rect,
    pub play: Rect,
    pub next: Rect,
    pub looping: Rect,
    pub shuffle: Rect,
    pub chevron: Rect,
    pub input: Rect,
    pub add: Rect,
    pub padding: Rect,
    pub volume: Rect,
    pub vol_down: Rect,
    pub vol_up: Rect,
    pub vol_bar: Rect,
    pub playlist: Rect,
    pub queue: Rect,
    pub save: Rect,
    pub load: Rect,
    pub clear: Rect,
    pub cancel: Rect,
    pub density: Rect,
    pub overwrite_no: Rect,
    pub overwrite_yes: Rect,
    pub scrub: Rect,
}

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    rect.width > 0
        && rect.height > 0
        && col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

pub(crate) fn player_action_at(app: &AppState, col: u16, row: u16) -> Option<PlayerAction> {
    let hits = player_hit_areas(app);
    if hits.player == Rect::default() || !contains(hits.player, col, row) {
        return None;
    }
    // Controls win over the title-row toggle so a real click on ▶ never
    // expands the box instead of talking to the daemon.
    if contains(hits.prev, col, row) {
        return Some(PlayerAction::Prev);
    }
    if contains(hits.play, col, row) {
        return Some(PlayerAction::PlayPause);
    }
    if contains(hits.next, col, row) {
        return Some(PlayerAction::Next);
    }
    if contains(hits.looping, col, row) {
        return Some(PlayerAction::Loop);
    }
    if contains(hits.shuffle, col, row) {
        return Some(PlayerAction::Shuffle);
    }
    if contains(hits.add, col, row) {
        return Some(match app.player_queue_mode {
            PlayerQueueMode::SaveName => PlayerAction::ConfirmSave,
            _ => PlayerAction::Add,
        });
    }
    if contains(hits.cancel, col, row) {
        return Some(PlayerAction::CancelSave);
    }
    if contains(hits.input, col, row) {
        return Some(PlayerAction::FocusInput);
    }
    if contains(hits.vol_down, col, row) {
        return Some(PlayerAction::VolumeDown);
    }
    if contains(hits.vol_up, col, row) {
        return Some(PlayerAction::VolumeUp);
    }
    if contains(hits.vol_bar, col, row) {
        return Some(PlayerAction::VolumeSet);
    }
    if contains(hits.volume, col, row) {
        return Some(PlayerAction::VolumeIdle);
    }
    if contains(hits.overwrite_yes, col, row) {
        return Some(PlayerAction::OverwriteYes);
    }
    if contains(hits.overwrite_no, col, row) {
        return Some(PlayerAction::OverwriteNo);
    }
    if contains(hits.save, col, row) {
        return Some(PlayerAction::Save);
    }
    if contains(hits.load, col, row) {
        return Some(PlayerAction::LoadSaved);
    }
    if contains(hits.clear, col, row) {
        return Some(PlayerAction::PlaylistClear);
    }
    if contains(hits.queue, col, row)
        && !matches!(app.player_queue_mode, PlayerQueueMode::Queue)
    {
        return Some(PlayerAction::CancelSave);
    }
    if contains(hits.playlist, col, row) {
        return match app.player_queue_mode {
            PlayerQueueMode::LoadPicker => {
                saved_pick_at(hits.playlist, col, row, app.player_playlist_scroll)
            }
            _ => playlist_action_at(hits.playlist, col, row, app.player_playlist_scroll),
        };
    }
    if contains(hits.scrub, col, row) {
        if current_snapshot().seekable() {
            return Some(PlayerAction::Seek);
        }
        return Some(PlayerAction::ScrubIdle);
    }
    if contains(hits.density, col, row) {
        return Some(PlayerAction::CycleDensity);
    }
    if contains(hits.title, col, row) || contains(hits.chevron, col, row) {
        return Some(PlayerAction::Toggle);
    }
    Some(PlayerAction::Background)
}

fn playlist_action_at(list: Rect, col: u16, row: u16, scroll: usize) -> Option<PlayerAction> {
    let playlist = current_playlist();
    if playlist.items.is_empty() {
        return Some(PlayerAction::Background);
    }
    let row_i = (row.saturating_sub(list.y) as usize).saturating_add(scroll);
    if row_i >= playlist.items.len() {
        return Some(PlayerAction::Background);
    }
    let remove_x = list.x.saturating_add(list.width.saturating_sub(2));
    if col >= remove_x {
        return Some(PlayerAction::PlaylistRemove(row_i));
    }
    Some(PlayerAction::PlaylistLoad(row_i))
}

fn saved_pick_at(list: Rect, col: u16, row: u16, scroll: usize) -> Option<PlayerAction> {
    let _ = col;
    let names = current_saved_names();
    if names.is_empty() {
        return Some(PlayerAction::Background);
    }
    let row_i = (row.saturating_sub(list.y) as usize).saturating_add(scroll);
    if row_i >= names.len() {
        return Some(PlayerAction::Background);
    }
    Some(PlayerAction::PickSaved(row_i))
}

pub(crate) fn player_hit_areas(app: &AppState) -> PlayerHitAreas {
    layout_hits_with_mode(
        app.view.player_rect,
        app.player_expanded,
        &current_snapshot(),
        &app.player_queue_mode,
    )
}

/// Expand/collapse hit target: the title slot plus the chevron, not the
/// transport icons. Those have their own actions.
#[cfg(test)]
pub(crate) fn player_toggle_rect(player_rect: Rect) -> Rect {
    let hits = layout_hits(player_rect, false, &PlayerSnapshot::Offline);
    if hits.title.width == 0 {
        return Rect::default();
    }
    Rect::new(
        hits.title.x,
        hits.title.y,
        hits.chevron
            .x
            .saturating_add(hits.chevron.width)
            .saturating_sub(hits.title.x)
            .max(hits.title.width),
        1,
    )
}

pub(crate) fn render_player(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width < 2 || area.height < PLAYER_COLLAPSED_ROWS {
        return;
    }

    let p = &app.palette;
    let border = Style::default().fg(if app.player_input_focused {
        p.accent
    } else {
        p.overlay0
    });
    draw_rounded_box(frame, area, border);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let snapshot = current_snapshot();
    if app.player_expanded {
        if player_full_too_small(area) {
            render_too_small_face(frame, inner, p);
        } else {
            render_full_face(frame, area, inner, app, p, &snapshot);
        }
    } else {
        render_bar_face(frame, inner, p, &snapshot);
    }
}

fn player_full_too_small(area: Rect) -> bool {
    area.width < PLAYER_FULL_MIN_WIDTH || area.height < PLAYER_FULL_MIN_ROWS
}

fn render_bar_face(frame: &mut Frame, inner: Rect, p: &Palette, snapshot: &PlayerSnapshot) {
    let bar_width = inner.width.saturating_sub(2);
    let mut bar = player_line(p, snapshot, bar_width);
    bar.spans.push(Span::raw(" "));
    bar.spans
        .push(Span::styled("▸", Style::default().fg(p.overlay0)));
    frame.render_widget(
        Paragraph::new(bar),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
}

fn render_too_small_face(frame: &mut Frame, inner: Rect, p: &Palette) {
    render_full_header(frame, inner, p);
    let hint_y = inner.y.saturating_add(1);
    if hint_y < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Span::styled(TOO_SMALL_HINT, Style::default().fg(p.overlay0)))
                .wrap(Wrap { trim: true }),
            Rect::new(
                inner.x,
                hint_y,
                inner.width,
                inner.height.saturating_sub(1),
            ),
        );
    }
}

fn header_chrome(inner_width: u16) -> (bool, &'static str) {
    let density = density_header_label();
    let density_w = UnicodeWidthStr::width(density.as_str()) as u16;
    let header_w = UnicodeWidthStr::width(HEADER_LABEL) as u16;
    let full = "collapse ▾";
    let short = "▾";
    let full_w = UnicodeWidthStr::width(full) as u16;
    let short_w = UnicodeWidthStr::width(short) as u16;
    let dens_cost = 1 + density_w;
    if header_w.saturating_add(dens_cost).saturating_add(1).saturating_add(full_w) <= inner_width {
        (true, full)
    } else if header_w.saturating_add(dens_cost).saturating_add(1).saturating_add(short_w) <= inner_width {
        (true, short)
    } else {
        (false, full)
    }
}

fn render_full_header(frame: &mut Frame, inner: Rect, p: &Palette) {
    let (show_density, collapse) = header_chrome(inner.width);
    let collapse_w = UnicodeWidthStr::width(collapse) as u16;
    let density = density_header_label();
    let density_w = UnicodeWidthStr::width(density.as_str()) as u16;
    let header_w = UnicodeWidthStr::width(HEADER_LABEL) as u16;
    let used = header_w
        .saturating_add(if show_density { 1 + density_w } else { 0 })
        .saturating_add(1)
        .saturating_add(collapse_w);
    let pad = inner.width.saturating_sub(used) as usize;
    let mut spans = vec![Span::styled(
        HEADER_LABEL,
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
    )];
    if show_density {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(density, Style::default().fg(p.overlay1)));
    }
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(collapse, Style::default().fg(p.overlay0)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
}

fn render_full_face(
    frame: &mut Frame,
    area: Rect,
    inner: Rect,
    app: &AppState,
    p: &Palette,
    snapshot: &PlayerSnapshot,
) {
    let hits = layout_hits_with_mode(area, true, snapshot, &app.player_queue_mode);
    render_full_header(frame, inner, p);

    if hits.input.width > 0 {
        render_link_row(frame, hits, app, p);
    }

    if hits.save.width > 0 || hits.load.width > 0 || hits.clear.width > 0 || hits.overwrite_yes.width > 0 {
        render_queue_chrome(frame, hits, app, p);
    }

    if hits.playlist.height > 0 && hits.playlist.width > 0 {
        render_playlist(frame, hits.playlist, app, p);
    }

    if hits.padding.height > 0 && hits.padding.width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "hidden tab mirrors here",
                Style::default().fg(p.overlay0),
            )))
            .alignment(ratatui::layout::Alignment::Center),
            hits.padding,
        );
    }

    let (title, artist) = match snapshot {
        PlayerSnapshot::Online { title, artist, .. } => {
            let locator = snapshot_locator(snapshot);
            let title = playlist_display_title(title.clone(), &locator);
            (
                if title.is_empty() {
                    "no track".into()
                } else {
                    title
                },
                artist.clone().unwrap_or_default(),
            )
        }
        PlayerSnapshot::Offline => ("player offline".into(), String::new()),
    };
    let np_y = hits.play.y.saturating_sub(2);
    if np_y >= inner.y && np_y + 1 < inner.y + inner.height {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    COVER,
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    truncate_width(&title, inner.width.saturating_sub(4) as usize),
                    Style::default().fg(p.text),
                ),
            ])),
            Rect::new(inner.x, np_y, inner.width, 1),
        );
        if !artist.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    truncate_width(&artist, inner.width as usize),
                    Style::default().fg(p.overlay0),
                )),
                Rect::new(inner.x + 2, np_y + 1, inner.width.saturating_sub(2), 1),
            );
        }
    }

    let playing = snapshot.playing();
    let play_glyph = if playing { PAUSE } else { PLAY };
    let transport = Line::from(vec![
        transport_span(LOOP, snapshot.looping(), p),
        Span::raw("  "),
        Span::styled(PREV, Style::default().fg(p.overlay0)),
        Span::raw("  "),
        Span::styled(
            play_glyph,
            if playing {
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.text)
            },
        ),
        Span::raw("  "),
        Span::styled(NEXT, Style::default().fg(p.overlay0)),
        Span::raw("  "),
        transport_span(SHUFFLE, snapshot.shuffle(), p),
    ]);
    if hits.play.width > 0 {
        frame.render_widget(
            Paragraph::new(transport).alignment(ratatui::layout::Alignment::Center),
            Rect::new(inner.x, hits.play.y, inner.width, 1),
        );
    }

    let (elapsed, duration) = match snapshot {
        PlayerSnapshot::Online {
            elapsed_sec,
            duration_sec,
            ..
        } => (*elapsed_sec, *duration_sec),
        PlayerSnapshot::Offline => (0, 0),
    };
    let scrub_y = hits.play.y.saturating_add(1);
    if scrub_y < inner.y + inner.height {
        render_scrub_row(
            frame,
            inner,
            hits,
            p,
            elapsed,
            duration,
            snapshot.seekable(),
        );
    }
    let foot_y = scrub_y.saturating_add(1);
    if foot_y < inner.y + inner.height {
        if hits.volume.width > 0 {
            render_volume_row(frame, inner, hits, p, snapshot.volume(), playing);
        } else {
            let status = if playing {
                "playing · via daemon"
            } else {
                "paused · via daemon"
            };
            frame.render_widget(
                Paragraph::new(Span::styled(status, Style::default().fg(p.overlay0))),
                Rect::new(inner.x, foot_y, inner.width, 1),
            );
        }
    }

    if app.player_input_focused && hits.input.width > 0 {
        let (_, col) = app.player_link.cursor_row();
        let cursor_x = hits.input.x + (col as u16).min(hits.input.width.saturating_sub(1));
        frame.set_cursor_position((cursor_x, hits.input.y));
    }
}

fn render_volume_row(
    frame: &mut Frame,
    inner: Rect,
    hits: PlayerHitAreas,
    p: &Palette,
    volume: f64,
    playing: bool,
) {
    let y = hits.volume.y;
    let dim = Style::default().fg(p.overlay0);
    let fill_style = Style::default().fg(p.accent);
    frame.render_widget(
        Paragraph::new(Span::styled(VOL_LABEL, dim)),
        Rect::new(inner.x, y, UnicodeWidthStr::width(VOL_LABEL) as u16, 1),
    );
    if hits.vol_down.width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("-", Style::default().fg(p.text))),
            Rect::new(hits.vol_down.x, y, 1, 1),
        );
    }
    if hits.vol_bar.width > 0 {
        let fill = ((volume.clamp(0.0, 1.0) * hits.vol_bar.width as f64).round() as u16)
            .min(hits.vol_bar.width);
        let bar = format!(
            "{}{}",
            "━".repeat(fill as usize),
            "─".repeat(hits.vol_bar.width.saturating_sub(fill) as usize)
        );
        frame.render_widget(
            Paragraph::new(Span::styled(bar, fill_style)),
            hits.vol_bar,
        );
    }
    if hits.vol_up.width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled("+", Style::default().fg(p.text))),
            Rect::new(hits.vol_up.x, y, 1, 1),
        );
    }
    let pct = format!("{:>3}%", (volume.clamp(0.0, 1.0) * 100.0).round() as u32);
    let pct_x = hits.vol_up.x.saturating_add(2);
    if pct_x + 4 <= inner.x + inner.width {
        frame.render_widget(
            Paragraph::new(Span::styled(pct, dim)),
            Rect::new(pct_x, y, 4, 1),
        );
    }
    let status_x = hits.volume.x.saturating_add(hits.volume.width).saturating_add(1);
    let status_w = (inner.x + inner.width).saturating_sub(status_x);
    if status_w >= 6 {
        let status = if playing {
            "playing · via daemon"
        } else {
            "paused · via daemon"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate_width(status, status_w as usize),
                dim,
            )),
            Rect::new(status_x, y, status_w, 1),
        );
    }
}

fn transport_span(glyph: &'static str, active: bool, p: &Palette) -> Span<'static> {
    if active {
        Span::styled(glyph, Style::default().fg(p.accent).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(glyph, Style::default().fg(p.overlay0))
    }
}

fn fmt_time(sec: u64) -> String {
    format!("{}:{:02}", sec / 60, sec % 60)
}

fn pad_time(sec: u64, width: u16) -> String {
    let text = fmt_time(sec);
    let pad = (width as usize).saturating_sub(UnicodeWidthStr::width(text.as_str()));
    format!("{text}{}", " ".repeat(pad))
}

fn render_queue_chrome(frame: &mut Frame, hits: PlayerHitAreas, app: &AppState, p: &Palette) {
    let y = chrome_row_y(&hits);
    let row_x = inner_row_x(&hits);
    let row_w = inner_row_w(&hits, row_x);
    if matches!(app.player_queue_mode, PlayerQueueMode::SaveOverwrite { .. }) {
        render_overwrite_row(frame, hits, app, p, row_x, row_w, y);
        return;
    }
    if hits.queue.width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(QUEUE_LABEL, Style::default().fg(p.overlay0))),
            hits.queue,
        );
    }
    if hits.save.width > 0 {
        let style = if matches!(app.player_queue_mode, PlayerQueueMode::SaveName) {
            Style::default().fg(p.accent)
        } else {
            Style::default().fg(p.overlay0)
        };
        frame.render_widget(Paragraph::new(Span::styled(SAVE_LABEL, style)), hits.save);
    }
    if hits.load.width > 0 {
        let style = if matches!(app.player_queue_mode, PlayerQueueMode::LoadPicker) {
            Style::default().fg(p.accent)
        } else {
            Style::default().fg(p.overlay0)
        };
        frame.render_widget(Paragraph::new(Span::styled(LOAD_LABEL, style)), hits.load);
    }
    if hits.clear.width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(CLEAR_LABEL, Style::default().fg(p.overlay0))),
            hits.clear,
        );
    }
}

fn chrome_row_y(hits: &PlayerHitAreas) -> u16 {
    if hits.save.height > 0 {
        hits.save.y
    } else if hits.load.height > 0 {
        hits.load.y
    } else if hits.clear.height > 0 {
        hits.clear.y
    } else {
        hits.overwrite_yes.y
    }
}

fn inner_row_x(hits: &PlayerHitAreas) -> u16 {
    if hits.playlist.width > 0 {
        hits.playlist.x
    } else if hits.queue.width > 0 {
        hits.queue.x
    } else {
        hits.save.x.saturating_sub(UnicodeWidthStr::width(QUEUE_LABEL) as u16 + 1)
    }
}

fn inner_row_w(hits: &PlayerHitAreas, row_x: u16) -> u16 {
    if hits.playlist.width > 0 {
        hits.playlist.width
    } else {
        let right = hits
            .clear
            .x
            .saturating_add(hits.clear.width)
            .max(hits.load.x.saturating_add(hits.load.width))
            .max(hits.save.x.saturating_add(hits.save.width))
            .max(hits.overwrite_yes.x.saturating_add(hits.overwrite_yes.width));
        right.saturating_sub(row_x)
    }
}

fn render_overwrite_row(
    frame: &mut Frame,
    hits: PlayerHitAreas,
    app: &AppState,
    p: &Palette,
    row_x: u16,
    row_w: u16,
    y: u16,
) {
    let name = match &app.player_queue_mode {
        PlayerQueueMode::SaveOverwrite { name } => name.as_str(),
        _ => return,
    };
    let prompt = format!("replace '{name}'?");
    let trailing = hits.overwrite_no.width.saturating_add(hits.overwrite_yes.width).saturating_add(1);
    let budget = row_w.saturating_sub(trailing.saturating_add(1)) as usize;
    let prompt = truncate_width(&prompt, budget);
    let pad = budget.saturating_sub(UnicodeWidthStr::width(prompt.as_str()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prompt, Style::default().fg(p.peach)),
            Span::raw(" ".repeat(pad)),
            Span::raw(" "),
            Span::styled(OVERWRITE_NO, Style::default().fg(p.overlay0)),
            Span::raw(" "),
            Span::styled(OVERWRITE_YES, Style::default().fg(p.accent)),
        ])),
        Rect::new(row_x, y, row_w, 1),
    );
}

fn render_playlist(frame: &mut Frame, area: Rect, app: &AppState, p: &Palette) {
    if matches!(app.player_queue_mode, PlayerQueueMode::LoadPicker) {
        render_saved_picker(frame, area, app, p);
        return;
    }
    let playlist = current_playlist();
    let scroll = app
        .player_playlist_scroll
        .min(playlist.items.len().saturating_sub(area.height as usize));
    if playlist.items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(PLAYLIST_EMPTY, Style::default().fg(p.overlay0))),
            Rect::new(area.x, area.y, area.width, 1),
        );
        return;
    }
    let remove_w = UnicodeWidthStr::width(REMOVE_GLYPH) as u16;
    for vis in 0..area.height {
        let idx = scroll + vis as usize;
        let y = area.y + vis;
        if idx >= playlist.items.len() {
            break;
        }
        let current = playlist.index == Some(idx);
        let title_style = if current {
            Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.text)
        };
        let title_budget = area.width.saturating_sub(remove_w.saturating_add(1)) as usize;
        let title = truncate_width(&playlist.items[idx].title, title_budget);
        let pad = title_budget.saturating_sub(UnicodeWidthStr::width(title.as_str()));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(title, title_style),
                Span::raw(" ".repeat(pad)),
                Span::raw(" "),
                Span::styled(REMOVE_GLYPH, Style::default().fg(p.overlay0)),
            ])),
            Rect::new(area.x, y, area.width, 1),
        );
    }
}

fn render_saved_picker(frame: &mut Frame, area: Rect, app: &AppState, p: &Palette) {
    let names = current_saved_names();
    let scroll = app
        .player_playlist_scroll
        .min(names.len().saturating_sub(area.height as usize));
    if names.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(SAVED_EMPTY, Style::default().fg(p.overlay0))),
            Rect::new(area.x, area.y, area.width, 1),
        );
        return;
    }
    for vis in 0..area.height {
        let idx = scroll + vis as usize;
        if idx >= names.len() {
            break;
        }
        let label = truncate_width(&names[idx], area.width as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(label, Style::default().fg(p.text))),
            Rect::new(area.x, area.y + vis, area.width, 1),
        );
    }
}

fn render_scrub_row(
    frame: &mut Frame,
    inner: Rect,
    hits: PlayerHitAreas,
    p: &Palette,
    elapsed: u64,
    duration: u64,
    seekable: bool,
) {
    let y = if hits.scrub.height > 0 {
        hits.scrub.y
    } else {
        hits.play.y.saturating_add(1)
    };
    let dim = Style::default().fg(p.overlay0);
    let elapsed_text = pad_time(elapsed, SCRUB_TIME_W);
    frame.render_widget(
        Paragraph::new(Span::styled(elapsed_text, dim)),
        Rect::new(inner.x, y, SCRUB_TIME_W.min(inner.width), 1),
    );
    if hits.scrub.width > 0 {
        let fill = if duration == 0 {
            0
        } else {
            ((elapsed as f64 / duration as f64) * hits.scrub.width as f64).round() as u16
        }
        .min(hits.scrub.width);
        let bar = format!(
            "{}{}",
            "━".repeat(fill as usize),
            "─".repeat(hits.scrub.width.saturating_sub(fill) as usize)
        );
        let bar_style = if seekable {
            Style::default().fg(p.accent)
        } else {
            dim
        };
        frame.render_widget(Paragraph::new(Span::styled(bar, bar_style)), hits.scrub);
    }
    let dur_x = inner.x + inner.width.saturating_sub(SCRUB_TIME_W);
    if dur_x + 1 < inner.x + inner.width && (hits.scrub.width == 0 || dur_x >= hits.scrub.x + hits.scrub.width) {
        let duration_text = format!("{:>width$}", fmt_time(duration), width = SCRUB_TIME_W as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(truncate_width(&duration_text, SCRUB_TIME_W as usize), dim)),
            Rect::new(dur_x, y, SCRUB_TIME_W, 1),
        );
    }
}

fn render_link_row(frame: &mut Frame, hits: PlayerHitAreas, app: &AppState, p: &Palette) {
    if hits.input.width == 0 {
        return;
    }
    let saving = matches!(app.player_queue_mode, PlayerQueueMode::SaveName);
    let typed = if saving {
        app.player_save_name.text()
    } else {
        app.player_link.text()
    };
    let empty = typed.is_empty();
    let show = if empty {
        if saving {
            SAVE_NAME_PLACEHOLDER.to_string()
        } else {
            LINK_PLACEHOLDER.to_string()
        }
    } else {
        typed
    };
    let placeholder = truncate_width(&show, hits.input.width as usize);
    let pad = (hits.input.width as usize).saturating_sub(UnicodeWidthStr::width(placeholder.as_str()));
    let invalid = saving && !empty && !valid_playlist_name(&app.player_save_name.text());
    let field_fg = if invalid {
        p.peach
    } else if empty {
        p.overlay0
    } else {
        p.text
    };
    let field_bg = if app.player_input_focused {
        p.surface1
    } else {
        p.surface0
    };
    let mut spans = vec![Span::styled(
        placeholder,
        Style::default().fg(field_fg).bg(field_bg),
    )];
    if pad > 0 {
        spans.push(Span::styled(
            " ".repeat(pad),
            Style::default().bg(field_bg),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), hits.input);
    if hits.cancel.width > 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                CANCEL_LABEL,
                Style::default().fg(p.overlay0),
            )),
            hits.cancel,
        );
    }
    if hits.add.width > 0 {
        let label = if saving { SAVE_CONFIRM_LABEL } else { ADD_LABEL };
        frame.render_widget(
            Paragraph::new(Span::styled(
                label,
                Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
            )),
            hits.add,
        );
    }
}

fn draw_rounded_box(frame: &mut Frame, rect: Rect, border: Style) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }
    let buf = frame.buffer_mut();
    let right = rect.x + rect.width.saturating_sub(1);
    let bottom = rect.y + rect.height.saturating_sub(1);

    buf[(rect.x, rect.y)].set_symbol("╭").set_style(border);
    buf[(right, rect.y)].set_symbol("╮").set_style(border);
    buf[(rect.x, bottom)].set_symbol("╰").set_style(border);
    buf[(right, bottom)].set_symbol("╯").set_style(border);
    for x in rect.x + 1..right {
        buf[(x, rect.y)].set_symbol("─").set_style(border);
        buf[(x, bottom)].set_symbol("─").set_style(border);
    }
    for y in rect.y + 1..bottom {
        buf[(rect.x, y)].set_symbol("│").set_style(border);
        buf[(right, y)].set_symbol("│").set_style(border);
    }
}

#[cfg(test)]
fn layout_hits(player_rect: Rect, expanded: bool, snapshot: &PlayerSnapshot) -> PlayerHitAreas {
    layout_hits_with_mode(
        player_rect,
        expanded,
        snapshot,
        &PlayerQueueMode::Queue,
    )
}

fn layout_hits_with_mode(
    player_rect: Rect,
    expanded: bool,
    snapshot: &PlayerSnapshot,
    mode: &PlayerQueueMode,
) -> PlayerHitAreas {
    let mut hits = PlayerHitAreas {
        player: player_rect,
        ..PlayerHitAreas::default()
    };
    if player_rect.width < 2 || player_rect.height == 0 {
        return hits;
    }
    let inner = Rect::new(
        player_rect.x + 1,
        player_rect.y + 1,
        player_rect.width.saturating_sub(2),
        player_rect.height.saturating_sub(2),
    );
    if inner.width == 0 || inner.height == 0 {
        if player_rect.height == 1 {
            hits.title = player_rect;
        }
        return hits;
    }

    if expanded {
        if player_full_too_small(player_rect) {
            return layout_cramped_hits(hits, inner);
        }
        return layout_full_hits(hits, inner, mode);
    }

    let bar_y = inner.y;
    let bar_width = inner.width.saturating_sub(2);
    let positions = transport_positions(snapshot, bar_width);
    hits.chevron = Rect::new(inner.x + inner.width.saturating_sub(1), bar_y, 1, 1);

    let first_transport_x = positions
        .first()
        .map(|(x, _, _)| inner.x + *x)
        .unwrap_or(hits.chevron.x);
    hits.title = Rect::new(
        inner.x,
        bar_y,
        first_transport_x.saturating_sub(inner.x).max(1),
        1,
    );

    for (x_off, width, kind) in positions {
        let rect = Rect::new(inner.x + x_off, bar_y, width.max(1).saturating_add(1), 1);
        match kind {
            TransportKind::Prev => hits.prev = rect,
            TransportKind::Play => hits.play = rect,
            TransportKind::Next => hits.next = rect,
            TransportKind::Loop => hits.looping = rect,
            TransportKind::Shuffle => hits.shuffle = rect,
        }
    }
    hits
}

fn layout_cramped_hits(mut hits: PlayerHitAreas, inner: Rect) -> PlayerHitAreas {
    let collapse = "collapse ▾";
    let collapse_w = (UnicodeWidthStr::width(collapse) as u16).min(inner.width);
    hits.chevron = Rect::new(
        inner.x + inner.width.saturating_sub(collapse_w),
        inner.y,
        collapse_w,
        1,
    );
    hits.title = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(collapse_w).max(1),
        1,
    );
    hits
}

fn layout_full_hits(
    mut hits: PlayerHitAreas,
    inner: Rect,
    mode: &PlayerQueueMode,
) -> PlayerHitAreas {
    let (show_density, collapse) = header_chrome(inner.width);
    let collapse_w = (UnicodeWidthStr::width(collapse) as u16).min(inner.width);
    hits.chevron = Rect::new(
        inner.x + inner.width.saturating_sub(collapse_w),
        inner.y,
        collapse_w,
        1,
    );
    let header_w = (UnicodeWidthStr::width(HEADER_LABEL) as u16).min(inner.width);
    hits.title = Rect::new(inner.x, inner.y, header_w, 1);
    if show_density {
        let density = density_header_label();
        let density_w = UnicodeWidthStr::width(density.as_str()) as u16;
        let density_x = inner.x.saturating_add(header_w).saturating_add(1);
        if density_x.saturating_add(density_w) <= hits.chevron.x {
            hits.density = Rect::new(density_x, inner.y, density_w, 1);
        }
    }

    if inner.height >= 2 {
        let input_y = inner.y + 1;
        let saving = matches!(mode, PlayerQueueMode::SaveName);
        let confirm_w = UnicodeWidthStr::width(if saving {
            SAVE_CONFIRM_LABEL
        } else {
            ADD_LABEL
        }) as u16;
        let cancel_w = if saving {
            UnicodeWidthStr::width(CANCEL_LABEL) as u16
        } else {
            0
        };
        let buttons = confirm_w
            .saturating_add(cancel_w)
            .saturating_add(u16::from(saving));
        let field_w = inner.width.saturating_sub(buttons.saturating_add(1));
        hits.input = Rect::new(inner.x, input_y, field_w.max(1), 1);
        let mut btn_x = inner.x.saturating_add(field_w).saturating_add(1);
        if cancel_w > 0 && btn_x + cancel_w <= inner.x + inner.width {
            hits.cancel = Rect::new(btn_x, input_y, cancel_w, 1);
            btn_x = btn_x.saturating_add(cancel_w).saturating_add(1);
        }
        if confirm_w > 0 && btn_x + confirm_w <= inner.x + inner.width {
            hits.add = Rect::new(btn_x, input_y, confirm_w, 1);
        }
    }

    // Bottom stack: now-playing (2), transport (1), scrub (1), volume/status (1).
    let foot = 5u16.min(inner.height.saturating_sub(3));
    let transport_y = inner.y + inner.height.saturating_sub(3.min(foot));
    if inner.height >= 5 {
        let playlist_y = inner.y + 2;
        let np_top = transport_y.saturating_sub(2);
        let leftover = np_top.saturating_sub(playlist_y);
        if leftover > 0 && playlist_y < np_top {
            let has_items = !current_playlist().items.is_empty();
            let chrome = leftover >= 2;
            if chrome {
                apply_chrome_hits(&mut hits, inner, playlist_y, mode, has_items);
                hits.playlist = Rect::new(
                    inner.x,
                    playlist_y + 1,
                    inner.width,
                    leftover
                        .saturating_sub(1)
                        .min(current_density().playlist_rows()),
                );
            } else {
                hits.playlist = Rect::new(
                    inner.x,
                    playlist_y,
                    inner.width,
                    leftover.min(current_density().playlist_rows()),
                );
            }
        }
        let embed_top = hits
            .playlist
            .y
            .saturating_add(hits.playlist.height)
            .max(playlist_y);
        let embed_h = np_top.saturating_sub(embed_top);
        if embed_h > 0 && !current_density().hide_embed() {
            hits.padding = Rect::new(inner.x, embed_top, inner.width, embed_h);
        }
    }

    if inner.height >= 4 {
        let glyphs = [LOOP, PREV, PLAY, NEXT, SHUFFLE];
        let kinds = [
            TransportKind::Loop,
            TransportKind::Prev,
            TransportKind::Play,
            TransportKind::Next,
            TransportKind::Shuffle,
        ];
        let gap = 2u16;
        let glyph_w: u16 = glyphs.iter().map(|g| UnicodeWidthStr::width(*g) as u16).sum();
        let total = glyph_w + gap * (glyphs.len() as u16 - 1);
        let mut x = inner.x + inner.width.saturating_sub(total) / 2;
        for (glyph, kind) in glyphs.into_iter().zip(kinds) {
            let w = (UnicodeWidthStr::width(glyph) as u16).max(1);
            let rect = Rect::new(x, transport_y, w.saturating_add(1), 1);
            match kind {
                TransportKind::Prev => hits.prev = rect,
                TransportKind::Play => hits.play = rect,
                TransportKind::Next => hits.next = rect,
                TransportKind::Loop => hits.looping = rect,
                TransportKind::Shuffle => hits.shuffle = rect,
            }
            x = x.saturating_add(w).saturating_add(gap);
        }
    }

    let vol_y = inner.y + inner.height.saturating_sub(1);
    if inner.height >= 5 && vol_y > transport_y {
        apply_volume_hits(&mut hits, inner, vol_y);
        let scrub_y = transport_y.saturating_add(1);
        if scrub_y < vol_y {
            apply_scrub_hits(&mut hits, inner, scrub_y);
        }
    }
    hits
}

fn apply_chrome_hits(
    hits: &mut PlayerHitAreas,
    inner: Rect,
    y: u16,
    mode: &PlayerQueueMode,
    has_items: bool,
) {
    if matches!(mode, PlayerQueueMode::SaveOverwrite { .. }) {
        let yes_w = UnicodeWidthStr::width(OVERWRITE_YES) as u16;
        let no_w = UnicodeWidthStr::width(OVERWRITE_NO) as u16;
        let mut x = inner.x + inner.width;
        x = x.saturating_sub(yes_w);
        hits.overwrite_yes = Rect::new(x, y, yes_w.min(inner.width), 1);
        x = x.saturating_sub(1).saturating_sub(no_w);
        hits.overwrite_no = Rect::new(x, y, no_w, 1);
        hits.queue = Rect::new(inner.x, y, x.saturating_sub(inner.x).saturating_sub(1), 1);
        return;
    }
    let mut x = inner.x + inner.width;
    if has_items {
        let clear_w = UnicodeWidthStr::width(CLEAR_LABEL) as u16;
        x = x.saturating_sub(clear_w);
        hits.clear = Rect::new(x, y, clear_w.min(inner.width), 1);
        x = x.saturating_sub(1);
    }
    let load_w = UnicodeWidthStr::width(LOAD_LABEL) as u16;
    x = x.saturating_sub(load_w);
    hits.load = Rect::new(x, y, load_w, 1);
    x = x.saturating_sub(1);
    let save_w = UnicodeWidthStr::width(SAVE_LABEL) as u16;
    x = x.saturating_sub(save_w);
    hits.save = Rect::new(x, y, save_w, 1);
    let queue_w = UnicodeWidthStr::width(QUEUE_LABEL) as u16;
    hits.queue = Rect::new(
        inner.x,
        y,
        queue_w.min(x.saturating_sub(inner.x).saturating_sub(1)),
        1,
    );
}

fn apply_scrub_hits(hits: &mut PlayerHitAreas, inner: Rect, y: u16) {
    if inner.width < SCRUB_TIME_W * 2 + 4 {
        return;
    }
    let bar_x = inner.x + SCRUB_TIME_W + 1;
    let bar_w = inner.width.saturating_sub(SCRUB_TIME_W * 2 + 2);
    if bar_w == 0 {
        return;
    }
    hits.scrub = Rect::new(bar_x, y, bar_w, 1);
}

fn apply_volume_hits(hits: &mut PlayerHitAreas, inner: Rect, y: u16) {
    // "vol" + " " + "-" + " " + bar + " " + "+" + " " + "100%"
    const PREFIX: u16 = 6;
    const SUFFIX: u16 = 7;
    const MIN_BAR: u16 = 4;
    if inner.width < PREFIX + SUFFIX + MIN_BAR {
        return;
    }
    let rest = inner.width.saturating_sub(PREFIX + SUFFIX);
    // Leave a gap + truncated status when there is room (" paused").
    let bar_w = if rest > 13 {
        rest.saturating_sub(9).max(MIN_BAR)
    } else {
        rest.max(MIN_BAR)
    };
    let down_x = inner.x + 4;
    let bar_x = inner.x + PREFIX;
    let up_x = bar_x + bar_w + 1;
    hits.vol_down = Rect::new(down_x, y, 2, 1);
    hits.vol_bar = Rect::new(bar_x, y, bar_w, 1);
    hits.vol_up = Rect::new(up_x, y, 2, 1);
    hits.volume = Rect::new(inner.x, y, PREFIX + bar_w + SUFFIX, 1);
}

#[derive(Clone, Copy)]
enum TransportKind {
    Prev,
    Play,
    Next,
    Loop,
    Shuffle,
}

fn transport_positions(snapshot: &PlayerSnapshot, width: u16) -> Vec<(u16, u16, TransportKind)> {
    let play_glyph = if snapshot.playing() { PAUSE } else { PLAY };
    let after_cover = width.saturating_sub(2);
    // Transport keeps first claim so a long title cannot hide Loop/Shuffle.
    let glyphs = visible_transport(after_cover, play_glyph);
    let transport_width: usize = glyphs
        .iter()
        .map(|(g, _)| UnicodeWidthStr::width(*g) + 1)
        .sum::<usize>()
        .saturating_sub(if glyphs.is_empty() { 0 } else { 1 });
    let title_budget = (after_cover as usize).saturating_sub(transport_width);
    let mut x = 2u16 + title_budget as u16;
    let mut out = Vec::new();
    for (i, (glyph, kind)) in glyphs.into_iter().enumerate() {
        if i > 0 {
            x = x.saturating_add(1);
        }
        let w = UnicodeWidthStr::width(glyph) as u16;
        out.push((x, w, kind));
        x = x.saturating_add(w);
    }
    out
}

fn title_of(snapshot: &PlayerSnapshot) -> String {
    match snapshot {
        PlayerSnapshot::Offline => "player offline".to_string(),
        PlayerSnapshot::Online { title, artist, .. } => {
            let locator = snapshot_locator(snapshot);
            let name = playlist_display_title(title.clone(), &locator);
            match (name.is_empty(), artist.as_deref()) {
                (true, Some(artist)) => artist.to_string(),
                (true, None) => "no track".into(),
                (false, Some(artist)) => format!("{name} — {artist}"),
                (false, None) => name,
            }
        }
    }
}

fn player_line(p: &Palette, snapshot: &PlayerSnapshot, width: u16) -> Line<'static> {
    let title = title_of(snapshot);
    let playing = snapshot.playing();
    let online = !matches!(snapshot, PlayerSnapshot::Offline);
    let dim = !online;
    let cover_bg = if dim { p.surface0 } else { p.accent };
    let cover_fg = if dim {
        p.overlay0
    } else {
        super::widgets::contrasting_label_fg(p, cover_bg)
    };
    let title_fg = if dim || (!playing && title == "no track") {
        p.overlay0
    } else {
        p.text
    };

    let mut spans = vec![Span::styled(
        COVER,
        Style::default()
            .fg(cover_fg)
            .bg(cover_bg)
            .add_modifier(Modifier::BOLD),
    )];
    let mut used = UnicodeWidthStr::width(COVER);

    let after_cover = (width as usize).saturating_sub(used).saturating_sub(1);
    let play_glyph = if playing { PAUSE } else { PLAY };
    let transport = visible_transport(after_cover as u16, play_glyph);
    let transport_width: usize = transport
        .iter()
        .map(|(glyph, _)| UnicodeWidthStr::width(*glyph))
        .sum::<usize>()
        + transport.len().saturating_sub(1);

    let title_budget = after_cover.saturating_sub(transport_width);
    let truncated = truncate_width(&title, title_budget);
    let truncated_width = UnicodeWidthStr::width(truncated.as_str());
    let pad = title_budget.saturating_sub(truncated_width);

    spans.push(Span::raw(" "));
    used += 1;
    if !truncated.is_empty() {
        spans.push(Span::styled(truncated, Style::default().fg(title_fg)));
        used += truncated_width;
    }
    if pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
        used += pad;
    }

    if !transport.is_empty() && used < width as usize {
        for (i, (glyph, kind)) in transport.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let active = match kind {
                TransportKind::Play => playing && !dim,
                TransportKind::Loop => snapshot.looping() && !dim,
                TransportKind::Shuffle => snapshot.shuffle() && !dim,
                _ => false,
            };
            let fg = if dim {
                p.overlay0
            } else if active {
                p.accent
            } else if matches!(kind, TransportKind::Play) {
                p.overlay1
            } else {
                p.overlay0
            };
            let style = if active {
                Style::default().fg(fg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg)
            };
            spans.push(Span::styled((*glyph).to_string(), style));
        }
    }

    Line::from(spans)
}

fn visible_transport(budget: u16, play_glyph: &'static str) -> Vec<(&'static str, TransportKind)> {
    let mut glyphs = vec![
        (PREV, TransportKind::Prev),
        (play_glyph, TransportKind::Play),
        (NEXT, TransportKind::Next),
        (LOOP, TransportKind::Loop),
        (SHUFFLE, TransportKind::Shuffle),
    ];
    while !glyphs.is_empty() {
        let width = glyphs
            .iter()
            .map(|(glyph, _)| UnicodeWidthStr::width(*glyph))
            .sum::<usize>()
            + glyphs.len().saturating_sub(1);
        if width <= budget as usize {
            return glyphs;
        }
        if glyphs.len() > 3 {
            glyphs.pop();
        } else if glyphs.len() == 3 {
            glyphs.remove(0);
            glyphs.pop();
        } else {
            glyphs.clear();
            glyphs.push((play_glyph, TransportKind::Play));
            let play_w = UnicodeWidthStr::width(play_glyph);
            return if play_w <= budget as usize {
                glyphs
            } else {
                Vec::new()
            };
        }
    }
    Vec::new()
}

fn truncate_width(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".into();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Palette;

    fn contents(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn online_idle() -> PlayerSnapshot {
        PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 1.0,
            seekable: true,
        }
    }

    #[test]
    fn bar_keeps_cover_title_and_transport_when_idle() {
        let p = Palette::catppuccin();
        let line = player_line(&p, &online_idle(), 26);
        let text = contents(&line);
        assert!(text.contains(COVER), "{text}");
        assert!(text.contains("no track"), "{text}");
        assert!(text.contains(PREV), "{text}");
        assert!(text.contains(PLAY), "{text}");
        assert!(text.contains(NEXT), "{text}");
        assert!(text.contains(LOOP), "{text}");
        assert!(text.contains(SHUFFLE), "{text}");
        assert!(!text.contains("player offline"), "{text}");
    }

    #[test]
    fn offline_keeps_the_same_bar_shape() {
        let p = Palette::catppuccin();
        let line = player_line(&p, &PlayerSnapshot::Offline, 26);
        let text = contents(&line);
        assert!(text.contains(COVER), "{text}");
        assert!(text.contains("player offline"), "{text}");
        assert!(text.contains(PLAY), "{text}");
        assert!(text.contains(PREV), "{text}");
        assert!(!text.contains("no track"), "{text}");
    }

    #[test]
    fn playing_uses_pause_glyph() {
        let p = Palette::catppuccin();
        let line = player_line(
            &p,
            &PlayerSnapshot::Online {
                title: Some("Crystal Quest Prelude".into()),
                artist: Some("zaethir".into()),
                url: None,
                playing: true,
                looping: false,
                shuffle: false,
                elapsed_sec: 57,
                duration_sec: 160,
                volume: 1.0,
                seekable: true,
            },
            40,
        );
        let text = contents(&line);
        assert!(text.contains("Crystal"), "{text}");
        assert!(text.contains(PAUSE), "{text}");
        assert!(!text.contains(PLAY), "{text}");
    }

    #[test]
    fn collapsed_box_matches_new_button_height() {
        let mut app = crate::app::state::AppState::test_new();
        assert!(!app.player_expanded);
        assert_eq!(player_rows(&app), PLAYER_COLLAPSED_ROWS);
        assert_eq!(PLAYER_COLLAPSED_ROWS, 3);
        app.player_expanded = true;
        assert_eq!(
            player_rows_for_sidebar(&app, 40),
            full_player_rows(40)
        );
        assert!(full_player_rows(40) > PLAYER_COLLAPSED_ROWS);
        assert!(full_player_rows(40) >= PLAYER_FULL_MIN_ROWS);
    }

    #[test]
    fn toggle_is_the_title_row_not_the_transport_icons() {
        let area = Rect::new(0, 20, 24, 3);
        let hits = layout_hits(area, false, &online_idle());
        assert!(hits.title.width > 0);
        assert!(hits.play.width > 0);
        assert!(hits.title.x + hits.title.width <= hits.play.x || hits.play.x == 0);
        assert_eq!(hits.play.y, hits.title.y);
    }

    #[test]
    fn long_title_does_not_eat_loop_and_shuffle() {
        let p = Palette::catppuccin();
        let snapshot = PlayerSnapshot::Online {
            title: Some("Example Domain".into()),
            artist: None,
            url: None,
            playing: false,
            looping: true,
            shuffle: true,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 0.3,
            seekable: true,
        };
        let line = player_line(&p, &snapshot, 21);
        let text = contents(&line);
        assert!(text.contains(LOOP), "{text}");
        assert!(text.contains(SHUFFLE), "{text}");
        let hits = layout_hits(Rect::new(0, 40, 25, 3), false, &snapshot);
        assert!(hits.looping.width > 0, "{hits:?}");
        assert!(hits.shuffle.width > 0, "{hits:?}");
    }

    #[test]
    fn full_mode_exposes_volume_hits_bar_does_not() {
        let bar = layout_hits(Rect::new(0, 40, 30, 3), false, &online_idle());
        assert_eq!(bar.volume, Rect::default());
        assert_eq!(bar.vol_down, Rect::default());
        assert_eq!(bar.vol_up, Rect::default());
        assert_eq!(bar.vol_bar, Rect::default());

        let full = layout_hits(Rect::new(0, 10, 32, 20), true, &online_idle());
        assert!(full.volume.width > 0, "{full:?}");
        assert!(full.vol_down.width > 0, "{full:?}");
        assert!(full.vol_up.width > 0, "{full:?}");
        assert!(full.vol_bar.width >= 4, "{full:?}");
        assert_eq!(full.volume.y, full.player.y + full.player.height - 2);
        assert_eq!(
            player_action_at_hits(&full, full.vol_down.x, full.vol_down.y),
            Some(PlayerAction::VolumeDown)
        );
        assert_eq!(
            player_action_at_hits(&full, full.vol_up.x, full.vol_up.y),
            Some(PlayerAction::VolumeUp)
        );
        assert_eq!(
            player_action_at_hits(&full, full.vol_bar.x, full.vol_bar.y),
            Some(PlayerAction::VolumeSet)
        );
    }

    fn player_action_at_hits(hits: &PlayerHitAreas, col: u16, row: u16) -> Option<PlayerAction> {
        if hits.player == Rect::default() || !contains(hits.player, col, row) {
            return None;
        }
        if contains(hits.vol_down, col, row) {
            return Some(PlayerAction::VolumeDown);
        }
        if contains(hits.vol_up, col, row) {
            return Some(PlayerAction::VolumeUp);
        }
        if contains(hits.vol_bar, col, row) {
            return Some(PlayerAction::VolumeSet);
        }
        if contains(hits.volume, col, row) {
            return Some(PlayerAction::VolumeIdle);
        }
        Some(PlayerAction::Background)
    }

    #[test]
    fn volume_nudge_posts_clamped_level() {
        let _guard = lock_test_player();
        take_test_posts();
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 1.0,
            seekable: true,
        });
        nudge_volume(-VOLUME_STEP);
        let posts = take_test_posts();
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert_eq!(posts[0].0, "/volume");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["level"], 0.9);
        assert_eq!(current_snapshot().volume(), 0.9);

        take_test_posts();
        nudge_volume(VOLUME_STEP);
        let posts = take_test_posts();
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["level"], 1.0);

        take_test_posts();
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 0.05,
            seekable: true,
        });
        nudge_volume(-VOLUME_STEP);
        let posts = take_test_posts();
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["level"], 0.0);
    }

    #[test]
    fn volume_bar_click_posts_fraction() {
        let _guard = lock_test_player();
        take_test_posts();
        let bar = Rect::new(10, 40, 10, 1);
        post_volume_at_bar(bar, 10);
        let body: serde_json::Value = serde_json::from_str(&take_test_posts()[0].1).unwrap();
        assert_eq!(body["level"], 0.05);
        take_test_posts();
        post_volume_at_bar(bar, 19);
        let body: serde_json::Value = serde_json::from_str(&take_test_posts()[0].1).unwrap();
        assert_eq!(body["level"], 0.95);
    }

    #[test]
    fn paste_row_is_add_only() {
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert!(hits.add.width >= 3, "Add hit missing: {hits:?}");
        assert_eq!(hits.add.y, hits.input.y);
        assert_eq!(hits.add.x, hits.input.x + hits.input.width + 1);
        assert_eq!(
            player_action_at(&app, hits.add.x, hits.add.y),
            Some(PlayerAction::Add)
        );
        assert_ne!(
            player_action_at(&app, hits.input.x, hits.input.y),
            Some(PlayerAction::Add)
        );
    }

    #[test]
    fn add_posts_then_clears_the_paste_field() {
        let _guard = lock_test_player();
        let mut app = crate::app::state::AppState::test_new();
        app.player_link.set_text(" /System/Library/Sounds/Glass.aiff ");
        take_test_posts();
        submit_player_add(&mut app);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/add");
        assert!(posts[0].1.contains("/System/Library/Sounds/Glass.aiff"));
        assert!(
            app.player_link.text().is_empty(),
            "Add must clear the field, got {:?}",
            app.player_link.text()
        );
        assert_eq!(app.player_link.cursor_row(), (0, 0));
    }

    #[test]
    fn empty_submit_does_not_post() {
        let _guard = lock_test_player();
        let mut app = crate::app::state::AppState::test_new();
        take_test_posts();
        submit_player_add(&mut app);
        assert!(take_test_posts().is_empty());
    }

    #[test]
    fn playlist_fallback_title_is_basename_not_raw_url() {
        let playlist = parse_playlist_body(
            r#"{"items":[{"url":"https://www.youtube.com/watch?v=abc"},{"path":"/Music/album/track.flac"},{"url":"https://x.test/a","title":"Crystal Quest"}]}"#,
        );
        assert_eq!(playlist.items[0].title, "watch?v=abc");
        assert_eq!(playlist.items[0].url, "https://www.youtube.com/watch?v=abc");
        assert_eq!(playlist.items[1].title, "track.flac");
        assert_eq!(playlist.items[1].url, "/Music/album/track.flac");
        assert_eq!(playlist.items[2].title, "Crystal Quest");
    }

    #[test]
    fn locator_title_uses_the_same_basename_fallback() {
        let playlist = parse_playlist_body(
            r#"{"items":[{"url":"https://www.youtube.com/watch?v=abc","title":"https://www.youtube.com/watch?v=abc"}]}"#,
        );
        assert_eq!(playlist.items[0].title, "watch?v=abc");
        let snapshot = PlayerSnapshot::Online {
            title: Some("https://example.com/music/Glass.aiff".into()),
            artist: None,
            url: Some("https://example.com/music/Glass.aiff".into()),
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 1.0,
            seekable: true,
        };
        assert_eq!(title_of(&snapshot), "Glass.aiff");
        let untitled = PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: Some("/Music/album/track.flac".into()),
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 1.0,
            seekable: true,
        };
        assert_eq!(title_of(&untitled), "track.flac");
    }

    #[test]
    fn empty_playlist_teaches_add() {
        let _guard = lock_test_player();
        set_test_playlist(PlaylistSnapshot::default());
        assert_eq!(PLAYLIST_EMPTY, "paste a link, then Add");
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert_eq!(hits.clear, Rect::default());
        assert!(hits.save.width > 0, "save stays on an empty queue: {hits:?}");
        assert!(hits.load.width > 0, "load stays on an empty queue: {hits:?}");
        assert_eq!(hits.playlist.y, hits.input.y + 2);
        assert_eq!(
            player_action_at(&app, hits.save.x, hits.save.y),
            Some(PlayerAction::Save)
        );
        assert_eq!(
            player_action_at(&app, hits.load.x, hits.load.y),
            Some(PlayerAction::LoadSaved)
        );
    }

    #[test]
    fn nonempty_playlist_exposes_clear_on_the_queue_chrome_row() {
        let _guard = lock_test_player();
        set_test_playlist(sample_playlist());
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert!(hits.clear.width >= 5, "clear hit missing: {hits:?}");
        assert_eq!(hits.clear.y, hits.input.y + 1);
        assert_eq!(hits.playlist.y, hits.clear.y + 1);
        assert_eq!(
            player_action_at(&app, hits.clear.x, hits.clear.y),
            Some(PlayerAction::PlaylistClear)
        );
        assert_ne!(
            player_action_at(&app, hits.playlist.x, hits.playlist.y),
            Some(PlayerAction::PlaylistClear)
        );
    }

    #[test]
    fn clear_posts_and_empties_the_local_queue() {
        let _guard = lock_test_player();
        set_test_playlist(sample_playlist());
        take_test_posts();
        post_playlist_clear();
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/clear");
        assert!(current_playlist().items.is_empty());
    }

    #[test]
    fn playlist_changed_fires_on_title_without_snapshot_move() {
        let _guard = lock_test_player();
        set_test_playlist(PlaylistSnapshot {
            items: vec![PlaylistItem {
                title: "watch?v=abc".into(),
                url: "https://www.youtube.com/watch?v=abc".into(),
            }],
            index: None,
        });
        let mut last = current_playlist();
        assert!(!playlist_changed(&mut last));
        set_test_playlist(PlaylistSnapshot {
            items: vec![PlaylistItem {
                title: "Crystal Quest".into(),
                url: "https://www.youtube.com/watch?v=abc".into(),
            }],
            index: None,
        });
        assert!(playlist_changed(&mut last));
        assert_eq!(last.items[0].title, "Crystal Quest");
        assert!(!playlist_changed(&mut last));
    }

    #[test]
    fn expanded_below_floor_has_no_paste_row() {
        let app = expanded_app(Rect::new(0, 10, 18, 12));
        let hits = player_hit_areas(&app);
        assert!(
            player_full_too_small(app.view.player_rect),
            "18x12 must be under the full-view floor"
        );
        assert_eq!(hits.input, Rect::default());
        assert_eq!(hits.add, Rect::default());
        assert_eq!(hits.playlist, Rect::default());
        assert!(hits.chevron.width > 0, "collapse must stay clickable");
        assert_eq!(
            player_action_at(&app, hits.chevron.x, hits.chevron.y),
            Some(PlayerAction::Toggle)
        );
    }

    fn expanded_app(rect: Rect) -> crate::app::state::AppState {
        let mut app = crate::app::state::AppState::test_new();
        app.player_expanded = true;
        app.view.player_rect = rect;
        app
    }

    fn sample_playlist() -> PlaylistSnapshot {
        PlaylistSnapshot {
            items: vec![
                PlaylistItem {
                    title: "Glass".into(),
                    url: "/System/Library/Sounds/Glass.aiff".into(),
                },
                PlaylistItem {
                    title: "Basso".into(),
                    url: "/System/Library/Sounds/Basso.aiff".into(),
                },
                PlaylistItem {
                    title: "https://www.youtube.com/watch?v=l-vSSYEuO88".into(),
                    url: "https://www.youtube.com/watch?v=l-vSSYEuO88".into(),
                },
            ],
            index: Some(2),
        }
    }

    #[test]
    fn full_mode_exposes_playlist_and_scrub_hits() {
        let _guard = lock_test_player();
        set_test_playlist(PlaylistSnapshot::default());
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert!(hits.playlist.width > 0, "{hits:?}");
        assert!(hits.playlist.height >= 1, "{hits:?}");
        assert!(hits.scrub.width >= 4, "{hits:?}");
        assert_eq!(hits.playlist.y, hits.input.y + 2);
        assert!(hits.playlist.y + hits.playlist.height <= hits.play.y.saturating_sub(2));
        assert_eq!(hits.scrub.y, hits.play.y + 1);
    }

    #[test]
    fn playlist_row_and_remove_actions() {
        let _guard = lock_test_player();
        set_test_playlist(sample_playlist());
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert_eq!(
            player_action_at(&app, hits.playlist.x, hits.playlist.y),
            Some(PlayerAction::PlaylistLoad(0))
        );
        assert_eq!(
            player_action_at(
                &app,
                hits.playlist.x + hits.playlist.width - 1,
                hits.playlist.y
            ),
            Some(PlayerAction::PlaylistRemove(0))
        );
        assert_eq!(
            player_action_at(&app, hits.playlist.x, hits.playlist.y + 1),
            Some(PlayerAction::PlaylistLoad(1))
        );
    }

    #[test]
    fn playlist_load_and_remove_post_to_daemon() {
        let _guard = lock_test_player();
        set_test_playlist(sample_playlist());
        take_test_posts();
        post_playlist_load(0);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/load");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["url"], "/System/Library/Sounds/Glass.aiff");

        take_test_posts();
        post_playlist_remove(1);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/remove");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["index"], 1);
        assert_eq!(current_playlist().items.len(), 2);
        assert_eq!(current_playlist().index, Some(1));
    }

    #[test]
    fn seek_posts_absolute_seconds_from_click() {
        let _guard = lock_test_player();
        take_test_posts();
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: true,
            looping: false,
            shuffle: false,
            elapsed_sec: 10,
            duration_sec: 100,
            volume: 1.0,
            seekable: true,
        });
        let bar = Rect::new(10, 40, 10, 1);
        post_seek_at_bar(bar, 10);
        let body: serde_json::Value = serde_json::from_str(&take_test_posts()[0].1).unwrap();
        assert_eq!(body["seconds"], 5.0);

        take_test_posts();
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: true,
            looping: false,
            shuffle: false,
            elapsed_sec: 10,
            duration_sec: 100,
            volume: 1.0,
            seekable: false,
        });
        post_seek_at_bar(bar, 19);
        assert!(take_test_posts().is_empty(), "unseekable must not POST /seek");
    }

    #[test]
    fn unseekable_scrub_is_idle_not_seek() {
        let _guard = lock_test_player();
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: true,
            looping: false,
            shuffle: false,
            elapsed_sec: 10,
            duration_sec: 100,
            volume: 1.0,
            seekable: false,
        });
        let app = expanded_app(Rect::new(0, 10, 32, 20));
        let hits = player_hit_areas(&app);
        assert_eq!(
            player_action_at(&app, hits.scrub.x, hits.scrub.y),
            Some(PlayerAction::ScrubIdle)
        );

        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: true,
            looping: false,
            shuffle: false,
            elapsed_sec: 10,
            duration_sec: 100,
            volume: 1.0,
            seekable: true,
        });
        assert_eq!(
            player_action_at(&app, hits.scrub.x, hits.scrub.y),
            Some(PlayerAction::Seek)
        );
    }

    #[test]
    fn parse_playlist_404_shape_is_empty() {
        let empty = parse_playlist_body("Cannot GET /playlist");
        assert!(empty.items.is_empty());
        let parsed = parse_playlist_body(
            r#"{"items":[{"kind":"local","url":"/a.aiff","path":"/a.aiff","title":"Glass"}],"index":0,"length":1}"#,
        );
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].title, "Glass");
        assert_eq!(parsed.index, Some(0));
    }

    #[test]
    fn playlist_name_rejects_path_chars_and_dotdot() {
        assert!(valid_playlist_name("gym"));
        assert!(valid_playlist_name("wave3-test"));
        assert!(valid_playlist_name("a.b_c-1"));
        assert!(!valid_playlist_name(""));
        assert!(!valid_playlist_name("gym/foo"));
        assert!(!valid_playlist_name("gym\\foo"));
        assert!(!valid_playlist_name(".."));
        assert!(!valid_playlist_name("foo..bar"));
        assert!(!valid_playlist_name("has space"));
    }

    #[test]
    fn save_posts_name_without_overwrite_and_skips_invalid() {
        let _guard = lock_test_player();
        let mut app = crate::app::state::AppState::test_new();
        enter_save_name(&mut app);
        app.player_save_name.set_text("gym/nope");
        take_test_posts();
        submit_player_save(&mut app, false);
        assert!(take_test_posts().is_empty());
        assert!(matches!(app.player_queue_mode, PlayerQueueMode::SaveName));

        app.player_save_name.set_text("gym");
        submit_player_save(&mut app, false);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/save");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["name"], "gym");
        assert!(body.get("overwrite").is_none());
        assert!(matches!(app.player_queue_mode, PlayerQueueMode::Queue));
    }

    #[test]
    fn save_409_asks_overwrite_then_posts_flag() {
        let _guard = lock_test_player();
        let mut app = crate::app::state::AppState::test_new();
        enter_save_name(&mut app);
        app.player_save_name.set_text("gym");
        push_test_reply(409, r#"{"error":"playlist exists","name":"gym"}"#);
        take_test_posts();
        submit_player_save(&mut app, false);
        assert!(matches!(
            app.player_queue_mode,
            PlayerQueueMode::SaveOverwrite { ref name } if name == "gym"
        ));
        let app = expanded_app_mode(Rect::new(0, 10, 32, 20), app);
        let hits = player_hit_areas(&app);
        assert_eq!(
            player_action_at(&app, hits.overwrite_yes.x, hits.overwrite_yes.y),
            Some(PlayerAction::OverwriteYes)
        );
        take_test_posts();
        let mut app = app;
        submit_player_save(&mut app, true);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/save");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["name"], "gym");
        assert_eq!(body["overwrite"], true);
        assert!(matches!(app.player_queue_mode, PlayerQueueMode::Queue));
    }

    #[test]
    fn load_replaces_queue_wholesale_and_never_merges() {
        let _guard = lock_test_player();
        set_test_playlist(sample_playlist());
        set_test_saved_names(vec!["gym".into()]);
        let mut app = expanded_app(Rect::new(0, 10, 32, 20));
        enter_load_picker(&mut app);
        assert!(matches!(app.player_queue_mode, PlayerQueueMode::LoadPicker));
        let hits = player_hit_areas(&app);
        assert_eq!(
            player_action_at(&app, hits.playlist.x, hits.playlist.y),
            Some(PlayerAction::PickSaved(0))
        );
        push_test_reply(
            200,
            r#"{"ok":true,"name":"gym","items":[{"url":"/tmp/only.aiff","title":"Only"}],"index":-1,"length":1}"#,
        );
        take_test_posts();
        submit_load_named(&mut app, 0);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/playlist/load");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["name"], "gym");
        let playlist = current_playlist();
        assert_eq!(playlist.items.len(), 1, "load must replace, not merge");
        assert_eq!(playlist.items[0].title, "Only");
        assert_eq!(playlist.index, None);
        assert!(matches!(app.player_queue_mode, PlayerQueueMode::Queue));
    }

    #[test]
    fn density_round_trips_settings_and_walks_allowed() {
        let _guard = lock_test_player();
        take_test_posts();
        assert_eq!(current_density(), PlayerDensity::Comfortable);
        cycle_player_density();
        assert_eq!(current_density(), PlayerDensity::Comfortable);
        assert!(
            take_test_posts().is_empty(),
            "do not POST until GET /settings binds density"
        );

        set_test_settings(vec![
            SettingsField {
                key: "loop".into(),
                field_type: "bool".into(),
                value: serde_json::json!(false),
                allowed: None,
            },
            SettingsField {
                key: "volume".into(),
                field_type: "number".into(),
                value: serde_json::json!(0.4),
                allowed: None,
            },
            SettingsField {
                key: "density".into(),
                field_type: "string".into(),
                value: serde_json::json!("comfortable"),
                allowed: Some(vec![
                    "compact".into(),
                    "comfortable".into(),
                    "large-text".into(),
                ]),
            },
        ]);
        assert_eq!(current_density(), PlayerDensity::Comfortable);
        assert_eq!(current_density_value(), "comfortable");
        take_test_posts();
        cycle_player_density();
        assert_eq!(current_density_value(), "large-text");
        assert_eq!(current_density(), PlayerDensity::LargeText);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/settings");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["key"], "density");
        assert_eq!(body["value"], "large-text");
        assert!(body["value"].is_string(), "settings value must be JSON string");

        set_test_settings(vec![SettingsField {
            key: "density".into(),
            field_type: "string".into(),
            value: serde_json::json!("compact"),
            allowed: Some(vec![
                "compact".into(),
                "comfortable".into(),
                "large-text".into(),
            ]),
        }]);
        assert_eq!(
            current_density_value(),
            "compact",
            "GET /settings must overwrite UI memory after a restart"
        );
        assert_eq!(current_density(), PlayerDensity::Compact);
    }

    #[test]
    fn density_cycle_uses_allowed_not_a_hardcoded_triple() {
        let _guard = lock_test_player();
        set_test_settings(vec![SettingsField {
            key: "density".into(),
            field_type: "string".into(),
            value: serde_json::json!("large-text"),
            allowed: Some(vec![
                "compact".into(),
                "comfortable".into(),
                "large-text".into(),
                "cozy".into(),
            ]),
        }]);
        take_test_posts();
        cycle_player_density();
        assert_eq!(current_density_value(), "cozy");
        let posts = take_test_posts();
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert_eq!(body["key"], "density");
        assert_eq!(body["value"], "cozy");
        assert_eq!(density_header_label(), "cozy");
    }

    #[test]
    fn settings_volume_is_zero_to_one_slider_stays_on_volume_route() {
        let _guard = lock_test_player();
        let fields = parse_settings_body(
            r#"{"fields":[{"key":"loop","type":"bool","value":false},{"key":"shuffle","type":"bool","value":true},{"key":"volume","type":"number","value":0.4}]}"#,
        );
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[2].key, "volume");
        assert_eq!(fields[2].value, serde_json::json!(0.4));
        assert!(density_field(&fields).is_none());
        let with_density = parse_settings_body(
            r#"{"fields":[{"key":"loop","type":"bool","value":false},{"key":"shuffle","type":"bool","value":false},{"key":"volume","type":"number","value":1},{"key":"density","type":"string","value":"comfortable","allowed":["compact","comfortable","large-text"]}]}"#,
        );
        let density = density_field(&with_density).expect("density field");
        assert_eq!(density.key, "density");
        assert_eq!(density.field_type, "string");
        assert_eq!(density.value, serde_json::json!("comfortable"));
        assert_eq!(
            density.allowed,
            Some(vec![
                "compact".into(),
                "comfortable".into(),
                "large-text".into()
            ])
        );
        set_test_snapshot(PlayerSnapshot::Online {
            title: None,
            artist: None,
            url: None,
            playing: false,
            looping: false,
            shuffle: false,
            elapsed_sec: 0,
            duration_sec: 0,
            volume: 1.0,
            seekable: true,
        });
        take_test_posts();
        nudge_volume(-0.1);
        let posts = take_test_posts();
        assert_eq!(posts[0].0, "/volume");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
        assert!(body["level"].as_f64().unwrap() <= 1.0);
        assert_ne!(posts[0].0, "/settings");
    }

    #[test]
    fn saved_names_parse_sorted_list() {
        let names = parse_saved_names(r#"{"names":["gym","wave3-test"]}"#);
        assert_eq!(names, vec!["gym", "wave3-test"]);
        assert!(parse_saved_names(r#"{"names":[]}"#).is_empty());
    }

    fn expanded_app_mode(
        rect: Rect,
        mut app: crate::app::state::AppState,
    ) -> crate::app::state::AppState {
        app.player_expanded = true;
        app.view.player_rect = rect;
        app
    }
}
