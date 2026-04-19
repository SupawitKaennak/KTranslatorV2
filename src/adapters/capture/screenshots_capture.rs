use anyhow::{Context, Result};
use screenshots::Screen;

use crate::core::{
    ports::{FrameRgba, FrameSource},
    types::Rect,
};

/// Real screen-capture adapter using the `screenshots` crate.
pub struct ScreenshotsCapture;

impl FrameSource for ScreenshotsCapture {
    fn capture_rect(&self, rect: Rect, display_id: u32) -> Result<FrameRgba> {
        let screens = Screen::all().context("enumerate screens")?;
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
