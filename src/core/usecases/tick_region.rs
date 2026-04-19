use anyhow::Result;

use crate::core::{
    model::RegionSlot,
    ports::{FrameSource, OcrEngine, Translator},
};

pub fn tick_region(
    slot: &mut RegionSlot,
    capture: &dyn FrameSource,
    ocr: &dyn OcrEngine,
    translator: &dyn Translator,
    now_ms: u64,
) -> Result<()> {
    if !slot.enabled {
        return Ok(());
    }
    let Some(rect) = slot.rect else {
        return Ok(());
    };

    // throttle capture/OCR by refresh_ms
    if now_ms < slot.next_tick_at_ms {
        return Ok(());
    }
    slot.next_tick_at_ms = now_ms.saturating_add(slot.refresh_ms.max(50));

    let frame = capture.capture_rect(rect, slot.display_id)?;
    let text = ocr.recognize(&frame, slot.source_lang.as_ref())?;

    if text.trim().is_empty() {
        return Ok(());
    }

    // When OCR changes, mark as pending translation.
    if text != slot.last_ocr_text {
        slot.last_ocr_text = text.clone();
        slot.pending_text = text;
    }

    if slot.pending_text.is_empty() {
        return Ok(());
    }

    // Backoff/rate-limit on translate failures (e.g., 429).
    if now_ms < slot.translate_next_try_at_ms {
        return Ok(());
    }

    match translator.translate(
        &slot.pending_text,
        slot.source_lang.as_ref(),
        &slot.target_lang,
    ) {
        Ok(translated) => {
            slot.last_translation = translated;
            slot.pending_text.clear();
            slot.translate_backoff_ms = 0;
            slot.translate_next_try_at_ms = now_ms.saturating_add(slot.refresh_ms.max(200));
        }
        Err(e) => {
            let next = match slot.translate_backoff_ms {
                0 => 1_000,
                x => (x.saturating_mul(2)).min(60_000),
            };
            slot.translate_backoff_ms = next;
            slot.translate_next_try_at_ms = now_ms.saturating_add(next);
            return Err(e);
        }
    }
    Ok(())
}

