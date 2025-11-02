use gtk::{gdk, glib, Window, DrawingArea};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::io::{self, BufRead, Write};
use std::cell::RefCell;
use std::rc::Rc;
use std::thread;
use serde::{Deserialize, Serialize};

mod progress_view;
mod picker_view;
mod trimmer_view;
mod capture_view;

use progress_view::ProgressView;
use picker_view::PickerView;
use trimmer_view::TrimmerView;
use capture_view::{CaptureView, CaptureStatus as CaptureStatusPayload, CaptureSettings as CaptureSettingsPayload};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Command {
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
enum Response {
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
#[serde(tag = "action", rename_all = "snake_case")]
enum CaptureActionPayload {
    Toggle { enable: bool },
    Save { duration_secs: u32 },
    UpdateSettings { settings: CaptureSettingsPayload },
    UpdateMode { mode: String },
    FailedUpload { upload_action: String, id: String },
}

#[derive(Clone)]
enum ViewMode {
    Progress,
    Picker,
    Trimmer,
    Capture,
}

struct AppState {
    window: Window,
    recording_indicator: Window,
    view_mode: Rc<RefCell<ViewMode>>,
    progress_view: ProgressView,
    picker_view: PickerView,
    trimmer_view: TrimmerView,
    capture_view: CaptureView,
}

impl AppState {
    fn new(initial_message: &str) -> Self {
        eprintln!("[OVERLAY] Creating window");
        let window = Window::builder()
            .title("Clips Overlay")
            .default_width(560)
            .default_height(180)
            .resizable(false)
            .build();
        
        // Make window background transparent
        window.add_css_class("transparent-window");

        eprintln!("[OVERLAY] Initializing layer shell");
        // Initialize as layer shell overlay
        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        
        // Get the display and list monitors to find the right one
        if let Some(display) = gdk::Display::default() {
            let n_monitors = display.monitors().n_items();
            eprintln!("[OVERLAY] Found {} monitors", n_monitors);
            
            // Try to find a landscape monitor (width > height) that's not the first one
            // This heuristic should find your right landscape monitor
            for i in 0..n_monitors {
                if let Some(monitor) = display.monitors().item(i).and_downcast::<gdk::Monitor>() {
                    let geometry = monitor.geometry();
                    eprintln!("[OVERLAY] Monitor {}: {}x{} at ({}, {})", 
                        i, geometry.width(), geometry.height(), geometry.x(), geometry.y());
                    
                    // Select the first landscape monitor that's not at x=0 (likely the right monitor)
                    if geometry.width() > geometry.height() && geometry.x() > 0 {
                        eprintln!("[OVERLAY] Selecting monitor {} as overlay target", i);
                        window.set_monitor(&monitor);
                        break;
                    }
                }
            }
        }
        
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Right, false);
        window.set_anchor(Edge::Bottom, false);
        // Start with OnDemand so we don't steal focus from fullscreen games
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        window.set_margin(Edge::Top, 24);
        window.set_margin(Edge::Left, 24);
        window.set_margin(Edge::Right, 24);
        window.set_margin(Edge::Bottom, 0);
        window.set_exclusive_zone(-1);  // -1 means don't reserve space, allow overlap
        
        // Apply global CSS for transparent window
        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_data(
            ".transparent-window {\
                \n  background-color: transparent;\
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
        
        eprintln!("[OVERLAY] Creating progress view");
        let progress_view = ProgressView::new(initial_message);
        eprintln!("[OVERLAY] Creating picker view");
        let picker_view = PickerView::new();
        eprintln!("[OVERLAY] Creating trimmer view");
        let trimmer_view = TrimmerView::new();
        eprintln!("[OVERLAY] Creating capture view");
        let capture_view = CaptureView::new();

        eprintln!("[OVERLAY] Setting initial view");
        // Start with progress view
        window.set_child(Some(progress_view.container()));

        // Create recording indicator window
        let recording_indicator = Window::builder()
            .title("Recording Indicator")
            .default_width(20)
            .default_height(20)
            .resizable(false)
            .decorated(false)
            .build();
        
        recording_indicator.init_layer_shell();
        recording_indicator.set_layer(Layer::Overlay);
        recording_indicator.set_anchor(Edge::Top, true);
        recording_indicator.set_anchor(Edge::Right, true);
        recording_indicator.set_anchor(Edge::Left, false);
        recording_indicator.set_anchor(Edge::Bottom, false);
        recording_indicator.set_keyboard_mode(KeyboardMode::None);
        recording_indicator.set_exclusive_zone(-1);
        recording_indicator.set_margin(Edge::Top, 24);
        recording_indicator.set_margin(Edge::Right, 24);
        
        // Create a simple red circle indicator
        let indicator_area = DrawingArea::builder()
            .width_request(20)
            .height_request(20)
            .build();
        
        indicator_area.set_draw_func(|_area, cr, width, height| {
            // Clear background
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cr.paint().unwrap();
            
            // Draw red circle
            let center_x = width as f64 / 2.0;
            let center_y = height as f64 / 2.0;
            let radius = (width.min(height) as f64 / 2.0) - 2.0;
            
            cr.set_source_rgb(1.0, 0.0, 0.0); // Red
            cr.arc(center_x, center_y, radius, 0.0, 2.0 * std::f64::consts::PI);
            cr.fill().unwrap();
            
            // Add a subtle white border
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.0);
            cr.arc(center_x, center_y, radius, 0.0, 2.0 * std::f64::consts::PI);
            cr.stroke().unwrap();
        });
        
        recording_indicator.set_child(Some(&indicator_area));
        recording_indicator.hide(); // Start hidden

        Self {
            window,
            recording_indicator,
            view_mode: Rc::new(RefCell::new(ViewMode::Progress)),
            progress_view,
            picker_view,
            trimmer_view,
            capture_view,
        }
    }

    fn switch_to_progress(&self) {
        *self.view_mode.borrow_mut() = ViewMode::Progress;
        self.window.set_child(Some(self.progress_view.container()));
        
        // Reset layer shell properties for progress view
        self.window.set_keyboard_mode(KeyboardMode::None);
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Left, true);
        self.window.set_anchor(Edge::Right, false);
        self.window.set_anchor(Edge::Bottom, false);
        
        // Resize window to fit progress view content (~560x180)
        self.window.set_default_size(560, 180);
        self.window.queue_resize();
    }

    fn switch_to_picker(&self) {
        *self.view_mode.borrow_mut() = ViewMode::Picker;
        self.window.set_child(Some(self.picker_view.container()));
        
        // Adjust layer shell properties for picker view (needs keyboard input)
        // Keep it anchored to top-left like progress view
        self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Left, true);
        self.window.set_anchor(Edge::Right, false);
        self.window.set_anchor(Edge::Bottom, false);
        
        // Resize window to fit picker view content (reduced without video preview)
        self.window.set_default_size(500, 400);
        self.window.queue_resize();
    }

    fn switch_to_trimmer(&self) {
        *self.view_mode.borrow_mut() = ViewMode::Trimmer;
        self.window.set_child(Some(self.trimmer_view.container()));
        
        // Trimmer needs keyboard input for controls
        self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Left, true);
        self.window.set_anchor(Edge::Right, false);
        self.window.set_anchor(Edge::Bottom, false);
        
        // Resize window to fit trimmer view content (~900x850)
        self.window.set_default_size(900, 850);
        self.window.queue_resize();
    }

    fn switch_to_capture(&self) {
        *self.view_mode.borrow_mut() = ViewMode::Capture;
        self.window.set_child(Some(self.capture_view.container()));

        // Use OnDemand so we don't steal focus from fullscreen games
        // Keyboard input will work when user clicks on the overlay
        self.window.set_keyboard_mode(KeyboardMode::OnDemand);
        self.window.set_anchor(Edge::Top, true);
        self.window.set_anchor(Edge::Left, true);
        self.window.set_anchor(Edge::Right, false);
        self.window.set_anchor(Edge::Bottom, false);
        
        // Set window size and make sure it doesn't grab input beyond its visible area
        self.window.set_default_size(450, 300);  // Smaller since settings are collapsed by default
        self.window.queue_resize();
    }
    
    fn set_recording_indicator_visible(&self, visible: bool) {
        if visible {
            self.recording_indicator.show();
        } else {
            self.recording_indicator.hide();
        }
    }

    fn handle_command(&self, cmd: Command) {
        match cmd {
            Command::Progress { stage, fraction, detail } => {
                self.switch_to_progress();
                self.progress_view.update(&stage, fraction, &detail);
            }
            Command::ShowPicker { preview_path, default_title, default_game, available_channels } => {
                self.switch_to_picker();
                self.picker_view.show(
                    preview_path.as_deref(),
                    &default_title,
                    &default_game,
                    &available_channels,
                );
            }
            Command::ShowTrimmer { video_path, duration } => {
                self.switch_to_trimmer();
                self.trimmer_view.show(&video_path, duration);
            }
            Command::ShowCapture { status } => {
                self.switch_to_capture();
                self.capture_view.update_status(&status);
            }
            Command::CaptureStatus { status } => {
                self.capture_view.update_status(&status);
            }
            Command::SetVisibility { visible } => {
                if visible {
                    self.window.present();
                } else {
                    self.window.hide();
                }
            }
            Command::Quit => {
                self.window.close();
            }
        }
    }
}

