# PixClip 0.2.1: เทียบต้นฉบับและผลทดสอบ

ตรวจเมื่อ 17 พฤศจิกายน 2026 บน Windows x64, Intel Core i5-9400F และ NVIDIA RTX 2060

ใช้โปรแกรม Python เดิมที่เปิดรันจริงเป็นตัวอ้างอิง ไม่ได้ใช้หน้าตาของ Rust รุ่นแรกเป็นมาตรฐาน
เก็บต้นฉบับไว้ใน `.cache/python-before-rust.zip` ใน working copy และตรวจ ZIP ทั้ง 11 ไฟล์แล้ว

## หน้าจอ

| หน้าจอ | Python ต้นฉบับ | Rust ที่แก้แล้ว | ขนาดเริ่มต้น |
| --- | --- | --- | --- |
| หน้าหลัก | [ภาพเดิม](screenshots/python-main.png) | [ภาพใหม่](screenshots/main.png) | 1000 × 860 |
| Queue / History | [ภาพเดิม](screenshots/python-queue.png) | [ภาพใหม่](screenshots/queue.png) | 1040 × 720 |
| Cut clip | [ภาพเดิม](screenshots/python-cut.png) | [ภาพใหม่](screenshots/cut.png) | 760 × 720 |
| Join clips | [ภาพเดิม](screenshots/python-join.png) | [ภาพใหม่](screenshots/join.png) | 760 × 560 |
| Help | [ภาพเดิม](screenshots/python-help.png) | [ภาพใหม่](screenshots/help.png) | 900 × 700 |

คืนผังเดิม: Download อยู่ข้างตัวเลือกโหมด, Convert เต็มแถว, เครื่องมือสองแถว,
ตัวเลือกคุณภาพ/Cookies/Queue, log และข้อมูลเครื่องอยู่ตำแหน่งเดิม ใช้รูปมาสคอต
และลายไอคอนพิกเซลเดิม พร้อมธีมสีและฟอนต์ระบบตระกูลเดียวกับต้นฉบับ
ภาพใช้สถานะทดสอบว่างและไม่อ่าน clipboard; ข้อมูลฮาร์ดแวร์ในภาพเป็นค่าทดสอบ
การเกลี่ยขอบตัวอักษรและรายละเอียด widget อาจต่างจาก Tk เล็กน้อย เพราะตัวแอปใช้ egui

คืนพฤติกรรมเลือกทั้งแถวและดับเบิลคลิกดู log, Files เป็นหน้าต่างแยก,
Cut/Join จับการใช้งานหน้าต่างหลัก, เลือกคลิปหลายรายการด้วย Ctrl/Shift,
จัดลำดับ/ลบคลิป, และเสนอ Compatible เมื่อรูปแบบคลิปต่างกัน
Help ใช้เนื้อหาไทย/อังกฤษเดิมและเปิดคู่มือเต็มได้
เมนู CLI กลับมาใช้หมายเลข 1–20 ตามต้นฉบับ

## ผลทดสอบ

| งานตรวจ | ผล |
| --- | --- |
| `cargo fmt --check` | ผ่าน |
| Clippy ทุก target/feature โดยไม่ยอมรับ warning | ผ่าน |
| Unit และ integration รวม 26 tests | ผ่านทั้งหมด ไม่มี skip ในรอบนี้ |
| EXE จริง: Queue, log, Files, Pause/Resume, modal Cut/Join, Help TH/EN/Close | ผ่าน |
| CLI พร้อม GUI และปิดแอปตามปกติ | ผ่าน |
| GUI เลือกไฟล์แล้วแปลง ขณะ Pause downloads | ผ่าน |
| GUI ตัดคลิปผ่านช่อง HH/MM/SS และหน้าต่าง Save | ผ่าน ตรวจความยาวด้วย ffprobe |
| GUI ต่อแบบ Fast และยืนยันเปลี่ยนเป็น Compatible เมื่อรูปแบบต่างกัน | ผ่าน ตรวจไฟล์ที่สร้างจริง |
| GUI ดาวน์โหลดจาก localhost | ผ่าน |
| MP4 Best, Up to 4K, Compatible และ MP3 | ผ่าน |
| Playlist MP4 Best, Compatible และ MP3 จำนวนสองรายการ | ผ่าน |
| ดาวน์โหลด TS พร้อมชื่อจาก metadata | ผ่าน |
| แปลง Fast MP4, H.264, H.265 NVENC และ HEVC → H.264 | ผ่าน ตรวจ codec จริง |
| Live Auto, NVENC และ Streamlink | ผ่านกับ HLS บน localhost |
| ตั้งเวลา HLS ต่อเนื่อง: รอก่อนเริ่ม เริ่มตามเวลา หยุดตาม deadline | ผ่าน ได้ TS และ MP4 ที่อ่านได้ |
| ยกเลิก Streamlink ผ่านปุ่มใน Queue | ผ่าน เก็บ TS บางส่วนไว้และปิดแอปได้ |
| คิวพร้อมกัน, ยกเลิก, ลบประวัติ, retry, ประวัติเดิมและฟิลด์เพิ่มเติม | ผ่าน regression tests |
| settings เสีย | เปิดด้วยค่าเริ่มต้น เก็บสำเนาเดิม และแสดงคำเตือน |
| ตัวติดตั้งเครื่องมือจากแหล่งจริง | ผ่าน SHA-256, แตก ZIP และรันเครื่องมือหลังย้ายตำแหน่ง |
| PATH ไม่มี Python, Cargo, winget หรือ media tools เดิม | ติดตั้งและรันเครื่องมือได้ |
| EXE ที่แตกจาก ZIP ในโฟลเดอร์ใหม่และ PATH ที่มีเฉพาะ System32 | GUI เปิด/ปิดได้, CLI help/menu ทำงาน, hash ตรงกับตัวที่ทดสอบ |
| Bash launchers และ PowerShell build script | ผ่านการตรวจ syntax |

