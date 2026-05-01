use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio, Child};
use std::io::{Write, BufRead, BufReader};
use std::sync::Arc;
use parking_lot::Mutex;
use base64::{Engine as _, engine::general_purpose};

use crate::core::{
    ports::{FrameRgba, OcrEngine, OcrTextLine},
    types::LanguageTag,
};

/// Adapter for PaddleOCR-json (https://github.com/hiroi-sora/PaddleOCR-json)
pub struct PaddleOcr {
    process: Arc<Mutex<Option<Child>>>,
    exe_path: String,
}

#[derive(Debug, Serialize)]
struct PaddleRequest {
    image_base64: String,
}

#[derive(Debug, Deserialize)]
struct PaddleResponse {
    code: i32,
    data: Option<Vec<PaddleItem>>,
}

#[derive(Debug, Deserialize)]
struct PaddleItem {
    text: String,
    #[serde(rename = "box")]
    points: Vec<Vec<i32>>, // [[x,y], [x,y], [x,y], [x,y]]
}

impl PaddleOcr {
    pub fn new(exe_path: String) -> Self {
        Self {
            process: Arc::new(Mutex::new(None)),
            exe_path,
        }
    }

    fn ensure_process(&self) -> Result<()> {
        let mut proc_guard = self.process.lock();
        if proc_guard.is_none() {
            if self.exe_path.is_empty() {
                return Err(anyhow!("PaddleOCR-json path is not configured in settings"));
            }

            let exe_path = std::path::Path::new(&self.exe_path);
            if !exe_path.exists() {
                return Err(anyhow!("PaddleOCR-json.exe not found at: {}\nPlease check the path in settings.", self.exe_path));
            }
            let working_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));

            use std::os::windows::process::CommandExt;
            let child = Command::new(&self.exe_path)
                .current_dir(working_dir)
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("Failed to spawn PaddleOCR-json process")?;
            
            *proc_guard = Some(child);
        }
        Ok(())
    }
}

impl OcrEngine for PaddleOcr {
    fn recognize(&self, frame: &FrameRgba, lang_hint: Option<&LanguageTag>) -> Result<String> {
        let lines = self.recognize_lines(frame, lang_hint)?;
        Ok(lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>().join("\n"))
    }

    fn recognize_lines(&self, frame: &FrameRgba, _lang_hint: Option<&LanguageTag>) -> Result<Vec<OcrTextLine>> {
        self.ensure_process()?;
        
        // Convert frame to JPEG/PNG for PaddleOCR
        // For simplicity, we'll encode as PNG via the image crate
        let img = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(frame.width, frame.height, frame.data.clone())
            .context("Failed to create image buffer")?;
        
        let mut buffer = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buffer, image::ImageFormat::Png).context("Failed to encode PNG")?;
        let b64 = general_purpose::STANDARD.encode(buffer.into_inner());

        let req = PaddleRequest { image_base64: b64 };
        let req_json = serde_json::to_string(&req)? + "\n";

        let mut proc_guard = self.process.lock();
        let child = proc_guard.as_mut().unwrap();
        
        let stdin = child.stdin.as_mut().context("No stdin for PaddleOCR")?;
        stdin.write_all(req_json.as_bytes())?;
        stdin.flush()?;

        let stdout = child.stdout.as_mut().context("No stdout for PaddleOCR")?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        
        // Skip any non-JSON lines (like startup status messages)
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            if line.is_empty() {
                *proc_guard = None;
                return Err(anyhow!("PaddleOCR process exited unexpectedly"));
            }
            if line.trim().starts_with('{') {
                break;
            }
        }

        let resp: PaddleResponse = serde_json::from_str(&line).context("Failed to parse PaddleOCR response")?;
        
        if resp.code != 100 {
             return Ok(vec![]); // No text or error
        }

        let mut out = Vec::new();
        if let Some(data) = resp.data {
            for item in data {
                // PaddleOCR returns 4 points. We use min/max for the rect.
                let min_x = item.points.iter().map(|p| p[0]).min().unwrap_or(0);
                let max_x = item.points.iter().map(|p| p[0]).max().unwrap_or(0);
                let min_y = item.points.iter().map(|p| p[1]).min().unwrap_or(0);
                let max_y = item.points.iter().map(|p| p[1]).max().unwrap_or(0);

                out.push(OcrTextLine {
                    text: item.text,
                    x: min_x as f32,
                    y: min_y as f32,
                    w: (max_x - min_x) as f32,
                    h: (max_y - min_y) as f32,
                });
            }
        }

        Ok(out)
    }
}
