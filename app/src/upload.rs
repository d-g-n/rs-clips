use std::path::Path;
use std::path::PathBuf;

use anyhow::{Context, Result};
use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::overlay::OverlayHandle;
use crate::progress::{format_stage_detail, Stage};

fn config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    Ok(PathBuf::from(home).join(".config/clips-app/request.token"))
}

pub async fn upload_to_youtube(
    config: &AppConfig,
    processed_path: &Path,
    title_with_game: &str,
    game_label: &str,
    overlay: &OverlayHandle,
) -> Result<Option<String>> {
    let _ = overlay.update(Stage::Upload, 0.0, "Starting upload…");

    let mut cmd = Command::new(&config.youtube_uploader);
    cmd.args([
        "-filename",
        processed_path.to_string_lossy().as_ref(),
        "-title",
        title_with_game,
        "-privacy",
        "unlisted",
        "-description",
        &format!("Game: {game_label}"),
        "-secrets",
        config.secrets_path.to_string_lossy().as_ref(),
        "-cache",
        config_path()?.to_string_lossy().as_ref(),
    ]);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().context("Failed to spawn youtubeuploader")?;
    let stdout = child.stdout.take().expect("stdout captured");
    let stderr = child.stderr.take().expect("stderr captured");
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let stdout_task = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    };

    let stderr_task = {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
    };

    drop(tx);

    let percent_re = Regex::new(r"(\d{1,3}(?:\.\d+)?)%")?;
    let mut last_fraction = 0.0;
    let mut output = String::new();
    let mut last_detail = String::from("Starting upload…");

    while let Some(mut line) = rx.recv().await {
        if line.is_empty() {
            continue;
        }

        if line.ends_with('\r') {
            line.pop();
        }

        output.push_str(&line);
        output.push('\n');
        if let Some(caps) = percent_re.captures(&line) {
            if let Ok(pct) = caps[1].parse::<f32>() {
                let fraction = (pct / 100.0).clamp(0.0, 1.0);
                last_fraction = fraction;
                let detail = format_stage_detail(Stage::Upload, fraction, "uploaded");
                last_detail = detail.clone();
                let _ = overlay.update(Stage::Upload, fraction, detail);
            }
        } else {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                let detail = format!("Upload: {}", trimmed);
                last_detail = detail.clone();
                let _ = overlay.update(Stage::Upload, last_fraction, detail);
            }
        }
    }

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let status = child.wait().await?;
    if !status.success() {
        if let Some(failure_line) = output
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
        {
            let detail = format!("Upload failed: {}", failure_line);
            let _ = overlay.update(Stage::Upload, last_fraction, detail);
        } else {
            let _ = overlay.update(Stage::Upload, last_fraction, last_detail.clone());
        }
        anyhow::bail!("youtubeuploader failed: {output}");
    }

    let video_id = output.lines().rev().find_map(|line| {
        line.split_once("Video ID:")
            .map(|(_, id)| id.trim().to_string())
    });
    let _ = overlay.update(Stage::Upload, 1.0, "Upload complete");
    Ok(video_id)
}
