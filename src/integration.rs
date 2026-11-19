use crate::{
    jobs::Engine,
    media,
    model::{self, Job, State},
    process::Context,
    tools,
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn context(folder: &Path) -> Context {
    Context {
        cancel: Arc::new(AtomicBool::new(false)),
        deadline: None,
        folder: folder.into(),
        reserve: 0.25,
        log: Arc::new(|_| {}),
    }
}

fn sample(folder: &Path) -> PathBuf {
    let output = folder.join("sample.mp4");
    let result = context(folder)
        .run(
            &tools::require("ffmpeg").unwrap(),
            &media::args(&[
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x180:rate=25",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                &media::text(&output),
            ]),
            true,
            false,
        )
        .unwrap();
    assert_eq!(result.code, 0, "{}", result.text);
    output
}

struct Server {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(source: &Path) -> Self {
        let bytes = fs::read(source).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}/sample.mp4", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let mut request = Vec::new();
                        let mut buffer = [0; 4096];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 65536 {
                            let count = stream.read(&mut buffer).unwrap_or(0);
                            if count == 0 {
                                break;
                            }
                            request.extend_from_slice(&buffer[..count]);
                        }
                        let header = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(header.as_bytes());
                        if !request.starts_with(b"HEAD ") {
                            let _ = stream.write_all(&bytes);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
#[ignore = "Requires real FFmpeg, ffprobe and yt-dlp; localhost only"]
fn real_download_queue_remux_preview_cut_join_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample(dir.path());
    let server = Server::new(&source);
    let mut engine = Engine::new(&dir.path().join("state")).unwrap();
    engine.history.limit = 2;
    let mut first = Job {
        url: server.address.clone(),
        mode: media::TS.into(),
        folder: dir.path().join("downloads"),
        auto_mp4: true,
        reserve_gb: 0.25,
        ..Job::default()
    };
    let first_id = engine.add(first.clone()).unwrap();
    first.id = uuid::Uuid::new_v4().simple().to_string();
    first.mode = media::MODES[1].into();
    let second_id = engine.add(first).unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut saw_parallel = false;
    while engine.pending() && Instant::now() < deadline {
        engine.tick();
        assert!(engine.active_count() <= 2);
        saw_parallel |= engine.active_count() == 2;
        thread::sleep(Duration::from_millis(30));
    }
    for id in [&first_id, &second_id] {
        let job = engine.job(id).unwrap();
        assert_eq!(job.state, State::Done, "{}\n{:?}", job.detail, job.logs);
        assert!(!job.directory().join(".work").exists());
    }
    assert!(saw_parallel);
    let outputs = engine.job(&first_id).unwrap().outputs.clone();
    let ts = outputs
        .iter()
        .find(|p| p.extension().unwrap() == "ts")
        .unwrap();
    let mp4 = outputs
        .iter()
        .find(|p| p.extension().unwrap() == "mp4")
        .unwrap();
    let ts_duration = media::duration(ts).unwrap();
    assert!(ts_duration > 2.5);
    assert!(
        media::preview(ts, 1.0, ts_duration)
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    assert!(media::duration(mp4).unwrap() > 2.5);
    let duration = media::duration(&source).unwrap();
    let png = media::preview(&source, 1.0, duration).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    assert!(duration > 2.5);
    let ctx = context(dir.path());
    let mut cuts = vec![];
    for index in 0..2 {
        let output = dir.path().join(format!("cut-{index}.mp4"));
        let args = media::cut(&source, &output, index as f64, index as f64 + 1.0, true).unwrap();
        let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = lines.clone();
        let mut cut_ctx = ctx.clone();
        cut_ctx.log = Arc::new(move |line| captured.lock().unwrap().push(line));
        media::execute(
            &Job {
                kind: "local".into(),
                mode: "Cutting clip".into(),
                folder: dir.path().into(),
                arguments: args,
                inputs: vec![source.clone()],
                reserve_gb: 0.25,
                ..Job::default()
            },
            &cut_ctx,
        )
        .unwrap();
        assert!(
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == "media_duration=1"),
            "Cut progress must use the selected segment length"
        );
        cuts.push(output);
    }
    let joined = dir.path().join("joined.mp4");
    assert!(media::fast_join_compatible(&cuts).unwrap());
    let list = dir.path().join("concat.txt");
    let (args, content) = media::join(&cuts, &joined, &list, false).unwrap();
    fs::write(list, content).unwrap();
    let output = ctx
        .run(&tools::require("ffmpeg").unwrap(), &args, true, false)
        .unwrap();
    assert_eq!(output.code, 0, "{}", output.text);
    assert!(media::duration(&joined).unwrap() > 1.8);
    let different = dir.path().join("different-size.mp4");
    let result = ctx
        .run(
            &tools::require("ffmpeg").unwrap(),
            &media::args(&[
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                &media::text(&source),
                "-vf",
                "scale=160:90",
                "-c:v",
                "libx264",
                "-c:a",
                "copy",
                &media::text(&different),
            ]),
            true,
            false,
        )
        .unwrap();
    assert_eq!(result.code, 0, "{}", result.text);
    assert!(!media::fast_join_compatible(&[source.clone(), different.clone()]).unwrap());
    let compatible_join = dir.path().join("compatible-join.mp4");
    let list = dir.path().join("compatible-concat.txt");
    let (args, content) =
        media::join(&[source.clone(), different], &compatible_join, &list, true).unwrap();
    fs::write(list, content).unwrap();
    let result = ctx
        .run(&tools::require("ffmpeg").unwrap(), &args, true, false)
        .unwrap();
    assert_eq!(result.code, 0, "{}", result.text);
    assert!(media::duration(&compatible_join).unwrap() > 5.5);
    for mode in [1, 3] {
        let (output, args) = media::convert(&source, media::Conversion::Video(mode)).unwrap();
        let result = ctx
            .run(&tools::require("ffmpeg").unwrap(), &args, true, false)
            .unwrap();
        assert_eq!(result.code, 0, "{}", result.text);
        assert!(media::duration(&output).unwrap() > 2.5);
        let info = media::probe(&output).unwrap();
        assert!(
            info["streams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["codec_name"] == "h264")
        );
        assert!(
            info["streams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["codec_name"] == "aac")
        );
    }
    let fast_cut = dir.path().join("fast-cut.mp4");
    let args = media::cut(&source, &fast_cut, 0.0, 2.0, false).unwrap();
    let result = ctx
        .run(&tools::require("ffmpeg").unwrap(), &args, true, false)
        .unwrap();
    assert_eq!(result.code, 0, "{}", result.text);
    assert!(media::duration(&fast_cut).unwrap() > 1.5);
    let before = fs::read(mp4).unwrap();
    assert!(media::remux(ts, &ctx).is_err());
    assert_eq!(fs::read(mp4).unwrap(), before);
    engine.cancel(&first_id);
    engine.delete(&first_id).unwrap();
    assert!(ts.is_file() && mp4.is_file());
    drop(engine);
    let restored = Engine::new(&dir.path().join("state")).unwrap();
    assert_eq!(restored.history.jobs.len(), 1);
}

#[test]
#[ignore = "Requires real FFmpeg and ffprobe; synthetic Live only"]
fn timed_live_and_metadata_deadline_stop_without_losing_ts() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample(dir.path());
    let mut ctx = context(dir.path());
    ctx.deadline = Some(model::now() + 2.0);
    let target = dir.path().join("live.ts");
    let mut args = media::live_arguments(&media::text(&source), &target, &crate::encoder::CPU);
    let input = args.iter().position(|s| s == "-i").unwrap();
    args.splice(input..input, media::args(&["-re", "-stream_loop", "-1"]));
    let start = Instant::now();
    let result = ctx
        .run(&tools::require("ffmpeg").unwrap(), &args, false, true)
        .unwrap();
    assert!(result.timed_out);
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(media::duration(&target).unwrap() > 0.0);
    assert!(media::remux(&target, &ctx).unwrap().is_file());
    assert!(target.is_file());
    ctx.deadline = Some(model::now() + 0.1);
    let mut metadata_args = args.clone();
    *metadata_args.last_mut().unwrap() = media::text(&dir.path().join("metadata.ts"));
    let start = Instant::now();
    let result = ctx.run(
        &tools::require("ffmpeg").unwrap(),
        &metadata_args,
        true,
        false,
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}
