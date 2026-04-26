use anyhow::{Context, Result};
use screenshots::Screen;

use crate::core::{
    ports::{FrameRgba, FrameSource},
    types::Rect,
};

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Real screen-capture adapter using the `screenshots` crate.
/// Includes an internal cache for screen enumeration to avoid heavy DPI/GDI calls every frame.
pub struct ScreenshotsCapture {
    cache: Mutex<Option<(Instant, Vec<Screen>)>>,
}

impl ScreenshotsCapture {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(None),
        }
    }
}

impl FrameSource for ScreenshotsCapture {
    fn capture_rect(&self, rect: Rect, display_id: u32) -> Result<FrameRgba> {
        let now = Instant::now();
        let mut cache_guard = self.cache.lock().unwrap();
        
        // Refresh cache if empty or older than 2 seconds
        let screens = if let Some((last_refresh, cached_screens)) = &*cache_guard {
            if now.duration_since(*last_refresh) > Duration::from_secs(2) {
                let fresh = Screen::all().context("enumerate screens")?;
                *cache_guard = Some((now, fresh));
                &cache_guard.as_ref().unwrap().1
            } else {
                cached_screens
            }
        } else {
            let fresh = Screen::all().context("enumerate screens")?;
            *cache_guard = Some((now, fresh));
            &cache_guard.as_ref().unwrap().1
        };

        let screen = screens
            .iter()
            .find(|s| s.display_info.id == display_id)
            .or_else(|| screens.iter().find(|s| s.display_info.is_primary))
            .or_else(|| screens.first())
            .ok_or_else(|| anyhow::anyhow!("no display found"))?;

        // Rect.x and Rect.y are absolute desktop coordinates.
        // Screen::capture_area expects coordinates relative to this specific screen's origin.
        let rel_x = rect.x - screen.display_info.x;
        let rel_y = rect.y - screen.display_info.y;

        let img = screen
            .capture_area(rel_x, rel_y, rect.w.max(1) as u32, rect.h.max(1) as u32)
            .context("capture screen area")?;

        let w = img.width();
        let h = img.height();
        let data = img.into_raw(); // RGBA8

        Ok(FrameRgba {
            width: w,
            height: h,
            data,
        })
    }
}
