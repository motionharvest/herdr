//! Sound notifications for agent state changes.
//!
//! Embeds mp3 files in the binary and plays them via system audio tools.
//! Uses afplay (macOS) or decoder-capable Linux audio players — no Rust audio dependencies.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::warn;

const DISABLE_SOUND_ENV: &str = "HERDR_DISABLE_SOUND";

static SOUND_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static SOUND_REQUEST: &[u8] = include_bytes!("../assets/sounds/request.mp3");

/// Which notification sound to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// Agent finished work (transitioned to Idle).
    Done,
    /// Agent needs input (transitioned to Blocked).
    Request,
}

/// One of the sounds shipped inside the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinSound {
    /// Config value and stable identity, e.g. `bell`.
    pub key: &'static str,
    /// One-line description shown in the settings picker.
    pub description: &'static str,
    bytes: &'static [u8],
}

/// Built-in choices for the "agent finished" sound, in picker order.
/// The first entry is the default, and is the chime Herdr has always used.
pub static DONE_SOUNDS: &[BuiltinSound] = &[
    BuiltinSound {
        key: "chime",
        description: "the original Herdr chime",
        bytes: include_bytes!("../assets/sounds/done.mp3"),
    },
    BuiltinSound {
        key: "bell",
        description: "a struck bell, ringing out",
        bytes: include_bytes!("../assets/sounds/bell.mp3"),
    },
    BuiltinSound {
        key: "arpeggio",
        description: "three rising notes landing on a chord",
        bytes: include_bytes!("../assets/sounds/arpeggio.mp3"),
    },
    BuiltinSound {
        key: "ping",
        description: "one short, high blip",
        bytes: include_bytes!("../assets/sounds/ping.mp3"),
    },
    BuiltinSound {
        key: "blip",
        description: "two quick wooden taps",
        bytes: include_bytes!("../assets/sounds/blip.mp3"),
    },
    BuiltinSound {
        key: "knock",
        description: "two low taps, easy to ignore",
        bytes: include_bytes!("../assets/sounds/knock.mp3"),
    },
];

/// The done sound used when nothing is configured.
pub fn default_done_sound() -> &'static BuiltinSound {
    &DONE_SOUNDS[0]
}

/// Look up a built-in done sound by its config key.
pub fn done_sound_by_key(key: &str) -> Option<&'static BuiltinSound> {
    DONE_SOUNDS.iter().find(|sound| sound.key == key)
}

/// What a settings preview should play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundPreview {
    /// A specific built-in, whatever the config currently says.
    Builtin(&'static BuiltinSound),
    /// The done sound the config resolves to right now, custom file included.
    ConfiguredDone,
}

impl SoundPreview {
    /// Wire form used to ask a client to play this preview.
    pub fn notify_message(self) -> String {
        match self {
            Self::Builtin(sound) => format!("preview {}", sound.key),
            Self::ConfiguredDone => "agent done".to_string(),
        }
    }
}

/// Play a notification sound in a background thread.
/// Silently does nothing if no audio player is available.
pub fn play(sound: Sound, config: &crate::config::SoundConfig) {
    let builtin = match sound {
        Sound::Done => config.done_sound(),
        Sound::Request => BuiltinSound {
            key: "request",
            description: "agent needs input",
            bytes: SOUND_REQUEST,
        },
    };
    play_with_override(builtin, config.path_for(sound));
}

/// Play a specific built-in, ignoring the configured done sound.
/// Custom sound file paths are ignored too — this is what makes the settings
/// picker audition the sound under the cursor rather than the saved one.
pub fn play_builtin(sound: &'static BuiltinSound) {
    play_with_override(*sound, None);
}

fn play_with_override(builtin: BuiltinSound, custom_path: Option<PathBuf>) {
    if playback_disabled() {
        return;
    }

    std::thread::spawn(move || {
        if let Some(path) = custom_path {
            match play_file(&path) {
                Ok(()) => return,
                Err(err) => {
                    warn!(path = %path.display(), sound = builtin.key, err = %err, "custom sound playback failed, falling back to built-in sound")
                }
            }
        }

        if let Err(err) = play_bytes(builtin.bytes) {
            warn!(sound = builtin.key, err = %err, "sound playback failed");
        }
    });
}

