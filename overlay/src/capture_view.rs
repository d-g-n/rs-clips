use gtk::{Adjustment, Box, Button, ComboBoxText, Entry, Label, Notebook, Orientation, Separator, SpinButton, Switch};
use gtk::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedUploadEntry {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStatus {
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
pub struct CaptureSettings {
    pub buffer_seconds: u32,
    pub bitrate: u32,
    pub fps: u32,
    pub target: String,
    pub audio_tracks: Vec<String>,
}

type ToggleCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(bool) + 'static>>>>;
type SaveCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(u32) + 'static>>>>;
type SettingsCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(CaptureSettings) + 'static>>>>;
type ModeCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(String) + 'static>>>>;
type FailedUploadCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(String, String) + 'static>>>>; // (action, id)

pub struct CaptureView {
    container: Box,
    status_label: Label,
    running_switch: Switch,
    mode_switch: Switch,
    save_1m_button: Button,
    save_5m_button: Button,
    buffer_spin: SpinButton,
    bitrate_spin: SpinButton,
    fps_spin: SpinButton,
    target_combo: ComboBoxText,
    audio_entries: Vec<Entry>,
    failed_uploads_list: Box,
    toggle_callback: ToggleCallback,
    save_callback: SaveCallback,
    settings_callback: SettingsCallback,
    mode_callback: ModeCallback,
    failed_upload_callback: FailedUploadCallback,
}

impl CaptureView {
    pub fn new() -> Self {
        let outer = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        outer.set_halign(gtk::Align::Start);
        outer.set_valign(gtk::Align::Start);

        let container = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .build();
        container.add_css_class("capture-box");

        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_data(
            ".capture-box {\
                \n  padding: 16px 24px;\
                \n  border-radius: 12px;\
                \n  background-color: rgba(30, 30, 30, 0.9);\
                \n}\
                \n.capture-label {\
                \n  color: white;\
                \n  font-weight: 600;\
                \n  font-size: 14px;\
                \n  font-family: monospace;\
                \n  margin-top: 8px;\
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
                \n.save-button {\
                \n  min-height: 50px;\
                \n  font-size: 16px;\
                \n  font-weight: 700;\
                \n}\
                \nvalue label {\
                \n  color: white;\
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
                \nentry {\
                \n  background-color: rgba(50, 50, 50, 0.9);\
                \n  color: white;\
                \n  border: 1px solid rgba(80, 80, 80, 0.8);\
                \n  border-radius: 4px;\
                \n  padding: 6px;\
                \n  font-family: monospace;\
                \n}\
                \nnotebook {\
                \n  background-color: transparent;\
                \n}\
                \nnotebook > header {\
                \n  background-color: rgba(40, 40, 40, 0.95);\
                \n  border-radius: 6px 6px 0 0;\
                \n}\
                \nnotebook > header > tabs > tab {\
                \n  color: rgba(255, 255, 255, 0.7);\
                \n  padding: 8px 16px;\
                \n  font-family: monospace;\
                \n  font-weight: 600;\
                \n}\
                \nnotebook > header > tabs > tab:checked {\
                \n  color: white;\
                \n  background-color: rgba(60, 60, 60, 0.95);\
                \n}\
                \nnotebook > stack {\
                \n  background-color: rgba(35, 35, 35, 0.95);\
                \n  border-radius: 0 0 6px 6px;\
                \n}\
            ",
        );
        let display = outer.display();
        gtk::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Status bar at top - visually separated
        let status_bar = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        status_bar.add_css_class("status-bar");
        
        let status_label = Label::new(Some("Replay recorder is stopped"));
        status_label.add_css_class("status-text");
        status_label.set_halign(gtk::Align::Center);
        status_bar.append(&status_label);
        container.append(&status_bar);

        // Three tabs: Main, Failed Uploads, Settings
        let notebook = Notebook::new();
        notebook.set_margin_top(12);
        notebook.set_vexpand(false); // Don't expand vertically
        notebook.set_hexpand(false);  // Don't expand horizontally
        notebook.set_size_request(-1, -1); // Let it size to content

        // ===== TAB 1: Main (Replay toggle + Save buttons) =====
        let main_tab = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .vexpand(false) // Don't expand vertically
            .hexpand(false) // Don't expand horizontally
            .build();
        
        // Replay enabled toggle - more prominent
        let running_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .build();
        let running_label = Label::new(Some("Replay enabled"));
        running_label.set_halign(gtk::Align::Start);
        running_label.set_hexpand(true);
        running_label.add_css_class("capture-label");
        running_box.append(&running_label);
        let running_switch = Switch::new();
        running_switch.set_halign(gtk::Align::End);
        running_box.append(&running_switch);
        main_tab.append(&running_box);

        // Separator
        let separator = Separator::new(Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        main_tab.append(&separator);

        // Save buttons - bigger and more prominent
        let buttons_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .homogeneous(true)
            .vexpand(false) // Don't expand vertically
            .hexpand(false) // Don't expand horizontally
            .build();

        let save_1m_button = Button::with_label("Save 1m");
        save_1m_button.add_css_class("save-button");
        let save_5m_button = Button::with_label("Save 5m");
        save_5m_button.add_css_class("save-button");
        buttons_box.append(&save_1m_button);
        buttons_box.append(&save_5m_button);
        main_tab.append(&buttons_box);

        let main_label = Label::new(Some("Main"));
        notebook.append_page(&main_tab, Some(&main_label));
        
        // ===== TAB 2: Failed Uploads =====
        let failed_uploads_container = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .vexpand(false) // Don't expand vertically
            .build();
        
        let failed_uploads_list = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();
        
        failed_uploads_container.append(&failed_uploads_list);
        
        let failed_uploads_label = Label::new(Some("Failed Uploads"));
        notebook.append_page(&failed_uploads_container, Some(&failed_uploads_label));

        // ===== TAB 3: Settings =====
        let settings_box = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .vexpand(false) // Don't expand vertically
            .build();

        // Replay mode toggle
        let mode_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .margin_bottom(12)
            .build();
        let mode_label = Label::new(Some("Auto-start with game"));
        mode_label.set_halign(gtk::Align::Start);
        mode_label.set_hexpand(true);
        mode_label.add_css_class("capture-label");
        mode_box.append(&mode_label);
        let mode_switch = Switch::new();
        mode_switch.set_halign(gtk::Align::End);
        mode_box.append(&mode_switch);
        settings_box.append(&mode_box);

        // Separator
        let mode_separator = Separator::new(Orientation::Horizontal);
        mode_separator.set_margin_top(8);
        mode_separator.set_margin_bottom(12);
        settings_box.append(&mode_separator);

        // Capture target dropdown
        let target_label = Label::new(Some("Capture target"));
        target_label.set_halign(gtk::Align::Start);
        target_label.add_css_class("capture-label");
        settings_box.append(&target_label);

        let target_combo = ComboBoxText::new();
        // Add common capture targets
        target_combo.append(Some("screen"), "screen (current monitor)");
        target_combo.append(Some("DP-1"), "DP-1");
        target_combo.append(Some("DP-2"), "DP-2");
        target_combo.append(Some("HDMI-A-1"), "HDMI-A-1");
        target_combo.append(Some("portal"), "portal (screencasting)");
        target_combo.append(Some("region"), "region (select area)");
        target_combo.set_active_id(Some("screen"));
        settings_box.append(&target_combo);

        // Buffer length
        let buffer_label = Label::new(Some("Buffer length (seconds)"));
        buffer_label.set_halign(gtk::Align::Start);
        buffer_label.add_css_class("capture-label");
        settings_box.append(&buffer_label);

        let buffer_adjustment = Adjustment::new(300.0, 30.0, 3600.0, 30.0, 60.0, 0.0);
        let buffer_spin = SpinButton::builder()
            .adjustment(&buffer_adjustment)
            .digits(0)
            .build();
        settings_box.append(&buffer_spin);

        // Bitrate
        let bitrate_label = Label::new(Some("Bitrate (kbps)"));
        bitrate_label.set_halign(gtk::Align::Start);
        bitrate_label.add_css_class("capture-label");
        settings_box.append(&bitrate_label);

        let bitrate_adjustment = Adjustment::new(60000.0, 1000.0, 200000.0, 1000.0, 5000.0, 0.0);
        let bitrate_spin = SpinButton::builder()
            .adjustment(&bitrate_adjustment)
            .digits(0)
            .build();
        settings_box.append(&bitrate_spin);

        // Frame rate
        let fps_label = Label::new(Some("Frame rate"));
        fps_label.set_halign(gtk::Align::Start);
        fps_label.add_css_class("capture-label");
        settings_box.append(&fps_label);

        let fps_adjustment = Adjustment::new(60.0, 30.0, 240.0, 5.0, 10.0, 0.0);
        let fps_spin = SpinButton::builder()
            .adjustment(&fps_adjustment)
            .digits(0)
            .build();
        settings_box.append(&fps_spin);

        // Audio tracks
        let audio_label = Label::new(Some("Audio tracks (one per line)"));
        audio_label.set_halign(gtk::Align::Start);
        audio_label.add_css_class("capture-label");
        settings_box.append(&audio_label);

        let audio_entries = vec![
            Entry::builder().placeholder_text("default_input").build(),
            Entry::builder().placeholder_text("app:discord").build(),
            Entry::builder()
                .placeholder_text("app-inverse:discord")
                .build(),
        ];

        for entry in &audio_entries {
            settings_box.append(entry);
        }

        // Apply button
        let apply_button = Button::with_label("Apply settings");
        settings_box.append(&apply_button);

        let settings_label = Label::new(Some("Settings"));
        notebook.append_page(&settings_box, Some(&settings_label));

        // Add notebook to container
        container.append(&notebook);
        
        // Set container to not expand beyond its content
        container.set_vexpand(false);
        container.set_hexpand(false);
        container.set_size_request(-1, -1); // Let it size to content

        outer.append(&container);

        let toggle_callback: ToggleCallback = Rc::new(RefCell::new(None));
        let save_callback: SaveCallback = Rc::new(RefCell::new(None));
        let settings_callback: SettingsCallback = Rc::new(RefCell::new(None));
        let mode_callback: ModeCallback = Rc::new(RefCell::new(None));
        let failed_upload_callback: FailedUploadCallback = Rc::new(RefCell::new(None));

        {
            let toggle_callback = toggle_callback.clone();
            running_switch.connect_state_set(move |_, state| {
                if let Some(cb) = toggle_callback.borrow().as_ref() {
                    cb(state);
                }
                gtk::glib::Propagation::Proceed
            });
        }

        {
            let save_callback = save_callback.clone();
            save_1m_button.connect_clicked(move |_| {
                if let Some(cb) = save_callback.borrow().as_ref() {
                    cb(60);
                }
            });
        }

        {
            let save_callback = save_callback.clone();
            save_5m_button.connect_clicked(move |_| {
                if let Some(cb) = save_callback.borrow().as_ref() {
                    cb(300);
                }
            });
        }

        {
            let settings_callback = settings_callback.clone();
            let buffer_spin = buffer_spin.clone();
            let bitrate_spin = bitrate_spin.clone();
            let fps_spin = fps_spin.clone();
            let target_combo = target_combo.clone();
            let audio_entries = audio_entries.clone();
            apply_button.connect_clicked(move |_| {
                let target = target_combo
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "screen".to_string());
                let settings = CaptureSettings {
                    buffer_seconds: buffer_spin.value() as u32,
                    bitrate: bitrate_spin.value() as u32,
                    fps: fps_spin.value() as u32,
                    target,
                    audio_tracks: audio_entries
                        .iter()
                        .map(|entry| entry.text().to_string())
                        .filter(|s| !s.trim().is_empty())
                        .collect(),
                };
                if let Some(cb) = settings_callback.borrow().as_ref() {
                    cb(settings);
                }
            });
        }

        {
            let mode_callback = mode_callback.clone();
            mode_switch.connect_state_set(move |_, state| {
                if let Some(cb) = mode_callback.borrow().as_ref() {
                    let mode_str = if state { "auto" } else { "manual" };
                    cb(mode_str.to_string());
                }
                gtk::glib::Propagation::Proceed
            });
        }

        Self {
            container: outer,
            status_label,
            running_switch,
            mode_switch,
            save_1m_button,
            save_5m_button,
            buffer_spin,
            bitrate_spin,
            fps_spin,
            target_combo,
            audio_entries,
            failed_uploads_list,
            toggle_callback,
            save_callback,
            settings_callback,
            mode_callback,
            failed_upload_callback,
        }
    }

    pub fn container(&self) -> &Box {
        &self.container
    }

    pub fn on_toggle<F>(&self, callback: F)
    where
        F: Fn(bool) + 'static,
    {
        *self.toggle_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }

    pub fn on_save<F>(&self, callback: F)
    where
        F: Fn(u32) + 'static,
    {
        *self.save_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }

    pub fn on_apply_settings<F>(&self, callback: F)
    where
        F: Fn(CaptureSettings) + 'static,
    {
        *self.settings_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }
    
    pub fn on_mode_change<F>(&self, callback: F)
    where
        F: Fn(String) + 'static,
    {
        *self.mode_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }
    
    pub fn on_failed_upload_action<F>(&self, callback: F)
    where
        F: Fn(String, String) + 'static, // (action, id)
    {
        *self.failed_upload_callback.borrow_mut() = Some(std::boxed::Box::new(callback));
    }

    pub fn update_status(&self, status: &CaptureStatus) {
        self.running_switch.set_active(status.running);
        self.mode_switch.set_active(status.replay_mode == "auto");
        self.buffer_spin.set_value(status.buffer_seconds as f64);
        self.bitrate_spin.set_value(status.bitrate as f64);
        self.fps_spin.set_value(status.fps as f64);
        
        // Set the combo box to the current target, or "screen" if not found
        if !self.target_combo.set_active_id(Some(&status.target)) {
            // Target not in list, default to screen
            self.target_combo.set_active_id(Some("screen"));
        }
        
        for (entry, value) in self.audio_entries.iter().zip(status.audio_tracks.iter()) {
            entry.set_text(value);
        }
        if status.audio_tracks.len() < self.audio_entries.len() {
            for entry in self.audio_entries.iter().skip(status.audio_tracks.len()) {
                entry.set_text("");
            }
        }

        let message = status
            .message
            .as_deref()
            .unwrap_or(if status.running {
                "Replay recorder ready"
            } else {
                "Replay recorder stopped"
            });
        self.status_label.set_text(message);
        
        // Handle saving state - disable save buttons when not running or currently saving
        let can_save = status.running && !status.is_saving;
        self.save_1m_button.set_sensitive(can_save);
        self.save_5m_button.set_sensitive(can_save);
        
        // Update failed uploads list
        self.update_failed_uploads_list(&status.failed_uploads);
    }
    
    fn update_failed_uploads_list(&self, uploads: &[FailedUploadEntry]) {
        // Clear existing children
        while let Some(child) = self.failed_uploads_list.first_child() {
            self.failed_uploads_list.remove(&child);
        }
        
        // Add each failed upload as a row
        for upload in uploads {
            let row = Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .build();
            row.add_css_class("failed-upload-row");
            
            // Video name label
            let name_label = Label::new(Some(&upload.display_name));
            name_label.set_halign(gtk::Align::Start);
            name_label.set_hexpand(true);
            name_label.add_css_class("capture-label");
            row.append(&name_label);
            
            // Buttons box (initially hidden, shown on hover)
            let buttons_box = Box::builder()
                .orientation(Orientation::Horizontal)
                .spacing(4)
                .build();
            buttons_box.set_visible(false);
            
            let retry_button = Button::with_label("Retry");
            let ignore_button = Button::with_label("Ignore");
            let discard_button = Button::with_label("Discard");
            
            buttons_box.append(&retry_button);
            buttons_box.append(&ignore_button);
            buttons_box.append(&discard_button);
            row.append(&buttons_box);
            
            // Setup hover behavior
            let motion_controller = gtk::EventControllerMotion::new();
            let buttons_box_enter = buttons_box.clone();
            motion_controller.connect_enter(move |_, _, _| {
                buttons_box_enter.set_visible(true);
            });
            let buttons_box_leave = buttons_box.clone();
            motion_controller.connect_leave(move |_| {
                buttons_box_leave.set_visible(false);
            });
            row.add_controller(motion_controller);
            
            // Wire up callbacks
            let upload_id = upload.id.clone();
            let callback = self.failed_upload_callback.clone();
            retry_button.connect_clicked(move |_| {
                if let Some(cb) = callback.borrow().as_ref() {
                    cb("retry".to_string(), upload_id.clone());
                }
            });
            
            let upload_id = upload.id.clone();
            let callback = self.failed_upload_callback.clone();
            ignore_button.connect_clicked(move |_| {
                if let Some(cb) = callback.borrow().as_ref() {
                    cb("ignore".to_string(), upload_id.clone());
                }
            });
            
            let upload_id = upload.id.clone();
            let callback = self.failed_upload_callback.clone();
            discard_button.connect_clicked(move |_| {
                if let Some(cb) = callback.borrow().as_ref() {
                    cb("discard".to_string(), upload_id.clone());
                }
            });
            
            self.failed_uploads_list.append(&row);
        }
    }
}
