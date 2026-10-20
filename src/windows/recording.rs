use super::{
    audio::{self, Audio},
    capture::{self, Grabber},
    media::{Com, Writer},
    paths,
};
use crate::{geometry::Rect, pixels::Frame, timeline::*};
use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub struct Outcome {
    pub path: PathBuf,
    pub warnings: Vec<String>,
    pub dropped: u64,
}

pub fn record(
    region: Rect,
    folder: &Path,
    stop: Arc<AtomicBool>,
    started: impl FnOnce(),
) -> Result<Outcome> {
    let _com = Com::new()?;
    let (w, h) = region.output_size()?;
    let (partial, reservation) = paths::reserve(folder, "partial.mp4")?;
    drop(reservation);
    let writer = Writer::new(&partial, w, h)?;
    let grabber = Grabber::new(region, false)?;
    let first = grabber.grab(true)?.nv12()?;
    drop(grabber);
    let epoch_qpc = audio::clock_100ns()?;
    let epoch = Instant::now();
    log::info!(
        "Recording started: {}; {}x{} at {FPS} fps",
        partial.display(),
        w,
        h
    );
    let mut audio = Audio::start(epoch_qpc);
    let capture_stop = Arc::new(AtomicBool::new(false));
    let end = capture_stop.clone();
    let (tx, rx) = mpsc::sync_channel(2);
    let capture = thread::spawn(move || -> Result<()> {
        let grabber = Grabber::new(region, false)?;
        while !end.load(Ordering::Acquire) {
            let index = (epoch.elapsed().as_secs_f64() * FPS as f64) as u64;
            let frame = grabber.grab(true)?.nv12()?;
            match tx.try_send((index, frame)) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
            let next = Duration::from_secs_f64((index + 1) as f64 / FPS as f64);
            if let Some(delay) = next.checked_sub(epoch.elapsed()) {
                thread::sleep(delay);
            }
        }
        Ok(())
    });
    started();
    let mut pending = (0, first);
    let mut audio_pos = 0;
    let mut dropped = 0;
    let result = (|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok((index, frame)) if index > pending.0 => {
                    writer.video(
                        &pending.1,
                        frame_time(pending.0),
                        frame_time(index - pending.0),
                    )?;
                    dropped += index - pending.0 - 1;
                    pending = (index, frame);
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Screen capture worker stopped")
                }
            }
            // Leave 100 ms for device packets to arrive; silence covers missing devices.
            let available = epoch.elapsed().saturating_sub(Duration::from_millis(100));
            write_audio(
                &writer,
                &mut audio,
                &mut audio_pos,
                (available.as_secs_f64() * SAMPLE_RATE as f64) as u64,
                false,
            )?;
        }
        Ok(())
    })();
    let duration = epoch.elapsed();
    if dropped > 0 {
        log::warn!(
            "Video skipped {dropped} frames under load in {:.1} seconds",
            duration.as_secs_f64()
        );
    }
    capture_stop.store(true, Ordering::Release);
    let capture_result = capture
        .join()
        .map_err(|_| anyhow::anyhow!("Capture worker panicked"))?;
    audio.stop();
    result?;
    capture_result?;
    let end = (duration.as_nanos() / 100) as i64;
    writer.video(
        &pending.1,
        frame_time(pending.0),
        (end - frame_time(pending.0)).max(1),
    )?;
    write_audio(
        &writer,
        &mut audio,
        &mut audio_pos,
        (duration.as_secs_f64() * SAMPLE_RATE as f64).ceil() as u64,
        true,
    )?;
    let warnings = audio.warnings();
    writer.finish()?;
    let path = partial.with_file_name(
        partial
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".partial.mp4", ".mp4"),
    );
    // Windows rename fails if destination already exists; never overwrite.
    std::fs::rename(&partial, &path).context("Publish completed MP4")?;
    log::info!(
        "Saved {}; {:.1} seconds",
        path.display(),
        duration.as_secs_f64()
    );
    Ok(Outcome {
        path,
        warnings,
        dropped,
    })
}

fn write_audio(
    writer: &Writer,
    audio: &mut Audio,
    position: &mut u64,
    end: u64,
    final_block: bool,
) -> Result<()> {
    let aligned_end = audio_boundary(end, final_block);
    while *position < aligned_end {
        let count = MP3_SAMPLES as usize;
        let pcm = pcm16(&audio.mix(*position, count));
        writer.audio(
            &pcm,
            sample_time(*position),
            sample_time(*position + count as u64) - sample_time(*position),
        )?;
        *position += count as u64;
    }
    Ok(())
}

pub fn smoke_test(folder: &Path, live: bool) -> Result<String> {
    if live {
        let region = capture::desktop()?;
        let frame = Grabber::new(region, false)?.grab(true)?;
        capture::clipboard(&frame)?;
        let jpeg = capture::jpeg(&frame, folder)?;
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let timer = thread::spawn(move || {
            thread::sleep(Duration::from_secs(4));
            end.store(true, Ordering::Release);
        });
        let result = record(region, folder, stop, || {});
        let _ = timer.join();
        let result = result?;
        return Ok(format!(
            "OK\nJPEG={}\nMP4={}\nWarnings={}\nDropped={}\n",
            jpeg.display(),
            result.path.display(),
            result.warnings.join("; "),
            result.dropped
        ));
    }
    let _com = Com::new()?;
    let (partial, reservation) = paths::reserve(folder, "partial.mp4")?;
    drop(reservation);
    let writer = Writer::new(&partial, 640, 360)?;
    let mut audio_pos = 0;
    for n in 0..30u64 {
        let mut frame = Frame {
            width: 640,
            height: 360,
            bgra: vec![0; 640 * 360 * 4],
        };
        for (i, p) in frame.bgra.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (x, y) = (i % 640, i / 640);
            p.copy_from_slice(&[
                if x < (n as usize * 20) % 640 { 240 } else { 20 },
                (y * 255 / 360) as u8,
                120,
                255,
            ]);
        }
        if n == 0 {
            capture::jpeg(&frame, folder)?;
        }
        writer.video(&frame.nv12()?, frame_time(n), frame_time(1))?;
        while audio_pos < audio_boundary((n + 1) * 4800, n == 29) {
            let samples = (0..MP3_SAMPLES)
                .map(|i| {
                    ((audio_pos + i) as f64 * std::f64::consts::TAU * 440.0 / 48000.0).sin() as f32
                        * 0.2
                })
                .collect::<Vec<_>>();
            writer.audio(
                &pcm16(&samples),
                sample_time(audio_pos),
                sample_time(MP3_SAMPLES),
            )?;
            audio_pos += MP3_SAMPLES;
        }
    }
    writer.finish()?;
    let path = partial.with_file_name(
        partial
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".partial.mp4", ".mp4"),
    );
    std::fs::rename(&partial, &path)?;
    Ok(format!(
        "OK\nMP4={}\nExpected=H264 640x360 10fps + MP3 mono 48000Hz 64kbps; 3 seconds\n",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_software_h264_mp3_smoke() {
        let folder = std::env::temp_dir().join(format!(
            "quickcap-smoke-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let result = super::smoke_test(&folder, false).unwrap();
        assert!(result.starts_with("OK\n"));
        let paths = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        let mp4 = paths
            .iter()
            .find(|p| p.extension().is_some_and(|e| e == "mp4"))
            .unwrap();
        assert!(!mp4.to_string_lossy().contains("partial"));
        let bytes = std::fs::read(mp4).unwrap();
        for tag in [b"ftyp", b"moov", b"avc1", b"mp4a"] {
            assert!(bytes.windows(4).any(|s| s == tag));
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
}
