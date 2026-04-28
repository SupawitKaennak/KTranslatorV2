use crate::core::ports::OcrTextLine;
use std::sync::Arc;

/// Results from background worker threads sent back to the main UI loop.
pub enum BgResult {
    /// Combined OCR + Translation completed successfully.
    Done {
        slot_idx: usize,
        language_version: u32,
        ocr_text: String,
        translated: String,
        frame_hash: u64,
        /// Per-line OCR bounding boxes for positional overlay rendering.
        ocr_lines: Vec<OcrTextLine>,
    },
    /// The captured frame is identical to the previous one — skip API call.
    Unchanged {
        slot_idx: usize,
    },
    /// The screen is changing. Update the stable hash tracker and skip API.
    HashChanged {
        slot_idx: usize,
        new_hash: u64,
    },
    /// The screen is stable but we are waiting for the debounce duration.
    WaitingDebounce {
        slot_idx: usize,
    },
    /// The frame matches a cached translation.
    CacheHit {
        slot_idx: usize,
        language_version: u32,
        ocr_text: String,
        translated: String,
        frame_hash: u64,
    },
    /// Background thread is now engaging Gemini or OCR (heavy work)
    Translating {
        slot_idx: usize,
    },
    /// Direct status update for the UI spinner/label
    StatusUpdate {
        slot_idx: usize,
        status: String,
    },
    /// An error occurred during OCR / Translation.
    Error {
        slot_idx: usize,
        language_version: u32,
        err: String,
    },
}

// ---------------------------------------------------------------------------
// Runtime state for each translation slot
// ---------------------------------------------------------------------------

pub struct SlotRuntimeState {
    /// True if the slot has a background task running (capture or API)
    pub busy: bool,
    /// True if the slot is currently waiting for an AI response
    pub processing: bool,
    /// Human-readable status shown in the UI
    pub status: String,
    /// Hash of the last captured frame to detect changes
    pub last_hash: u64,
    /// Native HWND of the overlay window for Win32 transparency
    pub overlay_hwnd: Arc<std::sync::atomic::AtomicIsize>,
    /// Track language changes to invalidate caches
    pub last_langs: (Option<String>, String),
}

impl SlotRuntimeState {
    pub fn new() -> Self {
        Self {
            busy: false,
            processing: false,
            status: "Idle".to_string(),
            last_hash: 0,
            overlay_hwnd: Arc::new(std::sync::atomic::AtomicIsize::new(0)),
            last_langs: (None, String::new()),
        }
    }
}

/// Smart hash converts RGBA to thresholded grayscale before hashing.
/// This prevents minor lighting/background particle changes from triggering text translation.
pub fn smart_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    
    // Sample every 4th pixel (16 bytes) for a balance of speed and precision
    let step: usize = 16;
    let mut i = 0;
    while i + 2 < data.len() {
        let r = data[i] as f32;
        let g = data[i+1] as f32;
        let b = data[i+2] as f32;
        
        let lum = 0.299 * r + 0.587 * g + 0.114 * b;
        let bw = if lum > 128.0 { 1u8 } else { 0u8 };
        
        h ^= bw as u64;
        h = h.wrapping_mul(0x100000001b3);
        
        i += step;
    }
    h
}