fn main() {
    eprintln!("[OVERLAY] Starting overlay application");
    let initial_message = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Work in progress".to_string());
    
    eprintln!("[OVERLAY] Initial message: {}", initial_message);
    
    eprintln!("[OVERLAY] Initializing GTK");
    if let Err(err) = gtk::init() {
        eprintln!("[OVERLAY] Failed to initialize GTK: {err}");
        std::process::exit(1);
    }
    
    eprintln!("[OVERLAY] Creating app state");
    let state = AppState::new(&initial_message);
    eprintln!("[OVERLAY] AppState created");
    
    // Set up stdin reader in a separate thread that communicates via channels
    let (tx, rx) = std::sync::mpsc::channel::<Option<Command>>();
    
    thread::spawn(move || {
        let stdin = io::stdin();
        eprintln!("[OVERLAY] Stdin reader thread started");
        for line in stdin.lock().lines() {
            match line {
                Ok(text) if !text.is_empty() => {
                    eprintln!("[OVERLAY] Received command: {}", text);
                    match serde_json::from_str::<Command>(&text) {
                        Ok(cmd) => {
                            let _ = tx.send(Some(cmd));
                        }
                        Err(e) => {
                            eprintln!("[OVERLAY] Failed to parse command: {}", e);
                        }
                    }
                }
                Ok(_) => {
                    // Empty line, quit
                    eprintln!("[OVERLAY] Received empty line, exiting");
                    let _ = tx.send(None);
                    break;
                }
                Err(e) => {
                    eprintln!("[OVERLAY] Error reading from stdin: {}, exiting", e);
                    let _ = tx.send(None);
                    break;
                }
            }
        }
        eprintln!("[OVERLAY] Stdin reader thread ended");
    });
    
    // Handle commands in the main GTK thread using idle_add
    let window_clone = state.window.clone();
    let state_rc = std::rc::Rc::new(state);
    let state_clone = state_rc.clone();
    
    glib::idle_add_local(move || {
        match rx.try_recv() {
            Ok(Some(cmd)) => {
                state_clone.handle_command(cmd);
                glib::ControlFlow::Continue
            }
            Ok(None) => {
                window_clone.close();
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                glib::ControlFlow::Continue
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                window_clone.close();
                glib::ControlFlow::Break
            }
        }
    });

    // Set up picker response handler
    let state_clone = state_rc.clone();
    state_rc.picker_view.on_submit(move |result| {
        let response = Response::PickerResult {
            title: result.title,
            game: result.game,
            action: result.action,
            channels: result.channels,
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
        state_clone.switch_to_progress();
    });

    let state_clone2 = state_rc.clone();
    state_rc.picker_view.on_cancel(move || {
        let response = Response::Cancelled;
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
        // Don't close the window, just hide it and go back to capture view
        state_clone2.switch_to_capture();
        state_clone2.window.hide();
    });

    // Set up trimmer response handler
    let state_clone = state_rc.clone();
    state_rc.trimmer_view.on_submit(move |result| {
        let response = Response::TrimmerResult {
            start_time: result.start_time,
            end_time: result.end_time,
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
        state_clone.switch_to_progress();
    });

    let state_clone3 = state_rc.clone();
    state_rc.trimmer_view.on_cancel(move || {
        let response = Response::Cancelled;
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
        // Pause video before hiding
        state_clone3.trimmer_view.pause_video();
        // Don't close the window, just hide it and go back to capture view
        state_clone3.switch_to_capture();
        state_clone3.window.hide();
    });

    // Capture view handlers
    let state_for_toggle = state_rc.clone();
    state_rc.capture_view.on_toggle(move |enable| {
        // Update recording indicator visibility
        state_for_toggle.set_recording_indicator_visible(enable);
        
        let response = Response::CaptureAction {
            action: CaptureActionPayload::Toggle { enable },
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
    });

    state_rc.capture_view.on_save(|duration_secs| {
        let response = Response::CaptureAction {
            action: CaptureActionPayload::Save { duration_secs },
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
    });

    state_rc.capture_view.on_apply_settings(|settings| {
        let response = Response::CaptureAction {
            action: CaptureActionPayload::UpdateSettings { settings },
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
    });
    
    state_rc.capture_view.on_mode_change(|mode| {
        let response = Response::CaptureAction {
            action: CaptureActionPayload::UpdateMode { mode },
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
    });
    
    state_rc.capture_view.on_failed_upload_action(|upload_action, id| {
        let response = Response::CaptureAction {
            action: CaptureActionPayload::FailedUpload { upload_action, id },
        };
        if let Ok(json) = serde_json::to_string(&response) {
            println!("{}", json);
            let _ = io::stdout().flush();
        }
    });

    eprintln!("[OVERLAY] Starting GTK main loop");
    let main_loop = glib::MainLoop::new(None, false);
    
    // Quit main loop when window closes
    let main_loop_clone = main_loop.clone();
    state_rc.window.connect_close_request(move |_| {
        eprintln!("[OVERLAY] Window close requested, quitting main loop");
        main_loop_clone.quit();
        glib::Propagation::Proceed
    });
    
    // Don't show window by default - let the main app control visibility
    eprintln!("[OVERLAY] Window initialized, waiting for visibility command");

    main_loop.run();
    eprintln!("[OVERLAY] GTK main loop exited");
}
