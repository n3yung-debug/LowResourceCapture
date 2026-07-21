//! Hardware video encoding (NVENC HEVC / H.264) via a Media Foundation
//! hardware encoder MFT, driven asynchronously.
//!
//! - L2c-1: enumerate + configure the encoder (prefer HEVC, fall back to H.264).
//! - L2c-2a: bind the shared D3D11 device (zero-copy GPU input), async-unlock,
//!   begin streaming, grab the event generator.
//! - L2c-2b: a pump thread drives the async MFT — on `METransformNeedInput`
//!   it feeds a captured NV12 frame (as an `IMFSample` wrapping the GPU
//!   texture); on `METransformHaveOutput` it drains an encoded sample and
//!   pushes it into the shared ring buffer.
//!
//! Whether a hardware HEVC MFT exists on this GPU is answered empirically and
//! logged. `UNVERIFIED` at runtime until it runs on an NVENC GPU.

use anyhow::{Context, Result};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFDXGIDeviceManager, IMFMediaEventGenerator, IMFMediaType, IMFSample,
    IMFTransform, MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
    MFCreateSample, MFStartup, MFTEnumEx, MFMediaType_Video, MFSampleExtension_CleanPoint,
    MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
    MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_ASYNCMFT,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
    MFT_REGISTER_TYPE_INFO, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_TRANSFORM_ASYNC_UNLOCK,
};
use windows::Win32::System::Com::CoTaskMemFree;

use crate::config::{Codec, EncoderConfig};
use crate::ringbuffer::{EncodedFrame, RingBuffer};

// MF_VERSION = (MF_SDK_VERSION << 16) | MF_API_VERSION = (0x2 << 16) | 0x70.
const MF_VERSION: u32 = (0x0002 << 16) | 0x0070;
const MFSTARTUP_FULL: u32 = 0;

// Async-MFT event types and the GetEvent "don't block" flag (well-known values
// not always surfaced as named constants by the bindings).
const ME_TRANSFORM_NEED_INPUT: u32 = 601;
const ME_TRANSFORM_HAVE_OUTPUT: u32 = 602;
const MF_EVENT_FLAG_NO_WAIT: u32 = 0x0000_0001;

/// MF stores size/ratio attributes as a packed u64 (high 32 = width/numerator,
/// low 32 = height/denominator). Replaces the `MFSetAttributeSize/Ratio`
/// inline helpers that windows-rs doesn't expose.
fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | (lo as u64)
}

/// What the capture side sends the pump: an NV12 GPU texture (which IS `Send`
/// in windows-rs) plus its presentation time and duration. The `IMFSample` is
/// created inside the pump thread, avoiding sending non-`Send` MF interfaces.
pub type FrameMsg = (ID3D11Texture2D, i64, i64);

/// The pump thread's Media Foundation interfaces. MF interfaces are not `Send`
/// in windows-rs, but the NVENC MFT is used single-threaded within the pump,
/// so moving them to that one thread is sound.
struct PumpCom {
    transform: IMFTransform,
    event_gen: IMFMediaEventGenerator,
    _device_manager: IMFDXGIDeviceManager,
}
unsafe impl Send for PumpCom {}

/// A configured hardware encoder, ready to start pumping.
pub struct VideoEncoder {
    transform: IMFTransform,
    device_manager: IMFDXGIDeviceManager,
    event_gen: IMFMediaEventGenerator,
    /// The encoder's compressed output media type (carries codec config) —
    /// reused by the muxer to write mp4 without re-encoding.
    output_type: IMFMediaType,
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
}

impl VideoEncoder {
    /// Enumerate + configure a hardware encoder for the given size/settings.
    pub fn new(device: &ID3D11Device, width: u32, height: u32, cfg: &EncoderConfig) -> Result<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.context("MFStartup")?;

        let order = match cfg.codec {
            Codec::Hevc => [Codec::Hevc, Codec::H264],
            Codec::H264 => [Codec::H264, Codec::Hevc],
        };

        let mut chosen: Option<(IMFTransform, Codec)> = None;
        for codec in order {
            match find_encoder(codec) {
                Ok(Some(t)) => {
                    chosen = Some((t, codec));
                    break;
                }
                Ok(None) => log::info!("no hardware {} encoder found", codec_name(codec)),
                Err(e) => log::warn!("enumerating {} encoder: {e:#}", codec_name(codec)),
            }
        }
        let (transform, codec) =
            chosen.context("no hardware HEVC or H.264 encoder MFT available")?;

        unsafe {
            let attrs = transform.GetAttributes().context("GetAttributes")?;
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
        }

