use super::media::Com;
use crate::timeline::{AudioPacket, SAMPLE_RATE, mix_packet};
use anyhow::{Context, Result, ensure};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Media::Audio::*,
    System::{Com::*, Performance::*},
};

pub fn clock_100ns() -> Result<u64> {
    let (mut count, mut frequency) = (0, 0);
    // SAFETY: Both output pointers refer to valid stack variables.
    unsafe {
        QueryPerformanceCounter(&mut count)?;
        QueryPerformanceFrequency(&mut frequency)?;
    }
    Ok((count as u128 * 10_000_000 / frequency as u128) as u64)
}

pub struct Audio {
    receivers: Vec<Receiver<AudioPacket>>,
    pending: Vec<VecDeque<AudioPacket>>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    warnings: Receiver<String>,
}
impl Audio {
    pub fn start(epoch: u64) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (warnings_tx, warnings) = mpsc::sync_channel(8);
        let mut receivers = Vec::new();
        let mut workers = Vec::new();
        for (flow, label) in [(eRender, "System audio"), (eCapture, "Microphone")] {
            let (tx, rx) = mpsc::sync_channel(32);
            receivers.push(rx);
            let stop = stop.clone();
            let warning = warnings_tx.clone();
            workers.push(thread::spawn(move || {
                if let Err(e) = capture(flow, epoch, &stop, tx) {
                    let _ = warning.try_send(format!("{label} unavailable: {e:#}"));
                }
            }));
        }
        Self {
            pending: vec![VecDeque::new(), VecDeque::new()],
            receivers,
            stop,
            workers,
            warnings,
        }
    }
    pub fn mix(&mut self, start: u64, len: usize) -> Vec<f32> {
        let mut output = vec![0.0; len];
        for (rx, pending) in self.receivers.iter().zip(&mut self.pending) {
            while let Ok(packet) = rx.try_recv() {
                // Limit queued future audio independently of device/clock behavior.
                if pending.len() < 64 {
                    pending.push_back(packet);
                }
            }
            for p in pending.iter() {
                mix_packet(&mut output, start, p);
            }
            pending.retain(|p| p.start + p.samples.len() as u64 > start + len as u64);
        }
        output
    }
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.try_iter().collect()
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.stop();
    }
}

fn capture(
    flow: EDataFlow,
    epoch: u64,
    stop: &AtomicBool,
    tx: SyncSender<AudioPacket>,
) -> Result<()> {
    let _com = Com::new()?;
    // SAFETY: COM interfaces stay on this initialized thread. WASAPI packet
    // buffers are copied before ReleaseBuffer; format allocation freed once.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(flow, eConsole)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        let format = client.GetMixFormat()?;
        let f = *format;
        let float = if f.wFormatTag == 0xfffe {
            ensure!(f.cbSize >= 22, "Invalid extensible audio format");
            std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>())
                .SubFormat
                .data1
                == 3
        } else {
            f.wFormatTag == 3
        };
        let flags = if flow == eRender {
            AUDCLNT_STREAMFLAGS_LOOPBACK
        } else {
            0
        };
        let initialized =
            client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, format, None);
        CoTaskMemFree(Some(format.cast()));
        initialized.context("Initialize WASAPI capture")?;
        ensure!(
            f.nChannels > 0 && f.nChannels <= 32 && f.nSamplesPerSec > 0,
            "Invalid device format"
        );
        let bits = f.wBitsPerSample;
        ensure!(
            (float && bits == 32) || (!float && [16, 24, 32].contains(&bits)),
            "Unsupported audio sample format"
        );
        let service: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let result = (|| -> Result<()> {
            while !stop.load(Ordering::Acquire) {
                while service.GetNextPacketSize()? > 0 {
                    let (mut data, mut frames, mut flags, mut qpc) =
                        (std::ptr::null_mut(), 0, 0, 0);
                    service.GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))?;
                    let packet = (|| -> Result<AudioPacket> {
                        let bytes_per_frame = f.nBlockAlign as usize;
                        let bytes_per_sample = (f.wBitsPerSample / 8) as usize;
                        ensure!(
                            bytes_per_frame >= bytes_per_sample * f.nChannels as usize,
                            "Invalid block alignment"
                        );
                        let mono = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                            vec![0.0; frames as usize]
                        } else {
                            ensure!(!data.is_null(), "Missing WASAPI data");
                            let bytes =
                                std::slice::from_raw_parts(data, frames as usize * bytes_per_frame);
                            bytes
                                .chunks_exact(bytes_per_frame)
                                .map(|frame| {
                                    frame[..bytes_per_sample * f.nChannels as usize]
                                        .chunks_exact(bytes_per_sample)
                                        .map(|s| {
                                            if float {
                                                f32::from_le_bytes(s.try_into().unwrap())
                                            } else {
                                                match bytes_per_sample {
                                                    2 => {
                                                        i16::from_le_bytes(s.try_into().unwrap())
                                                            as f32
                                                            / 32768.0
                                                    }
                                                    3 => {
                                                        ((s[0] as i32) << 8
                                                            | (s[1] as i32) << 16
                                                            | (s[2] as i32) << 24)
                                                            as f32
                                                            / 2147483648.0
                                                    }
                                                    _ => {
                                                        i32::from_le_bytes(s.try_into().unwrap())
                                                            as f32
                                                            / 2147483648.0
                                                    }
                                                }
                                            }
                                        })
                                        .sum::<f32>()
                                        / f.nChannels as f32
                                })
                                .collect::<Vec<_>>()
                        };
                        let count =
                            (frames as u64 * SAMPLE_RATE as u64 / f.nSamplesPerSec as u64) as usize;
                        let samples = (0..count)
                            .map(|i| {
                                let pos = i as f64 * f.nSamplesPerSec as f64 / SAMPLE_RATE as f64;
                                let at = (pos as usize).min(mono.len().saturating_sub(1));
                                let a = mono.get(at).copied().unwrap_or(0.0);
                                let b = mono.get(at + 1).copied().unwrap_or(a);
                                let value = a + (b - a) * (pos - pos.floor()) as f32;
                                if value.is_finite() { value } else { 0.0 }
                            })
                            .collect::<Vec<_>>();
                        let qpc = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                            clock_100ns()?.saturating_sub(
                                frames as u64 * 10_000_000 / f.nSamplesPerSec as u64,
                            )
                        } else {
                            qpc
                        };
                        // Discard pre-epoch samples instead of wrapping negative time.
                        let trim = ((epoch.saturating_sub(qpc) as u128 * SAMPLE_RATE as u128)
                            / 10_000_000) as usize;
                        Ok(AudioPacket {
                            start: qpc.saturating_sub(epoch) * SAMPLE_RATE as u64 / 10_000_000,
                            samples: samples.into_iter().skip(trim).collect(),
                        })
                    })();
                    let release = service.ReleaseBuffer(frames);
                    let packet = packet?;
                    release?;
                    let _ = tx.try_send(packet); // Bounded queue; drop late audio under load.
                }
                thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        })();
        let _ = client.Stop();
        result
    }
}
