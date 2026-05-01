pub mod windows_ocr;
pub mod paddle_ocr;
#[cfg(not(windows))]
pub mod tesseract_ocr;
