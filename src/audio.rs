//! Audio capture + encode via WASAPI + Media Foundation AAC.
//!
//! - L3a: capture desktop/game audio from the default render endpoint in
//!   *loopback* mode.
//! - L3b-1: convert the captured PCM to 16-bit and encode it to AAC with an MF
//!   audio encoder, pushing AAC frames into a shared audio ring buffer (muxed
//!   with video at save time). Mic (L3b-2) comes next.
//!
//! All COM lives on the capture thread (WASAPI/MF interfaces aren't `Send`),
//! so nothing crosses a thread boundary. `UNVERIFIED` until it runs on Windows.

use anyhow::{Context, Result};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaType, IMFSample, IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFStartup, MFTEnumEx, MFAudioFormat_AAC, MFAudioFormat_PCM, MFMediaType_Audio,
    MFT_CATEGORY_AUDIO_ENCODER, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_OUTPUT_DATA_BUFFER, MFT_REGISTER_TYPE_INFO, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use crate::config::AudioMode;
use crate::ringbuffer::{EncodedFrame, RingBuffer};

const MF_VERSION: u32 = (0x0002 << 16) | 0x0070;
const MFSTARTUP_FULL: u32 = 0;
const HNS_PER_SEC: i64 = 10_000_000;

/// Which encoded audio tracks a saved clip should carry (used from L4).
pub struct AudioTracks {
    pub game: Vec<EncodedFrame>,
    pub mic: Vec<EncodedFrame>,
}

struct Worker {
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Running audio capture (game loopback + optional mic). Dropping it stops all
/// capture threads.
pub struct AudioCapture {
    workers: Vec<Worker>,
}

impl AudioCapture {
    /// Start capture per `mode`: game audio (loopback) into `game_ring`, and —
    /// when the mode includes the mic — mic audio into `mic_ring`. `None` is a
    /// no-op.
    pub fn start(
        mode: AudioMode,
        game_ring: Arc<Mutex<RingBuffer>>,
        mic_ring: Arc<Mutex<RingBuffer>>,
    ) -> Result<AudioCapture> {
        let mut workers = Vec::new();
        if matches!(mode, AudioMode::None) {
            return Ok(AudioCapture { workers });
        }
        // Game/desktop audio via render-endpoint loopback.
        workers.push(spawn_worker("audio-game", game_ring, eRender, true));
        // Microphone (no loopback) when the mode wants it.
        if matches!(
            mode,
            AudioMode::GameAndMicSeparate | AudioMode::GameAndMicMixed
        ) {
            workers.push(spawn_worker("audio-mic", mic_ring, eCapture, false));
        }
        Ok(AudioCapture { workers })
    }

    /// L4 will return the buffered audio to pair with a saved clip.
    pub fn extract_last(&self, _seconds: u32) -> AudioTracks {
        AudioTracks {
            game: Vec::new(),
            mic: Vec::new(),
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        for w in &self.workers {
            w.shutdown.store(true, Ordering::Relaxed);
        }
        for w in &mut self.workers {
            if let Some(t) = w.thread.take() {
                let _ = t.join();
            }
        }
    }
}

fn spawn_worker(
    name: &str,
    ring: Arc<Mutex<RingBuffer>>,
    dataflow: EDataFlow,
    loopback: bool,
) -> Worker {
    let shutdown = Arc::new(AtomicBool::new(false));
    let sd = shutdown.clone();
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if let Err(e) = capture_loop(&sd, &ring, dataflow, loopback) {
                log::error!("audio capture failed: {e:#}");
            }
        })
        .expect("spawn audio capture thread");
    Worker {
        shutdown,
        thread: Some(thread),
    }
}

