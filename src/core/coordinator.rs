use std::sync::{mpsc, Arc};
use std::collections::HashMap;
use parking_lot::Mutex;
use crate::core::{
    model::AppModel,
    ports::{FrameSource, Translator},
    worker::{BgResult, SlotRuntimeState, smart_hash},
    text_cleaner::TextCleaner,
};
use crate::adapters::{
    ocr::windows_ocr::WindowsOcr,
};

pub struct BackgroundCoordinator {
    pub bg_tx: mpsc::Sender<BgResult>,
    pub bg_rx: mpsc::Receiver<BgResult>,
}

impl BackgroundCoordinator {
    pub fn new() -> Self {
        let (bg_tx, bg_rx) = mpsc::channel();
        Self { bg_tx, bg_rx }
    }

    pub fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    pub fn process_results(
        &self,
        model_arc: &Arc<Mutex<AppModel>>,
        slots_runtime: &mut Vec<SlotRuntimeState>,
        last_errors: &mut std::collections::BTreeMap<usize, String>,
        translation_cache: &Arc<Mutex<HashMap<(u64, Option<String>, String), (String, String)>>>,
    ) {
        while let Ok(result) = self.bg_rx.try_recv() {
            match result {
                BgResult::Done {
                    slot_idx,
                    language_version,
                    ocr_text,
                    translated,
                    frame_hash,
                    ocr_lines,
                } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        let mut model = model_arc.lock();
                        let slot = match model.slots.get_mut(slot_idx) {
                            Some(s) => s,
                            None => continue,
                        };

                        if language_version != slot.language_version {
                            runtime.busy = false;
                            runtime.processing = false;
                            runtime.first_unstable_at = 0; // Reset
                            slot.next_tick_at_ms = 0;
                            continue;
                        }

                        runtime.busy = false;
                        runtime.processing = false;
                        runtime.first_unstable_at = 0; // Reset on success
                        runtime.status = "Idle".to_string();
                        runtime.last_hash = frame_hash;

                        let now = Self::now_ms();
                        slot.next_tick_at_ms = now.saturating_add(slot.refresh_ms.max(500));

                        let new_ocr = ocr_text.trim();
                        let old_ocr = slot.last_ocr_text.trim();

                        if new_ocr.is_empty() {
                            slot.last_ocr_text = String::new();
                            slot.last_translation = String::new();
                            slot.last_ocr_lines.clear();
                            slot.last_trans_lines.clear();
                        } else if new_ocr != old_ocr {
                            slot.last_ocr_text = ocr_text.clone();
                            slot.last_translation = translated.clone();
                            slot.last_ocr_lines = ocr_lines.clone();
                            slot.last_trans_lines = Self::parse_numbered_lines(&translated, ocr_lines.len());

                            if frame_hash != 0 {
                                let cache_key = (frame_hash, slot.source_lang.as_ref().map(|l| l.0.clone()), slot.target_lang.0.clone());
                                translation_cache.lock().insert(cache_key, (ocr_text, translated));
                            }
                        } else {
                            if !translated.trim().is_empty() {
                                slot.last_trans_lines = Self::parse_numbered_lines(&translated, ocr_lines.len());
                                slot.last_ocr_lines = ocr_lines;
                                slot.last_translation = translated;
                            }
                        }
                    }
                    last_errors.remove(&slot_idx);
                }
                BgResult::Unchanged { slot_idx } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        runtime.busy = false;
                        runtime.status = "Idle".to_string();
                        runtime.first_unstable_at = 0; // Reset
                    }
                    let now = Self::now_ms();
                    let mut model = model_arc.lock();
                    if let Some(slot) = model.slots.get_mut(slot_idx) {
                        slot.next_tick_at_ms = now.saturating_add(slot.refresh_ms.max(200));
                    }
                }
                BgResult::HashChanged { slot_idx, new_hash } => {
                    let mut model = model_arc.lock();
                    let now = Self::now_ms();
                    if let Some(slot) = model.slots.get_mut(slot_idx) {
                        slot.stable_hash = new_hash;
                        slot.stable_since_ms = now;
                        slot.next_tick_at_ms = now.saturating_add(150);
                    }
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        runtime.busy = false;
                        runtime.status = "Settling...".to_string();
                        // Initialize first_unstable_at if it's 0
                        if runtime.first_unstable_at == 0 {
                            runtime.first_unstable_at = now;
                        }
                    }
                }
                BgResult::WaitingDebounce { slot_idx } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        runtime.busy = false;
                        runtime.status = "Waiting...".to_string();
                    }
                    let mut model = model_arc.lock();
                    if let Some(slot) = model.slots.get_mut(slot_idx) {
                        slot.next_tick_at_ms = Self::now_ms() + 50;
                    }
                }
                BgResult::CacheHit { slot_idx, language_version, ocr_text, translated, frame_hash, ocr_lines } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        let mut model = model_arc.lock();
                        let slot = match model.slots.get_mut(slot_idx) {
                            Some(s) => s,
                            None => continue,
                        };

                        if language_version != slot.language_version {
                            runtime.busy = false;
                            slot.next_tick_at_ms = 0;
                            continue;
                        }

                        runtime.busy = false;
                        runtime.status = "Idle (Cached)".to_string();
                        runtime.first_unstable_at = 0; // Reset
                        runtime.last_hash = frame_hash;

                        slot.last_ocr_text = ocr_text;
                        slot.last_translation = translated.clone();
                        slot.last_ocr_lines = ocr_lines; // Update positions!
                        
                        // Re-align cached translation to the current OCR lines
                        slot.last_trans_lines = Self::parse_numbered_lines(&translated, slot.last_ocr_lines.len());

                        slot.next_tick_at_ms = Self::now_ms() + slot.refresh_ms.max(200);
                    }
                }
                BgResult::StatusUpdate { slot_idx, status } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        runtime.status = status;
                    }
                }
                BgResult::Error { slot_idx, language_version, err } => {
                    if let Some(runtime) = slots_runtime.get_mut(slot_idx) {
                        let mut model = model_arc.lock();
                        let slot = match model.slots.get_mut(slot_idx) {
                            Some(s) => s,
                            None => continue,
                        };

                        runtime.busy = false;
                        runtime.processing = false;
                        runtime.status = "Error".to_string();

                        let friendly = if err.contains("quota") || err.contains("429") {
                            let secs = 30;
                            format!("Region {}: API quota exceeded — retrying in {secs}s", slot_idx + 1)
                        } else {
                            let first_line = err.lines().next().unwrap_or(&err).trim().to_string();
                            format!("Region {}: {first_line}", slot_idx + 1)
                        };
                        last_errors.insert(slot_idx, friendly);
                        
                        if language_version == slot.language_version {
                            slot.next_tick_at_ms = Self::now_ms() + 2000;
                        }
                    }
                }
            }
        }
    }

    pub fn tick(
        &self,
        model_arc: &Arc<Mutex<AppModel>>,
        slots_runtime: &mut Vec<SlotRuntimeState>,
        capture: &Arc<dyn FrameSource>,
        windows_ocr: &Arc<WindowsOcr>,
        translator: &Arc<dyn Translator + Send + Sync>,
        translation_cache: &Arc<Mutex<HashMap<(u64, Option<String>, String), (String, String)>>>,
        text_translation_cache: &Arc<Mutex<HashMap<(u64, Option<String>, String), String>>>,
    ) {
        let now = Self::now_ms();
        let snapshot = { model_arc.lock().clone() };
        if !snapshot.running { return; }

        for (i, slot) in snapshot.slots.iter().enumerate() {
            if !slot.enabled || slot.rect.is_none() { continue; }

            if slots_runtime.len() <= i {
                slots_runtime.push(SlotRuntimeState::new());
            }

            // Language change detection logic (moved from app.rs)
            let cur_src = slot.source_lang.as_ref().map(|l| l.0.clone());
            let cur_tgt = slot.target_lang.0.clone();
            let lang_changed = slots_runtime[i].last_langs != (cur_src.clone(), cur_tgt.clone());
            if lang_changed {
                slots_runtime[i].last_langs = (cur_src, cur_tgt);
                slots_runtime[i].last_hash = 0;
                translation_cache.lock().clear();
                text_translation_cache.lock().clear();
                
                let mut model = model_arc.lock();
                if let Some(m_slot) = model.slots.get_mut(i) {
                    m_slot.language_version = m_slot.language_version.wrapping_add(1);
                    m_slot.last_trans_lines.clear();
                    m_slot.last_ocr_lines.clear();
                    m_slot.last_translation.clear();
                    m_slot.last_ocr_text.clear();
                    m_slot.next_tick_at_ms = 0;
                    m_slot.stable_hash = 0;
                    m_slot.stable_since_ms = 0;
                }
                slots_runtime[i].last_hash = 1;
                continue;
            }

            if slots_runtime[i].busy || now < slot.next_tick_at_ms {
                continue;
            }

            slots_runtime[i].busy = true;
            {
                let mut m = model_arc.lock();
                if let Some(s) = m.slots.get_mut(i) {
                    s.next_tick_at_ms = u64::MAX;
                }
            }

            let rect = slot.rect.unwrap();
            let display_id = slot.display_id;
            let source_lang = slot.source_lang.clone();
            let target_lang = slot.target_lang.clone();
            let capture = capture.clone();
            let windows_ocr = windows_ocr.clone();
            let translator = translator.clone();
            let tx = self.bg_tx.clone();
            let prev_hash = slots_runtime[i].last_hash;
            let stable_hash = slot.stable_hash;
            let stable_since_ms = slot.stable_since_ms;
            let language_version = slot.language_version;
            let cache_arc = translation_cache.clone();
            let text_cache_arc = text_translation_cache.clone();
            let first_unstable_at = slots_runtime[i].first_unstable_at;

            std::thread::spawn(move || {
                let _ = tx.send(BgResult::StatusUpdate { slot_idx: i, status: "Capturing...".to_string() });
                let tx_for_panic = tx.clone();
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    let tx_inner = tx.clone();
                    let result = (|| -> anyhow::Result<BgResult> {
                        let frame = capture.capture_rect(rect, display_id)?;
                        let _ = tx_inner.send(BgResult::StatusUpdate { slot_idx: i, status: "Hashing...".to_string() });
                        let hash = smart_hash(&frame.data);
                        let now = Self::now_ms();

                        // Stability Logic
                        let is_changing = hash != stable_hash;
                        let unstable_dur = now.saturating_sub(stable_since_ms);
                        
                        // If we've been unstable for a long time (e.g. 1.5s), FORCE proceed.
                        let unstable_since_start = if first_unstable_at == 0 { 0 } else { now.saturating_sub(first_unstable_at) };
                        let force_proceed = unstable_since_start > 1500; 

                        if is_changing && !force_proceed && unstable_dur < 500 {
                            return Ok(BgResult::HashChanged { slot_idx: i, new_hash: hash });
                        }
                        if !force_proceed && unstable_dur < 400 {
                            return Ok(BgResult::WaitingDebounce { slot_idx: i });
                        }
                        if hash == prev_hash && prev_hash != 0 && !force_proceed {
                            return Ok(BgResult::Unchanged { slot_idx: i });
                        }

                        let _ = tx_inner.send(BgResult::StatusUpdate { slot_idx: i, status: "OCR...".to_string() });
                        let ocr_lines = windows_ocr.recognize_lines(&frame, source_lang.as_ref())?;
                        
                        // --- Grouping disabled as it caused UI chaos and AI confusion ---
                        // let ocr_lines = Self::group_ocr_lines(raw_ocr_lines);

                        let cache_key = (hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                        {
                            let cache = cache_arc.lock();
                            if let Some((ocr, tra)) = cache.get(&cache_key) {
                                return Ok(BgResult::CacheHit {
                                    slot_idx: i, language_version, ocr_text: ocr.clone(), translated: tra.clone(), frame_hash: hash, ocr_lines
                                });
                            }
                        }
                        
                        let raw_ocr_text = ocr_lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
                        let ocr_text = TextCleaner::clean(&raw_ocr_text);

                        if ocr_text.is_empty() {
                            return Ok(BgResult::Done {
                                slot_idx: i, language_version, ocr_text: String::new(), translated: String::new(), frame_hash: hash, ocr_lines: Vec::new(),
                            });
                        }

                        // --- NEW: Aggressive Text-Level Cache Check ---
                        // Even if the frame hash changed, if the OCR text is identical to what we
                        // already translated, reuse it and update the frame-hash cache.
                        {
                            let text_hash = {
                                let mut h: u64 = 0xcbf29ce484222325;
                                for b in ocr_text.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
                                h
                            };
                            let tc_key = (text_hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                            let cached = { let tc = text_cache_arc.lock(); tc.get(&tc_key).cloned() };
                            
                            if let Some(cached_trans) = cached {
                                // Update frame-hash cache so next time we hit the fast-path earlier
                                let mut fc = cache_arc.lock();
                                fc.insert(cache_key, (ocr_text.clone(), cached_trans.clone()));
                                
                                return Ok(BgResult::Done {
                                    slot_idx: i, language_version, ocr_text, translated: cached_trans, frame_hash: hash, ocr_lines
                                });
                            }
                        }

                        if source_lang.as_ref().map(|s| s.0 == target_lang.0).unwrap_or(false) {
                            let mut cache = cache_arc.lock();
                            cache.insert(cache_key, (ocr_text.clone(), ocr_text.clone()));
                            return Ok(BgResult::Done {
                                slot_idx: i, language_version, ocr_text: ocr_text.clone(), translated: ocr_text, frame_hash: hash, ocr_lines,
                            });
                        }

                        let _ = tx_inner.send(BgResult::StatusUpdate { slot_idx: i, status: "Translating (AI)...".to_string() });
                        let translated = translator.translate(&ocr_text, source_lang.as_ref(), &target_lang)?;

                        {
                            let text_hash = {
                                let mut h: u64 = 0xcbf29ce484222325;
                                for b in ocr_text.as_bytes() { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); }
                                h
                            };
                            let text_cache_key = (text_hash, source_lang.as_ref().map(|l| l.0.clone()), target_lang.0.clone());
                            let mut frame_cache = cache_arc.lock();
                            frame_cache.insert(cache_key, (ocr_text.clone(), translated.clone()));
                            drop(frame_cache);
                            let mut text_cache = text_cache_arc.lock();
                            text_cache.insert(text_cache_key, translated.clone());
                        }
                        Ok(BgResult::Done { slot_idx: i, language_version, ocr_text, translated, frame_hash: hash, ocr_lines })
                    })();

                    match result {
                        Ok(res) => { let _ = tx.send(res); }
                        Err(e) => {
                            let _ = tx.send(BgResult::Error { slot_idx: i, language_version, err: format!("{e:#}") });
                        }
                    }
                }));

                if res.is_err() {
                    let _ = tx_for_panic.send(BgResult::Error {
                        slot_idx: i, language_version, err: "Background thread panicked (system error)".to_string(),
                    });
                }
            });
        }
    }

    /// Parse a numbered translation response back into a Vec aligned to
    /// the original OCR lines.
    fn parse_numbered_lines(raw: &str, ocr_count: usize) -> Vec<String> {
        if ocr_count == 0 {
            return vec![];
        }

        let mut result = vec![String::new(); ocr_count];
        // Stricter regex to ensure we don't accidentally include the number in the content
        let re_numbered = regex::Regex::new(r"^\s*(\d+)[\.\):\->\s]+(.*)$").unwrap();

        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }

            if let Some(caps) = re_numbered.captures(line) {
                if let Ok(num) = caps[1].parse::<usize>() {
                    if num > 0 && num <= ocr_count {
                        let mut content = caps[2].trim().to_string();
                        // Strip leading junk that AI sometimes adds
                        content = content.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ' || c == '-').trim().to_string();
                        
                        if result[num - 1].is_empty() || result[num - 1].len() < content.len() {
                            result[num - 1] = content;
                        }
                    }
                }
            } else {
                for i in 0..ocr_count {
                    if result[i].is_empty() {
                        result[i] = line.to_string();
                        break;
                    }
                }
            }
        }

        for s in result.iter_mut() {
            *s = TextCleaner::clean(s);
        }
        result
    }
}
