# PixClip

GUI แบบ Terminal / Pixel art สำหรับ Windows และ Linux ใช้ Python standard library + Tkinter
ร่วมกับ yt-dlp, FFmpeg, ffprobe และ Streamlink ไม่ต้องติดตั้ง Pillow

## เปิดโปรแกรม

- Windows: ดับเบิลคลิก `PixClip.bat` หรือ `PixClip.lnk`
- หลังแตก ZIP ในเครื่องใหม่: ดับเบิลคลิก `Create-PixClip-Shortcut.vbs` เพื่อสร้าง shortcut พร้อมไอคอนในโฟลเดอร์นี้
- Linux Desktop: ใช้ `install.sh` แบบ curl ด้านล่าง แล้วเรียก `PixClip` (`Install-Media-Tools.py` จะติดตั้ง `python3-tk` อัตโนมัติบน Linux ที่ใช้ `apt-get` หากยังไม่มี)
- Linux ไม่มีจอ: หลังติดตั้งใช้ `PixClip --cli`; ถ้าเปิดจากโฟลเดอร์ source ใช้ `./PixClip --cli` — คิวและตารางอัดแบบใหม่อยู่ใน GUI เท่านั้น
- เก็บ `media_toolkit.py`, `pixclip_jobs.py`, `pixclip_ui.py` และโฟลเดอร์ `assets` ไว้ด้วยกัน

กดปุ่ม `วิธีใช้ / HELP` ในแอปเพื่อเปิดคู่มือแบบ Popup เลือกภาษา `TH` หรือ `EN` ได้ หรือกด `เปิดคู่มือเต็ม` จาก Popup เพื่อเปิดไฟล์นี้
เมื่อกด `Choose folder` โปรแกรมจะจำโฟลเดอร์ล่าสุดไว้ในไฟล์ตั้งค่าของผู้ใช้ และเรียกกลับมาให้อัตโนมัติเมื่อเปิดโปรแกรมครั้งถัดไป

## ติดตั้งแบบง่ายสำหรับการแจก Open Source

หลังอัปโหลด repository ไป GitHub แล้ว ใช้ repository นี้ได้เลย:

- Linux / WSL:
  `curl -fsSL https://raw.githubusercontent.com/PeakNAtanon/PixClip/main/install.sh | bash -s -- PeakNAtanon/PixClip`
- Windows: แตก ZIP หรือ clone repository แล้วดับเบิลคลิก `Install-PixClip.bat` โปรแกรมจะตรวจ/ติดตั้ง Python ผ่าน `winget`, ติดตั้ง media tools, สร้าง shortcut และเปิด GUI ให้
- Linux ที่ไม่มี GUI: รันคำสั่ง curl จาก terminal ได้ แต่การเปิด GUI ต้องใช้ WSLg/desktop และติดตั้ง `python3-tk` สำเร็จ

## ดิสโทร Linux ที่รองรับและสิ่งที่ต้องติดตั้งเอง

PixClip ใช้ Python 3, Tkinter, yt-dlp, FFmpeg/ffprobe และ Streamlink จึงใช้ได้กับดิสโทร Linux หลัก ๆ ดังนี้:

| ดิสโทร | GUI | ต้องติดตั้งเอง |
| --- | --- | --- |
| Ubuntu, Debian, Linux Mint, Pop!_OS | ได้ | `python3-pip`, `ffmpeg`, `curl`, `tar`; `python3-tk` จะถูกติดตั้งอัตโนมัติเมื่อใช้ `apt-get` และมีสิทธิ์ `sudo` |
| Fedora | ได้ | `python3`, `python3-pip`, `python3-tkinter`, `ffmpeg`, `curl`, `tar` |
| Arch Linux, Manjaro | ได้ | `python`, `python-pip`, `tk`, `ffmpeg`, `curl`, `tar` |
| WSL2 + WSLg | ได้เมื่อมี WSLg | ใช้แพ็กเกจแบบ Ubuntu/Debian; ถ้าไม่มี WSLg ให้ใช้ `PixClip --cli` |

### คำสั่งเตรียมเครื่อง

Ubuntu / Debian / Mint / Pop!_OS:

```bash
sudo apt update
sudo apt install -y python3 python3-pip python3-tk ffmpeg curl tar
```

Fedora:

```bash
sudo dnf install -y python3 python3-pip python3-tkinter curl tar
```

จากนั้นติดตั้ง `ffmpeg` จาก repository ที่เปิดใช้งานอยู่ หาก Fedora เครื่องนั้นยังไม่มีแพ็กเกจ (บางระบบต้องเปิด RPM Fusion ก่อน) และตรวจด้วย `ffmpeg -version` กับ `ffprobe -version`

Arch Linux / Manjaro:

```bash
sudo pacman -S --needed python python-pip tk ffmpeg curl tar
```

