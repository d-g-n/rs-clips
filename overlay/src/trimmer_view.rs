use gtk::{gdk, Box, Button, DrawingArea, Label, Orientation, Video};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct TrimmerResult {
    pub start_time: f64,
    pub end_time: f64,
}

type SubmitCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(TrimmerResult) + 'static>>>>;
type CancelCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn() + 'static>>>>;

pub struct TrimmerView {
    container: Box,
    video: Video,
    timeline: DrawingArea,
    duration_label: Label,
    start_label: Label,
    end_label: Label,
    duration: Rc<RefCell<f64>>,
    start_pos: Rc<RefCell<f64>>, // 0.0 to 1.0
    end_pos: Rc<RefCell<f64>>,   // 0.0 to 1.0
    current_pos: Rc<RefCell<f64>>, // Current playback position 0.0 to 1.0
    #[allow(dead_code)]
    dragging: Rc<RefCell<Option<DragTarget>>>,
    submit_callback: SubmitCallback,
    cancel_callback: CancelCallback,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragTarget {
    Playhead,
}

impl TrimmerView {
    pub fn new() -> Self {
        // Outer container to prevent expansion
        let outer = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        outer.set_halign(gtk::Align::Start);
        outer.set_valign(gtk::Align::Start);
        
        let container = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .build();
        container.add_css_class("trimmer-box");

        // Apply CSS matching other views
        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_data(
            ".trimmer-box {\
                \n  padding: 16px 24px;\
                \n  border-radius: 12px;\
                \n  background-color: rgba(30, 30, 30, 0.9);\
                \n}\
                \n.status-bar {\
                \n  padding: 12px;\
                \n  background-color: rgba(45, 45, 45, 0.95);\
                \n  border-radius: 6px;\
                \n  margin-bottom: 12px;\
                \n}\
                \n.status-text {\
                \n  color: white;\
                \n  font-weight: 700;\
                \n  font-size: 16px;\
                \n  font-family: monospace;\
                \n}\
                \n.trimmer-label {\
                \n  color: white;\
                \n  font-weight: 600;\
                \n  font-size: 14px;\
                \n  font-family: monospace;\
                \n  margin-top: 8px;\
                \n}\
                \n.time-label {\
                \n  color: #4CAF50;\
                \n  font-weight: 600;\
                \n  font-size: 13px;\
                \n  font-family: monospace;\
                \n}\
                \nscale {\
                \n  min-width: 700px;\
                \n}\
                \nbutton {\
                \n  background-color: rgba(50, 50, 50, 0.9);\
                \n  color: white;\
                \n  border: 1px solid rgba(80, 80, 80, 0.8);\
                \n  border-radius: 4px;\
                \n  padding: 6px 12px;\
                \n  font-family: monospace;\
                \n}\
                \nbutton:hover {\
                \n  background-color: rgba(70, 70, 70, 0.95);\
                \n}\
                \n.control-button {\
                \n  background: #2a2a2a;\
                \n  color: #e0e0e0;\
                \n  border: 1px solid #404040;\
                \n  border-radius: 4px;\
                \n  padding: 8px 16px;\
                \n  min-width: 100px;\
                \n}\
                \n.control-button:hover {\
                \n  background: #353535;\
                \n  border-color: #505050;\
                \n}\
                \n.control-button:active {\
                \n  background: #252525;\
                \n}\
            ",
        );
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Status bar at top
        let status_bar = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        status_bar.add_css_class("status-bar");
        
        let status_label = Label::new(Some("Trim video"));
        status_label.add_css_class("status-text");
        status_label.set_halign(gtk::Align::Center);
        status_bar.append(&status_label);
        container.append(&status_bar);

        // Video player (hide all default controls)
        let video = Video::builder()
            .width_request(800)
            .height_request(450)
            .autoplay(false)
            .loop_(false)
            .build();
        
        // Completely hide the default video controls
        video.set_can_target(false);
        
        container.append(&video);

        // Time labels row
        let time_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        
        let start_label = Label::new(Some("Start: 0:00"));
        start_label.add_css_class("time-label");
        start_label.set_halign(gtk::Align::Start);
        start_label.set_hexpand(true);
        time_box.append(&start_label);
        
        let duration_label = Label::new(Some("Duration: 0:00"));
        duration_label.add_css_class("time-label");
        duration_label.set_halign(gtk::Align::Center);
        time_box.append(&duration_label);
        
        let end_label = Label::new(Some("End: 0:00"));
        end_label.add_css_class("time-label");
        end_label.set_halign(gtk::Align::End);
        end_label.set_hexpand(true);
        time_box.append(&end_label);
        
        container.append(&time_box);

        // Playback and cut controls - arranged with cut buttons on sides, play/pause in center
        let controls_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        
        let cut_start_button = Button::with_label("◀ Cut Start");
        cut_start_button.add_css_class("control-button");
        cut_start_button.set_halign(gtk::Align::Start);
        
        // Spacer to push play/pause to center
        let left_spacer = Box::new(Orientation::Horizontal, 0);
        left_spacer.set_hexpand(true);
        
        let play_pause_button = Button::with_label("▶ Play");
        play_pause_button.add_css_class("control-button");
        
        // Spacer to push cut end to right
        let right_spacer = Box::new(Orientation::Horizontal, 0);
        right_spacer.set_hexpand(true);
        
        let cut_end_button = Button::with_label("Cut End ▶");
        cut_end_button.add_css_class("control-button");
        cut_end_button.set_halign(gtk::Align::End);
        
        controls_box.append(&cut_start_button);
        controls_box.append(&left_spacer);
        controls_box.append(&play_pause_button);
        controls_box.append(&right_spacer);
        controls_box.append(&cut_end_button);
        container.append(&controls_box);

        // Custom timeline drawing area
        let timeline = DrawingArea::builder()
            .width_request(760)
            .height_request(60)
            .build();
        container.append(&timeline);

        let duration = Rc::new(RefCell::new(0.0));
        let start_pos = Rc::new(RefCell::new(0.0));
        let end_pos = Rc::new(RefCell::new(1.0));
        let current_pos = Rc::new(RefCell::new(0.0));
        let dragging: Rc<RefCell<Option<DragTarget>>> = Rc::new(RefCell::new(None));
        let was_playing: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        // Counter to skip multiple sync cycles after seeking (need ~3 cycles for 50ms delay + seek)
        let seeking_skip_cycles: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        // Setup timeline drawing
        let start_pos_draw = start_pos.clone();
        let end_pos_draw = end_pos.clone();
        let current_pos_draw = current_pos.clone();
        
        timeline.set_draw_func(move |_area, cr, width, height| {
            let start = *start_pos_draw.borrow();
            let end = *end_pos_draw.borrow();
            let current = *current_pos_draw.borrow();
            
            // Background
            cr.set_source_rgb(0.2, 0.2, 0.2);
            let _ = cr.paint();
            
            // Selected region (between start and end)
            cr.set_source_rgb(0.3, 0.5, 0.7);
            let start_x = start * width as f64;
            let end_x = end * width as f64;
            cr.rectangle(start_x, 0.0, end_x - start_x, height as f64);
            let _ = cr.fill();
            
            // Playhead (current position)
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            let playhead_x = current * width as f64;
            cr.move_to(playhead_x, 0.0);
            cr.line_to(playhead_x, height as f64);
            let _ = cr.stroke();
            
            // Start marker (left finger icon - simplified as triangle)
            cr.set_source_rgb(0.3, 0.8, 0.3);
            cr.move_to(start_x, 0.0);
            cr.line_to(start_x + 15.0, 0.0);
            cr.line_to(start_x + 7.5, 15.0);
            cr.close_path();
            let _ = cr.fill();
            
            // Start marker line
            cr.set_line_width(3.0);
            cr.move_to(start_x, 15.0);
            cr.line_to(start_x, height as f64);
            let _ = cr.stroke();
            
            // End marker (right finger icon - simplified as triangle)
            cr.set_source_rgb(0.8, 0.3, 0.3);
            cr.move_to(end_x, 0.0);
            cr.line_to(end_x - 15.0, 0.0);
            cr.line_to(end_x - 7.5, 15.0);
            cr.close_path();
            let _ = cr.fill();
            
            // End marker line
            cr.set_line_width(3.0);
            cr.move_to(end_x, 15.0);
            cr.line_to(end_x, height as f64);
            let _ = cr.stroke();
        });

        // Mouse event handling for timeline - only allow dragging playhead within bounds
        let gesture = gtk::GestureDrag::new();
        gesture.set_button(gdk::ffi::GDK_BUTTON_PRIMARY as u32);
        
        let current_pos_drag = current_pos.clone();
        let dragging_start = dragging.clone();
        let was_playing_start = was_playing.clone();
        let timeline_drag = timeline.clone();
        let video_drag = video.clone();
        let duration_drag = duration.clone();
        
        gesture.connect_drag_begin(move |_gesture, x, _y| {
            let width = timeline_drag.width() as f64;
            let click_pos = (x / width).clamp(0.0, 1.0);
            
            // Allow dragging anywhere on the timeline
            *dragging_start.borrow_mut() = Some(DragTarget::Playhead);
            
            // Pause video if it's playing and remember state
            if let Some(media_stream) = video_drag.media_stream() {
                *was_playing_start.borrow_mut() = media_stream.is_playing();
                if media_stream.is_playing() {
                    media_stream.set_playing(false);
                }
            }
            
            // Update position (unconstrained during manual scrubbing)
            *current_pos_drag.borrow_mut() = click_pos;
            
            // Seek video immediately
            let dur = *duration_drag.borrow();
            let time = click_pos * dur;
            if let Some(media_stream) = video_drag.media_stream() {
                media_stream.seek((time * 1_000_000.0) as i64);
            }
            
            timeline_drag.queue_draw();
        });
        
        let current_pos_update = current_pos.clone();
        let dragging_update = dragging.clone();
        let timeline_update = timeline.clone();
        let video_update = video.clone();
        let duration_update = duration.clone();
        
        gesture.connect_drag_update(move |gesture, _dx, _dy| {
            if let Some(DragTarget::Playhead) = *dragging_update.borrow() {
                // Get absolute mouse position relative to the timeline widget
                if let Some((x, _y)) = gesture.start_point() {
                    if let Some((offset_x, _offset_y)) = gesture.offset() {
                        let absolute_x = x + offset_x;
                        let width = timeline_update.width() as f64;
                        let new_pos = (absolute_x / width).clamp(0.0, 1.0);
                        
                        // Allow scrubbing anywhere (unconstrained during manual dragging)
                        *current_pos_update.borrow_mut() = new_pos;
                        
                        // Seek video in real-time for smooth scrubbing
                        let dur = *duration_update.borrow();
                        let time = new_pos * dur;
                        if let Some(media_stream) = video_update.media_stream() {
                            media_stream.seek((time * 1_000_000.0) as i64);
                        }
                        
                        timeline_update.queue_draw();
                    }
                }
            }
        });
        
        let dragging_end = dragging.clone();
        let was_playing_end = was_playing.clone();
        let start_pos_end = start_pos.clone();
        let end_pos_end = end_pos.clone();
        let current_pos_end = current_pos.clone();
        let video_end = video.clone();
        
        gesture.connect_drag_end(move |_, _, _| {
            *dragging_end.borrow_mut() = None;
            
            let current = *current_pos_end.borrow();
            let start = *start_pos_end.borrow();
            let end = *end_pos_end.borrow();
            
            // Check if playhead is outside the clipped region
            let is_outside = current < start || current > end;
            
            // If dragged outside clipped area, pause (don't resume even if was playing)
            // If dragged inside clipped area and was playing, resume playback
            if !is_outside && *was_playing_end.borrow() {
                if let Some(media_stream) = video_end.media_stream() {
                    media_stream.set_playing(true);
                }
            }
            // If outside, ensure it stays paused (do nothing)
            
            *was_playing_end.borrow_mut() = false;
        });
        
        timeline.add_controller(gesture);

        // Play/Pause toggle button handler
        let video_toggle = video.clone();
        let current_pos_toggle = current_pos.clone();
        let start_pos_toggle = start_pos.clone();
        let end_pos_toggle = end_pos.clone();
        let duration_toggle = duration.clone();
        let button_toggle = play_pause_button.clone();
        let seeking_skip_toggle = seeking_skip_cycles.clone();
        
        play_pause_button.connect_clicked(move |btn| {
            if let Some(media_stream) = video_toggle.media_stream() {
                if media_stream.is_playing() {
                    // Currently playing, so pause
                    media_stream.set_playing(false);
                    btn.set_label("▶ Play");
                } else {
                    // Currently paused, so play
                    let current = *current_pos_toggle.borrow();
                    let start = *start_pos_toggle.borrow();
                    let end = *end_pos_toggle.borrow();
                    let dur = *duration_toggle.borrow();
                    
                    eprintln!("[TRIMMER] Play clicked: current={:.3}, start={:.3}, end={:.3}", current, start, end);
                    
                    // If playhead is outside clipped region OR at the exact start, seek and restart
                    // Use small epsilon for floating point comparison
                    let epsilon = 0.001;
                    if current < start || current > end || (current - start).abs() < epsilon {
                        eprintln!("[TRIMMER] Seeking to start position");
                        *current_pos_toggle.borrow_mut() = start;
                        let start_time = start * dur;
                        media_stream.seek((start_time * 1_000_000.0) as i64);
                        // Skip next 5 sync cycles (~165ms) to allow seek + playback restart
                        *seeking_skip_toggle.borrow_mut() = 5;
                        // Longer delay to ensure seek completes before starting playback
                        let media_stream_clone = media_stream.clone();
                        let btn_clone = btn.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                            eprintln!("[TRIMMER] Starting playback after seek");
                            media_stream_clone.set_playing(true);
                            btn_clone.set_label("⏸ Pause");
                        });
                    } else {
                        eprintln!("[TRIMMER] Already in valid range, starting immediately");
                        // Already in valid range, start playing immediately
                        media_stream.set_playing(true);
                        btn.set_label("⏸ Pause");
                    }
                }
            }
        });
        