รอบสื่อใช้ไฟล์สังเคราะห์ 320 × 180 และ HLS ที่ประกาศความละเอียด เพื่อทดสอบการเลือก format
ไม่ได้ใช้เป็น benchmark วิดีโอ 4K จริง ใช้ `PIXCLIP_STATE_DIR` แยกประวัติทุกกรณี
ตรวจ hash ว่าไฟล์ต้นฉบับไม่เปลี่ยน และตรวจผลลัพธ์ด้วย ffprobe
บันทึกผลแบบ JSON อยู่ที่ [windows.json](test-results/windows.json)

## จุดผิดพลาดที่แก้จากการทดสอบจริง

- Pause queue เคยขวางงาน local ด้วย; ตอนนี้หยุดเฉพาะการเริ่ม download/schedule
- Cancel task เคยยกเลิก downloads ทั้งหมด; ตอนนี้ควบคุมเฉพาะงาน local/tools
- Progress ของ Cut อิงความยาวทั้งไฟล์; ตอนนี้อิงช่วงที่เลือก
- ไฟล์ TS ของงาน Live สั้นอาจยังค้างใน buffer เมื่อหมดเวลา; ตอนนี้ flush packet ระหว่างอัด
- ZIP ของ Streamlink มีโฟลเดอร์ครอบ; ตัวติดตั้งตรวจหา launcher และรักษาโครงสร้าง portable ที่ถูกต้อง
- ชื่อไฟล์ Live/TS และการแปลง TS สำรองเป็น H.264/AAC กลับมาครบ
- Kick VOD ใช้เส้นทาง playback โดยตรงก่อนเรียก yt-dlp ตามต้นฉบับ

ตัวติดตั้งใช้ [yt-dlp releases](https://github.com/yt-dlp/yt-dlp/releases),
[Gyan FFmpeg builds](https://www.gyan.dev/ffmpeg/builds/),
[Streamlink Windows builds](https://github.com/streamlink/windows-builds)
และ [Streamlink AppImage](https://github.com/streamlink/streamlink-appimage)
ตัวแอปเป็น Rust; เครื่องมือภายนอกใช้ runtime ของแพ็กเกจตัวเองและไม่ต้องติดตั้ง Python แยกในเครื่อง

## ขอบเขตที่ยังไม่ได้ยืนยัน

- ไม่ได้ทดสอบเว็บไซต์สาธารณะหรือบัญชีที่ต้องใช้ Cookies; ผล localhost ไม่ยืนยัน YouTube/Kick/TikTok ทุกกรณี
- Kick VOD ตรวจ routing และโค้ดเทียบต้นฉบับแล้ว แต่ไม่ได้ดาวน์โหลดจาก Kick จริงในรอบนี้
- ไม่ได้รัน Linux desktop หรือ AMD AMF/VA-API บนฮาร์ดแวร์จริงในรอบ Windows นี้
- คิวและงานตั้งเวลายังต้องเปิด PixClip และให้เครื่องตื่นอยู่ เหมือนต้นฉบับ

โค้ดอยู่ใน `src/` โดยแยก UI, queue, media, process runner, settings/history และ tools
GUI และ CLI ใช้เส้นทางทำงานร่วมกัน ไม่มี Python worker หรือ Tkinter ในตัวแอป