`install.sh` จะดาวน์โหลดและตรวจสอบ yt-dlp ให้ และ `Install-Media-Tools.py` จะติดตั้ง Streamlink ผ่าน pip แต่จะไม่ติดตั้ง FFmpeg/ffprobe บน Linux ดังนั้นดิสโทรที่ไม่ใช่ apt ต้องติดตั้ง Tkinter และ FFmpeg เองก่อน ส่วนการเข้ารหัสด้วย NVIDIA/AMD ต้องมีไดรเวอร์ที่ถูกต้องและ FFmpeg ที่มี encoder ตรงกับ GPU; หากไม่พร้อมให้เลือก CPU ได้

## License

PixClip is released under the MIT License. See [LICENSE](LICENSE).

yt-dlp, FFmpeg, ffprobe, Streamlink และไลบรารี/เครื่องมือภายนอกยังอยู่ภายใต้ใบอนุญาตของเจ้าของโครงการนั้น ๆ

## ดาวน์โหลดและเลือกคุณภาพ

1. วาง URL เลือกโหมดและโฟลเดอร์ปลายทาง
2. เลือก Best available, ความละเอียดสูงสุด 2160p/1080p/720p/480p หรือ Audio only (MP3)
3. กด DOWNLOAD เพื่อเพิ่มงานในคิวและเปิดหน้าคิว

ตัวเลือกความละเอียดเป็นเพดาน ไม่ได้ขยายวิดีโอให้ละเอียดขึ้น หากต้นทางไม่มีรูปแบบที่เข้ากันได้ งานจะล้มเหลวพร้อม log ให้เลือก Best available แล้วลองใหม่
โหมด Live ไม่รองรับตัวเลือก Audio only ส่วนโหมด MP3 จะเก็บเฉพาะเสียงเสมอ
Kick VOD ที่มีเสียงแยกใน HLS อาจต้องใช้ Best available เพื่อรักษาเสียงครบ

## Cookies สำหรับ Live ที่ต้องล็อกอิน

หาก TikTok หรือเว็บไซต์ต้นทางมองไม่เห็น Live ทั้งที่เปิดดูได้ในเบราว์เซอร์ ให้ export เซสชันเป็นไฟล์ Netscape `cookies.txt` จากเบราว์เซอร์ที่ล็อกอินอยู่ แล้วทำตามนี้:

1. กด `Choose` ข้าง `Cookies: OFF` บนหน้าหลัก แล้วเลือกไฟล์ `cookies.txt`
2. ตรวจให้สถานะเปลี่ยนเป็นชื่อไฟล์ Cookies แล้วเลือกโหมด Live
3. กด `DOWNLOAD` หรือเพิ่มงานใน `Queue / History`
4. กด `Clear` เมื่อต้องการกลับไปใช้การเข้าถึงแบบสาธารณะ

PixClip จะจำเฉพาะตำแหน่งไฟล์และจะไม่พิมพ์ค่า Cookies ใน Log แต่ไฟล์ต้นฉบับยังเป็นข้อมูลล็อกอิน ห้ามส่งต่อหรือ commit เข้า Git

สำหรับ CLI ใช้ `--cookies-file` ได้ เช่น:

```bash
PixClip --record-live-streamlink "LIVE_URL" "$HOME/Videos" --cookies-file "$HOME/private/cookies.txt"
```

## Queue / History

- วางหลาย URL โดยหนึ่งลิงก์ต่อหนึ่งบรรทัด แล้วกด Add downloads now
- ตั้ง Parallel downloads ได้ 1–4 งาน (ค่าเริ่มต้น 1)
- Pause queue หยุดเริ่มงานใหม่ แต่ไม่หยุดงานที่กำลังทำ
- เลือกงานแล้ว Cancel, Delete, Retry, Files, Open folder หรือ Job log; Delete ลบเฉพาะรายการจาก Queue/History ไม่ลบไฟล์สื่อ
- History only แสดงงานที่จบแล้ว รวมงานแปลง ตัด และต่อคลิป
- งานดาวน์โหลดแต่ละงานอยู่ใน `PixClip-<job-id>` ภายใต้โฟลเดอร์ที่เลือก ป้องกันไฟล์ชนกัน
- Retry สร้างงานใหม่ ไม่เขียนทับโฟลเดอร์ดาวน์โหลดเดิม ส่วนการลองงานตัด/แปลงซ้ำจะถามก่อนแทนที่ไฟล์ปลายทาง
- ประวัติ Windows: `%LOCALAPPDATA%/PixClip/jobs.json`
- ประวัติ Linux: `${XDG_STATE_HOME:-~/.local/state}/PixClip/jobs.json`
- หากแอปหยุดผิดปกติ งานที่กำลังทำจะเป็น Interrupted และต้องกด Retry เอง ส่วนคิวที่ยังไม่เริ่มยังคงอยู่
- ควรเปิด PixClip เพียงหนึ่งหน้าต่างหลักต่อผู้ใช้ เพื่อไม่ให้ไฟล์ประวัติถูกเขียนทับจากหลาย instance

งานตัด/แปลง/ต่อคลิปทำทีละงานและรอคิวดาวน์โหลดว่างก่อน ส่วน Install/Update tools ถูกปิดระหว่างทำงานเพื่อไม่เปลี่ยน executable กลางงาน

