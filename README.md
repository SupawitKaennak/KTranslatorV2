# KTranslator (Gemini Powered)

A modern, fast, and user-friendly screen translator for Windows. This tool uses **Offline Windows OCR** for text recognition and **Google Gemini AI** for high-quality, context-aware translation.

<img width="559" height="333" alt="{C93780CF-C104-4925-8ECD-1726E73F654D}" src="https://github.com/user-attachments/assets/6855e97f-9d64-4574-a113-ede638fd1443" /><!-- Use your generate_image tool or capture a real one if needed, but for now placeholder is fine if I don't have a final capture -->

## ✨ Features

- **🚀 Modern UI/UX:** Clean, card-based interface with dark/light mode support.
- **📺 Overlay Mode:** View translated text directly on top of your screen content (perfect for movies/games).
- **🖱️ Mouse Passthrough:** Interact with windows behind the translation overlay without interruption.
- **🌐 Multi-Region Support:** Translate multiple parts of your screen simultaneously with different language settings.
- **⚡ Smart Caching:** Saves API costs and improves performance by only translating when screen content changes.
- **🧠 Gemini Powered:** High-quality translations that understand context better than standard engines.

## 📋 Prerequisites

- **OS:** Windows 10 or Windows 11.
- **Language Packs:** Ensure you have the Windows Language Pack installed for the languages you want to translate FROM (check Windows Settings > Time & Language > Language).
- **Rust:** [Install Rust](https://rustup.rs/) to build the project.
- **Gemini API Key:** Obtain an API key from the [Google AI Studio](https://aistudio.google.com/).

## 🚀 Installation & Running

1. **Clone the repository:**
   ```bash
    git clone https://github.com/yourusername/ktranslator.git
    cd ktranslator
   ```

2. **Run the application:**
   ```bash
   cargo run --release
   ```

## 📖 How to Use

1. **Configure Gemini:**
   - Click the ⚙ (Gear) icon in the top right.
   - Paste your **Gemini API Key**.
   - Select your preferred model (e.g., `gemini-1.5-flash` for speed).
2. **Add a Region:**
   - Click **➕ Add Region**.
   - Click **Select Area** to drag and select the part of the screen you want to watch.
3. **Translate:**
   - Set the **From** and **To** languages.
   - Toggle **Active** via the Start/Stop button at the top.
   - (Optional) Enable **📺 Overlay Mode** to see text in-place.
4. **Overlay Management:**
   - You can drag the green frame box (if visible) to move the translation area in real-time.

## 🛠️ Tech Stack

- **UI Framework:** [egui](https://github.com/emilk/egui) with `eframe`.
- **OCR:** Native Windows.Media.Ocr (via the `windows` crate).
- **Translation:** Google Gemini API.
- **Capture:** `screenshots` crate.

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.
