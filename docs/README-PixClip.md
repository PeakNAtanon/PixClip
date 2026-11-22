# PixClip

โปรแกรมจัดการวิดีโอสำหรับ Windows, Linux และ macOS เขียนด้วย Rust
คงธีม Terminal / Pixel art สีมิ้นต์/อำพัน มาสคอต และขั้นตอนใช้งานเดิม
ใช้ yt-dlp, FFmpeg, ffprobe และ Streamlink เป็นเครื่องมือภายนอก
ตัวแอปและระบบคิวไม่ต้องใช้ Python หรือ Tkinter

## เปิดโปรแกรม

- Windows: ดาวน์โหลด MSI สำหรับติดตั้งและสร้างทางลัดใน Start หรือใช้ EXE แบบพกพาจาก [GitHub Releases](https://github.com/PeakNAtanon/PixClip/releases/latest)
- สร้าง shortcut: เปิด `scripts/windows/Create-PixClip-Shortcut.vbs`
- Linux และ macOS เปิดด้วย `./PixClip` หรือใช้ `./PixClip --cli` ในเทอร์มินัล
- บิลด์จากซอร์สด้วย `scripts/build.ps1` บน Windows หรือ `bash scripts/build.sh` บน Linux และ macOS
- โค้ด Rust อยู่ใน `src/`, รูปอยู่ใน `assets/`, คู่มืออยู่ใน `docs/`, สคริปต์อยู่ใน `scripts/`
- เครื่องมือภายนอกอยู่ใน `tools/`; ไฟล์ ZIP อยู่ใน `dist/` และไม่เก็บใน Git

กด `วิธีใช้ / HELP` เพื่อเปิดคู่มือภาษาไทยหรืออังกฤษ
`Choose folder` และ `Choose` Cookies จะจำตำแหน่งไฟล์สำหรับครั้งถัดไป

## ติดตั้งและ build

บน Windows เปิดไฟล์ MSI แล้วเลือก **Install tools** ใน PixClip เมื่อใช้ครั้งแรก ตัวโปรแกรมติดตั้งใน `%LOCALAPPDATA%\PixClipApp` และเก็บประวัติไว้ใน `%LOCALAPPDATA%\PixClip`

สำหรับแพ็กเกจ ZIP ให้แตกไฟล์แล้วเปิด `Install-PixClip.bat`
ตัวติดตั้งจะดาวน์โหลด yt-dlp และ FFmpeg ที่ตรวจ checksum แล้ว พร้อม Streamlink แบบ portable จากโครงการทางการและตรวจ SHA-256 โดยไม่ต้องมี winget หรือ Python ในเครื่อง
หากเป็น source code ต้องติดตั้ง Rust และเครื่องมือ MSVC หรือ MinGW ที่ตรงกับ toolchain ก่อน

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1
.\PixClip.bat
```

Linux ต้องติดตั้ง Rust, C compiler, pkg-config, OpenSSL และไลบรารี desktop สำหรับ build
Ubuntu/Debian:

```bash
sudo apt install build-essential pkg-config libssl-dev libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxkbcommon-dev libwayland-dev libasound2-dev ffmpeg
bash scripts/build.sh
./PixClip
```

บน macOS ติดตั้ง Rust และ Xcode Command Line Tools แล้วติดตั้ง FFmpeg กับ Streamlink ผ่าน Homebrew

```bash
brew install ffmpeg streamlink
bash scripts/build.sh
./PixClip --install-tools
./PixClip
```

ตัวติดตั้งบน macOS จะดาวน์โหลด yt-dlp รุ่น universal ที่ตรวจ checksum และใช้ FFmpeg, ffprobe และ Streamlink ที่ติดตั้งไว้

## ประวัติเดิม

อ่าน `jobs.json` และ `settings.json` จากตำแหน่งเดิมได้
ก่อนบันทึกประวัติครั้งแรกจะเก็บสำเนา `jobs.pre-rust.json`
หากประวัติเสีย จะเก็บไฟล์เดิมและใช้ไฟล์ recovery สำหรับงานใหม่
เปิดแอปได้ครั้งละหนึ่ง instance ต่อผู้ใช้ เพื่อป้องกันข้อมูลถูกเขียนทับ

## ดาวน์โหลดและเลือกคุณภาพ

1. วาง URL เลือกโหมดและโฟลเดอร์ปลายทาง
2. เลือก Best available, ความละเอียดสูงสุด 2160p/1080p/720p/480p หรือ Audio only (MP3)
3. กด DOWNLOAD เพื่อเพิ่มงานในคิวและเปิดหน้าคิว

- `Playlist - Select items` เปิดรายชื่อเพลย์ลิสต์ให้ติ๊กรายการ เลือก MP4, Compatible MP4 หรือ Audio แล้วกด Add to queue
- `Download - Time range` ใส่ Start และ End เป็นวินาทีหรือ MM:SS เช่น 00:30 ถึง 01:15 แล้วกด DOWNLOAD เพื่อบันทึกช่วงนั้น
- `Subtitles only` เลือก English, Thai หรือ All languages เพื่อบันทึกซับ SRT เมื่อแหล่งวิดีโอมีซับ

ตัวเลือกความละเอียดเป็นเพดาน ไม่ได้ขยายวิดีโอให้ละเอียดขึ้น หากต้นทางไม่มีรูปแบบที่เข้ากันได้ งานจะล้มเหลวพร้อม log ให้เลือก Best available แล้วลองใหม่
โหมด Compatible จะสร้าง MP4 แบบ H.264/AAC โดยเข้ารหัสใหม่เมื่อรูปแบบต้นทางต้องแปลง
การดาวน์โหลดจาก YouTube ควรใช้ yt-dlp รุ่นปัจจุบันและ Deno ที่อยู่ใน PATH ตาม [คู่มือการรองรับ YouTube ของ yt-dlp](https://github.com/yt-dlp/yt-dlp/wiki/EJS)
โหมด Live ไม่รองรับตัวเลือก Audio only

สำหรับเพลงเดี่ยวเลือก `Audio - choose format` และสำหรับเพลย์ลิสต์เลือก `Playlist - Audio` จากนั้นเลือกระหว่าง MP3, M4A, FLAC และ WAV
MP3 และ M4A มี Best quality และบิตเรตเป้าหมาย 320, 192 หรือ 128 kbps ส่วน FLAC และ WAV ใช้การเข้ารหัสแบบ lossless
ตัวเลือกใช้กับทุกลิงก์ที่เพิ่มเข้าคิวในรอบนั้น

```powershell
PixClip --download 3 "VIDEO_URL" "D:\Music" --audio-format mp3 --audio-bitrate 192
PixClip --download 6 "PLAYLIST_URL" "D:\Music" --audio-format flac
```

ถ้าดาวน์โหลด Playlist ค้างหรือล้มเหลว ให้เลือกงานเดิมใน Queue / History แล้วกด `Retry` เพื่อใช้โฟลเดอร์เดิมและดาวน์โหลดต่อจากรายการที่บันทึกไว้
Kick VOD ที่มีเสียงแยกใน HLS อาจต้องใช้ Best available เพื่อรักษาเสียงครบ

### ลิงก์เพลง เพลย์ลิสต์ และอัลบั้มจาก Spotify

1. ในเมนูดาวน์โหลดเลือก `Spotify - Track` สำหรับเพลงเดียว หรือ `Spotify - Playlist / Album` สำหรับเพลย์ลิสต์หรืออัลบั้ม แล้ววางลิงก์สาธารณะจาก `open.spotify.com` และกด `DOWNLOAD`
2. หน้าต่าง **Spotify matches** แสดงชื่อเพลง ศิลปิน และผลค้นหาจาก YouTube เลือกแหล่งเสียงที่ตรงกับเพลงและมีสิทธิ์ดาวน์โหลด หรือข้ามเพลงนั้น
3. เลือกรูปแบบและคุณภาพเสียง แล้วกด `Add to queue` การดาวน์โหลดจะใช้ลิงก์ YouTube ที่คุณยืนยัน

PixClip อ่านรายการเพลงที่หน้า Spotify สาธารณะแสดงได้ เพลย์ลิสต์และอัลบั้มจัดเก็บใน `Playlists/Spotify/` และแบ่งตามเลขลำดับเพลง

## Cookies สำหรับ Live ที่ต้องล็อกอิน

หาก TikTok หรือเว็บไซต์ต้นทางมองไม่เห็น Live ทั้งที่เปิดดูได้ในเบราว์เซอร์ ให้ export เซสชันเป็นไฟล์ Netscape `cookies.txt` จากเบราว์เซอร์ที่ล็อกอินอยู่ แล้วทำตามนี้:

1. กด `Choose` ข้าง `Cookies: OFF` บนหน้าหลัก แล้วเลือกไฟล์ `cookies.txt`
2. ตรวจให้สถานะเปลี่ยนเป็นชื่อไฟล์ Cookies แล้วเลือกโหมด Live
3. กด `DOWNLOAD` หรือเพิ่มงานใน `Queue / History`
4. กด `Clear` เมื่อต้องการกลับไปใช้การเข้าถึงแบบสาธารณะ

ชี้เมาส์ที่สถานะ Cookies เพื่อดูชื่อไฟล์เต็มได้ PixClip จะจำเฉพาะตำแหน่งไฟล์และจะไม่พิมพ์ค่า Cookies ใน Log แต่ไฟล์ต้นฉบับยังเป็นข้อมูลล็อกอิน ห้ามส่งต่อหรือ commit เข้า Git

ถ้า YouTube ขอให้ยืนยันอายุ ให้เปิดคลิปในเบราว์เซอร์ด้วยบัญชีที่ยืนยันอายุแล้วและตรวจว่าเล่นได้
จากนั้นเลือกไฟล์ Netscape Cookies ที่ส่งออกจากเซสชันนั้น แล้วกด `DOWNLOAD` เพื่อเพิ่มงานใหม่
งานเก่าที่กด Retry ยังอ้างอิงไฟล์ Cookies ที่บันทึกไว้กับงานนั้น
[คู่มือการส่งออก YouTube Cookies ของ yt-dlp](https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies)

สำหรับ CLI ใช้ `--cookies-file` ได้ เช่น:

```bash
PixClip --record-live-streamlink "LIVE_URL" "$HOME/Videos" --cookies-file "$HOME/private/cookies.txt"
```

## TUI

เปิดเมนู Terminal จากปุ่ม TUI ข้อ 21–26 ใช้ดาวน์โหลดช่วงเวลา, โหลดเฉพาะซับ, เลือกเลขรายการในเพลย์ลิสต์, บีบอัด MP4, แยกเสียง และตัดคลิปตามเวลา

## Queue / History

- วางหลาย URL โดยหนึ่งลิงก์ต่อหนึ่งบรรทัด แล้วกด Add downloads now
- ตั้ง Parallel downloads ได้ 1–5 งาน (ค่าเริ่มต้น 1)
- Pause queue หยุดเริ่มงานใหม่ แต่ไม่หยุดงานที่กำลังทำ
- เลือกงานแล้ว Cancel, Delete, Retry, Files, Open folder หรือ Job log; Delete ลบเฉพาะรายการจาก Queue/History ไม่ลบไฟล์สื่อ
- ใน `Files` เลือกวิดีโอแล้วกด `Open in Cut clip` เพื่อเปิดบน Timeline
- History only แสดงงานที่จบแล้ว รวมงานแปลง ตัด และต่อคลิป
- งานดาวน์โหลดใหม่จัดตามประเภท เว็บไซต์ และวันเวลาที่เพิ่มงาน ภายใต้โฟลเดอร์ที่เลือก

  ```text
  Videos/YouTube/2026-11-19 14-30-00 [a1b2c3d4]/ชื่อคลิป.mp4
  Audio/YouTube/2026-11-19 14-31-00 [b2c3d4e5]/ชื่อเพลง.mp3
  Playlists/YouTube/2026-11-19 14-32-00 [c3d4e5f6]/ชื่อเพลย์ลิสต์/001 - ชื่อเพลง.mp3
  Live/Kick/2026-11-19 14-33-00 - channel [d4e5f6a7]/ชื่อไลฟ์.ts
  ```

  GUI และ CLI ใช้โครงสร้างเดียวกัน รหัสสั้นกันชื่อโฟลเดอร์ซ้ำเมื่อเพิ่มหลายงานในวินาทีเดียวกัน ประวัติเก่าเปิดจากตำแหน่งเดิม ส่วนไฟล์แปลง ตัด และต่อคลิปบันทึกตามตำแหน่งที่เลือก
- Retry ของ Playlist ที่ยังโหลดไม่ครบจะใช้โฟลเดอร์และรายการดาวน์โหลดเดิม ส่วนการลองงานตัดหรือแปลงซ้ำจะถามก่อนแทนที่ไฟล์ปลายทาง
- ประวัติ Windows: `%LOCALAPPDATA%/PixClip/jobs.json`
- ประวัติ Linux: `${XDG_STATE_HOME:-~/.local/state}/PixClip/jobs.json`
- หากแอปหยุดผิดปกติ งานที่กำลังทำจะเป็น Interrupted และต้องกด Retry เอง ส่วนคิวที่ยังไม่เริ่มยังคงอยู่
- ควรเปิด PixClip เพียงหนึ่งหน้าต่างหลักต่อผู้ใช้ เพื่อไม่ให้ไฟล์ประวัติถูกเขียนทับจากหลาย instance

งานตัด/แปลง/ต่อคลิปทำทีละงานและรอคิวดาวน์โหลดว่างก่อน ส่วน Install/Update tools ถูกปิดระหว่างทำงานเพื่อไม่เปลี่ยน executable กลางงาน

## แปลงไฟล์ในเครื่อง

เลือกโหมดใน `CONVERT` แล้วกด `CHOOSE VIDEO FILES` เพื่อเลือกหนึ่งไฟล์หรือหลายไฟล์ คิวจะแปลงทีละไฟล์และเก็บผลลัพธ์ไว้ข้างต้นฉบับ งานหลายไฟล์ใช้ชื่อใหม่เมื่อชื่อผลลัพธ์ซ้ำ

- `Compress MP4` ลดขนาดด้วย H.264/AAC เลือก Balanced (ค่าเริ่มต้น), Smaller หรือ Smallest แล้วเลือกว่าจะคงความละเอียด (Keep res.) หรือจำกัดที่ 1080p / 720p โปรแกรมคงแนววิดีโอและบันทึกเป็นไฟล์ใหม่ในโฟลเดอร์เดียวกัน ขนาดที่ลดได้ขึ้นกับไฟล์ต้นฉบับ
- `Extract Audio` ดึงเสียงเป็น MP3, M4A, FLAC หรือ WAV โดยเลือกบิตเรตสำหรับ MP3 และ M4A ได้

## TS → MP4 อัตโนมัติ

เปิด `TS -> MP4 (keep TS)` ก่อนเพิ่มงาน เมื่อได้ TS สำเร็จ โปรแกรมจะ remux ด้วย stream copy
ตรวจว่าไฟล์ไม่ว่างและความยาวสอดคล้องกับต้นฉบับ แล้วจึงใช้ชื่อ MP4 สุดท้าย
TS ต้นฉบับไม่ถูกลบ ถ้า codec ไม่เข้ากับ MP4 ให้ใช้โหมด Compatible H.264 ด้วยตนเอง
ต้องมีพื้นที่สำหรับ TS และ MP4 พร้อมกัน หากยกเลิกหรืออัดล้มเหลว โปรแกรมไม่แปลงไฟล์บางส่วนอัตโนมัติ

## ตั้งเวลาอัด Live

1. เลือก `Live - Auto GPU/CPU` ให้ PixClip ใช้ผลทดสอบความเร็วระหว่าง CPU และตัวเข้ารหัสฮาร์ดแวร์ หรือเลือก `Live - Streamlink` / `Live - NVIDIA NVENC` เอง
2. เปิด Queue / History วางลิงก์
3. ระบุวันเวลาในอนาคตตามรูปแบบ `YYYY-MM-DD HH:MM` (ถ้าจะอัดทันทีให้ใช้ปุ่ม `Download` หลัก)
4. ระบุ Duration เป็นนาที แล้วกด `Schedule Live`

ต้องเปิด PixClip และให้เครื่องตื่นอยู่ ไม่ใช่บริการอัดเบื้องหลังเมื่อปิดแอป
ระยะเวลานับเป็นช่วงเวลาอัดตามนาฬิกา รวมเวลารอต้นทางและเชื่อมต่อใหม่ จึงไม่ได้รับประกันความยาวคลิปเต็มช่วงหากสตรีมหลุด
งานตามเวลาที่ถึงกำหนดจะได้ช่องว่างก่อนคิวทั่วไป แต่ไม่แย่งหยุดงานเดิม หากช่องเต็มอาจเริ่มช้า และหากเลยเวลาสิ้นสุดจะรายงานว่าพลาดช่วงอัด
เมื่อโปรเซสอัดหลุด จะลองเปิดใหม่สูงสุด 5 ครั้ง โดยเก็บเป็นไฟล์ส่วนใหม่ ไม่เขียนทับส่วนก่อนหน้า
ใช้ Join clips รวมส่วนภายหลังได้ หากรูปแบบตรงกัน
Live ไม่มีกำหนดจบให้ใช้ปุ่ม `Download` หลัก หรือ `Add downloads now` และกด `Cancel` ในหน้าคิวเมื่อต้องการหยุด

## ตัดคลิป

รองรับคลิปแนวตั้ง เช่น 9:16 รวมถึงคลิปจากมือถือที่เก็บข้อมูลการหมุนไว้ในไฟล์ ภาพพรีวิวและไฟล์ MP4 ที่บันทึกจะคงแนวของคลิป

1. เปิด `Cut clip` แล้วกด `Choose video`
2. ลากด้ามสีมิ้นต์เพื่อกำหนด **Start** และด้ามสีอำพันเพื่อกำหนด **End**
3. คลิกแถบเพื่อดูภาพที่ตำแหน่งนั้น หรือกด **← / →** เพื่อเลื่อนทีละ 1 วินาที กด `Play` หรือ Spacebar เพื่อเล่นช่วงที่เลือกพร้อมเสียงและหยุดชั่วคราว
4. ปรับเวลาในช่อง HH:MM:SS ได้โดยตรง หรือใช้ `Set start here` / `Set end here` เพื่อใช้ตำแหน่งที่กำลังดู
5. กด `Save MP4` เพื่อเลือกไฟล์ปลายทาง ค่าเริ่มต้น `Compatible MP4` ตัดตามเวลาที่เลือก ส่วน `Fast` เร็วกว่าและเริ่มจากคีย์เฟรม

## พื้นที่ว่างและมาสคอต

Reserve GiB ในหน้าคิวเป็นพื้นที่สำรองขั้นต่ำ (เริ่มต้น 1 GiB, ต่ำสุด 0.25 GiB)
ตรวจพื้นที่ก่อนเริ่มและระหว่างงาน หากต่ำกว่าเกณฑ์จะหยุดงานและแจ้งเตือน ไม่ลบไฟล์ต้นฉบับเพื่อเคลียร์พื้นที่
พื้นที่ที่แสดงในหน้าหลักเป็นพื้นที่จริง ไม่ใช่การประมาณขนาดดาวน์โหลดทั้งหมด
มาสคอตขยับพร้อมสถานะ READY / WORKING / OK / CHECK เพื่อบอกสถานะโดยไม่เปลี่ยนธีมเดิม
PixClip ทดสอบเข้ารหัสคลิปจำลอง 1080p แล้วเก็บตัวที่เร็วที่สุดแยกสำหรับ Live และงานไฟล์ H.264 แถบสถานะจะแสดงผลทั้งสองแบบ ส่วน log จะแสดง encoder ที่ใช้ในงาน

Auto ครอบคลุม Convert H.264, Compress MP4, Cut และ Join แบบ Compatible และการแปลงไฟล์ดาวน์โหลดที่ต้องเข้ารหัสใหม่ หากฮาร์ดแวร์ทำงานพลาด โปรแกรมจะลองใช้ CPU ตัวที่รองรับขึ้นอยู่กับ FFmpeg และไดรเวอร์บนเครื่อง เช่น NVENC, AMF, Quick Sync, VA-API และ VideoToolbox บน macOS

## อัปเดตโปรแกรม

ตั้งแต่ PixClip 0.2.22 แอปจะตรวจ GitHub Releases ตอนเปิดและทุก 6 ชั่วโมง
หรือกด **HELP → ตรวจอัปเดต PixClip** เพื่อตรวจเอง เมื่อพบรุ่นใหม่จะมีหน้าต่างแจ้งเตือน

กด **อัปเดตและเปิดใหม่** เพื่อดาวน์โหลดแพ็กเกจที่ตรงกับเครื่องและตรวจขนาดกับ SHA-256
จากนั้นโปรแกรมจะปิด ติดตั้ง และเปิดใหม่ โดยต้องให้งานในคิวเสร็จก่อน ประวัติ การตั้งค่า
และไฟล์สื่อจะยังอยู่ ระบบอาจขอรหัสผ่านหรือยืนยันสิทธิ์ก่อนติดตั้ง

- Windows x64: ใช้ MSI; EXE พกพาจะย้ายไปเป็นแอปของบัญชีผู้ใช้
- Debian/Ubuntu: ใช้ apt ติดตั้ง DEB, Fedora/openSUSE ใช้ dnf/zypper ติดตั้ง RPM,
  Arch ใช้ pacman โดยยืนยันสิทธิ์ผ่าน PolicyKit (`pkexec`)
- macOS: ดาวน์โหลด Universal DMG ตรวจแอปใหม่ก่อนเปลี่ยนไฟล์ และคืนแอปเดิมหากเปลี่ยนไม่สำเร็จ
- Flatpak: ดาวน์โหลดและตรวจไฟล์ในแอป แล้วเปิดตัวติดตั้งระบบให้ยืนยัน จากนั้นเปิด PixClip ใหม่

หากดาวน์โหลดผิดพลาดแอปจะยังเปิดอยู่ หากติดตั้งผิดพลาดจะลองเปิดแอปเดิมและแสดงข้อผิดพลาด
รุ่นเก่าที่ยังไม่มีฟีเจอร์นี้ต้องติดตั้ง build ใหม่ด้วยตนเองครั้งแรก

## ทดสอบ

```text
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo test --locked -- --ignored
```

ชุด integration ใช้คลิปสังเคราะห์และ localhost ต้องมี FFmpeg/ffprobe และ yt-dlp
ไม่ได้ยืนยันเว็บไซต์สาธารณะหรือ GPU ทุกเครื่อง
ผู้พัฒนาใช้ `PIXCLIP_STATE_DIR` ชี้ไปโฟลเดอร์ทดสอบเพื่อแยกจากประวัติจริง

## License

[MIT](../LICENSE). ฟอนต์ Noto Sans Thai ใช้ [OFL](../assets/fonts/OFL.txt)
เครื่องมือภายนอกอยู่ภายใต้ใบอนุญาตของเจ้าของโครงการนั้น

## สนับสนุน PixClip

หาก PixClip มีประโยชน์และต้องการสนับสนุนการพัฒนาต่อ สามารถโดเนทผ่าน Ko-fi ได้ที่:

[สนับสนุน PixClip ผ่าน Ko-fi](https://ko-fi.com/peaknatanon)
