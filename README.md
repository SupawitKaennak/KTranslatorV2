# KTranslator V2

โปรแกรมแปลภาษาบนหน้าจอประสิทธิภาพสูง เขียนด้วยภาษา Rust รองรับการแปลแบบ Real-time พร้อมระบบ Overlay 

<img width="559" height="333" alt="{C93780CF-C104-4925-8ECD-1726E73F654D}" src="https://github.com/user-attachments/assets/6855e97f-9d64-4574-a113-ede638fd1443" /><br>
<img width="562" height="332" alt="{A77DBB38-1A99-4DB6-A647-D0B3F0C39459}" src="https://github.com/user-attachments/assets/2efd1545-468b-4e9b-9f83-df0c20280ae4" />

## ✨ ความสามารถเด่น (Features)

- **Real-time Translation:** จับภาพและแปลภาษาบนหน้าจอได้ทันที
- **Dual OCR Engine:**
  - **Windows OCR:** รวดเร็ว กินทรัพยากรน้อย เหมาะสำหรับบทความทั่วไป
  - **PaddleOCR:** แม่นยำสูงสุดสำหรับมังงะ รองรับฟอนต์เอียง/โค้ง/มน ได้ดีเยี่ยม (ต้องการโปรแกรมเสริม)
- **Multi-Provider Support:**
  - **Gemini:** AI ทรงพลังจาก Google แปลลื่นไหลเหมือนมนุษย์
  - **Groq:** แปลไวที่สุดในโลก รองรับ Llama 3.3 70B
  - **Ollama:** แปลแบบ Offline 100% ฟรีและเป็นส่วนตัว
- **Positional Overlay:** แสดงคำแปลทับตัวอักษรเดิมบนหน้าจอโดยตรง พร้อมรักษาตำแหน่งเดิมไว้
- **Exclusion Capture:** ระบบซ่อนหน้าต่างโปรแกรมตัวเองขณะจับภาพ เพื่อไม่ให้เกิดภาพซ้อน

---

## 🛠️ ความต้องการของระบบ (Requirements)

เพื่อให้โปรแกรมทำงานได้สมบูรณ์ คุณต้องติดตั้งสิ่งเหล่านี้:

### 1. ภาษาและ OCR (จำเป็น)
- **Windows Language Packs:** หากใช้ Windows OCR ต้องติดตั้ง Language Pack ของภาษาที่จะแปล **ต้นทาง** (เช่น ญี่ปุ่น, จีน) โดยต้องเลือกติ๊กถูกที่ **"Optical Character Recognition"** ตอนติดตั้งด้วย
- **PaddleOCR-json (แนะนำสำหรับมังงะ):**
  - ดาวน์โหลดจาก: [hiroi-sora/PaddleOCR-json](https://github.com/hiroi-sora/PaddleOCR-json/releases)
  - แตกไฟล์ไว้ในเครื่อง และนำ Path ของไฟล์ `.exe` ไปใส่ในหน้า Settings ของ KTranslator

### 2. ตัวแปลภาษา (เลือกอย่างใดอย่างหนึ่ง)
- **Cloud API (แนะนำ):**
  - [Google AI Studio](https://aistudio.google.com/) สำหรับรับ API Key ของ Gemini (ฟรี)
  - [Groq Console](https://console.groq.com/) สำหรับรับ API Key ของ Groq (ฟรีและเร็วมาก)
- **Local LLM (แปลออฟไลน์):**
  - [Ollama](https://ollama.com/) ติดตั้งและรันโมเดล (เช่น `ollama pull qwen2.5:1.5b`)

### 3. สำหรับนักพัฒนา (Build from Source)
- **Rust Toolchain:** [ติดตั้ง Rust](https://rustup.rs/)

---

## 🚀 วิธีการใช้งาน

1. **ตั้งค่า OCR:** เข้าไปที่ฟันเฟือง (Settings) เลือก OCR Engine ที่ต้องการ
   - หากใช้ PaddleOCR ให้ระบุที่อยู่ไฟล์ `.exe` ให้ถูกต้อง
2. **ตั้งค่า Translator:** ใส่ API Key ของ Gemini หรือ Groq หรือตั้งค่า URL ของ Ollama
3. **เพิ่มพื้นที่แปล:** กดปุ่ม **➕ Add Region** และเลือกพื้นที่บนหน้าจอที่ต้องการแปล
4. **เริ่มการทำงาน:** กดปุ่ม **▶ Start**
5. **Overlay Mode:** เปิดโหมดนี้เพื่อระบายสีทับตัวอักษรเดิมและแสดงคำแปลทับลงไป

---

## 🔧 การแก้ปัญหาเบื้องต้น (Troubleshooting)

- **Access is denied (os error 5):** ตรวจสอบว่าในช่อง PaddleOCR Path คุณระบุถึงตัวไฟล์ `.exe` หรือยัง (ต้องไม่ใช่แค่ชื่อโฟลเดอร์)
- **แปลไม่ออก/ตัวหนังสือเพี้ยน:** หากใช้ Windows OCR ให้ตรวจสอบว่าลง Language Pack ครบถ้วนหรือไม่ หากเป็นมังงะแนะนำให้เปลี่ยนไปใช้ PaddleOCR
- **หน้าจอขาว/ค้างตอนเริ่ม:** ครั้งแรกที่ใช้ PaddleOCR ระบบอาจใช้เวลาโหลดโมเดลครู่หนึ่ง

---

## 🛡️ สัญญาอนุญาต (License)

Copyright (c) 2026 Supawit Kaennak [GPL v3.0](LICENSE). All rights reserved.
