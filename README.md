# KTranslator V2

A high-performance, real-time screen translator written in Rust.

<img width="559" height="333" alt="{C93780CF-C104-4925-8ECD-1726E73F654D}" src="https://github.com/user-attachments/assets/6855e97f-9d64-4574-a113-ede638fd1443" /><br><!-- Use your generate_image tool or capture a real one if needed, but for now placeholder is fine if I don't have a final capture -->
<img width="562" height="332" alt="{A77DBB38-1A99-4DB6-A647-D0B3F0C39459}" src="https://github.com/user-attachments/assets/2efd1545-468b-4e9b-9f83-df0c20280ae4" />

##  Features

- **Real-time Translation:** Captures screen regions and translates text instantly.
- **Multi-Provider Support:**
  - **Gemini:** Google's powerful AI (Online).
  - **Groq:** Ultra-fast cloud translation (supports Llama 3.3 70B).
  - **Ollama:** 100% Offline/Local translation for privacy and zero cost.
- **Positional Overlay:** Renders translated text directly over the original text on your screen with perfect alignment.
- **Smart Debounce:** Only translates when the screen content is stable to save API quota and prevent flickering.
- **Multilingual Support:** Supports 50+ world languages with automatic font fallback for Thai, CJK, Arabic, and more.
- **Exclusion Capture:** Overlay windows are automatically excluded from capture to prevent feedback loops.<br>
<img width="563" height="336" alt="{E8220A9A-EEF3-41B1-B523-75EBF029AC23}" src="https://github.com/user-attachments/assets/6517f66f-a38b-4988-8c50-7c17a833b5f6" />

##  Prerequisites

- **Windows:** Currently supports Windows 10/11 (uses native Windows OCR).
- **Language Packs:** Ensure you have the Windows Language Pack installed for the languages you want to translate FROM (check Windows Settings > Time & Language > Language).
- **Rust:** [Install Rust](https://rustup.rs/) to build the project.
- **API Keys:** 
  - [Google AI Studio](https://aistudio.google.com/) for Gemini.
  - [Groq Console](https://console.groq.com/) for Groq.
- **Ollama (Optional):** [Download Ollama](https://ollama.com/) if you want to use offline translation.

##  Installation & Running

1. **Clone the repository:**
   ```bash
   git clone https://github.com/SupawitKaennak/KTranslatorV2.git
   cd KTranslatorV2
   ```

2. **Run the application:**
   ```bash
   cargo run --release
   ```

##  How to Use

### 1. Prepare Windows OCR (Crucial)
To recognize text from other languages (e.g., Japanese, Chinese), you **must** install the corresponding Windows Language Pack:
1. Open **Windows Settings** > **Time & Language** > **Language & region**.
2. Click **Add a language**.
3. Search for the language you want to translate **FROM** (e.g., Japanese).
4. Ensure the **"Optical Character Recognition"** feature is checked during installation.

### 2. Configure Translation Provider
1. Open the app and click the **⚙ (Gear)** icon in the top right.
2. Select your preferred **Provider**:
   - **Gemini:** Great all-rounder. Requires API Key from [AI Studio](https://aistudio.google.com/).
   - **Groq:** Ultra-fast. Use `llama-3.3-70b-versatile` for best quality. API Key from [Groq Console](https://console.groq.com/).
   - **Ollama:** Offline mode. Run `ollama run llama3.2:1b` in your terminal first.
3. Click **Save & Apply**.

### 3. Start Translating
1. Click **➕ Add Region** in the main window.
2. Click **Select Area** — your screen will dim. Drag your mouse to select the area you want to translate.
3. Set **From** (Source language) and **To** (Target language).
   - *Note: Auto-detect works best with Gemini/Groq.*
4. Click the **▶ Start** button at the top.
5. **📺 Overlay Mode:** Toggle this to see translated text appear directly on top of the original text. You can interact with windows behind the overlay normally.

##  Troubleshooting

- **Text appears as boxes (□):** The app tries to load system fonts automatically, but you might be missing specific script support. Ensure you have the corresponding Windows font installed.
- **Ollama Error:** Ensure the Ollama server is running (check your system tray) and you have "pulled" the model using `ollama pull <model_name>`.
- **OCR not recognizing text:** Double-check that the source language matches the installed Windows Language Pack (Step 1).

##  Tech Stack

- **UI Framework:** [egui](https://github.com/emilk/egui) with `eframe`.
- **OCR:** Native Windows.Media.Ocr (via the `windows` crate).
- **Translators:** 
  - Google Gemini API.
  - Groq API (OpenAI-compatible).
  - Ollama API (Local REST).
- **Capture:** `screenshots` crate with custom stabilization logic.
- **Graphics:** Raw Win32 API for transparent overlay window management.

##  License

Copyright (c) 2026 Supawit Kaennak. All rights reserved.