        let mut token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }
            .context("MFCreateDXGIDeviceManager")?;
        let device_manager = manager.context("null DXGI device manager")?;
        unsafe { device_manager.ResetDevice(device, token) }.context("ResetDevice")?;
        unsafe {
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, device_manager.as_raw() as usize)
        }
        .context("SET_D3D_MANAGER")?;

        configure(&transform, codec, width, height, cfg)
            .with_context(|| format!("configuring {} encoder", codec_name(codec)))?;

        // Capture the compressed output type for the muxer (carries codec
        // config so mp4 can be written without re-encoding).
        let output_type =
            unsafe { transform.GetOutputCurrentType(0) }.context("GetOutputCurrentType")?;

        unsafe {
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }

        let event_gen: IMFMediaEventGenerator = transform
            .cast()
            .context("IMFTransform -> IMFMediaEventGenerator")?;

        log::info!(
            "encoder ready: {} {}x{} @ {}fps, {} Mbps",
            codec_name(codec),
            width,
            height,
            cfg.fps,
            cfg.bitrate_mbps
        );

        Ok(Self {
            transform,
            device_manager,
            event_gen,
            output_type,
            codec,
            width,
            height,
        })
    }

    /// Start the pump thread. It feeds submitted NV12 samples through the MFT
    /// and pushes encoded frames into `ring`. Returns a handle used to submit
    /// frames and to stop the pump on drop.
    pub fn start_pump(self, ring: Arc<Mutex<RingBuffer>>) -> EncoderPump {
        let VideoEncoder {
            transform,
            device_manager,
            event_gen,
            output_type,
            codec,
            ..
        } = self;

        let (tx, rx) = std::sync::mpsc::channel::<FrameMsg>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let sd = shutdown.clone();

        let com = PumpCom {
            transform,
            event_gen,
            _device_manager: device_manager,
        };

        let thread = std::thread::Builder::new()
            .name("nvenc-pump".into())
            .spawn(move || {
                let com = com; // hold all MF interfaces on this thread
                pump_loop(&com.transform, &com.event_gen, &rx, &ring, &sd);
            })
            .expect("spawn nvenc pump thread");

        EncoderPump {
            input_tx: tx,
            shutdown,
            thread: Some(thread),
            output_type,
            codec,
        }
    }
}

/// Live encoder pump: submit NV12 samples, stops on drop.
pub struct EncoderPump {
    input_tx: Sender<FrameMsg>,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    output_type: IMFMediaType,
    pub codec: Codec,
}

impl EncoderPump {
    /// A cloned sender for the capture callback to submit NV12 frames.
    pub fn sender(&self) -> Sender<FrameMsg> {
        self.input_tx.clone()
    }

    /// The compressed output media type — used by the muxer.
    pub fn output_type(&self) -> IMFMediaType {
        self.output_type.clone()
    }
}

impl Drop for EncoderPump {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Wrap an NV12 GPU texture as an `IMFSample` for the encoder (zero-copy).
pub fn make_nv12_sample(tex: &ID3D11Texture2D, pts_100ns: i64, dur_100ns: i64) -> Result<IMFSample> {
    let buffer = unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, tex, 0, false) }
        .context("MFCreateDXGISurfaceBuffer")?;
    let sample = unsafe { MFCreateSample() }.context("MFCreateSample")?;
    unsafe {
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(pts_100ns)?;
        sample.SetSampleDuration(dur_100ns)?;
    }
    Ok(sample)
}

fn pump_loop(
    transform: &IMFTransform,
    event_gen: &IMFMediaEventGenerator,
    rx: &Receiver<FrameMsg>,
    ring: &Arc<Mutex<RingBuffer>>,
    shutdown: &AtomicBool,
) {
    let mut pending_input: u32 = 0;
    let mut frames_out: u64 = 0;
    let mut key_out: u64 = 0;
    let mut last_stats = Instant::now();
    // First-time markers so a crash on the first encode shows the last good step.
    let (mut lg_need, mut lg_sample, mut lg_in, mut lg_out) = (false, false, false, false);
    macro_rules! once {
        ($flag:ident, $($a:tt)*) => {
            if !$flag { log::info!($($a)*); $flag = true; }
        };
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Satisfy any outstanding input requests with captured frames.
        if pending_input > 0 {
            match rx.try_recv() {
                Ok((tex, pts, dur)) => match make_nv12_sample(&tex, pts, dur) {
                    Ok(sample) => {
                        once!(lg_sample, "pump: first NV12 sample built");
                        match unsafe { transform.ProcessInput(0, &sample, 0) } {
                            Ok(()) => {
                                once!(lg_in, "pump: first ProcessInput accepted");
                                pending_input -= 1;
                            }
                            Err(e) => once!(lg_in, "pump: first ProcessInput error: {e:?}"),
                        }
                    }
                    Err(e) => once!(lg_sample, "pump: make_nv12_sample error: {e:#}"),
                },
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => break,
            }
        }

        match unsafe {
            event_gen.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(MF_EVENT_FLAG_NO_WAIT))
        } {
            Ok(event) => {
                let met = unsafe { event.GetType() }.unwrap_or(0);
                if met == ME_TRANSFORM_NEED_INPUT {
                    once!(lg_need, "pump: first NeedInput event");
                    pending_input += 1;
                } else if met == ME_TRANSFORM_HAVE_OUTPUT {
                    if let Some(frame) = drain_output(transform) {
                        once!(lg_out, "pump: first encoded output ({} bytes)", frame.data.len());
                        let key = frame.keyframe;
                        if let Ok(mut r) = ring.lock() {
                            r.push(frame);
                        }
                        frames_out += 1;
                        if key {
                            key_out += 1;
                        }
                    }
                }
            }
            Err(_) => {
                // No event pending — brief nap to avoid a busy spin.
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        if last_stats.elapsed().as_secs() >= 1 {
            let kb = ring.lock().map(|r| r.bytes_used() / 1024).unwrap_or(0);
            log::info!("encode: {frames_out} frames ({key_out} keyframes), buffer {kb} KB");
            last_stats = Instant::now();
        }
    }

    log::info!("encoder pump stopped: {frames_out} frames encoded ({key_out} keyframes)");
}

/// Pull one encoded sample out of the MFT, if available.
fn drain_output(transform: &IMFTransform) -> Option<EncodedFrame> {
    let mut out = [MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: 0,
        pSample: ManuallyDrop::new(None),
        dwStatus: 0,
        pEvents: ManuallyDrop::new(None),
    }];
    let mut status = 0u32;
    if unsafe { transform.ProcessOutput(0, &mut out, &mut status) }.is_err() {
        return None;
    }
    let sample = unsafe { ManuallyDrop::take(&mut out[0].pSample) };
    unsafe {
        let _ = ManuallyDrop::take(&mut out[0].pEvents);
    }
    extract_encoded(&sample?).ok()
}