/// Capture audio from `dataflow` (render+loopback for game, or capture for
/// mic), encode to AAC, push into `ring`.
fn capture_loop(
    shutdown: &AtomicBool,
    ring: &Arc<Mutex<RingBuffer>>,
    dataflow: EDataFlow,
    loopback: bool,
) -> Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("CoInitializeEx")?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup(audio)")?;

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("MMDeviceEnumerator")?;
        let device = enumerator
            .GetDefaultAudioEndpoint(dataflow, eConsole)
            .context("GetDefaultAudioEndpoint")?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None).context("Activate")?;

        let mix = client.GetMixFormat().context("GetMixFormat")?;
        let (rate, src_channels, src_bits, _src_block) = {
            let w = &*mix;
            (w.nSamplesPerSec, w.nChannels, w.wBitsPerSample, w.nBlockAlign)
        };
        // AAC input: 16-bit PCM, 1–2 channels. Downmix >2 channels to stereo.
        let channels: u16 = src_channels.min(2).max(1);

        let stream_flags = if loopback {
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        } else {
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        };
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                stream_flags,
                10_000_000,
                0,
                mix,
                None,
            )
            .context("IAudioClient::Initialize")?;

        let event = CreateEventW(None, false, false, None).context("CreateEventW")?;
        client.SetEventHandle(event).context("SetEventHandle")?;
        let capture: IAudioCaptureClient = client.GetService().context("GetService capture")?;

        let encoder = create_aac_encoder(rate, channels).context("create AAC encoder")?;

        client.Start().context("IAudioClient::Start")?;
        log::info!(
            "audio started: capture {rate}Hz {src_channels}ch {src_bits}-bit -> AAC {channels}ch"
        );

        let mut total_frames: i64 = 0;
        let mut aac_frames: u64 = 0;
        let mut last = Instant::now();
        // First-buffer markers (per worker) so an audio-path crash is pinpointed.
        let tag = if loopback { "audio-game" } else { "audio-mic" };
        let (mut lg_buf, mut lg_pcm, mut lg_in) = (false, false, false);
        macro_rules! once {
            ($flag:ident, $($a:tt)*) => {
                if !$flag { log::info!($($a)*); $flag = true; }
            };
        }

        while !shutdown.load(Ordering::Relaxed) {
            let _ = WaitForSingleObject(event, 200);
            loop {
                let mut pdata: *mut u8 = std::ptr::null_mut();
                let mut nframes: u32 = 0;
                let mut flags: u32 = 0;
                if capture
                    .GetBuffer(&mut pdata, &mut nframes, &mut flags, None, None)
                    .is_err()
                    || nframes == 0
                {
                    break;
                }
                once!(lg_buf, "{tag}: first buffer ({nframes} frames, {src_bits}-bit src)");

                let pcm = to_i16_pcm(pdata, nframes, src_channels, src_bits, channels);
                once!(lg_pcm, "{tag}: first PCM converted ({} bytes)", pcm.len());
                let pts = total_frames * HNS_PER_SEC / rate as i64;
                let dur = nframes as i64 * HNS_PER_SEC / rate as i64;
                total_frames += nframes as i64;

                if let Ok(sample) = make_pcm_sample(&pcm, pts, dur) {
                    if encoder.ProcessInput(0, &sample, 0).is_ok() {
                        once!(lg_in, "{tag}: first AAC ProcessInput accepted");
                        drain_aac(&encoder, ring, &mut aac_frames);
                    }
                }
                let _ = capture.ReleaseBuffer(nframes);
            }
            if last.elapsed().as_secs() >= 1 {
                let kb = ring.lock().map(|r| r.bytes_used() / 1024).unwrap_or(0);
                log::info!("audio: {aac_frames} AAC frames, buffer {kb} KB");
                last = Instant::now();
            }
        }

        let _ = client.Stop();
        CoTaskMemFree(Some(mix as *const _));
        CoUninitialize();
    }
    Ok(())
}

