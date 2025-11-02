use std::path::{Path, PathBuf};

use anyhow::Result;
use tokio::process::Command;
use std::time::Duration;

use crate::config::AppConfig;
use crate::constants::CHANNEL_OPTIONS;
use crate::ffmpeg::{probe_duration, run_with_progress};
use crate::overlay::OverlayHandle;
use crate::progress::{format_stage_detail, Stage};

pub async fn mix_audio(
    config: &AppConfig,
    channels: &[String],
    overlay: &OverlayHandle,
) -> Result<PathBuf> {
    let mut selected: Vec<String> = CHANNEL_OPTIONS
        .iter()
        .filter_map(|candidate| {
            let candidate = *candidate;
            channels
                .iter()
                .any(|chosen| chosen.eq_ignore_ascii_case(candidate))
                .then(|| candidate.to_string())
        })
        .collect();

    if channels.is_empty() || selected.is_empty() {
        selected = CHANNEL_OPTIONS
            .iter()
            .map(|channel| channel.to_string())
            .collect();
    }

    let limiter_linear = linear_from_db(std::env::var("LIM_DB").ok().as_deref().unwrap_or("-1.0"));
    let weights = AudioWeights::from_env();

    let output_path = transformed_path(&config.source);
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        "-y",
        "-nostats",
        "-progress",
        "pipe:1",
        "-i",
        config.source.to_string_lossy().as_ref(),
        "-filter_complex",
        &build_filter_chain(&selected, limiter_linear, &weights),
        "-map",
        "0:v:0",
        "-map",
        "[aout]",
        "-c:v",
        "copy",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-ar",
        "48000",
        "-ac",
        "2",
        "-movflags",
        "+faststart",
        output_path.to_string_lossy().as_ref(),
    ]);

    let total_duration = match probe_duration(&config.source).await {
        Ok(seconds) if seconds.is_finite() && seconds > 0.0 => Some(Duration::from_secs_f64(seconds)),
        Ok(_) => {
            eprintln!("[CLIPS_APP] Ignoring non-finite duration for {}", config.source.display());
            None
        }
        Err(err) => {
            eprintln!("[CLIPS_APP] Failed to probe duration for {}: {err:?}", config.source.display());
            None
        }
    };

    let _ = overlay.update(Stage::Transform, 0.0, "Preparing ffmpeg…");

    run_with_progress(cmd, total_duration, |fraction| {
        let detail = format_stage_detail(Stage::Transform, fraction, "encoded");
        let _ = overlay.update(Stage::Transform, fraction, detail);
    })
    .await?;

    Ok(output_path)
}

fn transformed_path(source: &Path) -> PathBuf {
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
    let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
    source.with_file_name(format!("{stem}_transformed.{ext}"))
}

#[derive(Debug, Clone)]
struct AudioWeights {
    mic: String,
    discord: String,
    game: String,
}

impl AudioWeights {
    fn from_env() -> Self {
        Self {
            mic: std::env::var("MIC_W").unwrap_or_else(|_| "1.0".into()),
            discord: std::env::var("VC_W").unwrap_or_else(|_| "1.0".into()),
            game: std::env::var("GAME_W").unwrap_or_else(|_| "1.0".into()),
        }
    }

    fn weight_for(&self, channel: &str) -> &str {
        match channel {
            "voice" => &self.mic,
            "discord" => &self.discord,
            "game" => &self.game,
            _ => "1.0",
        }
    }
}

fn build_filter_chain(channels: &[String], limiter_linear: f64, weights: &AudioWeights) -> String {
    const AUDIO_FILTER: &str = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";
    // Mic is mono - convert to stereo by duplicating the channel to both L+R
    const MIC_FILTER: &str = "aformat=sample_fmts=fltp:sample_rates=48000,pan=stereo|FL<c0|FR<c0";

    let mut sections = Vec::new();
    let mut inputs = Vec::new();
    let mut mix_weights = Vec::new();

    for channel in channels {
        match channel.as_str() {
            "voice" => {
                sections.push(format!("[0:a:0]{MIC_FILTER}[mic]"));
                inputs.push("[mic]".to_string());
                mix_weights.push(weights.weight_for("voice").to_string());
            }
            "discord" => {
                sections.push(format!("[0:a:1]{AUDIO_FILTER}[vc]"));
                inputs.push("[vc]".to_string());
                mix_weights.push(weights.weight_for("discord").to_string());
            }
            "game" => {
                sections.push(format!("[0:a:2]{AUDIO_FILTER}[game]"));
                inputs.push("[game]".to_string());
                mix_weights.push(weights.weight_for("game").to_string());
            }
            _ => {}
        }
    }

    if inputs.is_empty() {
        sections.push(format!("[0:a:0]{AUDIO_FILTER}[mix]"));
        inputs.push("[mix]".to_string());
        mix_weights.push("1.0".into());
    }

    let limiter = format!("{:.6}", limiter_linear);

    if inputs.len() == 1 {
        let input = &inputs[0];
        sections.push(format!(
            "{input}alimiter=limit={limiter},{AUDIO_FILTER}[aout]",
            input = input,
            limiter = limiter,
        ));
    } else {
        let input_concat = inputs.join("");
        let weights_expr = mix_weights.join("|");
        sections.push(format!(
            "{inputs}amix=inputs={count}:duration=longest:normalize=1:weights={weights},alimiter=limit={limiter},{AUDIO_FILTER}[aout]",
            inputs = input_concat,
            count = inputs.len(),
            weights = weights_expr,
            limiter = limiter,
        ));
    }

    sections.join(";")
}

fn linear_from_db(db: &str) -> f64 {
    db.parse::<f64>()
        .map(|db| 10f64.powf(db / 20.0))
        .unwrap_or(1.0)
}
