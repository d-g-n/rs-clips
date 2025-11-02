use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::task::JoinHandle;

use evdev::{Device, InputEventKind, Key};
use clips_app::capture::{ReplayController, ReplaySettings};
use clips_app::config::{AppConfig, AppMode, CaptureConfig, Cli};
use clips_app::constants::CHANNEL_OPTIONS;
use clips_app::ffmpeg;
use clips_app::overlay;
use clips_app::overlay::{CaptureActionPayload, CaptureStatusPayload};
use clips_app::process;
use clips_app::progress::{format_stage_detail, Stage};
use clips_app::settings::{PersistedSettings, ReplayMode};
use clips_app::upload;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.into_mode()? {
        AppMode::Capture(cfg) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("Failed to build tokio runtime")?;
            runtime.block_on(run_capture_mode(cfg))
        }
        AppMode::Process(config) => run_process_mode(config),
    }
}

fn run_process_mode(config: AppConfig) -> Result<()> {
    let overlay = overlay::Overlay::spawn(&config.overlay_bin, "Work in progress")?;
    let overlay_handle = overlay.handle();
    overlay_handle.set_visibility(true)?;

    config.validate_source()?;
    config.ensure_dirs()?;

    // Load failed uploads list for process mode too
    let mut failed_uploads_list = clips_app::failed_uploads::FailedUploadsList::load()
        .unwrap_or_default();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?;

    runtime.block_on(process_clip(&config, &overlay_handle, &mut failed_uploads_list))?;

    overlay.close()?;
    Ok(())
}