/// Convert a WASAPI capture buffer to interleaved 16-bit PCM, downmixing to
/// `out_channels`. Handles 32-bit float and 16-bit PCM sources.
unsafe fn to_i16_pcm(
    data: *const u8,
    nframes: u32,
    src_channels: u16,
    src_bits: u16,
    out_channels: u16,
) -> Vec<u8> {
    let frames = nframes as usize;
    let sc = src_channels as usize;
    let oc = out_channels as usize;
    let mut out = Vec::with_capacity(frames * oc * 2);

    if src_bits == 32 {
        let samples = std::slice::from_raw_parts(data as *const f32, frames * sc);
        for f in 0..frames {
            for c in 0..oc {
                let s = samples[f * sc + c.min(sc - 1)];
                let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    } else {
        // Assume already 16-bit PCM.
        let samples = std::slice::from_raw_parts(data as *const i16, frames * sc);
        for f in 0..frames {
            for c in 0..oc {
                out.extend_from_slice(&samples[f * sc + c.min(sc - 1)].to_le_bytes());
            }
        }
    }
    out
}

/// Wrap 16-bit PCM bytes in an `IMFSample`.
unsafe fn make_pcm_sample(pcm: &[u8], pts_100ns: i64, dur_100ns: i64) -> Result<IMFSample> {
    let buffer = MFCreateMemoryBuffer(pcm.len() as u32).context("MFCreateMemoryBuffer")?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    buffer.Lock(&mut ptr, None, None).context("buffer Lock")?;
    std::ptr::copy_nonoverlapping(pcm.as_ptr(), ptr, pcm.len());
    buffer.Unlock().ok();
    buffer.SetCurrentLength(pcm.len() as u32).ok();

    let sample = MFCreateSample().context("MFCreateSample")?;
    sample.AddBuffer(&buffer)?;
    sample.SetSampleTime(pts_100ns)?;
    sample.SetSampleDuration(dur_100ns)?;
    Ok(sample)
}

/// Drain all available AAC output frames into the ring.
///
/// The AAC encoder is a **synchronous** MFT that does NOT allocate its own
/// output samples (unlike the async NVENC video MFT). We must hand it an output
/// sample + buffer sized per `GetOutputStreamInfo`; passing a null sample is
/// what caused the 0xC0000005 access violation on the first mic buffer.
unsafe fn drain_aac(encoder: &IMFTransform, ring: &Arc<Mutex<RingBuffer>>, count: &mut u64) {
    let out_size = match encoder.GetOutputStreamInfo(0) {
        Ok(info) => info.cbSize.max(1),
        Err(_) => return,
    };
    loop {
        // Fresh caller-allocated output sample for the MFT to write into.
        let (buffer, sample) = match (MFCreateMemoryBuffer(out_size), MFCreateSample()) {
            (Ok(b), Ok(s)) => (b, s),
            _ => break,
        };
        if sample.AddBuffer(&buffer).is_err() {
            break;
        }

        let mut out = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(Some(sample)),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        let mut status = 0u32;
        let hr = encoder.ProcessOutput(0, &mut out, &mut status);
        // Reclaim the sample we passed in (the MFT filled its buffer in place).
        let produced = ManuallyDrop::take(&mut out[0].pSample);
        let _ = ManuallyDrop::take(&mut out[0].pEvents);
        if hr.is_err() {
            break; // MF_E_TRANSFORM_NEED_MORE_INPUT etc.
        }
        let Some(sample) = produced else { break };
        if let Ok(frame) = read_encoded(&sample) {
            if let Ok(mut r) = ring.lock() {
                r.push(frame);
            }
            *count += 1;
        }
    }
}

unsafe fn read_encoded(sample: &IMFSample) -> Result<EncodedFrame> {
    let buffer = sample.ConvertToContiguousBuffer()?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut cur: u32 = 0;
    buffer.Lock(&mut ptr, None, Some(&mut cur))?;
    let data = std::slice::from_raw_parts(ptr, cur as usize).to_vec();
    buffer.Unlock().ok();
    let pts = sample.GetSampleTime().unwrap_or(0);
    let dur = sample.GetSampleDuration().unwrap_or(0);
    Ok(EncodedFrame {
        data,
        pts_100ns: pts,
        dur_100ns: dur,
        keyframe: true, // every AAC frame is independently decodable
    })
}

/// Create + configure an MF AAC audio encoder for 16-bit PCM input.
unsafe fn create_aac_encoder(rate: u32, channels: u16) -> Result<IMFTransform> {
    let in_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_PCM,
    };
    let out_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Audio,
        guidSubtype: MFAudioFormat_AAC,
    };

    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count: u32 = 0;
    MFTEnumEx(
        MFT_CATEGORY_AUDIO_ENCODER,
        MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
        Some(&in_info as *const MFT_REGISTER_TYPE_INFO),
        Some(&out_info as *const MFT_REGISTER_TYPE_INFO),
        &mut activates,
        &mut count,
    )
    .context("MFTEnumEx AAC")?;
    if count == 0 || activates.is_null() {
        anyhow::bail!("no AAC encoder MFT available");
    }
    let list = std::slice::from_raw_parts_mut(activates, count as usize);
    let first = list[0].take();
    for item in list.iter_mut() {
        let _ = item.take();
    }
    CoTaskMemFree(Some(activates as *const _));
    let activate = first.context("null AAC activate")?;
    let transform: IMFTransform = activate.ActivateObject().context("ActivateObject AAC")?;

    let block_align = channels as u32 * 2;
    let avg_bytes = rate * block_align;

    // Output (AAC): 128 kbps-ish.
    let out: IMFMediaType = MFCreateMediaType().context("MFCreateMediaType AAC out")?;
    out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
    out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
    out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
    out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16000)?;
    transform.SetOutputType(0, &out, 0)?;

    // Input (PCM).
    let inp: IMFMediaType = MFCreateMediaType().context("MFCreateMediaType PCM in")?;
    inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
    inp.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
    inp.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    inp.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
    inp.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels as u32)?;
    inp.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, block_align)?;
    inp.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, avg_bytes)?;
    transform.SetInputType(0, &inp, 0)?;

    Ok(transform)
}
