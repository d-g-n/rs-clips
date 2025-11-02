use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

pub async fn run_with_progress<F>(
    mut command: Command,
    total_duration: Option<Duration>,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(f32),
{
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command.spawn().context("Failed to spawn ffmpeg command")?;
    let stdout = child
        .stdout
        .take()
        .context("ffmpeg stdout not captured (progress output)")?;
    let mut reader = BufReader::new(stdout).lines();

    let mut stats = BTreeMap::new();
    let total_micros = total_duration.map(|dur| dur.as_micros() as f64);

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim().to_string();
        let value = parts.next().unwrap_or("").trim().to_string();
        if key.is_empty() {
            continue;
        }
        stats.insert(key.clone(), value.clone());

        if key == "progress" {
            let mut fraction = 0.0;
            if let (Some(total), Some(out_time)) =
                (total_micros, stats.get("out_time_ms").and_then(|v| v.parse::<f64>().ok()))
            {
                fraction = (out_time / 1_000_000.0 / (total / 1_000_000.0))
                    .clamp(0.0, 1.0);
            }

            if value == "end" {
                callback(1.0);
                break;
            } else {
                callback(fraction as f32);
                stats.clear();
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        let stderr = if let Some(stderr) = child.stderr {
            use tokio::io::AsyncReadExt;
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            let _ = reader.read_to_string(&mut buf).await;
            buf
        } else {
            String::new()
        };
        anyhow::bail!("ffmpeg exited with status {status:?}: {stderr}");
    }
    Ok(())
}

pub async fn probe_duration(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path.to_str().context("path is not valid UTF-8")?,
        ])
        .output()
        .await
        .context("Failed to run ffprobe")?;

    if !output.status.success() {
        anyhow::bail!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let duration_str = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 from ffprobe")?
        .trim()
        .to_string();
    
    duration_str
        .parse::<f64>()
        .context("Failed to parse duration")
}

pub async fn trim_video<F>(
    input: &Path,
    output: &Path,
    start_time: f64,
    end_time: f64,
    on_progress: F,
) -> Result<()>
where
    F: FnMut(f32),
{
    let duration = end_time - start_time;
    if !duration.is_finite() || duration <= 0.0 {
        bail!("Trim duration must be positive");
    }

    let input_str = input
        .to_str()
        .context("input path is not valid UTF-8")?;
    let output_str = output
        .to_str()
        .context("output path is not valid UTF-8")?;

    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        "-y",
        "-nostats",
        "-progress",
        "pipe:1",
        "-ss",
        &start_time.to_string(),
        "-i",
        input_str,
        "-t",
        &duration.to_string(),
        "-c",
        "copy",
        "-avoid_negative_ts",
        "make_zero",
        output_str,
    ]);

    run_with_progress(
        command,
        Some(Duration::from_secs_f64(duration)),
        on_progress,
    )
    .await
}
