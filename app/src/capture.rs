use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use std::convert::TryFrom;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayStorage {
    Ram,
    Disk,
}

impl ReplayStorage {
    fn as_str(self) -> &'static str {
        match self {
            ReplayStorage::Ram => "ram",
            ReplayStorage::Disk => "disk",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySettings {
    #[serde(skip)]
    pub binary: PathBuf,
    pub target: String,
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub audio_tracks: Vec<String>,
    #[serde(skip)]
    pub restore_portal_session: bool,
    pub replay_storage: ReplayStorage,
    #[serde(skip)]
    pub output_dir: PathBuf,
}

impl ReplaySettings {
    pub fn sanitize(&mut self) {
        if self.buffer_seconds == 0 {
            self.buffer_seconds = 60;
        }
        if self.bitrate == 0 {
            self.bitrate = 60_000;
        }
        if self.fps == 0 {
            self.fps = 60;
        }
        if self.audio_tracks.is_empty() {
            self.audio_tracks.push("default_input".to_string());
        }
        self.audio_tracks
            .retain(|track| !track.trim().is_empty());
    }
}

#[derive(Debug, Clone)]
pub struct ReplayStatus {
    pub running: bool,
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub target: String,
    pub audio_tracks: Vec<String>,
    pub last_saved: Option<PathBuf>,
    pub message: Option<String>,
}

pub struct ReplayController {
    settings: ReplaySettings,
    child: Option<Child>,
    last_saved: Option<PathBuf>,
    last_message: Option<String>,
}

impl ReplayController {
    pub fn new(settings: ReplaySettings) -> Self {
        Self {
            settings,
            child: None,
            last_saved: None,
            last_message: None,
        }
    }

    fn current_pid(&self) -> Option<Pid> {
        self.child
            .as_ref()
            .and_then(|child| child.id())
            .map(|pid| Pid::from_raw(pid as i32))
    }

    fn build_command(&self) -> Command {
        let mut cmd = Command::new(&self.settings.binary);
        cmd.kill_on_drop(true);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        cmd.arg("-w").arg(&self.settings.target);
        cmd.arg("-c").arg("mp4");
        cmd.arg("-f").arg(self.settings.fps.to_string());
        cmd.arg("-bm").arg("cbr");
        cmd.arg("-q").arg(self.settings.bitrate.to_string());
        cmd.arg("-r").arg(self.settings.buffer_seconds.to_string());
        cmd.arg("-o").arg(self.settings.output_dir.to_string_lossy().to_string());
        cmd.arg("-ro").arg(self.settings.output_dir.to_string_lossy().to_string());
        cmd.arg("-replay-storage").arg(self.settings.replay_storage.as_str());
        cmd.arg("-v").arg("no");

        if self.settings.restore_portal_session {
            cmd.arg("-restore-portal-session").arg("yes");
        }

        for track in &self.settings.audio_tracks {
            if !track.trim().is_empty() {
                cmd.arg("-a").arg(track);
            }
        }

        if let Ok(val) = std::env::var("WAYLAND_DISPLAY") {
            cmd.env("WAYLAND_DISPLAY", val);
        }
        if let Ok(val) = std::env::var("DISPLAY") {
            cmd.env("DISPLAY", val);
        }
        if let Ok(val) = std::env::var("XDG_RUNTIME_DIR") {
            cmd.env("XDG_RUNTIME_DIR", val);
        }

        cmd
    }

    fn take_child_streams(child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    eprintln!("[GPU-SR][stdout] {}", line);
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    eprintln!("[GPU-SR][stderr] {}", line);
                }
            });
        }
    }

    fn refresh_child(&mut self) -> Result<()> {
        if let Some(child) = self.child.as_mut() {
            if let Some(status) = child.try_wait()? {
                self.child = None;
                self.last_message = Some(format!("Replay recorder exited: {:?}", status));
            }
        }
        Ok(())
    }

    pub async fn ensure_running(&mut self) -> Result<()> {
        self.refresh_child()?;
        if self.child.is_some() {
            return Ok(());
        }

        if !self.settings.output_dir.exists() {
            std::fs::create_dir_all(&self.settings.output_dir)
                .with_context(|| format!(
                    "creating replay output directory {:?}",
                    self.settings.output_dir
                ))?;
        }

        let mut settings = self.settings.clone();
        settings.sanitize();
        self.settings = settings;

        let mut cmd = self.build_command();
        eprintln!("[CLIPS_APP] Spawning gpu-screen-recorder: {:?}", cmd);
        let mut child = cmd.spawn().context("failed to spawn gpu-screen-recorder")?;
        Self::take_child_streams(&mut child);
        self.child = Some(child);
        self.last_message = Some("Replay recorder started".to_string());
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if self.child.is_none() {
            return Ok(());
        }

        if let Some(pid) = self.current_pid() {
            if let Ok(signal) = Signal::try_from(libc::SIGINT) {
                let _ = signal::kill(pid, signal);
            }
        }

        if let Some(mut child) = self.child.take() {
            let _ = child.wait().await;
        }
        self.last_message = Some("Replay recorder stopped".to_string());
        Ok(())
    }

    pub async fn apply_settings(&mut self, settings: ReplaySettings) -> Result<()> {
        let mut new_settings = settings;
        new_settings.sanitize();
        self.settings = new_settings;
        if self.child.is_some() {
            self.stop().await?;
            self.ensure_running().await?;
        }
        Ok(())
    }

    pub async fn save_recent(&mut self, duration_secs: Option<u32>) -> Result<Option<PathBuf>> {
        self.refresh_child()?;
        let pid = self
            .current_pid()
            .context("replay recorder is not running")?;

        let base_rtmin = libc::SIGRTMIN();
        let raw_signal = match duration_secs {
            Some(10) => base_rtmin + 1,
            Some(30) => base_rtmin + 2,
            Some(60) => base_rtmin + 3,
            Some(300) => base_rtmin + 4,
            Some(600) => base_rtmin + 5,
            Some(1800) => base_rtmin + 6,
            None => libc::SIGUSR1,
            Some(other) => {
                eprintln!("[CLIPS_APP] Unsupported save duration {}s, using full buffer", other);
                libc::SIGUSR1
            }
        };

        // Use libc::kill directly for real-time signals since nix::Signal doesn't support them
        let result = unsafe { libc::kill(pid.as_raw(), raw_signal) };
        if result != 0 {
            return Err(anyhow!("failed to signal gpu-screen-recorder: {}", 
                std::io::Error::last_os_error()));
        }

        // Give encoder a moment to flush file before we look for it.
        sleep(Duration::from_secs(2)).await;
        self.last_saved = find_latest_file(&self.settings.output_dir)?;
        if self.last_saved.is_some() {
            self.last_message = Some("Replay saved".to_string());
        }
        Ok(self.last_saved.clone())
    }

    pub fn status(&mut self) -> Result<ReplayStatus> {
        self.refresh_child()?;
        Ok(ReplayStatus {
            running: self.child.is_some(),
            buffer_seconds: self.settings.buffer_seconds,
            bitrate: self.settings.bitrate,
            fps: self.settings.fps,
            target: self.settings.target.clone(),
            audio_tracks: self.settings.audio_tracks.clone(),
            last_saved: self.last_saved.clone(),
            message: self.last_message.clone(),
        })
    }

    pub fn settings(&self) -> &ReplaySettings {
        &self.settings
    }

    pub fn set_message<S>(&mut self, message: S)
    where
        S: Into<String>,
    {
        self.last_message = Some(message.into());
    }

    pub fn clear_message(&mut self) {
        self.last_message = None;
    }

    pub fn clear_last_saved(&mut self) {
        self.last_saved = None;
    }
}

fn find_latest_file(dir: &Path) -> Result<Option<PathBuf>> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading replay output directory {:?}", dir))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let metadata = entry
            .metadata()
            .with_context(|| format!("metadata for {:?}", path))?;
        if let Ok(modified) = metadata.modified() {
            match newest {
                Some((current_time, _)) if current_time >= modified => {}
                _ => {
                    newest = Some((modified, path));
                }
            }
        }
    }
    Ok(newest.map(|(_, path)| path))
}
