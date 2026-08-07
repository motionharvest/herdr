use std::path::PathBuf;

use serde::Deserialize;

use crate::detect::Agent;

use super::io::resolve_config_relative_path;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SoundConfig {
    pub enabled: bool,
    /// Which built-in sound plays when an agent finishes, by key
    /// (`chime`, `bell`, …). The settings panel writes this. Naming a sound
    /// here beats the catch-all `path`, but `done_path` still wins over both.
    pub done: Option<String>,
    /// Optional mp3 file path used for all notification sounds.
    /// Relative paths are resolved from the config file's directory.
    pub path: Option<PathBuf>,
    /// Optional mp3 file path for "done" notifications.
    /// Relative paths are resolved from the config file's directory.
    pub done_path: Option<PathBuf>,
    /// Optional mp3 file path for "request" notifications.
    /// Relative paths are resolved from the config file's directory.
    pub request_path: Option<PathBuf>,
    pub agents: AgentSoundOverrides,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AgentSoundOverrides {
    pub pi: AgentSoundSetting,
    pub claude: AgentSoundSetting,
    pub codex: AgentSoundSetting,
    pub gemini: AgentSoundSetting,
    pub cursor: AgentSoundSetting,
    pub agy: AgentSoundSetting,
    pub cline: AgentSoundSetting,
    pub open_code: AgentSoundSetting,
    pub github_copilot: AgentSoundSetting,
    pub kimi: AgentSoundSetting,
    pub kiro: AgentSoundSetting,
    pub droid: AgentSoundSetting,
    pub amp: AgentSoundSetting,
    pub grok: AgentSoundSetting,
    pub hermes: AgentSoundSetting,
    pub kilo: AgentSoundSetting,
    pub qodercli: AgentSoundSetting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSoundSetting {
    #[default]
    Default,
    On,
    Off,
}

impl SoundConfig {
    pub fn allows(&self, agent: Option<Agent>) -> bool {
        if !self.enabled {
            return false;
        }

        !matches!(self.agents.for_agent(agent), AgentSoundSetting::Off)
    }

    /// The built-in sound that plays when an agent finishes. Unknown names
    /// fall back to the default; `diagnostics` reports them.
    pub fn done_sound(&self) -> crate::sound::BuiltinSound {
        self.done
            .as_deref()
            .and_then(|key| crate::sound::done_sound_by_key(key.trim()))
            .copied()
            .unwrap_or_else(|| *crate::sound::default_done_sound())
    }

    /// True when a custom mp3 overrides the picked done sound.
    pub fn done_sound_overridden_by_path(&self) -> bool {
        self.path_for(crate::sound::Sound::Done).is_some()
    }

    pub fn path_for(&self, sound: crate::sound::Sound) -> Option<PathBuf> {
        let path = match sound {
            // A named done sound outranks the catch-all `path`, so picking one
            // in settings takes effect without rewriting the rest of the config.
            crate::sound::Sound::Done if self.done.is_some() => self.done_path.as_ref(),
            crate::sound::Sound::Done => self.done_path.as_ref().or(self.path.as_ref()),
            crate::sound::Sound::Request => self.request_path.as_ref().or(self.path.as_ref()),
        }?;

        Some(resolve_config_relative_path(path))
    }

    pub fn diagnostics(&self) -> Vec<String> {
        let mut diagnostics = Vec::new();
        if let Some(done) = self
            .done
            .as_deref()
            .filter(|key| crate::sound::done_sound_by_key(key.trim()).is_none())
        {
            let known = crate::sound::DONE_SOUNDS
                .iter()
                .map(|sound| sound.key)
                .collect::<Vec<_>>()
                .join(", ");
            diagnostics.push(format!(
                "unknown sound: ui.sound.done = {done}; expected one of {known}; using {}",
                crate::sound::default_done_sound().key
            ));
        }
        for (field, path) in [
            ("ui.sound.path", self.path.as_ref()),
            ("ui.sound.done_path", self.done_path.as_ref()),
            ("ui.sound.request_path", self.request_path.as_ref()),
        ] {
            let Some(path) = path else {
                continue;
            };

            let resolved = resolve_config_relative_path(path);
            if resolved
                .extension()
                .and_then(|ext| ext.to_str())
                .is_none_or(|ext: &str| !ext.eq_ignore_ascii_case("mp3"))
            {
                diagnostics.push(format!(
                    "unsupported sound file format: {field} = {} resolves to {}; expected an mp3 file; using default sound",
                    path.display(),
                    resolved.display()
                ));
                continue;
            }

            if !resolved.exists() {
                diagnostics.push(format!(
                    "missing sound file: {field} = {} resolves to {}; using default sound",
                    path.display(),
                    resolved.display()
                ));
            } else if !resolved.is_file() {
                diagnostics.push(format!(
                    "invalid sound file: {field} = {} resolves to {}; using default sound",
                    path.display(),
                    resolved.display()
                ));
            }
        }
        diagnostics
    }
}

impl AgentSoundOverrides {
    pub fn for_agent(&self, agent: Option<Agent>) -> AgentSoundSetting {
        match agent {
            Some(Agent::Pi) => self.pi,
            Some(Agent::Claude) => self.claude,
            Some(Agent::Codex) => self.codex,
            Some(Agent::Gemini) => self.gemini,
            Some(Agent::Cursor) => self.cursor,
            Some(Agent::Antigravity) => self.agy,
            Some(Agent::Cline) => self.cline,
            Some(Agent::OpenCode) => self.open_code,
            Some(Agent::GithubCopilot) => self.github_copilot,
            Some(Agent::Kimi) => self.kimi,
            Some(Agent::Kiro) => self.kiro,
            Some(Agent::Droid) => self.droid,
            Some(Agent::Amp) => self.amp,
            Some(Agent::Grok) => self.grok,
            Some(Agent::Hermes) => self.hermes,
            Some(Agent::Kilo) => self.kilo,
            Some(Agent::Qodercli) => self.qodercli,
            None => AgentSoundSetting::Default,
        }
    }
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            done: None,
            path: None,
            done_path: None,
            request_path: None,
            agents: AgentSoundOverrides::default(),
        }
    }
}

