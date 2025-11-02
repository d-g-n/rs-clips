use std::path::PathBuf;

use anyhow::{anyhow, ensure, Context, Result};
use clap::{ArgAction, Parser};

use crate::capture::ReplayStorage;

#[derive(Parser, Debug)]
#[command(author, version, about = "All-in-one clips processing and replay control")]
pub struct Cli {
    /// Source media file to process (should live in the unprocessed directory)
    pub source: Option<PathBuf>,

    #[arg(long, value_name = "DIR")]
    pub unprocessed_dir: Option<PathBuf>,

    #[arg(long, value_name = "DIR")]
    pub processed_dir: Option<PathBuf>,

    #[arg(long = "youtubeuploader-bin", value_name = "CMD")]
    pub youtube_uploader: Option<PathBuf>,

    #[arg(long, value_name = "PATH")]
    pub secrets_path: Option<PathBuf>,

    #[arg(long = "overlay-bin", value_name = "CMD")]
    pub overlay_bin: PathBuf,

    #[arg(long = "capture-mode", default_value_t = false)]
    pub capture_mode: bool,

    #[arg(long = "gpu-screen-recorder-bin", value_name = "CMD")]
    pub gpu_screen_recorder: Option<PathBuf>,

    #[arg(long = "capture-target", default_value = "portal")]
    pub capture_target: String,

    #[arg(long = "capture-buffer-seconds", default_value_t = 300)]
    pub capture_buffer_seconds: u32,

    #[arg(long = "capture-bitrate", default_value_t = 60_000)]
    pub capture_bitrate: u32,

    #[arg(long = "capture-fps", default_value_t = 60)]
    pub capture_fps: u32,

    #[arg(long = "capture-audio", value_name = "TRACK", action = ArgAction::Append)]
    pub capture_audio_tracks: Vec<String>,

    #[arg(long = "capture-restore-portal", default_value_t = true)]
    pub capture_restore_portal: bool,

    #[arg(long = "capture-storage", default_value = "ram")]
    pub capture_storage: String,

    #[arg(long = "capture-hotkey", default_value = "Alt+X")]
    pub capture_hotkey: String,

    #[arg(long = "capture-auto-start", default_value_t = false)]
    pub capture_auto_start: bool,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub source: PathBuf,
    pub unprocessed_dir: PathBuf,
    pub processed_dir: PathBuf,
    pub youtube_uploader: PathBuf,
    pub secrets_path: PathBuf,
    pub overlay_bin: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub overlay_bin: PathBuf,
    pub gpu_screen_recorder: PathBuf,
    pub output_dir: PathBuf,
    pub processed_dir: PathBuf,
    pub youtube_uploader: PathBuf,
    pub secrets_path: PathBuf,
    pub target: String,
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub audio_tracks: Vec<String>,
    pub restore_portal_session: bool,
    pub replay_storage: ReplayStorage,
    pub hotkey: String,
    pub auto_start: bool,
}

#[derive(Debug, Clone)]
pub enum AppMode {
    Process(AppConfig),
    Capture(CaptureConfig),
}

impl Cli {
    pub fn into_mode(self) -> Result<AppMode> {
        if self.capture_mode {
            self.into_capture_mode()
        } else {
            self.into_process_mode()
        }
    }

    fn into_process_mode(self) -> Result<AppMode> {
        let source = self
            .source
            .ok_or_else(|| anyhow!("source file is required unless --capture-mode is set"))?;
        let unprocessed_dir = self
            .unprocessed_dir
            .ok_or_else(|| anyhow!("--unprocessed-dir must be provided"))?;
        let processed_dir = self
            .processed_dir
            .ok_or_else(|| anyhow!("--processed-dir must be provided"))?;
        let youtube_uploader = self
            .youtube_uploader
            .ok_or_else(|| anyhow!("--youtubeuploader-bin must be provided"))?;
        let secrets_path = self
            .secrets_path
            .ok_or_else(|| anyhow!("--secrets-path must be provided"))?;

        let overlay_bin = self
            .overlay_bin
            .canonicalize()
            .context("overlay binary missing")?;

        let config = AppConfig::new(
            source,
            unprocessed_dir,
            processed_dir,
            youtube_uploader,
            secrets_path,
            overlay_bin,
        )?;

        Ok(AppMode::Process(config))
    }