/// Copy the encoded bytes + timing out of an output sample.
fn extract_encoded(sample: &IMFSample) -> Result<EncodedFrame> {
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }.context("ConvertToContiguousBuffer")?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut cur: u32 = 0;
    unsafe { buffer.Lock(&mut ptr, None, Some(&mut cur)) }.context("buffer Lock")?;
    let data = unsafe { std::slice::from_raw_parts(ptr, cur as usize) }.to_vec();
    unsafe { buffer.Unlock() }.ok();

    let pts = unsafe { sample.GetSampleTime() }.unwrap_or(0);
    let dur = unsafe { sample.GetSampleDuration() }.unwrap_or(0);
    let keyframe = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }
        .map(|v| v != 0)
        .unwrap_or(true);

    Ok(EncodedFrame {
        data,
        pts_100ns: pts,
        dur_100ns: dur,
        keyframe,
    })
}

fn codec_name(c: Codec) -> &'static str {
    match c {
        Codec::Hevc => "HEVC",
        Codec::H264 => "H.264",
    }
}

fn subtype(c: Codec) -> windows::core::GUID {
    match c {
        Codec::Hevc => MFVideoFormat_HEVC,
        Codec::H264 => MFVideoFormat_H264,
    }
}

/// Enumerate hardware encoder MFTs producing `codec` and activate the first.
fn find_encoder(codec: Codec) -> Result<Option<IMFTransform>> {
    let output_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype(codec),
    };

    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count: u32 = 0;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            None,
            Some(&output_info as *const MFT_REGISTER_TYPE_INFO),
            &mut activates,
            &mut count,
        )
    }
    .context("MFTEnumEx")?;

    if count == 0 || activates.is_null() {
        return Ok(None);
    }

    let list = unsafe { std::slice::from_raw_parts_mut(activates, count as usize) };
    let first = list[0].take();
    for item in list.iter_mut() {
        let _ = item.take();
    }
    unsafe { CoTaskMemFree(Some(activates as *const _)) };

    let activate = match first {
        Some(a) => a,
        None => return Ok(None),
    };
    let transform: IMFTransform =
        unsafe { activate.ActivateObject() }.context("ActivateObject IMFTransform")?;
    Ok(Some(transform))
}

/// Set the encoder's output (compressed) and input (NV12) media types.
fn configure(
    transform: &IMFTransform,
    codec: Codec,
    width: u32,
    height: u32,
    cfg: &EncoderConfig,
) -> Result<()> {
    let bitrate = cfg.bitrate_mbps.saturating_mul(1_000_000);

    let out: IMFMediaType = unsafe { MFCreateMediaType() }.context("MFCreateMediaType out")?;
    unsafe {
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &subtype(codec))?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        out.SetUINT64(&MF_MT_FRAME_RATE, pack(cfg.fps, 1))?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        transform.SetOutputType(0, &out, 0)?;
    }

    let inp: IMFMediaType = unsafe { MFCreateMediaType() }.context("MFCreateMediaType in")?;
    unsafe {
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        inp.SetUINT64(&MF_MT_FRAME_RATE, pack(cfg.fps, 1))?;
        inp.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        transform.SetInputType(0, &inp, 0)?;
    }

    Ok(())
}
