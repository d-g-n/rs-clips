use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{bail, ensure, Context, Result};
use crate::progress::Stage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum OverlayCommand {
    #[serde(rename = "progress")]
    Progress {
        stage: String,
        fraction: f32,
        detail: String,
    },
    #[serde(rename = "show_picker")]
    ShowPicker {
        preview_path: Option<String>,
        default_title: String,
        default_game: String,
        available_channels: Vec<String>,
    },
    #[serde(rename = "show_trimmer")]
    ShowTrimmer {
        video_path: String,
        duration: f64,
    },
    #[serde(rename = "show_capture")]
    ShowCapture {
        status: CaptureStatusPayload,
    },
    #[serde(rename = "capture_status")]
    CaptureStatus {
        status: CaptureStatusPayload,
    },
    #[serde(rename = "set_visibility")]
    SetVisibility {
        visible: bool,
    },
    #[serde(rename = "quit")]
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OverlayResponse {
    #[serde(rename = "picker_result")]
    PickerResult {
        title: String,
        game: String,
        action: String,
        channels: Vec<String>,
    },
    #[serde(rename = "trimmer_result")]
    TrimmerResult {
        start_time: f64,
        end_time: f64,
    },
    #[serde(rename = "capture_action")]
    CaptureAction {
        action: CaptureActionPayload,
    },
    #[serde(rename = "cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedUploadEntry {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStatusPayload {
    pub running: bool,
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub target: String,
    pub audio_tracks: Vec<String>,
    pub last_saved: Option<String>,
    pub hotkey: String,
    pub message: Option<String>,
    pub is_saving: bool,
    pub failed_uploads: Vec<FailedUploadEntry>,
    pub replay_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureSettingsPayload {
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub target: String,
    pub audio_tracks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CaptureActionPayload {
    Toggle { enable: bool },
    Save { duration_secs: u32 },
    UpdateSettings { settings: CaptureSettingsPayload },
    UpdateMode { mode: String },
    FailedUpload { upload_action: String, id: String },
}

#[derive(Clone)]
pub struct OverlayHandle {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    stdout: Arc<Mutex<Option<BufReader<ChildStdout>>>>,
}

impl OverlayHandle {
    fn send_command(&self, command: &OverlayCommand) -> Result<()> {
        let json = serde_json::to_string(command)?;
        let mut stdin_guard = self.stdin.lock()
            .map_err(|e| anyhow::anyhow!("stdin lock poisoned: {}", e))?;
        let stdin = stdin_guard
            .as_mut()
            .context("overlay stdin is no longer available")?;
        eprintln!("[CLIPS_APP] Sending command: {}", json);
        writeln!(stdin, "{}", json)?;
        stdin.flush()?;
        Ok(())
    }

    fn recv_response(&self) -> Result<Option<OverlayResponse>> {
        let mut line = String::new();
        let mut stdout_guard = self.stdout.lock()
            .map_err(|e| anyhow::anyhow!("stdout lock poisoned: {}", e))?;
        let reader = stdout_guard
            .as_mut()
            .context("overlay stdout is no longer available")?;
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        eprintln!("[CLIPS_APP] Received response: {}", line.trim_end());
        let response = serde_json::from_str(&line)
            .context("overlay returned invalid response")?;
        Ok(Some(response))
    }

    pub fn update<S>(&self, stage: Stage, fraction: f32, detail: S) -> Result<()>
    where
        S: Into<String>,
    {
        self.send_command(&OverlayCommand::Progress {
            stage: stage.label().to_string(),
            fraction,
            detail: detail.into(),
        })
    }

    pub fn show_picker(
        &self,
        preview_path: Option<&std::path::Path>,
        default_title: &str,
        default_game: &str,
        available_channels: &[String],
    ) -> Result<Option<PickerResult>> {
        let cmd = OverlayCommand::ShowPicker {
            preview_path: preview_path.map(|p| p.to_string_lossy().to_string()),
            default_title: default_title.to_string(),
            default_game: default_game.to_string(),
            available_channels: available_channels.to_vec(),
        };

        self.send_command(&cmd)?;
        eprintln!("[CLIPS_APP] Waiting for picker response...");
        let response = match self.recv_response()? {
            Some(resp) => resp,
            None => bail!("overlay closed while waiting for picker response"),
        };

        match response {
            OverlayResponse::PickerResult {
                title,
                game,
                action,
                channels,
            } => {
                let action = match action.as_str() {
                    "upload" => ActionChoice::Upload,
                    "move" => ActionChoice::Move,
                    "discard" => ActionChoice::Discard,
                    other => bail!("overlay returned unknown picker action: {other}"),
                };
                Ok(Some(PickerResult {
                    title,
                    game,
                    action,
                    channels,
                }))
            }
            OverlayResponse::Cancelled => Ok(None),
            other => bail!("overlay returned unexpected picker response: {:?}", other),
        }
    }

    pub fn show_trimmer(
        &self,
        video_path: &std::path::Path,
        duration: f64,
    ) -> Result<Option<TrimmerResult>> {
        let cmd = OverlayCommand::ShowTrimmer {
            video_path: video_path.to_string_lossy().to_string(),
            duration,
        };

        self.send_command(&cmd)?;
        eprintln!("[CLIPS_APP] Waiting for trimmer response...");
        let response = match self.recv_response()? {
            Some(resp) => resp,
            None => bail!("overlay closed while waiting for trimmer response"),
        };

        match response {
            OverlayResponse::TrimmerResult { start_time, end_time } => Ok(Some(TrimmerResult {
                start_time,
                end_time,
            })),
            OverlayResponse::Cancelled => Ok(None),
            other => bail!("overlay returned unexpected trimmer response: {:?}", other),
        }
    }

    pub fn show_capture(&self, status: CaptureStatusPayload) -> Result<CaptureSession> {
        self.send_command(&OverlayCommand::ShowCapture { status })?;
        Ok(CaptureSession {
            handle: self.clone(),
        })
    }

    pub fn send_capture_status(&self, status: CaptureStatusPayload) -> Result<()> {
        self.send_command(&OverlayCommand::CaptureStatus { status })
    }

    pub fn set_visibility(&self, visible: bool) -> Result<()> {
        self.send_command(&OverlayCommand::SetVisibility { visible })
    }

    pub fn try_recv(&self) -> Result<Option<OverlayResponse>> {
        self.recv_response()
    }
}

pub struct Overlay {
    process: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    stdout: Arc<Mutex<Option<BufReader<ChildStdout>>>>,
}

#[derive(Clone)]
pub struct CaptureSession {
    handle: OverlayHandle,
}

impl CaptureSession {
    pub fn wait_for_action(&self) -> Result<Option<CaptureActionPayload>> {
        loop {
            match self.handle.try_recv()? {
                Some(OverlayResponse::CaptureAction { action }) => return Ok(Some(action)),
                Some(OverlayResponse::Cancelled) => return Ok(None),
                Some(other) => {
                    eprintln!("[CLIPS_APP] Ignoring unexpected overlay response: {:?}", other);
                    continue;
                }
                None => return Ok(None),
            }
        }
    }

    pub fn update_status(&self, status: CaptureStatusPayload) -> Result<()> {
        self.handle.send_capture_status(status)
    }
}

impl Overlay {
    pub fn spawn(bin: &std::path::Path, initial_message: &str) -> Result<Self> {
        ensure!(
            bin.is_file(),
            "overlay binary {:?} does not exist or is not a file",
            bin
        );

        eprintln!("[CLIPS_APP] Spawning overlay: {:?}", bin);
        
        // Ensure the overlay inherits the Wayland/X11 display environment
        let mut cmd = Command::new(bin);
        cmd.arg(initial_message)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        
        // Explicitly pass through display environment variables
        if let Ok(wayland_display) = std::env::var("WAYLAND_DISPLAY") {
            eprintln!("[CLIPS_APP] Setting WAYLAND_DISPLAY={}", wayland_display);
            cmd.env("WAYLAND_DISPLAY", wayland_display);
        }
        if let Ok(display) = std::env::var("DISPLAY") {
            eprintln!("[CLIPS_APP] Setting DISPLAY={}", display);
            cmd.env("DISPLAY", display);
        }
        if let Ok(xdg_runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            eprintln!("[CLIPS_APP] Setting XDG_RUNTIME_DIR={}", xdg_runtime_dir);
            cmd.env("XDG_RUNTIME_DIR", xdg_runtime_dir);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn overlay binary at {:?}", bin))?;

        eprintln!("[CLIPS_APP] Overlay spawned with PID: {:?}", child.id());
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);

        Ok(Self { 
            process: Arc::new(Mutex::new(Some(child))),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(stdout)),
        })
    }

    pub fn handle(&self) -> OverlayHandle {
        OverlayHandle {
            stdin: self.stdin.clone(),
            stdout: self.stdout.clone(),
        }
    }

    pub fn close(self) -> Result<()> {
        // Send quit command
        if let Ok(mut stdin_guard) = self.stdin.lock() {
            if let Some(ref mut stdin) = *stdin_guard {
                let cmd = OverlayCommand::Quit;
                if let Ok(json) = serde_json::to_string(&cmd) {
                    let _ = writeln!(stdin, "{}", json);
                    let _ = stdin.flush();
                }
            }
        }
        
        if let Ok(mut process_guard) = self.process.lock() {
            if let Some(mut child) = process_guard.take() {
                let _ = child.wait();
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionChoice {
    Upload,
    Move,
    Discard,
}

#[derive(Debug, Clone)]
pub struct PickerResult {
    pub title: String,
    pub game: String,
    pub action: ActionChoice,
    pub channels: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TrimmerResult {
    pub start_time: f64,
    pub end_time: f64,
}
