use crate::timeline::{FPS, SAMPLE_RATE, VIDEO_BITRATE};
use anyhow::{Context, Result};
use std::path::Path;
use windows::{
    Win32::{Media::MediaFoundation::*, System::Com::*},
    core::*,
};

pub struct Com;
impl Com {
    pub fn new() -> Result<Self> {
        // SAFETY: Each worker initializes MTA once and drops COM objects before guard.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: Matches successful CoInitializeEx on this thread.
        unsafe {
            CoUninitialize();
        }
    }
}
struct Foundation;
impl Foundation {
    fn new() -> Result<Self> {
        // SAFETY: Balanced MF startup/shutdown; no work-queue callbacks call shutdown.
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        }
        Ok(Self)
    }
}
impl Drop for Foundation {
    fn drop(&mut self) {
        // SAFETY: All writer references have been dropped before this guard.
        unsafe {
            let _ = MFShutdown();
        }
    }
}

/// Sink writer's hardware-transform flag is explicitly disabled. NV12 input
/// avoids a GPU video processor or RGB conversion transform altogether.
pub struct Writer {
    writer: IMFSinkWriter,
    video: u32,
    audio: u32,
    _mf: Foundation,
}
impl Writer {
    pub fn new(path: &Path, width: u32, height: u32) -> Result<Self> {
        let mf = Foundation::new()?;
        // SAFETY: Media types are initialized before use, strings remain live,
        // and all COM calls execute on the owning worker's MTA.
        unsafe {
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs = attrs.unwrap();
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 0)?;
            attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let writer = MFCreateSinkWriterFromURL(PCWSTR(name.as_ptr()), None, &attrs)
                .context("Create MP4 writer")?;
            let vout = MFCreateMediaType()?;
            vout.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            vout.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            video_attrs(&vout, width, height)?;
            vout.SetUINT32(&MF_MT_AVG_BITRATE, VIDEO_BITRATE)?;
            vout.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
            let video = writer.AddStream(&vout)?;
            let vin = MFCreateMediaType()?;
            vin.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            vin.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            video_attrs(&vin, width, height)?;
            let mut encoding = None;
            MFCreateAttributes(&mut encoding, 3)?;
            let encoding = encoding.unwrap();
            encoding.SetUINT32(
                &CODECAPI_AVEncNumWorkerThreads,
                std::thread::available_parallelism().map_or(2, |n| n.get().min(8)) as u32,
            )?;
            encoding.SetUINT32(&CODECAPI_AVEncCommonQualityVsSpeed, 25)?;
            encoding.SetUINT32(
                &CODECAPI_AVEncCommonRateControlMode,
                eAVEncCommonRateControlMode_CBR.0 as u32,
            )?;
            writer
                .SetInputMediaType(video, &vin, &encoding)
                .context("Configure software H.264 encoder")?;
            // Use the encoder's advertised type, including its required
            // MPEGLAYER3WAVEFORMAT extra data; a hand-built partial type fails.
            let types = MFTranscodeGetAudioOutputAvailableTypes(
                &MFAudioFormat_MP3,
                MFT_ENUM_FLAG_SYNCMFT.0 as u32
                    | MFT_ENUM_FLAG_LOCALMFT.0 as u32
                    | MFT_ENUM_FLAG_SORTANDFILTER.0 as u32,
                None,
            )?;
            let mut aout = None;
            for i in 0..types.GetElementCount()? {
                let candidate: IMFMediaType = types.GetElement(i)?.cast()?;
                if candidate.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) == Ok(1)
                    && candidate.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
                        == Ok(SAMPLE_RATE as u32)
                    && candidate.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND) == Ok(8000)
                {
                    aout = Some(candidate);
                    break;
                }
            }
            let aout =
                aout.context("Windows MP3 encoder does not advertise 48 kHz mono / 64 kbps")?;
            let audio = writer.AddStream(&aout)?;
            let ain = MFCreateMediaType()?;
            ain.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            ain.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            ain.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
            ain.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, SAMPLE_RATE as u32)?;
            ain.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            ain.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2)?;
            ain.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, SAMPLE_RATE as u32 * 2)?;
            writer
                .SetInputMediaType(audio, &ain, None)
                .context("Configure MP3 encoder")?;
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                video,
                audio,
                _mf: mf,
            })
        }
    }
    pub fn video(&self, bytes: &[u8], time: i64, duration: i64) -> Result<()> {
        self.write(self.video, bytes, time, duration)
    }
    pub fn audio(&self, bytes: &[u8], time: i64, duration: i64) -> Result<()> {
        self.write(self.audio, bytes, time, duration)
    }
    fn write(&self, stream: u32, bytes: &[u8], time: i64, duration: i64) -> Result<()> {
        // SAFETY: Buffer allocation matches slice length; unlock precedes sample
        // submission. Writer retains COM references, never Rust slice pointers.
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes.len() as u32)?;
            let mut dst = std::ptr::null_mut();
            buffer.Lock(&mut dst, None, None)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer
                .WriteSample(stream, &sample)
                .context("Encode media sample")?;
        }
        Ok(())
    }
    pub fn finish(self) -> Result<()> {
        // SAFETY: Called once after the final sample; writer is still live.
        unsafe { self.writer.Finalize().context("Finalize MP4") }
    }
}
unsafe fn video_attrs(mt: &IMFMediaType, w: u32, h: u32) -> windows::core::Result<()> {
    // SAFETY: Caller provides a live, initialized media type; all values are scalars.
    unsafe {
        mt.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        mt.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        mt.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        mt.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32)?;
        Ok(())
    }
}
use std::os::windows::ffi::OsStrExt;
