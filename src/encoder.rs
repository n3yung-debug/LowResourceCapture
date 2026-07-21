//! Hardware video encoding (NVENC HEVC / H.264) via a Media Foundation
//! hardware encoder MFT.
//!
//! Layer 2c-1: enumerate and configure the hardware encoder — prefer HEVC,
//! fall back to H.264 if no hardware HEVC MFT is present — and report which
//! one we got. On NVIDIA hardware this MFT is NVENC. The async pump that
//! actually feeds frames and drains encoded samples into the ring buffer is
//! L2c-2.
//!
//! `UNVERIFIED` until it builds/runs on Windows with an NVENC GPU. Whether a
//! hardware HEVC MFT is exposed on this GPU/driver is the open question this
//! step answers empirically (logged at startup).

use anyhow::{Context, Result};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFDXGIDeviceManager, IMFMediaEventGenerator, IMFMediaType, IMFTransform,
    MFCreateDXGIDeviceManager, MFCreateMediaType, MFStartup, MFTEnumEx, MFMediaType_Video,
    MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_ASYNCMFT, MFT_ENUM_FLAG_HARDWARE,
    MFT_ENUM_FLAG_SORTANDFILTER, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_REGISTER_TYPE_INFO,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC_UNLOCK,
};
use windows::Win32::System::Com::CoTaskMemFree;

use crate::config::{Codec, EncoderConfig};

// MF_VERSION = (MF_SDK_VERSION << 16) | MF_API_VERSION = (0x2 << 16) | 0x70.
const MF_VERSION: u32 = (0x0002 << 16) | 0x0070;
const MFSTARTUP_FULL: u32 = 0;

/// MF stores size/ratio attributes as a packed u64 (high 32 bits = width or
/// numerator, low 32 = height or denominator). Replaces the `MFSetAttributeSize`
/// / `MFSetAttributeRatio` inline helpers that windows-rs doesn't expose.
fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | (lo as u64)
}

/// A configured hardware video encoder. The async pump (L2c-2) will drive this
/// transform to produce encoded frames.
pub struct VideoEncoder {
    // Held for the async pump (L2c-2b). Kept alive; not yet driven.
    _transform: IMFTransform,
    _device_manager: IMFDXGIDeviceManager,
    _event_gen: IMFMediaEventGenerator,
    /// The codec we actually got (may differ from requested if HEVC hardware
    /// encoding isn't available and we fell back to H.264).
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
}

impl VideoEncoder {
    /// Enumerate + configure a hardware encoder for the given size/settings.
    /// Tries HEVC first (unless H.264 was requested), falling back to H.264.
    pub fn new(device: &ID3D11Device, width: u32, height: u32, cfg: &EncoderConfig) -> Result<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }.context("MFStartup")?;

        // Try requested codec first, then the other as fallback.
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
                Ok(None) => {
                    log::info!("no hardware {} encoder found", codec_name(codec));
                }
                Err(e) => log::warn!("enumerating {} encoder: {e:#}", codec_name(codec)),
            }
        }

        let (transform, codec) =
            chosen.context("no hardware HEVC or H.264 encoder MFT available")?;

        // Unlock the async hardware MFT before use.
        unsafe {
            let attrs = transform.GetAttributes().context("GetAttributes")?;
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
        }

        // Bind a DXGI device manager (shared D3D11 device) so the encoder takes
        // GPU textures as input — zero-copy from capture.
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

        // Begin streaming so the MFT starts requesting input.
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
            _transform: transform,
            _device_manager: device_manager,
            _event_gen: event_gen,
            codec,
            width,
            height,
        })
    }
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

    // Take ownership of the first activate; release the rest; free the array.
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

    // Output type (compressed) must be set before the input type.
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

    // Input type: NV12.
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