/// Test builds never reach the speakers. Tests that build a real `App` get
/// `local_sound_playback`, so driving a pane to Idle plays a notification for
/// real — under `cargo nextest` the env check caught that, but a bare
/// `cargo test` chimed out loud on the developer's machine, with the default
/// sound rather than the configured one. Silence belongs to the build, not to
/// the runner that happens to launch it.
fn playback_disabled() -> bool {
    cfg!(test) || sound_playback_disabled_by_env()
}

fn sound_playback_disabled_by_env() -> bool {
    std::env::var_os(DISABLE_SOUND_ENV).is_some() || std::env::var_os("NEXTEST").is_some()
}

fn play_file(path: &Path) -> Result<(), String> {
    match run_player(path) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!("player exited with {}", output.status)),
        Err(err) => Err(err),
    }
}

fn play_bytes(data: &[u8]) -> Result<(), String> {
    // Write to a temp file because the supported audio players need a file path.
    let tmp = temp_sound_path();
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(data).map_err(|e| e.to_string())?;
    drop(file);

    let result = run_player(&tmp);

    let _ = std::fs::remove_file(&tmp);

    match result {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!("player exited with {}", output.status)),
        Err(e) => Err(e),
    }
}

fn temp_sound_path() -> PathBuf {
    let id = SOUND_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("herdr-sound-{}-{id}.mp3", std::process::id()))
}

fn run_player(path: &Path) -> Result<Output, String> {
    if cfg!(target_os = "macos") {
        Command::new("afplay")
            .arg(path)
            .output()
            .map_err(|e| format!("no audio player available: {e}"))
    } else {
        run_linux_player(path)
    }
}

#[derive(Debug, Clone, Copy)]
struct AudioPlayer {
    program: &'static str,
    args: &'static [&'static str],
}

impl AudioPlayer {
    fn output(self, path: &Path) -> std::io::Result<Output> {
        Command::new(self.program)
            .args(self.args)
            .arg(path)
            .output()
    }
}

fn linux_audio_players() -> &'static [AudioPlayer] {
    // Do not add bare aplay here. It does not decode MP3 and plays MP3 bytes as raw PCM.
    &[
        AudioPlayer {
            program: "paplay",
            args: &[],
        },
        AudioPlayer {
            program: "pw-play",
            args: &[],
        },
        AudioPlayer {
            program: "ffplay",
            args: &["-nodisp", "-autoexit", "-loglevel", "quiet"],
        },
        AudioPlayer {
            program: "mpg123",
            args: &["-q"],
        },
        AudioPlayer {
            program: "mpv",
            args: &["--no-video", "--really-quiet"],
        },
    ]
}

fn run_linux_player(path: &Path) -> Result<Output, String> {
    let mut errors = Vec::new();

    for player in linux_audio_players() {
        match player.output(path) {
            Ok(output) if output.status.success() => return Ok(output),
            Ok(output) => errors.push(player_error(*player, &output)),
            Err(err) => errors.push(format!("{} failed: {err}", player.program)),
        }
    }

    Err(format!(
        "no mp3-capable audio player available: {}",
        errors.join("; ")
    ))
}

fn player_error(player: AudioPlayer, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();

    if stderr.is_empty() {
        format!("{} exited with {}", player.program, output.status)
    } else {
        format!("{} exited with {}: {stderr}", player.program, output.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builds_never_play_audio() {
        // Not a tautology in practice: this is the guard that keeps a bare
        // `cargo test` from firing notification sounds out of the speakers.
        assert!(playback_disabled());
    }

    #[test]
    fn temp_sound_paths_are_unique() {
        assert_ne!(temp_sound_path(), temp_sound_path());
    }

    #[test]
    fn linux_audio_players_are_mp3_capable() {
        let programs: Vec<&str> = linux_audio_players()
            .iter()
            .map(|player| player.program)
            .collect();

        assert_eq!(programs, ["paplay", "pw-play", "ffplay", "mpg123", "mpv"]);
        assert!(!programs.contains(&"aplay"));
    }
}