## TS → MP4 อัตโนมัติ

เปิด `TS -> MP4 (keep TS)` ก่อนเพิ่มงาน เมื่อได้ TS สำเร็จ โปรแกรมจะ remux ด้วย stream copy
ตรวจว่าไฟล์ไม่ว่างและความยาวสอดคล้องกับต้นฉบับ แล้วจึงใช้ชื่อ MP4 สุดท้าย
TS ต้นฉบับไม่ถูกลบ ถ้า codec ไม่เข้ากับ MP4 ให้ใช้โหมด Compatible H.264 ด้วยตนเอง
ต้องมีพื้นที่สำหรับ TS และ MP4 พร้อมกัน หากยกเลิกหรืออัดล้มเหลว โปรแกรมไม่แปลงไฟล์บางส่วนอัตโนมัติ

## ตั้งเวลาอัด Live

1. เลือก `Live - Auto GPU/CPU` ให้ PixClip ตรวจ NVIDIA/AMD และเลือก NVENC/AMF/VA-API หรือ CPU อัตโนมัติ หรือเลือก `Live - Streamlink` / `Live - NVIDIA NVENC` เอง
2. เปิด Queue / History วางลิงก์
3. ระบุวันเวลาเครื่องในรูปแบบ `YYYY-MM-DD HH:MM` หรือเว้นว่างเพื่อเริ่มทันที
4. ระบุ Minutes และกด Add timed Live

ต้องเปิด PixClip และให้เครื่องตื่นอยู่ ไม่ใช่บริการอัดเบื้องหลังเมื่อปิดแอป
ระยะเวลานับเป็นช่วงเวลาอัดตามนาฬิกา รวมเวลารอต้นทางและเชื่อมต่อใหม่ จึงไม่ได้รับประกันความยาวคลิปเต็มช่วงหากสตรีมหลุด
งานตามเวลาที่ถึงกำหนดจะได้ช่องว่างก่อนคิวทั่วไป แต่ไม่แย่งหยุดงานเดิม หากช่องเต็มอาจเริ่มช้า และหากเลยเวลาสิ้นสุดจะรายงานว่าพลาดช่วงอัด
เมื่อโปรเซสอัดหลุด จะลองเปิดใหม่สูงสุด 5 ครั้ง โดยเก็บเป็นไฟล์ส่วนใหม่ ไม่เขียนทับส่วนก่อนหน้า
ใช้ Join clips รวมส่วนภายหลังได้ หากรูปแบบตรงกัน
Live ไม่มีกำหนดจบให้ใช้ DOWNLOAD/Add downloads now และ Cancel ในหน้าคิวเมื่อต้องการหยุด

## พรีวิวก่อนตัด

เปิด Cut clip → Choose video → เลื่อนแถบเวลาเพื่อดูภาพเฟรม แล้วกด Set start here / Set end here
นี่คือพรีวิวภาพนิ่ง ไม่ใช่เครื่องเล่นวิดีโอพร้อมเสียง หากพรีวิวอ่านไม่ได้ ยังพิมพ์เวลาเองได้

## พื้นที่ว่างและมาสคอต

Reserve GiB ในหน้าคิวเป็นพื้นที่สำรองขั้นต่ำ (เริ่มต้น 1 GiB, ต่ำสุด 0.25 GiB)
ตรวจพื้นที่ก่อนเริ่มและระหว่างงาน หากต่ำกว่าเกณฑ์จะหยุดงานและแจ้งเตือน ไม่ลบไฟล์ต้นฉบับเพื่อเคลียร์พื้นที่
พื้นที่ที่แสดงในหน้าหลักเป็นพื้นที่จริง ไม่ใช่การประมาณขนาดดาวน์โหลดทั้งหมด
มาสคอตขยับพร้อมสถานะ READY / WORKING / OK / CHECK เพื่อบอกสถานะโดยไม่เปลี่ยนธีมเดิม
แถบสถานะด้านล่างจะแสดงชื่อรุ่น GPU/CPU และ encoder ที่โหมด Auto จะเลือกใช้

## ทดสอบ

`python -m unittest -v test_pixclip_jobs`

Integration ใช้คลิปสังเคราะห์และ HTTP server ที่ localhost เท่านั้น:

`python -c "import os,unittest; os.environ['PIXCLIP_INTEGRATION']='1'; unittest.main(module='test_pixclip_integration',verbosity=2)`

การทดสอบครอบคลุมไฟล์จริง, TS/MP4, GUI และ Live จำลอง ไม่ได้ยืนยันการเชื่อมต่อ Kick/Twitch/YouTube จริงทุกบัญชีหรือทุก URL
การเข้าถึงต้นทางยังขึ้นกับเว็บไซต์ เครื่องมือที่ติดตั้ง และสิทธิ์ของผู้ใช้

## สนับสนุน PixClip

หาก PixClip มีประโยชน์และต้องการสนับสนุนการพัฒนาต่อ สามารถโดเนทผ่าน Ko-fi ได้ที่:

[สนับสนุน PixClip ผ่าน Ko-fi](https://ko-fi.com/peaknatanon)