    fn into_capture_mode(self) -> Result<AppMode> {
        let gpu_screen_recorder = self
            .gpu_screen_recorder
            .ok_or_else(|| anyhow!("--gpu-screen-recorder-bin must be provided in capture mode"))?;
        let output_dir = self
            .unprocessed_dir
            .ok_or_else(|| anyhow!("--unprocessed-dir must be provided in capture mode"))?;
        let processed_dir = self
            .processed_dir
            .ok_or_else(|| anyhow!("--processed-dir must be provided in capture mode"))?;
        let youtube_uploader = self
            .youtube_uploader
            .ok_or_else(|| anyhow!("--youtubeuploader-bin must be provided in capture mode"))?;
        let secrets_path = self
            .secrets_path
            .ok_or_else(|| anyhow!("--secrets-path must be provided in capture mode"))?;

        let gpu_screen_recorder = gpu_screen_recorder
            .canonicalize()
            .context("gpu-screen-recorder binary missing")?;
        ensure!(
            gpu_screen_recorder.is_file(),
            "gpu-screen-recorder must be an executable file (got {:?})",
            gpu_screen_recorder
        );
        let overlay_bin = self
            .overlay_bin
            .canonicalize()
            .context("overlay binary missing")?;
        ensure!(
            overlay_bin.is_file(),
            "overlay binary must be an executable file (got {:?})",
            overlay_bin
        );
        let output_dir = output_dir
            .canonicalize()
            .context("capture output directory missing")?;
        let processed_dir = processed_dir
            .canonicalize()
            .context("processed directory missing")?;
        let youtube_uploader = youtube_uploader
            .canonicalize()
            .context("youtubeuploader binary missing")?;
        ensure!(
            youtube_uploader.is_file(),
            "youtubeuploader binary must be an executable file (got {:?})",
            youtube_uploader
        );
        let secrets_path = secrets_path
            .canonicalize()
            .context("youtubeuploader secrets file missing")?;

        let replay_storage = match self.capture_storage.to_ascii_lowercase().as_str() {
            "ram" => ReplayStorage::Ram,
            "disk" => ReplayStorage::Disk,
            other => anyhow::bail!(
                "unsupported replay storage '{}', expected 'ram' or 'disk'",
                other
            ),
        };

        let mut audio_tracks = self.capture_audio_tracks;
        if audio_tracks.is_empty() {
            audio_tracks = vec![
                "default_input".to_string(),
                "app:discord".to_string(),
                "app-inverse:discord".to_string(),
            ];
        }

        Ok(AppMode::Capture(CaptureConfig {
            overlay_bin,
            gpu_screen_recorder,
            output_dir,
            processed_dir,
            youtube_uploader,
            secrets_path,
            target: self.capture_target,
            buffer_seconds: self.capture_buffer_seconds,
            bitrate: self.capture_bitrate,
            fps: self.capture_fps,
            audio_tracks,
            restore_portal_session: self.capture_restore_portal,
            replay_storage,
            hotkey: self.capture_hotkey,
            auto_start: self.capture_auto_start,
        }))
    }
}

impl AppConfig {
    pub fn new(
        source: PathBuf,
        unprocessed_dir: PathBuf,
        processed_dir: PathBuf,
        youtube_uploader: PathBuf,
        secrets_path: PathBuf,
        overlay_bin: PathBuf,
    ) -> Result<Self> {
        let source = source.canonicalize().context("source file missing")?;
        let unprocessed_dir = unprocessed_dir
            .canonicalize()
            .context("unprocessed directory missing")?;
        let youtube_uploader = youtube_uploader
            .canonicalize()
            .context("youtubeuploader binary missing")?;
        let secrets_path = secrets_path
            .canonicalize()
            .context("youtubeuploader secrets file missing")?;
        let overlay_bin = overlay_bin
            .canonicalize()
            .context("overlay binary missing")?;

        ensure!(
            overlay_bin.is_file(),
            "overlay binary must be an executable file (got {:?})",
            overlay_bin
        );
        ensure!(
            youtube_uploader.is_file(),
            "youtubeuploader binary must be an executable file (got {:?})",
            youtube_uploader
        );

        Ok(Self {
            source,
            unprocessed_dir,
            processed_dir,
            youtube_uploader,
            secrets_path,
            overlay_bin,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        if !self.processed_dir.exists() {
            std::fs::create_dir_all(&self.processed_dir)
                .with_context(|| format!("Creating processed directory {:?}", self.processed_dir))?;
        }
        Ok(())
    }

    pub fn source_file_name(&self) -> String {
        self.source
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "<unknown>".into())
    }

    pub fn validate_source(&self) -> Result<()> {
        if self.source.parent() != Some(self.unprocessed_dir.as_path()) {
            anyhow::bail!(
                "Source {:?} not inside unprocessed directory {:?}",
                self.source,
                self.unprocessed_dir
            );
        }
        Ok(())
    }
}
