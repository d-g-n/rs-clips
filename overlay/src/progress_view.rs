use gtk::{Box, CssProvider, Label, Orientation, DrawingArea};
use gtk::prelude::*;
use gtk::cairo;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

pub struct ProgressView {
    container: Box,
    stage_label: Label,
    detail_label: Label,
    progress_bar: DrawingArea,
    fraction: Rc<RefCell<f32>>,
    start_time: Rc<RefCell<Instant>>,
    current_stage: Rc<RefCell<String>>,
}

impl ProgressView {
    pub fn new(initial_message: &str) -> Self {
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
        container.set_halign(gtk::Align::Start);
        container.set_valign(gtk::Align::Start);
        container.add_css_class("overlay-box");

        // Status bar at top
        let status_bar = Box::builder()
            .orientation(Orientation::Vertical)
            .build();
        status_bar.add_css_class("status-bar");
        
        let stage_label = Label::builder()
            .label(initial_message)
            .build();
        stage_label.add_css_class("status-text");
        stage_label.set_halign(gtk::Align::Center);
        status_bar.append(&stage_label);
        container.append(&status_bar);

        // Custom progress bar
        let progress_bar = DrawingArea::builder()
            .width_request(500)
            .height_request(40)
            .build();
        
        let fraction = Rc::new(RefCell::new(0.0));
        let fraction_draw = fraction.clone();
        
        progress_bar.set_draw_func(move |_area, cr, width, height| {
            let frac = *fraction_draw.borrow();
            
            // Background (unfilled part)
            cr.set_source_rgb(0.2, 0.2, 0.2);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
            
            // Filled part (animated gradient)
            if frac > 0.0 {
                let fill_width = (width as f64) * (frac as f64);
                
                // Create gradient for filled portion
                let gradient = cairo::LinearGradient::new(0.0, 0.0, fill_width, 0.0);
                gradient.add_color_stop_rgb(0.0, 0.3, 0.5, 0.7);
                gradient.add_color_stop_rgb(0.5, 0.4, 0.6, 0.8);
                gradient.add_color_stop_rgb(1.0, 0.3, 0.5, 0.7);
                
                cr.set_source(&gradient).unwrap();
                cr.rectangle(0.0, 0.0, fill_width, height as f64);
                let _ = cr.fill();
            }
            
            // Border
            cr.set_source_rgb(0.5, 0.5, 0.5);
            cr.set_line_width(2.0);
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.stroke();
            
            // Percentage text in center
            let pct = ((frac * 100.0) as f64).round() as i32;
            let text = format!("{}%", pct);
            
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(18.0);
            
            let extents = cr.text_extents(&text).unwrap();
            let text_x = (width as f64 - extents.width()) / 2.0 - extents.x_bearing();
            let text_y = (height as f64 - extents.height()) / 2.0 - extents.y_bearing();
            
            cr.move_to(text_x, text_y);
            let _ = cr.show_text(&text);
        });
        
        container.append(&progress_bar);

        // Detail label (e.g., "5% encoded", "Elapsed: 2s")
        let detail_label = Label::builder()
            .label("")
            .build();
        detail_label.add_css_class("detail-label");
        detail_label.set_xalign(0.0);
        container.append(&detail_label);

        outer.append(&container);

        // Apply CSS
        let css_provider = CssProvider::new();
        css_provider.load_from_data(
            ".overlay-box {\
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
                \n.detail-label {\
                \n  color: rgba(255, 255, 255, 0.8);\
                \n  font-weight: 500;\
                \n  font-size: 14px;\
                \n  margin-top: 8px;\
                \n  font-family: monospace;\
                \n}\
            ",
        );
        let display = outer.display();
        gtk::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let current_stage = Rc::new(RefCell::new(initial_message.to_string()));
        Self {
            container: outer,
            stage_label,
            detail_label,
            progress_bar,
            fraction,
            start_time: Rc::new(RefCell::new(Instant::now())),
            current_stage,
        }
    }

    pub fn container(&self) -> &Box {
        &self.container
    }

    pub fn update(&self, stage: &str, fraction: f32, detail: &str) {
        {
            let mut current_stage = self.current_stage.borrow_mut();
            if current_stage.as_str() != stage {
                *self.start_time.borrow_mut() = Instant::now();
                *current_stage = stage.to_string();
            }
        }
        // Update stage
        self.stage_label.set_text(stage);
        
        // Update fraction
        *self.fraction.borrow_mut() = fraction.clamp(0.0, 1.0);
        self.progress_bar.queue_draw();
        
        // Calculate elapsed time and estimated time remaining
        let elapsed = self.start_time.borrow().elapsed().as_secs();
        let elapsed_str = format_time(elapsed);
        
        let eta_str = if fraction > 0.01 {
            let total_estimated = (elapsed as f32) / fraction;
            let remaining = (total_estimated - elapsed as f32).max(0.0) as u64;
            format!(" • ETA: {}", format_time(remaining))
        } else {
            String::new()
        };
        
        // Update detail with timing info
        let detail_with_time = format!("{} • Elapsed: {}{}", detail, elapsed_str, eta_str);
        self.detail_label.set_text(&detail_with_time);
    }
}

fn format_time(seconds: u64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    }
}
