# KTranslator V2

A high-performance, real-time screen translator written in Rust.

## ✨ Features

- **Real-time Translation:** Captures screen regions and translates text instantly.
- **Multi-Provider Support:**
  - **Gemini:** Google's powerful AI (Online).
  - **Groq:** Ultra-fast cloud translation (supports Llama 3.3 70B).
  - **Ollama:** 100% Offline/Local translation for privacy and zero cost.
- **Positional Overlay:** Renders translated text directly over the original text on your screen with perfect alignment.
- **Smart Debounce:** Only translates when the screen content is stable to save API quota and prevent flickering.
- **Multilingual Support:** Supports 50+ world languages with automatic font fallback for Thai, CJK, Arabic, and more.
- **Exclusion Capture:** Overlay windows are automatically excluded from capture to prevent feedback loops.

## 🛠️ Prerequisites

- **Windows:** Currently supports Windows 10/11 (uses native Windows OCR).
- **Language Packs:** Ensure you have the Windows Language Pack installed for the languages you want to translate FROM (check Windows Settings > Time & Language > Language).
- **Rust:** [Install Rust](https://rustup.rs/) to build the project.
- **API Keys:** 
  - [Google AI Studio](https://aistudio.google.com/) for Gemini.
  - [Groq Console](https://console.groq.com/) for Groq.
- **Ollama (Optional):** [Download Ollama](https://ollama.com/) if you want to use offline translation.

## 🚀 Installation & Running

1. **Clone the repository:**
   ```bash
   git clone https://github.com/SupawitKaennak/KTranslatorV2.git
   cd KTranslatorV2
   ```

2. **Run the application:**
   ```bash
   cargo run --release
   ```

## 📖 How to Use

1. **Configure Translation Provider:**
   - Click the ⚙ (Gear) icon in the top right.
   - Choose your **Provider** (Gemini, Groq, or Ollama).
   - Enter your API Key or Server URL and select/type the model name.
   - **Recommended Models:**
     - Gemini: `gemini-2.0-flash`
     - Groq: `llama-3.3-70b-versatile`
     - Ollama: `llama3.2:1b` (fast) or `gemma2:2b` (better Thai)
   - Click **Save & Apply**.
2. **Add a Translation Region:**
   - Click **➕ Add Region**.
   - Click **Select Area** to drag and select the part of the screen you want to watch.
3. **Translate:**
   - Set the **From** (Source) and **To** (Target) languages.
   - Toggle **Active** via the Start/Stop button at the top.
   - Enable **📺 Overlay Mode** to see translated text in-place.

## 🛠️ Tech Stack

- **UI Framework:** [egui](https://github.com/emilk/egui) with `eframe`.
- **OCR:** Native Windows.Media.Ocr (via the `windows` crate).
- **Translators:** 
  - Google Gemini API.
  - Groq API (OpenAI-compatible).
  - Ollama API (Local REST).
- **Capture:** `screenshots` crate with custom stabilization logic.
- **Graphics:** Raw Win32 API for transparent overlay window management.

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.
