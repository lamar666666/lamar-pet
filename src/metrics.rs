use std::sync::atomic::AtomicU64;

#[derive(Default)]
pub struct Metrics {
    pub panel_frames: AtomicU64,
    pub pet_frames: AtomicU64,
    pub reminder_frames: AtomicU64,
}
