#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    Detected,
    Transform,
    AwaitExport,
    Finalise,
    Upload,
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Detected => "Detected",
            Stage::Transform => "Transforming audio/video",
            Stage::AwaitExport => "Awaiting export",
            Stage::Finalise => "Finalising files",
            Stage::Upload => "Uploading to YouTube",
            Stage::Done => "Completed",
        }
    }
}

pub fn format_stage_detail(_stage: Stage, fraction: f32, suffix: &str) -> String {
    let pct = (fraction * 100.0).clamp(0.0, 100.0);
    format!("{:>3}% {suffix}", pct.round() as i32)
}