impl Default for AgentSoundOverrides {
    fn default() -> Self {
        Self {
            pi: AgentSoundSetting::Default,
            claude: AgentSoundSetting::Default,
            codex: AgentSoundSetting::Default,
            gemini: AgentSoundSetting::Default,
            cursor: AgentSoundSetting::Default,
            agy: AgentSoundSetting::Default,
            cline: AgentSoundSetting::Default,
            open_code: AgentSoundSetting::Default,
            github_copilot: AgentSoundSetting::Default,
            kimi: AgentSoundSetting::Default,
            kiro: AgentSoundSetting::Default,
            droid: AgentSoundSetting::Off,
            amp: AgentSoundSetting::Default,
            grok: AgentSoundSetting::Default,
            hermes: AgentSoundSetting::Default,
            kilo: AgentSoundSetting::Default,
            qodercli: AgentSoundSetting::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{config_path, Config};

    #[test]
    fn sound_table_config_parses() {
        let toml = r#"
[ui.sound]
enabled = true
path = "sounds/all.mp3"
done_path = "sounds/done.mp3"
request_path = "/tmp/request.mp3"

[ui.sound.agents]
droid = "off"
claude = "on"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.ui.sound.enabled);
        assert_eq!(config.ui.sound.path, Some(PathBuf::from("sounds/all.mp3")));
        assert_eq!(
            config.ui.sound.done_path,
            Some(PathBuf::from("sounds/done.mp3"))
        );
        assert_eq!(
            config.ui.sound.request_path,
            Some(PathBuf::from("/tmp/request.mp3"))
        );
        assert_eq!(config.ui.sound.agents.droid, AgentSoundSetting::Off);
        assert_eq!(config.ui.sound.agents.claude, AgentSoundSetting::On);
        assert_eq!(config.ui.sound.agents.pi, AgentSoundSetting::Default);
    }

    #[test]
    fn sound_path_resolution_prefers_specific_over_global() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
path = "sounds/all.mp3"
done_path = "sounds/done.mp3"
"#,
        )
        .unwrap();

        let config_root = config_path().parent().unwrap().to_path_buf();
        assert_eq!(
            config.ui.sound.path_for(crate::sound::Sound::Done),
            Some(config_root.join("sounds/done.mp3"))
        );
        assert_eq!(
            config.ui.sound.path_for(crate::sound::Sound::Request),
            Some(config_root.join("sounds/all.mp3"))
        );
    }

    #[test]
    fn done_sound_defaults_to_the_original_chime_and_parses_a_pick() {
        let default_config = Config::default();
        assert_eq!(default_config.ui.sound.done, None);
        assert_eq!(
            default_config.ui.sound.done_sound().key,
            crate::sound::default_done_sound().key
        );

        let config: Config = toml::from_str(
            r#"
[ui.sound]
done = "bell"
"#,
        )
        .unwrap();
        assert_eq!(config.ui.sound.done_sound().key, "bell");
        assert!(config.collect_diagnostics().is_empty());
    }

    #[test]
    fn picked_done_sound_outranks_the_catch_all_path() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
done = "ping"
path = "sounds/all.mp3"
"#,
        )
        .unwrap();

        // The pick wins for "done"...
        assert_eq!(config.ui.sound.path_for(crate::sound::Sound::Done), None);
        assert_eq!(config.ui.sound.done_sound().key, "ping");
        // ...and leaves the request sound alone.
        assert!(config
            .ui
            .sound
            .path_for(crate::sound::Sound::Request)
            .is_some());
    }

    #[test]
    fn done_path_still_outranks_a_picked_done_sound() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
done = "ping"
done_path = "/tmp/herdr-custom.mp3"
"#,
        )
        .unwrap();

        assert_eq!(
            config.ui.sound.path_for(crate::sound::Sound::Done),
            Some(PathBuf::from("/tmp/herdr-custom.mp3"))
        );
        assert!(config.ui.sound.done_sound_overridden_by_path());
    }

    #[test]
    fn unknown_done_sound_falls_back_with_a_diagnostic() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
done = "foghorn"
"#,
        )
        .unwrap();

        assert_eq!(
            config.ui.sound.done_sound().key,
            crate::sound::default_done_sound().key
        );
        assert!(config
            .collect_diagnostics()
            .iter()
            .any(|diag| diag.contains("ui.sound.done = foghorn") && diag.contains("chime")));
    }

    #[test]
    fn missing_sound_file_produces_diagnostic() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
done_path = "sounds/missing.mp3"
"#,
        )
        .unwrap();

        let diagnostics = config.collect_diagnostics();
        assert!(diagnostics.iter().any(
            |diag| diag.contains("ui.sound.done_path") && diag.contains("using default sound")
        ));
    }

    #[test]
    fn non_mp3_sound_file_produces_diagnostic() {
        let config: Config = toml::from_str(
            r#"
[ui.sound]
path = "sounds/notification.wav"
"#,
        )
        .unwrap();

        let diagnostics = config.collect_diagnostics();
        assert!(diagnostics.iter().any(|diag| {
            diag.contains("ui.sound.path") && diag.contains("expected an mp3 file")
        }));
    }
}
