use gtk::{Box, Button, CheckButton, Entry, Label, Orientation};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct PickerResult {
    pub title: String,
    pub game: String,
    pub action: String,
    pub channels: Vec<String>,
}

type SubmitCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn(PickerResult) + 'static>>>>;
type CancelCallback = Rc<RefCell<Option<std::boxed::Box<dyn Fn() + 'static>>>>;

pub struct PickerView {
    container: Box,
    title_entry: Entry,
    game_entry: Entry,
    channels_box: Box,
    channel_checkboxes: Rc<RefCell<Vec<(String, CheckButton)>>>,
    action_radio_upload: CheckButton,
    submit_callback: SubmitCallback,
    cancel_callback: CancelCallback,
}

impl PickerView {
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
        container.add_css_class("picker-box");

        // Apply CSS matching progress view style
        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_data(
            ".picker-box {\
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
                \n.picker-label {\
                \n  color: white;\
                \n  font-weight: 600;\
                \n  font-size: 14px;\
                \n  font-family: monospace;\
                \n  margin-top: 8px;\
                \n}\
                \n.picker-entry {\
                \n  background-color: rgba(50, 50, 50, 0.9);\
                \n  color: white;\
                \n  border: 1px solid rgba(80, 80, 80, 0.8);\
                \n  border-radius: 4px;\
                \n  padding: 6px;\
                \n  font-family: monospace;\
                \n}\
                \ncheckbutton, checkbutton label {\
                \n  color: white;\
                \n  font-family: monospace;\
                \n  font-size: 13px;\
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
            ",
        );
        let display = container.display();
        gtk::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Status bar at top
        let status_bar = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        status_bar.add_css_class("status-bar");
        
        let status_label = Label::new(Some("Select clip details"));
        status_label.add_css_class("status-text");
        status_label.set_halign(gtk::Align::Center);
        status_bar.append(&status_label);
        container.append(&status_bar);

        // Title entry
        let title_label = Label::new(Some("Title:"));
        title_label.set_halign(gtk::Align::Start);
        title_label.add_css_class("picker-label");
        container.append(&title_label);
        
        let title_entry = Entry::builder()
            .placeholder_text("Enter clip title")
            .build();
        title_entry.add_css_class("picker-entry");
        container.append(&title_entry);

        // Game entry
        let game_label = Label::new(Some("Game:"));
        game_label.set_halign(gtk::Align::Start);
        game_label.add_css_class("picker-label");
        container.append(&game_label);
        
        let game_entry = Entry::builder()
            .placeholder_text("Enter game name")
            .build();
        game_entry.add_css_class("picker-entry");
        container.append(&game_entry);

        // Audio channels
        let channels_label = Label::new(Some("Audio Channels:"));
        channels_label.set_halign(gtk::Align::Start);
        channels_label.add_css_class("picker-label");
        container.append(&channels_label);

        let channel_checkboxes: Rc<RefCell<Vec<(String, CheckButton)>>> = Rc::new(RefCell::new(Vec::new()));
        let channels_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .build();
        container.append(&channels_box);

        // Action selection
        let action_label = Label::new(Some("Action:"));
        action_label.set_halign(gtk::Align::Start);
        action_label.add_css_class("picker-label");
        container.append(&action_label);

        let action_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .build();

        let action_radio_upload = CheckButton::with_label("Upload to YouTube");
        let action_radio_move = CheckButton::with_label("Move (no upload)");
        action_radio_move.set_group(Some(&action_radio_upload));
        let action_radio_discard = CheckButton::with_label("Discard");
        action_radio_discard.set_group(Some(&action_radio_upload));

        action_box.append(&action_radio_upload);
        action_box.append(&action_radio_move);
        action_box.append(&action_radio_discard);
        container.append(&action_box);

        // Buttons
        let buttons_box = Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(gtk::Align::End)
            .margin_top(8)
            .build();

        let cancel_button = Button::with_label("Cancel");
        let ok_button = Button::with_label("OK");

        buttons_box.append(&cancel_button);
        buttons_box.append(&ok_button);
        container.append(&buttons_box);
        
        outer.append(&container);

        let submit_callback: SubmitCallback = Rc::new(RefCell::new(None));
        let cancel_callback: CancelCallback = Rc::new(RefCell::new(None));

        // Wire up OK button
        let title_entry_clone = title_entry.clone();
        let game_entry_clone = game_entry.clone();
        let channel_checkboxes_clone = channel_checkboxes.clone();
        let action_radio_upload_clone = action_radio_upload.clone();
        let action_radio_move_clone = action_radio_move.clone();
        let action_radio_discard_clone = action_radio_discard.clone();
        let submit_callback_clone = submit_callback.clone();

        ok_button.connect_clicked(move |_| {
            let title = title_entry_clone.text().to_string();
            let game = game_entry_clone.text().to_string();
            
            let channels: Vec<String> = channel_checkboxes_clone
                .borrow()
                .iter()
                .filter(|(_, cb)| cb.is_active())
                .map(|(name, _)| name.clone())
                .collect();

            let action = if action_radio_upload_clone.is_active() {
                "upload".to_string()
            } else if action_radio_move_clone.is_active() {
                "move".to_string()
            } else if action_radio_discard_clone.is_active() {
                "discard".to_string()
            } else {
                "upload".to_string()
            };

            let result = PickerResult {
                title,
                game,
                action,
                channels,
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
            title_entry,
            game_entry,
            channels_box,
            channel_checkboxes,
            action_radio_upload,
            submit_callback,
            cancel_callback,
        }
    }

    pub fn container(&self) -> &Box {
        &self.container
    }

    pub fn show(
        &self,
        _preview_path: Option<&str>,
        default_title: &str,
        default_game: &str,
        available_channels: &[String],
    ) {
        // Set default values
        self.title_entry.set_text(default_title);
        self.game_entry.set_text(default_game);

        // Clear and rebuild channel checkboxes
        self.channel_checkboxes.borrow_mut().clear();
        
        // Clear existing checkboxes
        while let Some(child) = self.channels_box.first_child() {
            self.channels_box.remove(&child);
        }
        
        // Add new checkboxes
        for channel in available_channels {
            let label = match channel.as_str() {
                "voice" => "Voice",
                "discord" => "Discord",
                "game" => "Game",
                other => other,
            };
            let checkbox = CheckButton::with_label(label);
            checkbox.set_active(true); // Default to all channels enabled
            self.channels_box.append(&checkbox);
            self.channel_checkboxes.borrow_mut().push((channel.clone(), checkbox));
        }

        // Set default action to upload
        self.action_radio_upload.set_active(true);
    }

    pub fn on_submit<F>(&self, callback: F)
    where
        F: Fn(PickerResult) + 'static,
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

