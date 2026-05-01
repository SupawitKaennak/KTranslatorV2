# KTranslator V2

[ภาษาไทย (Thai)](#thai) | [English](#english)

---

<a name="thai"></a>
## ภาษาไทย (Thai)

โปรแกรมแปลภาษาจากการจับภาพหน้าจอ (Screen Translator) เขียนด้วยภาษา Rust 

### 🎯 ลักษณะการใช้งาน
- **แปลเกม:** ใช้แปลบทสนทนาหรือเมนูในเกม
- **แปลมังงะ:** อ่านข้อความจากภาพมังงะหรือคอมมิค (รองรับตัวหนังสือเอียง/โค้ง)
- **แปลบทความ:** แปลข้อความจากหน้าเว็บ เอกสาร หรือ PDF ที่ไม่สามารถก๊อปปี้ข้อความได้

### 🛠️ ความต้องการของระบบ (Requirements)

**1. ระบบ OCR (ตัวอ่านข้อความ)**
- **Windows OCR:** (ติดมากับ Windows) ต้องติดตั้ง Language Pack ของภาษาต้นทางที่จะแปลให้เรียบร้อย (เช่น ญี่ปุ่น, จีน)
- **PaddleOCR:** (แนะนำสำหรับมังงะ) ต้องดาวน์โหลดตัวโปรแกรม [PaddleOCR-json](https://github.com/hiroi-sora/PaddleOCR-json/releases) และระบุที่อยู่ไฟล์ `.exe` ในหน้า Settings ของโปรแกรม

**2. ระบบการแปล (Translator)**
- **Gemini:** ต้องใช้ API Key สมัครฟรีได้ที่ [Google AI Studio](https://aistudio.google.com/)
- **Groq:** ต้องใช้ API Key สมัครฟรีได้ที่ [Groq Console](https://console.groq.com/)
- **Ollama:** สำหรับการแปลแบบ Offline ดาวน์โหลดได้ที่ [Ollama.com](https://ollama.com/)

### 💻 เทคโนโลยีที่ใช้ (Tech Stack)
- **Language:** Rust (edition 2024)
- **UI Framework:** [egui](https://github.com/emilk/egui)
- **OCR Engines:** Windows.Media.Ocr & PaddleOCR
- **Graphics:** Win32 API (สำหรับระบบ Overlay โปร่งใส)
- **Capture:** Screenshots crate พร้อมระบบ stabilization

### 🚀 การติดตั้งและใช้งาน

**วิธีติดตั้ง (สำหรับนักพัฒนา):**
1. ติดตั้ง [Rust Toolchain](https://rustup.rs/)
2. Clone โปรเจกต์:
   ```bash
   git clone https://github.com/SupawitKaennak/KTranslatorV2.git
   cd KTranslatorV2
   ```
3. รันโปรแกรม:
   ```bash
   cargo run --release
   ```

**ขั้นตอนการใช้งาน:**
1. เข้าไปที่ **Settings** (ไอคอนฟันเฟือง) เพื่อเลือก OCR และใส่ API Key
2. กด **Add Region** และเลือกพื้นที่บนหน้าจอที่ต้องการแปล
3. เลือกภาษาต้นทาง (From) และภาษาปลายทาง (To)
4. กดปุ่ม **Start** เพื่อเริ่มการแปล
5. เปิดโหมด **Overlay Mode** หากต้องการให้คำแปลแสดงทับตำแหน่งเดิมบนหน้าจอ

---

<a name="english"></a>
## English

A powerful Screen Translator written in Rust for seamless real-time translation.

### 🎯 Key Features
- **Game Translation:** Translate in-game dialogues, menus, and item descriptions.
- **Manga/Comics:** Read manga with specialized support for stylized or curved text.
- **Article/Documents:** Translate text from websites, PDFs, or images that don't allow text copying.

### 🛠️ System Requirements

**1. OCR Engines (Text Recognition)**
- **Windows OCR:** Built-in. Requires language packs for source languages (e.g., Japanese, Chinese).
- **PaddleOCR:** Recommended for manga. Download [PaddleOCR-json](https://github.com/hiroi-sora/PaddleOCR-json/releases) and specify the `.exe` path in the app settings.

**2. Translation Providers**
- **Gemini:** API Key required. Get it for free at [Google AI Studio](https://aistudio.google.com/).
- **Groq:** High-speed API. Get your key at [Groq Console](https://console.groq.com/).
- **Ollama:** For local/offline translation. Download at [Ollama.com](https://ollama.com/).

### 💻 Tech Stack
- **Language:** Rust (edition 2024)
- **UI Framework:** [egui](https://github.com/emilk/egui)
- **OCR Engines:** Windows.Media.Ocr & PaddleOCR
- **Graphics:** Win32 API (for transparent overlay system)
- **Capture:** Screenshots crate with stabilization logic

### 🚀 Getting Started

**Installation (Developers):**
1. Install [Rust Toolchain](https://rustup.rs/).
2. Clone the repository:
   ```bash
   git clone https://github.com/SupawitKaennak/KTranslatorV2.git
   cd KTranslatorV2
   ```
3. Run the application:
   ```bash
   cargo run --release
   ```

**Basic Usage:**
1. Open **Settings** (gear icon) to select your OCR engine and enter API keys.
2. Click **Add Region** to select the area of the screen you want to translate.
3. Select Source (From) and Target (To) languages.
4. Click **Start** to begin the real-time translation loop.
5. Enable **Overlay Mode** to display translations directly over the original text.

---

### 🛡️ License
Copyright (c) 2024 Supawit Kaennak [GPL v3.0](LICENSE). All rights reserved.
