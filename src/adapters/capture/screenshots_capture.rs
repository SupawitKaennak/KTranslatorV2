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
        // 1. Get the screen info and release the lock immediately
        let screen = {
            let now = Instant::now();
            let mut cache_guard = self.cache.lock().unwrap();
            
            // Refresh cache if empty or older than 5 seconds (increased from 2s)
            if cache_guard.is_none() || now.duration_since(cache_guard.as_ref().unwrap().0) > Duration::from_secs(5) {
                if let Ok(fresh) = Screen::all() {
                    *cache_guard = Some((now, fresh));
                }
            }

            let screens = &cache_guard.as_ref().ok_or_else(|| anyhow::anyhow!("failed to init screens"))?.1;
            screens
                .iter()
                .find(|s| s.display_info.id == display_id)
                .or_else(|| screens.iter().find(|s| s.display_info.is_primary))
                .or_else(|| screens.first())
                .cloned() // Clone the Screen object so we can release the lock
                .ok_or_else(|| anyhow::anyhow!("no display found"))?
        };
        // Mutex lock is released here because `cache_guard` goes out of scope

        // 2. Perform heavy capture WITHOUT holding the lock.
        // CRITICAL: screen.capture() can BLOCK INDEFINITELY when a game is in
        // exclusive fullscreen (DXGI Desktop Duplication waits for a new frame
        // that never comes). We wrap it in a thread with a timeout to prevent
        // the entire translation pipeline from hanging.
        let full_img = {
            let (cap_tx, cap_rx) = std::sync::mpsc::channel();
            let screen_for_thread = screen.clone();
            std::thread::spawn(move || {
                let result = screen_for_thread.capture();
                let _ = cap_tx.send(result);
            });
            cap_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| anyhow::anyhow!("Screen capture timed out (3s) — game may be in exclusive fullscreen. Will retry."))?
                .context("capture full screen")?
        };
        
        // Rect.x and Rect.y are absolute desktop coordinates.
        // Convert to relative coordinates for the crop.
        let rel_x = (rect.x - screen.display_info.x).max(0) as u32;
        let rel_y = (rect.y - screen.display_info.y).max(0) as u32;
        let crop_w = rect.w.max(1) as u32;
        let crop_h = rect.h.max(1) as u32;

        // Ensure crop is within bounds
        let img_w = full_img.width();
        let img_h = full_img.height();
        
        let safe_x = rel_x.min(img_w.saturating_sub(1));
        let safe_y = rel_y.min(img_h.saturating_sub(1));
        let safe_w = crop_w.min(img_w - safe_x);
        let safe_h = crop_h.min(img_h - safe_y);

        // Perform the crop manually in memory
        // full_img is likely Rgba8 (4 bytes per pixel)
        let raw = full_img.into_raw();
        let mut cropped_data = Vec::with_capacity((safe_w * safe_h * 4) as usize);
        
        for row in 0..safe_h {
            let start = ((safe_y + row) * img_w + safe_x) as usize * 4;
            let end = start + (safe_w as usize * 4);
            if end <= raw.len() {
                cropped_data.extend_from_slice(&raw[start..end]);
            } else {
                // Padding if something goes wrong
                cropped_data.resize(cropped_data.len() + (safe_w as usize * 4), 0);
            }
        }

        Ok(FrameRgba {
            width: safe_w,
            height: safe_h,
            data: cropped_data,
        })
    }
}
