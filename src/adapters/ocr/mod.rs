pub mod windows_ocr;
#[cfg(not(windows))]
pub mod tesseract_ocr;
