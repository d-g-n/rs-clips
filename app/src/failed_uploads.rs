use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedUpload {
    pub id: String,  // Unique identifier (timestamp-based)
    pub title: String,
    pub game: String,
    pub processed_path: PathBuf,
    pub full_path: PathBuf,
    pub timestamp: u64,  // Unix timestamp
}

impl FailedUpload {
    pub fn new(title: String, game: String, processed_path: PathBuf, full_path: PathBuf) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let id = format!("{}-{}", timestamp, title.chars().take(20).collect::<String>());
        
        Self {
            id,
            title,
            game,
            processed_path,
            full_path,
            timestamp,
        }
    }
    
    pub fn display_name(&self) -> String {
        if self.game.is_empty() {
            self.title.clone()
        } else {
            format!("{} [{}]", self.title, self.game)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FailedUploadsList {
    pub uploads: Vec<FailedUpload>,
}

impl FailedUploadsList {
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        
        let contents = fs::read_to_string(&path)
            .context("failed to read failed uploads file")?;
        let list: FailedUploadsList = serde_json::from_str(&contents)
            .context("failed to parse failed uploads file")?;
        
        eprintln!("[CLIPS_APP] Loaded {} failed uploads from {:?}", list.uploads.len(), path);
        Ok(list)
    }
    
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .context("failed to create config directory")?;
        }
        
        let contents = serde_json::to_string_pretty(self)
            .context("failed to serialize failed uploads")?;
        fs::write(&path, contents)
            .context("failed to write failed uploads file")?;
        
        eprintln!("[CLIPS_APP] Saved {} failed uploads to {:?}", self.uploads.len(), path);
        Ok(())
    }
    
    pub fn add(&mut self, upload: FailedUpload) {
        self.uploads.push(upload);
    }
    
    pub fn remove(&mut self, id: &str) -> Option<FailedUpload> {
        if let Some(pos) = self.uploads.iter().position(|u| u.id == id) {
            Some(self.uploads.remove(pos))
        } else {
            None
        }
    }
    
    pub fn get(&self, id: &str) -> Option<&FailedUpload> {
        self.uploads.iter().find(|u| u.id == id)
    }
    
    fn config_path() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".config/clips-app/failed-uploads.json"))
    }
}