        // Update button label based on playback state (poll every 100ms)
        let video_label_update = video.clone();
        let button_label_update = button_toggle.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            if let Some(media_stream) = video_label_update.media_stream() {
                let label = if media_stream.is_playing() {
                    "⏸ Pause"
                } else {
                    "▶ Play"
                };
                button_label_update.set_label(label);
            }
            glib::ControlFlow::Continue
        });

        // Cut start button handler
        let current_pos_cut_start = current_pos.clone();
        let start_pos_cut_start = start_pos.clone();
        let end_pos_cut_start_check = end_pos.clone();
        let duration_cut_start = duration.clone();
        let start_label_cut_start = start_label.clone();
        let timeline_cut_start = timeline.clone();
        
        cut_start_button.connect_clicked(move |_| {
            let current = *current_pos_cut_start.borrow();
            let end = *end_pos_cut_start_check.borrow();
            
            // Set start to current position (but not past end)
            let new_start = current.min(end - 0.01);
            *start_pos_cut_start.borrow_mut() = new_start;
            
            let dur = *duration_cut_start.borrow();
            let time = new_start * dur;
            start_label_cut_start.set_text(&format!("Start: {}", format_time(time)));
            timeline_cut_start.queue_draw();
        });

        // Cut end button handler
        let current_pos_cut_end = current_pos.clone();
        let start_pos_cut_end_check = start_pos.clone();
        let end_pos_cut_end = end_pos.clone();
        let duration_cut_end = duration.clone();
        let end_label_cut_end = end_label.clone();
        let timeline_cut_end = timeline.clone();
        
        cut_end_button.connect_clicked(move |_| {
            let current = *current_pos_cut_end.borrow();
            let start = *start_pos_cut_end_check.borrow();
            
            // Set end to current position (but not before start)
            let new_end = current.max(start + 0.01);
            *end_pos_cut_end.borrow_mut() = new_end;
            
            let dur = *duration_cut_end.borrow();
            let time = new_end * dur;
            end_label_cut_end.set_text(&format!("End: {}", format_time(time)));
            timeline_cut_end.queue_draw();
        });

        // Sync video playback position with timeline and loop within bounds when playing
        let current_pos_sync = current_pos.clone();
        let start_pos_sync = start_pos.clone();
        let end_pos_sync = end_pos.clone();
        let duration_sync = duration.clone();
        let timeline_sync = timeline.clone();
        let video_sync = video.clone();
        let dragging_sync = dragging.clone();
        let seeking_skip_sync = seeking_skip_cycles.clone();
        
        glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
            if let Some(media_stream) = video_sync.media_stream() {
                let timestamp = media_stream.timestamp(); // in microseconds
                let dur = *duration_sync.borrow();
                
                if dur > 0.0 {
                    let current_time = (timestamp as f64) / 1_000_000.0;
                    let new_pos = (current_time / dur).clamp(0.0, 1.0);
                    
                    let start = *start_pos_sync.borrow();
                    let end = *end_pos_sync.borrow();
                    let is_dragging = dragging_sync.borrow().is_some();
                    let skip_cycles = *seeking_skip_sync.borrow();
                    
                    // Skip updates if we're currently seeking (wait for seek + playback restart)
                    if skip_cycles > 0 {
                        *seeking_skip_sync.borrow_mut() = skip_cycles - 1;
                        // On the last skip cycle, force a redraw to show the seeked position
                        if skip_cycles == 1 {
                            timeline_sync.queue_draw();
                        }
                        return glib::ControlFlow::Continue;
                    }
                    
                    // Only update position and redraw when NOT dragging
                    if !is_dragging {
                        // Only update from video when playing
                        // When paused, the UI state is authoritative
                        if media_stream.is_playing() {
                            // Check if we're at or past the end boundary (with small tolerance)
                            // Use a tolerance of 0.5 seconds to catch the end reliably
                            let tolerance = 0.5 / dur;
                            
                            // Use epsilon for floating point comparisons
                            let epsilon = 0.001; // ~0.1% tolerance
                            
                            if new_pos >= (end - tolerance) {
                                // At or near end, loop back to start
                                eprintln!("[TRIMMER] Loop detected: new_pos={:.3} >= end={:.3}, seeking to start={:.3}", new_pos, end, start);
                                // CRITICAL: Pause before seeking, otherwise seek doesn't work reliably
                                media_stream.set_playing(false);
                                let start_time = start * dur;
                                media_stream.seek((start_time * 1_000_000.0) as i64);
                                *current_pos_sync.borrow_mut() = start;
                                // Skip next 5 sync cycles (~165ms) to allow seek + playback restart
                                *seeking_skip_sync.borrow_mut() = 5;
                                // Longer delay to ensure seek completes before starting playback
                                let media_stream_clone = media_stream.clone();
                                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                                    eprintln!("[TRIMMER] Restarting playback after loop");
                                    media_stream_clone.set_playing(true);
                                });
                            } else if new_pos < (start - epsilon) {
                                // If CLEARLY before start (not just floating point noise), jump to start
                                eprintln!("[TRIMMER] Position before start: new_pos={:.3} < start={:.3}, seeking to start", new_pos, start);
                                // CRITICAL: Pause before seeking
                                media_stream.set_playing(false);
                                let start_time = start * dur;
                                media_stream.seek((start_time * 1_000_000.0) as i64);
                                *current_pos_sync.borrow_mut() = start;
                                // Skip next 5 sync cycles (~165ms) to allow seek + playback restart
                                *seeking_skip_sync.borrow_mut() = 5;
                                // Longer delay to ensure seek completes before starting playback
                                let media_stream_clone = media_stream.clone();
                                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                                    eprintln!("[TRIMMER] Restarting playback after boundary correction");
                                    media_stream_clone.set_playing(true);
                                });
                            } else {
                                *current_pos_sync.borrow_mut() = new_pos;
                            }
                            
                            timeline_sync.queue_draw();
                        }
                        // When paused and not dragging, don't update from video
                        // The UI position is authoritative until playback resumes
                    }
                    // If dragging, completely skip sync updates - drag handler has full control
                }
            }
            glib::ControlFlow::Continue
        });

        // Buttons
        let buttons_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::End)
            .margin_top(8)
            .build();

        let cancel_button = Button::with_label("Cancel");
        let ok_button = Button::with_label("Export");

        buttons_box.append(&cancel_button);
        buttons_box.append(&ok_button);
        container.append(&buttons_box);
        
        outer.append(&container);

        let submit_callback: SubmitCallback = Rc::new(RefCell::new(None));
        let cancel_callback: CancelCallback = Rc::new(RefCell::new(None));

        // Wire up Export button
        let start_pos_export = start_pos.clone();
        let end_pos_export = end_pos.clone();
        let duration_export = duration.clone();
        let current_pos_export = current_pos.clone();
        let timeline_export = timeline.clone();
        let video_export = video.clone();
        let play_button_export = play_pause_button.clone();
        let was_playing_export = was_playing.clone();
        let submit_callback_clone = submit_callback.clone();

        ok_button.connect_clicked(move |_| {
            let dur = *duration_export.borrow();

            if let Some(media_stream) = video_export.media_stream() {
                media_stream.set_playing(false);
                *was_playing_export.borrow_mut() = false;

                if dur > 0.0 {
                    let current_time = (media_stream.timestamp() as f64) / 1_000_000.0;
                    let new_pos = (current_time / dur).clamp(0.0, 1.0);
                    *current_pos_export.borrow_mut() = new_pos;
                    timeline_export.queue_draw();
                }
            }
            play_button_export.set_label("▶ Play");

            let start_pct = *start_pos_export.borrow();
            let end_pct = *end_pos_export.borrow();

            let result = TrimmerResult {
                start_time: start_pct * dur,
                end_time: end_pct * dur,
            };

            if let Some(callback) = submit_callback_clone.borrow().as_ref() {
                callback(result);
            }
        });

        // Wire up Cancel button
        let cancel_callback_clone = cancel_callback.clone();
        cancel_button.connect_clicked(move |_| {
            if let Some(callback) = cancel_callback_clone.borrow().as_ref() {
                callback();
            }
        });

        Self {
            container: outer,
            video,
            timeline,
            duration_label,
            start_label,
            end_label,
            duration,
            start_pos,
            end_pos,
            current_pos,
            dragging,
            submit_callback,
            cancel_callback,
        }
    }

    pub fn container(&self) -> &Box {
        &self.container
    }
    
    pub fn pause_video(&self) {
        // Pause the video to stop it playing in the background
        if let Some(media) = self.video.media_stream() {
            media.pause();
        }
    }

    pub fn show(&self, video_path: &str, duration: f64) {
        // Set video file
        let file = gtk::gio::File::for_path(video_path);
        self.video.set_file(Some(&file));
        
        // Store duration
        *self.duration.borrow_mut() = duration;
        
        // Update duration label
        self.duration_label.set_text(&format!("Duration: {}", format_time(duration)));
        
        // Reset positions
        *self.start_pos.borrow_mut() = 0.0;
        *self.end_pos.borrow_mut() = 1.0;
        *self.current_pos.borrow_mut() = 0.0;
        
        // Update time labels
        self.start_label.set_text(&format!("Start: {}", format_time(0.0)));
        self.end_label.set_text(&format!("End: {}", format_time(duration)));
        
        // Redraw timeline
        self.timeline.queue_draw();
    }

    pub fn on_submit<F>(&self, callback: F)
    where
        F: Fn(TrimmerResult) + 'static,
    {
        *self.submit_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }

    pub fn on_cancel<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.cancel_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }
}

fn format_time(seconds: f64) -> String {
    let total_secs = seconds as i64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}