async fn run_capture_mode(cfg: CaptureConfig) -> Result<()> {
    let overlay = overlay::Overlay::spawn(&cfg.overlay_bin, "Replay control")?;
    let overlay_handle = overlay.handle();
    overlay_handle.set_visibility(false)?;

    // Load saved settings or use defaults from config
    let mut settings = ReplaySettings {
        binary: cfg.gpu_screen_recorder.clone(),
        target: cfg.target.clone(),
        buffer_seconds: cfg.buffer_seconds,
        bitrate: cfg.bitrate,
        fps: cfg.fps,
        audio_tracks: cfg.audio_tracks.clone(),
        restore_portal_session: cfg.restore_portal_session,
        replay_storage: cfg.replay_storage,
        output_dir: cfg.output_dir.clone(),
    };

    // Load replay mode and enabled state from persisted settings
    let mut replay_mode = ReplayMode::Manual;
    let mut should_start = cfg.auto_start;
    
    // Apply persisted settings if they exist
    if let Ok(Some(persisted)) = PersistedSettings::load() {
        eprintln!("[CLIPS_APP] Restoring saved settings");
        persisted.apply_to_replay_settings(&mut settings);
        replay_mode = persisted.replay_mode;
        
        // In manual mode, respect the persisted enabled state
        if replay_mode == ReplayMode::Manual {
            should_start = persisted.replay_enabled;
        }
    }
    
    // Load failed uploads list
    let mut failed_uploads_list = clips_app::failed_uploads::FailedUploadsList::load()
        .unwrap_or_default();
    eprintln!("[CLIPS_APP] Loaded {} failed uploads", failed_uploads_list.uploads.len());

    let mut controller = ReplayController::new(settings);

    // In manual mode, start based on persisted/config state
    // In auto mode, we'll handle it in the game detection loop
    if replay_mode == ReplayMode::Manual && should_start {
        if let Err(err) = controller.ensure_running().await {
            controller.set_message(format!("Failed to start replay: {err:#}"));
        }
    }
    
    let replay_mode = Arc::new(std::sync::Mutex::new(replay_mode));

    let visible = Arc::new(AtomicBool::new(false));

    let overlay_for_hotkey = overlay_handle.clone();
    let visible_for_hotkey = visible.clone();
    let hotkey = cfg.hotkey.clone();
    let hotkey_task = spawn_hotkey_listener(hotkey.clone(), overlay_for_hotkey, visible_for_hotkey).await;

    if hotkey_task.is_none() {
        eprintln!("[CLIPS_APP] Global shortcut portal unavailable, showing overlay by default");
        set_overlay_visible(&overlay_handle, &visible, true)?;
    }

    // Game detection is handled inside the capture loop
    loop {
        // Ensure overlay reflects latest status when opened
        let mode = *replay_mode.lock().unwrap();
        let status = build_capture_status(&controller.status()?, &cfg.hotkey, &failed_uploads_list, mode);
        let session = overlay_handle
            .show_capture(status)
            .context("failed to show capture panel")?;

        let outcome = run_capture_loop(&cfg, &mut controller, session.clone(), &overlay_handle, &visible, &mut failed_uploads_list, &replay_mode).await?;

        match outcome {
            CaptureLoopOutcome::Saved(path) => {
                let app_config = AppConfig::new(
                    path,
                    cfg.output_dir.clone(),
                    cfg.processed_dir.clone(),
                    cfg.youtube_uploader.clone(),
                    cfg.secrets_path.clone(),
                    cfg.overlay_bin.clone(),
                )?;
                app_config.ensure_dirs()?;
                set_overlay_visible(&overlay_handle, &visible, true)?;
                match process_clip(&app_config, &overlay_handle, &mut failed_uploads_list).await {
                    Ok(_) => {},
                    Err(err) => {
                        // Show error for 5 seconds before returning to capture view
                        eprintln!("[CLIPS_APP] Clip processing error: {err:#}");
                        let error_msg = format!("Error: {}", err);
                        let _ = overlay_handle.update(Stage::Done, 1.0, &error_msg);
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
                // After clip processing (cancelled or completed), hide overlay and sync state
                set_overlay_visible(&overlay_handle, &visible, false)?;
                controller.clear_last_saved();
                controller.set_message("Replay recorder ready");
            }
            CaptureLoopOutcome::Exit => {
                break;
            }
        }

        // After processing, refresh capture view status before waiting for next loop
        let mode = *replay_mode.lock().unwrap();
        overlay_handle.send_capture_status(build_capture_status(&controller.status()?, &cfg.hotkey, &failed_uploads_list, mode))?;
    }

    if let Some(handle) = hotkey_task {
        handle.abort();
    }

    if let Err(err) = set_overlay_visible(&overlay_handle, &visible, false) {
        eprintln!("[CLIPS_APP] Failed to hide overlay on shutdown: {err:#}");
    }

    overlay.close()?;
    Ok(())
}

enum CaptureLoopOutcome {
    Saved(PathBuf),
    Exit,
}


async fn run_capture_loop(
    cfg: &CaptureConfig,
    controller: &mut ReplayController,
    session: overlay::CaptureSession,
    overlay_handle: &overlay::OverlayHandle,
    visible: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    failed_uploads_list: &mut clips_app::failed_uploads::FailedUploadsList,
    replay_mode: &Arc<std::sync::Mutex<ReplayMode>>,
) -> Result<CaptureLoopOutcome> {
    fn spawn_action_task(
        session: overlay::CaptureSession,
    ) -> JoinHandle<Result<Option<CaptureActionPayload>>> {
        tokio::task::spawn_blocking(move || session.wait_for_action())
    }

    let mut action_task = spawn_action_task(session.clone());

    let mut detection_interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
    detection_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut game_was_running = false;

    {
        let mode = *replay_mode.lock().unwrap();
        if mode == ReplayMode::AutoWithGame
            && maybe_handle_game_detection(controller, &mut game_was_running).await? {
                let status = build_capture_status(
                    &controller.status()?,
                    &cfg.hotkey,
                    failed_uploads_list,
                    mode,
                );
                session
                    .update_status(status)
                    .context("failed to update capture status")?;
            }
    }

    let mut outcome: Option<CaptureLoopOutcome> = None;

    while outcome.is_none() {
        tokio::select! {
            action = &mut action_task => {
                let action = action.context("failed to join capture action task")??;

                match action {
                    None => {
                        outcome = Some(CaptureLoopOutcome::Exit);
                    }
                    Some(payload) => {
                        match payload {
                            CaptureActionPayload::Toggle { enable } => {
                                let mode = *replay_mode.lock().unwrap();
                                if mode == ReplayMode::Manual {
                                    if enable {
                                        match controller.ensure_running().await {
                                            Ok(_) => controller.set_message("Replay recorder started"),
                                            Err(err) => {
                                                eprintln!("[CLIPS_APP] Failed to enable replay: {err:#}");
                                                controller.set_message(format!("Failed to enable replay: {err:#}"));
                                            }
                                        }
                                    } else {
                                        match controller.stop().await {
                                            Ok(_) => controller.set_message("Replay recorder stopped"),
                                            Err(err) => {
                                                eprintln!("[CLIPS_APP] Failed to stop replay: {err:#}");
                                                controller.set_message(format!("Failed to stop replay: {err:#}"));
                                            }
                                        }
                                    }

                                    let persisted = PersistedSettings::from_replay_settings(
                                        controller.settings(),
                                        mode,
                                        enable,
                                    );
                                    if let Err(err) = persisted.save() {
                                        eprintln!("[CLIPS_APP] Failed to save settings: {err:#}");
                                    }
                                } else {
                                    controller.set_message("Toggle disabled in auto mode");
                                }
                            }
                            CaptureActionPayload::Save { duration_secs } => {
                                controller.set_message("Saving replay clip...");
                                let mode = *replay_mode.lock().unwrap();
                                let mut status = build_capture_status(
                                    &controller.status()?,
                                    &cfg.hotkey,
                                    failed_uploads_list,
                                    mode,
                                );
                                status.is_saving = true;
                                session
                                    .update_status(status)
                                    .context("failed to update saving status")?;

                                let duration = if duration_secs == 0 {
                                    None
                                } else {
                                    Some(duration_secs)
                                };
                                let save_result = controller.save_recent(duration).await;

                                let mut status = build_capture_status(
                                    &controller.status()?,
                                    &cfg.hotkey,
                                    failed_uploads_list,
                                    mode,
                                );
                                status.is_saving = false;

                                match save_result {
                                    Ok(Some(path)) => {
                                        set_overlay_visible(overlay_handle, visible, true)?;
                                        controller.set_message("Processing clip…");
                                        session
                                            .update_status(status)
                                            .context("failed to update capture status")?;
                                        outcome = Some(CaptureLoopOutcome::Saved(path));
                                    }
                                    Ok(None) => {
                                        controller.set_message("No new replay file generated");
                                        session
                                            .update_status(status)
                                            .context("failed to update capture status")?;
                                    }
                                    Err(err) => {
                                        controller.set_message(format!("Save failed: {err:#}"));
                                        session
                                            .update_status(status)
                                            .context("failed to update capture status")?;
                                    }
                                }
                            }
                            CaptureActionPayload::UpdateSettings { settings } => {
                                let mut new_settings = controller.settings().clone();
                                if !settings.target.trim().is_empty() {
                                    new_settings.target = settings.target.clone();
                                }
                                new_settings.buffer_seconds = settings.buffer_seconds;
                                new_settings.bitrate = settings.bitrate;
                                new_settings.fps = settings.fps;
                                if !settings.audio_tracks.is_empty() {
                                    new_settings.audio_tracks = settings.audio_tracks.clone();
                                }

                                if let Err(err) = controller.apply_settings(new_settings.clone()).await {
                                    controller.set_message(format!("Apply failed: {err:#}"));
                                } else {
                                    controller.set_message("Settings updated");
                                    let mode = *replay_mode.lock().unwrap();
                                    let is_running = controller.status().map(|s| s.running).unwrap_or(false);
                                    let persisted = PersistedSettings::from_replay_settings(
                                        &new_settings,
                                        mode,
                                        is_running,
                                    );
                                    if let Err(err) = persisted.save() {
                                        eprintln!("[CLIPS_APP] Failed to save settings: {err:#}");
                                    }
                                }
                            }
                            CaptureActionPayload::UpdateMode { mode: mode_str } => {
                                let new_mode = match mode_str.as_str() {
                                    "auto" => ReplayMode::AutoWithGame,
                                    _ => ReplayMode::Manual,
                                };

                                let old_mode = {
                                    let mut mode_guard = replay_mode.lock().unwrap();
                                    let old = *mode_guard;
                                    *mode_guard = new_mode;
                                    old
                                };

                                if new_mode != old_mode {
                                    match new_mode {
                                        ReplayMode::Manual => {
                                            if let Err(err) = controller.stop().await {
                                                eprintln!("[CLIPS_APP] Failed to stop replay: {err:#}");
                                                controller.set_message(format!("Failed to stop replay: {err:#}"));
                                            } else {
                                                controller.set_message("Manual mode enabled");
                                            }
                                            game_was_running = false;
                                        }
                                        ReplayMode::AutoWithGame => {
                                            controller.set_message("Auto mode enabled");
                                            game_was_running = false;
                                            if maybe_handle_game_detection(controller, &mut game_was_running).await? {
                                                // helper already adjusted state
                                            }
                                        }
                                    }

                                    let is_running = controller.status().map(|s| s.running).unwrap_or(false);
                                    let persisted = PersistedSettings::from_replay_settings(
                                        controller.settings(),
                                        new_mode,
                                        is_running,
                                    );
                                    if let Err(err) = persisted.save() {
                                        eprintln!("[CLIPS_APP] Failed to save settings: {err:#}");
                                    }
                                }
                            }
                            CaptureActionPayload::FailedUpload { upload_action, id } => {
                                match upload_action.as_str() {
                                    "retry" => {
                                        if let Some(failed_upload) = failed_uploads_list.get(&id) {
                                            eprintln!("[CLIPS_APP] Retrying upload for: {}", failed_upload.display_name());
                                            controller.set_message("Retrying upload...");

                                            let result = upload::upload_to_youtube(
                                                &AppConfig {
                                                    source: failed_upload.processed_path.clone(),
                                                    unprocessed_dir: cfg.output_dir.clone(),
                                                    processed_dir: cfg.processed_dir.clone(),
                                                    youtube_uploader: cfg.youtube_uploader.clone(),
                                                    secrets_path: cfg.secrets_path.clone(),
                                                    overlay_bin: cfg.overlay_bin.clone(),
                                                },
                                                &failed_upload.processed_path,
                                                &failed_upload.title,
                                                &failed_upload.game,
                                                overlay_handle,
                                            ).await;

                                            match result {
                                                Ok(Some(video_id)) => {
                                                    eprintln!("[CLIPS_APP] Retry successful, video ID: {}", video_id);
                                                    let title_with_game = if failed_upload.game.is_empty() {
                                                        failed_upload.title.clone()
                                                    } else {
                                                        format!("{} [{}]", failed_upload.title, failed_upload.game)
                                                    };

                                                    if let Err(err) = handle_upload_action(
                                                        &AppConfig {
                                                            source: failed_upload.processed_path.clone(),
                                                            unprocessed_dir: cfg.output_dir.clone(),
                                                            processed_dir: cfg.processed_dir.clone(),
                                                            youtube_uploader: cfg.youtube_uploader.clone(),
                                                            secrets_path: cfg.secrets_path.clone(),
                                                            overlay_bin: cfg.overlay_bin.clone(),
                                                        },
                                                        &failed_upload.full_path,
                                                        &failed_upload.processed_path,
                                                        &title_with_game,
                                                        &failed_upload.title,
                                                        &video_id,
                                                    ) {
                                                        eprintln!("[CLIPS_APP] Failed to move files: {err:#}");
                                                    }

                                                    failed_uploads_list.remove(&id);
                                                    if let Err(err) = failed_uploads_list.save() {
                                                        eprintln!("[CLIPS_APP] Failed to save failed uploads list: {err:#}");
                                                    }
                                                    controller.set_message("Retry successful!");
                                                }
                                                Ok(None) => {
                                                    controller.set_message("Retry failed: no video id");
                                                }
                                                Err(err) => {
                                                    eprintln!("[CLIPS_APP] Retry upload failed: {err:#}");
                                                    controller.set_message(format!("Retry failed: {err:#}"));
                                                }
                                            }
                                        }
                                    }
                                    "ignore" => {
                                        eprintln!("[CLIPS_APP] Ignoring failed upload: {}", id);
                                        failed_uploads_list.remove(&id);
                                        if let Err(err) = failed_uploads_list.save() {
                                            eprintln!("[CLIPS_APP] Failed to save failed uploads list: {err:#}");
                                        }
                                        controller.set_message("Upload removed from list");
                                    }
                                    "discard" => {
                                        if let Some(failed_upload) = failed_uploads_list.remove(&id) {
                                            eprintln!("[CLIPS_APP] Discarding failed upload: {}", failed_upload.display_name());
                                            if failed_upload.processed_path.exists() {
                                                if let Err(err) = std::fs::remove_file(&failed_upload.processed_path) {
                                                    eprintln!("[CLIPS_APP] Failed to delete processed file: {err:#}");
                                                }
                                            }
                                            if failed_upload.full_path.exists() {
                                                if let Err(err) = std::fs::remove_file(&failed_upload.full_path) {
                                                    eprintln!("[CLIPS_APP] Failed to delete full file: {err:#}");
                                                }
                                            }

                                            if let Err(err) = failed_uploads_list.save() {
                                                eprintln!("[CLIPS_APP] Failed to save failed uploads list: {err:#}");
                                            }
                                            controller.set_message("Upload discarded");
                                        }
                                    }
                                    _ => {
                                        eprintln!("[CLIPS_APP] Unknown failed upload action: {}", upload_action);
                                    }
                                }
                            }
                        }

                        if outcome.is_none() {
                            let mode = *replay_mode.lock().unwrap();
                            session
                                .update_status(build_capture_status(
                                    &controller.status()?,
                                    &cfg.hotkey,
                                    failed_uploads_list,
                                    mode,
                                ))
                                .context("failed to update capture status")?;
                        }
                    }
                }

                if outcome.is_none() {
                    action_task = spawn_action_task(session.clone());
                }
            }
            _ = detection_interval.tick(), if outcome.is_none() => {
                let mode = *replay_mode.lock().unwrap();
                if mode == ReplayMode::AutoWithGame {
                    if maybe_handle_game_detection(controller, &mut game_was_running).await? {
                        session
                            .update_status(build_capture_status(
                                &controller.status()?,
                                &cfg.hotkey,
                                failed_uploads_list,
                                mode,
                            ))
                            .context("failed to update capture status")?;
                    }
                } else if game_was_running {
                    game_was_running = false;
                }
            }
        }
    }

    if !action_task.is_finished() {
        action_task.abort();
        let _ = action_task.await;
    }

    Ok(outcome.expect("capture loop must produce an outcome"))
}

async fn maybe_handle_game_detection(
    controller: &mut ReplayController,
    game_was_running: &mut bool,
) -> Result<bool> {
    match detect_steam_game() {
        Ok(game_name) => {
            let game_running = !game_name.is_empty();
            eprintln!(
                "[CLIPS_APP] Game detection check: '{}' (running: {})",
                game_name, game_running
            );

            if game_running && !*game_was_running {
                eprintln!("[CLIPS_APP] Game detected, ensuring replay is running");
                match controller.ensure_running().await {
                    Ok(_) => {
                        let message = if game_name.is_empty() {
                            "Replay started (game detected)".to_string()
                        } else {
                            format!("Replay started ({game_name})")
                        };
                        controller.set_message(message);
                        *game_was_running = true;
                    }
                    Err(err) => {
                        eprintln!("[CLIPS_APP] Failed to start replay: {err:#}");
                        controller.set_message(format!("Failed to start replay: {err:#}"));
                    }
                }
                return Ok(true);
            } else if !game_running && *game_was_running {
                eprintln!("[CLIPS_APP] Game exited, stopping replay");
                match controller.stop().await {
                    Ok(_) => {
                        controller.set_message("Replay stopped (game exited)");
                    }
                    Err(err) => {
                        eprintln!("[CLIPS_APP] Failed to stop replay: {err:#}");
                        controller.set_message(format!("Failed to stop replay: {err:#}"));
                    }
                }
                *game_was_running = false;
                return Ok(true);
            }
        }
        Err(err) => {
            eprintln!("[CLIPS_APP] Game detection error: {err:#}");
        }
    }

    Ok(false)
}

fn failed_uploads_to_entries(uploads: &clips_app::failed_uploads::FailedUploadsList) -> Vec<overlay::FailedUploadEntry> {
    uploads.uploads.iter().map(|u| overlay::FailedUploadEntry {
        id: u.id.clone(),
        display_name: u.display_name(),
    }).collect()
}

fn build_capture_status(
    status: &clips_app::capture::ReplayStatus, 
    hotkey: &str,
    failed_uploads: &clips_app::failed_uploads::FailedUploadsList,
    replay_mode: ReplayMode,
) -> CaptureStatusPayload {
    let mode_str = match replay_mode {
        ReplayMode::Manual => "manual",
        ReplayMode::AutoWithGame => "auto",
    };
    
    CaptureStatusPayload {
        running: status.running,
        buffer_seconds: status.buffer_seconds,
        bitrate: status.bitrate,
        fps: status.fps,
        target: status.target.clone(),
        audio_tracks: status.audio_tracks.clone(),
        last_saved: status
            .last_saved
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        hotkey: hotkey.to_string(),
        message: status.message.clone(),
        is_saving: false,
        failed_uploads: failed_uploads_to_entries(failed_uploads),
        replay_mode: mode_str.to_string(),
    }
}

async fn process_clip(
    config: &AppConfig, 
    overlay_handle: &overlay::OverlayHandle,
    failed_uploads_list: &mut clips_app::failed_uploads::FailedUploadsList,
) -> Result<()> {
    overlay_handle.set_visibility(true)?;
    overlay_handle
        .update(Stage::Detected, 0.0, format!("Detected: {}", config.source_file_name()))?;

    let detected_game = detect_game_name();

    let available_channels = CHANNEL_OPTIONS
        .iter()
        .map(|channel| channel.to_string())
        .collect::<Vec<_>>();
    let picker_result = match overlay_handle.show_picker(
        Some(&config.source),
        &config.source_file_name(),
        &detected_game,
        &available_channels,
    )? {
        Some(result) => result,
        None => {
            std::fs::remove_file(&config.source).ok();
            overlay_handle.update(Stage::Done, 1.0, "Cancelled")?;
            return Ok(());
        }
    };

    let safe_title = sanitize_text(&picker_result.title);
    let safe_game = sanitize_text(&picker_result.game);

    if matches!(picker_result.action, overlay::ActionChoice::Discard) {
        std::fs::remove_file(&config.source).ok();
        overlay_handle.update(Stage::Done, 1.0, "Discarded")?;
        return Ok(());
    }

    let transformed = process::mix_audio(config, &picker_result.channels, overlay_handle).await?;

    overlay_handle.update(Stage::AwaitExport, 0.0, "Probing video duration...")?;
    let duration = ffmpeg::probe_duration(&transformed).await?;

    overlay_handle.update(Stage::AwaitExport, 0.05, "Waiting for trim selection...")?;
    let trim_result = match overlay_handle.show_trimmer(&transformed, duration)? {
        Some(result) => result,
        None => {
            std::fs::remove_file(&config.source).ok();
            std::fs::remove_file(&transformed).ok();
            overlay_handle.update(Stage::Done, 1.0, "Cancelled")?;
            return Ok(());
        }
    };

    overlay_handle.update(Stage::AwaitExport, 0.1, "Preparing trim…")?;
    let parent = config.source.parent().context("source file has no parent directory")?;
    let stem = config.source.file_stem().context("source file has no stem")?;
    let trimmed = parent.join(format!("{}_trimmed.mp4", stem.to_string_lossy()));
    ffmpeg::trim_video(
        &transformed,
        &trimmed,
        trim_result.start_time,
        trim_result.end_time,
        |fraction| {
            let stage_fraction = (0.1 + fraction * 0.9).min(1.0);
            let detail = format_stage_detail(Stage::AwaitExport, fraction, "trimmed");
            let _ = overlay_handle.update(Stage::AwaitExport, stage_fraction, detail);
        },
    )
    .await?;
    overlay_handle.update(Stage::AwaitExport, 1.0, "Trim complete")?;

    overlay_handle.update(Stage::Finalise, 0.0, "Finalising files…")?;
    let (out_full, out_processed) = finalise_files(
        config,
        &config.source,
        &trimmed,
        &transformed,
        &safe_title,
    )?;
    overlay_handle.update(Stage::Finalise, 1.0, "Files ready")?;

    let base_title_with_game = if safe_game.is_empty() {
        safe_title.clone()
    } else {
        format!("{safe_title} [{safe_game}]")
    };

    match picker_result.action {
        overlay::ActionChoice::Move => {
            let dest_dir = handle_move_action(
                config,
                &out_full,
                &out_processed,
                &base_title_with_game,
                &safe_title,
            )?;
            overlay_handle.update(Stage::Done, 1.0, "Saved (no upload)")?;
            println!("Saved clip to {:?}", dest_dir);
        }
        overlay::ActionChoice::Upload => {
            let title_with_game = base_title_with_game.clone();
            let video_id = upload::upload_to_youtube(
                config,
                &out_processed,
                &title_with_game,
                &safe_game,
                overlay_handle,
            )
            .await?;
            
            if let Some(id) = video_id {
                overlay_handle.update(Stage::Done, 1.0, "Upload complete")?;
                let dest_dir = handle_upload_action(
                    config,
                    &out_full,
                    &out_processed,
                    &title_with_game,
                    &safe_title,
                    &id,
                )?;
                println!("Uploaded video id: {id} (stored at {:?})", dest_dir);
                open_uploaded_video(&id);
            } else {
                // Upload failed - treat like a move action (save without YouTube ID)
                overlay_handle.update(Stage::Done, 1.0, "Upload failed - saved locally")?;
                let dest_dir = handle_move_action(
                    config,
                    &out_full,
                    &out_processed,
                    &title_with_game,
                    &safe_title,
                )?;
                println!("Upload failed, saved clip to {:?}", dest_dir);
                
                // Add to failed uploads list for retry later
                let processed_file = dest_dir.join(format!("{title_with_game}.mp4"));
                let full_file = dest_dir.join(format!("{safe_title}_raw.mp4"));
                
                let failed_upload = clips_app::failed_uploads::FailedUpload::new(
                    safe_title.clone(),
                    safe_game.clone(),
                    processed_file,
                    full_file,
                );
                
                failed_uploads_list.add(failed_upload);
                if let Err(err) = failed_uploads_list.save() {
                    eprintln!("[CLIPS_APP] Failed to save failed uploads list: {err:#}");
                }
            }
        }
        overlay::ActionChoice::Discard => {
            anyhow::bail!("discard action should have been handled earlier");
        }
    }

    Ok(())
}

fn set_overlay_visible(
    overlay_handle: &overlay::OverlayHandle,
    visible_state: &Arc<AtomicBool>,
    visible: bool,
) -> Result<()> {
    if visible_state.load(Ordering::SeqCst) != visible {
        overlay_handle.set_visibility(visible)?;
        visible_state.store(visible, Ordering::SeqCst);
    }
    Ok(())
}

async fn spawn_hotkey_listener(
    hotkey: String,
    overlay_handle: overlay::OverlayHandle,
    visible_state: Arc<AtomicBool>,
) -> Option<JoinHandle<()>> {
    // Parse the hotkey string (e.g., "Alt+X")
    let (modifier_keys, target_key) = parse_hotkey(&hotkey);
    
    eprintln!("[CLIPS_APP] Attempting to register global hotkey: {}", hotkey);
    
    // Find all keyboard devices
    let keyboard_devices = match find_keyboard_devices() {
        Ok(devices) if !devices.is_empty() => devices,
        Ok(_) => {
            eprintln!("[CLIPS_APP] No keyboard devices found in /dev/input");
            eprintln!("[CLIPS_APP] Make sure your user is in the 'input' group");
            return None;
        }
        Err(err) => {
            eprintln!("[CLIPS_APP] Failed to enumerate keyboard devices: {err:#}");
            eprintln!("[CLIPS_APP] Make sure your user is in the 'input' group");
            return None;
        }
    };

    eprintln!("[CLIPS_APP] Found {} keyboard device(s), monitoring for hotkey...", keyboard_devices.len());

    Some(tokio::task::spawn_blocking(move || {
        let mut pressed_modifiers = std::collections::HashSet::new();
        
        // Monitor all keyboard devices
        for device_path in keyboard_devices {
            let mut device = match Device::open(&device_path) {
                Ok(dev) => dev,
                Err(err) => {
                    eprintln!("[CLIPS_APP] Failed to open {}: {}", device_path, err);
                    continue;
                }
            };

            eprintln!("[CLIPS_APP] Monitoring keyboard: {:?}", device.name());

            loop {
                match device.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            if let InputEventKind::Key(key) = ev.kind() {
                                let pressed = ev.value() == 1;
                                
                                // Track modifier keys
                                if modifier_keys.contains(&key) {
                                    if pressed {
                                        pressed_modifiers.insert(key);
                                    } else {
                                        pressed_modifiers.remove(&key);
                                    }
                                }
                                
                                // Check if hotkey is pressed
                                if key == target_key && pressed {
                                    let all_modifiers_pressed = modifier_keys.iter()
                                        .all(|m| pressed_modifiers.contains(m));
                                    
                                    if all_modifiers_pressed {
                                        let desired = !visible_state.load(Ordering::SeqCst);
                                        if let Err(err) = overlay_handle.set_visibility(desired) {
                                            eprintln!("[CLIPS_APP] Failed to toggle overlay: {err:#}");
                                        } else {
                                            visible_state.store(desired, Ordering::SeqCst);
                                            eprintln!("[CLIPS_APP] Toggled overlay to: {}", desired);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(err) => {
                        eprintln!("[CLIPS_APP] Error reading events: {}", err);
                        break;
                    }
                }
            }
        }
    }))
}

fn find_keyboard_devices() -> Result<Vec<String>> {
    use std::fs;
    
    let mut keyboards = Vec::new();
    
    for entry in fs::read_dir("/dev/input")? {
        let entry = entry?;
        let path = entry.path();
        
        if let Some(filename) = path.file_name() {
            if filename.to_string_lossy().starts_with("event") {
                if let Ok(device) = Device::open(&path) {
                    // Check if it's a keyboard (has KEY_X capability)
                    if device.supported_keys().is_some_and(|keys| keys.contains(Key::KEY_X)) {
                        keyboards.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    
    Ok(keyboards)
}

fn parse_hotkey(hotkey: &str) -> (Vec<Key>, Key) {
    let parts: Vec<&str> = hotkey.split('+').map(|s| s.trim()).collect();
    let mut modifiers = Vec::new();
    let mut target = Key::KEY_X;
    
    for part in parts {
        match part.to_lowercase().as_str() {
            "alt" => modifiers.push(Key::KEY_LEFTALT),
            "ctrl" | "control" => modifiers.push(Key::KEY_LEFTCTRL),
            "shift" => modifiers.push(Key::KEY_LEFTSHIFT),
            "super" | "meta" | "win" => modifiers.push(Key::KEY_LEFTMETA),
            "x" => target = Key::KEY_X,
            "c" => target = Key::KEY_C,
            "v" => target = Key::KEY_V,
            "s" => target = Key::KEY_S,
            _ => {}
        }
    }
    
    (modifiers, target)
}

fn finalise_files(
    config: &AppConfig,
    source: &Path,
    new_clip: &Path,
    transformed: &Path,
    safe_title: &str,
) -> Result<(PathBuf, PathBuf)> {
    use std::fs;

    let processed_dir = &config.processed_dir;
    fs::create_dir_all(processed_dir).context("creating processed directory")?;

    let base_full = processed_dir.join(format!("{}_full.mp4", safe_title));
    let out_full = unique_path(&base_full)?;
    let base_processed = processed_dir.join(format!("{safe_title}.mp4"));
    let out_processed = unique_path(&base_processed)?;

    fs::rename(source, &out_full).context("moving original file")?;
    if new_clip != out_processed.as_path() {
        if let Some(parent) = out_processed.parent() {
            fs::create_dir_all(parent).context("creating processed destination directory")?;
        }
        fs::rename(new_clip, &out_processed).context("moving processed clip")?;
    }
    if transformed.exists() && transformed != out_processed.as_path() {
        let _ = fs::remove_file(transformed);
    }
    Ok((out_full, out_processed))
}

fn unique_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip");
    let extension = path.extension().and_then(|s| s.to_str());

    const MAX_ATTEMPTS: u32 = 9999;
    for idx in 1..=MAX_ATTEMPTS {
        let candidate_name = match extension {
            Some(ext) => format!("{stem}-{idx}.{ext}"),
            None => format!("{stem}-{idx}"),
        };
        let candidate = if parent.as_os_str().is_empty() {
            PathBuf::from(&candidate_name)
        } else {
            parent.join(&candidate_name)
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("failed to find unique path after {} attempts", MAX_ATTEMPTS);
}

fn detect_game_name() -> String {
    // Check environment variable first as override
    if let Ok(game) = std::env::var("CLIPS_DEFAULT_GAME") {
        if !game.is_empty() {
            return game;
        }
    }
    
    // Detect running Steam games by checking process environments for SteamAppId
    if let Ok(game) = detect_steam_game() {
        if !game.is_empty() {
            eprintln!("[CLIPS_APP] Detected game: {}", game);
            return game;
        }
    }
    
    String::new()
}

fn detect_steam_game() -> Result<String> {
    use std::fs;
    use std::io::Read;
    
    // Find Steam base directory
    let home = std::env::var("HOME")?;
    let steam_base = if std::path::Path::new(&format!("{}/.steam/steam", home)).exists() {
        format!("{}/.steam/steam", home)
    } else {
        format!("{}/.local/share/Steam", home)
    };
    
    // Get current user ID for filtering processes
    let current_uid = unsafe { libc::getuid() };
    
    // Search /proc for processes with SteamAppId
    let proc_dir = fs::read_dir("/proc").context("Failed to read /proc")?;
    
    for entry in proc_dir.flatten() {
        let path = entry.path();
        
        // Only check numeric directories (PIDs)
        if let Some(name) = path.file_name() {
            if name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
                // Check if process belongs to current user
                if let Ok(status) = fs::read_to_string(path.join("status")) {
                    if !status.lines().any(|line| {
                        line.starts_with("Uid:") && line.split_whitespace()
                            .nth(1)
                            .and_then(|uid_str| uid_str.parse::<u32>().ok())
                            .is_some_and(|uid| uid == current_uid)
                    }) {
                        continue;
                    }
                }
                
                // Read environment variables
                if let Ok(mut env_file) = fs::File::open(path.join("environ")) {
                    let mut env_data = Vec::new();
                    if env_file.read_to_end(&mut env_data).is_ok() {
                        // Parse null-separated environment variables
                        let env_str = String::from_utf8_lossy(&env_data);
                        
                        // Look for SteamAppId or SteamGameId
                        for env_var in env_str.split('\0') {
                            if let Some(appid_str) = env_var.strip_prefix("SteamAppId=")
                                .or_else(|| env_var.strip_prefix("SteamGameId="))
                            {
                                if let Ok(appid) = appid_str.parse::<u32>() {
                                    // Found an AppID! Look up the game name
                                    let manifest_path = format!(
                                        "{}/steamapps/appmanifest_{}.acf",
                                        steam_base, appid
                                    );
                                    
                                    if let Ok(manifest) = fs::read_to_string(&manifest_path) {
                                        // Parse the manifest for the game name
                                        for line in manifest.lines() {
                                            if let Some(name) = parse_acf_name_line(line) {
                                                return Ok(name);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    Ok(String::new())
}

fn parse_acf_name_line(line: &str) -> Option<String> {
    // Parse lines like:  "name"  "Game Name"
    let trimmed = line.trim();
    if trimmed.starts_with("\"name\"") {
        // Find the second quoted string
        if let Some(start) = trimmed.find('"').and_then(|i| trimmed[i + 1..].find('"').map(|j| i + j + 2)) {
            if let Some(value_start) = trimmed[start..].find('"') {
                let value_start_abs = start + value_start + 1;
                if let Some(value_end) = trimmed[value_start_abs..].find('"') {
                    return Some(trimmed[value_start_abs..value_start_abs + value_end].to_string());
                }
            }
        }
    }
    None
}

fn sanitize_text(value: &str) -> String {
    let mut cleaned = value
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'))
        .collect::<String>();
    cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "clip".into()
    } else {
        cleaned.to_string()
    }
}

fn handle_move_action(
    config: &AppConfig,
    out_full: &Path,
    out_processed: &Path,
    title_with_game: &str,
    safe_title: &str,
) -> Result<PathBuf> {
    let dest_dir = config.processed_dir.join(title_with_game);
    std::fs::create_dir_all(&dest_dir).context("creating destination directory")?;
    if out_processed.exists() {
        let target_base = dest_dir.join(format!("{title_with_game}.mp4"));
        let target = unique_path(&target_base)?;
        std::fs::rename(out_processed, &target)?;
    }
    if out_full.exists() {
        let target_base = dest_dir.join(format!("{safe_title}_raw.mp4"));
        let target = unique_path(&target_base)?;
        std::fs::rename(out_full, &target)?;
    }
    Ok(dest_dir)
}

fn handle_upload_action(
    config: &AppConfig,
    out_full: &Path,
    out_processed: &Path,
    title_with_game: &str,
    safe_title: &str,
    video_id: &str,
) -> Result<PathBuf> {
    let dest_dir = config
        .processed_dir
        .join(format!("{title_with_game} [{video_id}]"));
    std::fs::create_dir_all(&dest_dir).context("creating upload destination")?;
    if out_processed.exists() {
        let target_base = dest_dir.join(format!("{title_with_game} [{video_id}].mp4"));
        let target = unique_path(&target_base)?;
        std::fs::rename(out_processed, &target)?;
    }
    if out_full.exists() {
        let target_base = dest_dir.join(format!("{safe_title}_raw.mp4"));
        let target = unique_path(&target_base)?;
        std::fs::rename(out_full, &target)?;
    }
    Ok(dest_dir)
}

fn open_uploaded_video(video_id: &str) {
    let url = format!("https://youtu.be/{video_id}");
    use std::process::Command;

    let status = Command::new("/run/current-system/sw/bin/xdg-open")
        .arg(&url)
        .status();

    match status {
        Ok(code) if code.success() => {}
        Ok(code) => eprintln!(
            "[CLIPS_APP] xdg-open exited with status {:?} while opening {}",
            code, url
        ),
        Err(err) => eprintln!(
            "[CLIPS_APP] Failed to launch browser via xdg-open for {}: {}",
            url, err
        ),
    }
}
