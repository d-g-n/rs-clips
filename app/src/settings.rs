use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::capture::ReplaySettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayMode {
    Manual,
    AutoWithGame,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSettings {
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub target: String,
    pub audio_tracks: Vec<String>,
    pub replay_mode: ReplayMode,
    pub replay_enabled: bool,
}

impl PersistedSettings {
    pub fn load() -> Result<Option<Self>> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(None);
        }
        
        let contents = fs::read_to_string(&path)
            .context("failed to read settings file")?;
        let settings: PersistedSettings = serde_json::from_str(&contents)
            .context("failed to parse settings file")?;
        
        eprintln!("[CLIPS_APP] Loaded settings from {:?}", path);
        Ok(Some(settings))
    }
    
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .context("failed to create config directory")?;
        }
        
        let contents = serde_json::to_string_pretty(self)
            .context("failed to serialize settings")?;
        fs::write(&path, contents)
            .context("failed to write settings file")?;
        
        eprintln!("[CLIPS_APP] Saved settings to {:?}", path);
        Ok(())
    }
    
    pub fn from_replay_settings(settings: &ReplaySettings, replay_mode: ReplayMode, replay_enabled: bool) -> Self {
        Self {
            buffer_seconds: settings.buffer_seconds,
            bitrate: settings.bitrate,
            fps: settings.fps,
            target: settings.target.clone(),
            audio_tracks: settings.audio_tracks.clone(),
            replay_mode,
            replay_enabled,
        }
    }
    
    pub fn apply_to_replay_settings(&self, settings: &mut ReplaySettings) {
        settings.buffer_seconds = self.buffer_seconds;
        settings.bitrate = self.bitrate;
        settings.fps = self.fps;
        settings.target = self.target.clone();
        settings.audio_tracks = self.audio_tracks.clone();
    }
    
    fn config_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config/clips-app/settings.json"))
    }
}

