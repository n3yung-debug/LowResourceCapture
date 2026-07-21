//! Screen capture via Windows.Graphics.Capture (WGC).
//!
//! Layer 2a: capture the **primary monitor** as GPU BGRA textures.
//! Layer 2b: convert each captured frame to NV12 on the GPU (encoder input).
//! Encoding (L2c) attaches next; window capture + fps throttle (L2e) after.
//!
//! Frames never leave the GPU: WGC texture -> NV12 texture, no CPU copy.
//! `UNVERIFIED` until it builds/runs on Windows.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::core::{IInspectable, Interface, Ref};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

use crate::config::EncoderConfig;
use crate::convert::Nv12Converter;
use crate::encoder::{EncoderPump, FrameMsg, VideoEncoder};
use crate::ringbuffer::RingBuffer;

/// A running WGC capture of the primary monitor. Holds every COM object alive
/// for the lifetime of the capture; dropping it stops the session.
pub struct MonitorCapture {
    session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
    _device: ID3D11Device,
    _context: ID3D11DeviceContext,
    // Dropping this stops the encoder pump thread.
    _pump: EncoderPump,
    frames: Arc<AtomicU64>,
}

/// What to capture. L5's game-detect chooses the game window; the debug menu
/// uses the primary monitor.
pub enum CaptureTarget {
    PrimaryMonitor,
    ForegroundWindow,
}

impl MonitorCapture {
    /// Start capturing `target`: convert each frame to NV12 and feed it to the
    /// hardware encoder, whose pump pushes encoded frames into `ring`. Frames
    /// are throttled to the configured fps.
    pub fn start(
        target: CaptureTarget,
        encoder_cfg: EncoderConfig,
        ring: Arc<Mutex<RingBuffer>>,
    ) -> Result<Self> {
        let (device, context) = create_d3d11_device()?;
        let winrt_device = to_winrt_device(&device)?;
        let item = match target {
            CaptureTarget::PrimaryMonitor => primary_monitor_item()?,
            CaptureTarget::ForegroundWindow => foreground_window_item()?,
        };
        let size = item.Size().context("capture item size")?;
        let width = size.Width.max(0) as u32;
        let height = size.Height.max(0) as u32;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .context("CreateFreeThreaded frame pool")?;

        let session = frame_pool
            .CreateCaptureSession(&item)
            .context("CreateCaptureSession")?;

        // GPU BGRA->NV12 converter (moved into the callback).
        let converter = Nv12Converter::new(&device, &context, width, height)?;

        // Hardware encoder + its pump thread; the callback submits NV12 samples.
        let encoder = VideoEncoder::new(&device, width, height, &encoder_cfg)?;
        let pump = encoder.start_pump(ring);
        let tx = pump.sender();

        let fps = encoder_cfg.fps.max(1);
        let dur_100ns: i64 = 10_000_000 / fps as i64;
        // Minimum spacing between kept frames (throttle to target fps). WGC can
        // deliver up to the monitor refresh (e.g. 360 Hz); we drop the excess.
        let min_interval: i64 = dur_100ns;

        let frames = Arc::new(AtomicU64::new(0));
        let frames_cb = frames.clone();
        let last_ts = Arc::new(AtomicI64::new(i64::MIN));
        let last_ts_cb = last_ts.clone();

        // FrameArrived fires on a background MTA thread: throttle to fps, then
        // convert to a fresh NV12 texture and submit it to the encoder pump.
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _args: Ref<IInspectable>| {
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        if FIRST_ARRIVAL.swap(false, Ordering::Relaxed) {
                            log::info!("FrameArrived: first callback fired");
                        }
                        let ts = frame.SystemRelativeTime().map(|t| t.Duration).unwrap_or(0);
                        let last = last_ts_cb.load(Ordering::Relaxed);
                        // saturating_sub, NOT wrapping_sub: the sentinel start
                        // value (i64::MIN) made wrapping_sub underflow negative
                        // on every frame, so the throttle dropped 100% of frames.
                        // saturating_sub(ts, i64::MIN) saturates high, so the
                        // first frame always passes; later deltas are normal.
                        if ts.saturating_sub(last) >= min_interval {
                            last_ts_cb.store(ts, Ordering::Relaxed);
                            let n = frames_cb.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Err(e) = process_frame(&frame, &converter, &tx, ts, dur_100ns) {
                                if n % 120 == 1 {
                                    log::warn!("frame {n} pipeline error: {e:#}");
                                }
                            } else if n % 120 == 1 {
                                log::info!("capture->encode: {n} frames ({width}x{height})");
                            }
                        }
                        let _ = frame.Close();
                    }
                }
                Ok(())
            },
        );
        frame_pool
            .FrameArrived(&handler)
            .context("register FrameArrived")?;

        session.StartCapture().context("StartCapture")?;
        log::info!("monitor capture started: {width}x{height}");

        Ok(Self {
            session,
            frame_pool,
            _item: item,
            _device: device,
            _context: context,
            _pump: pump,
            frames,
        })
    }
}

impl MonitorCapture {
    /// The video encoder's compressed output media type (for the muxer).
    pub fn video_output_type(&self) -> windows::Win32::Media::MediaFoundation::IMFMediaType {
        self._pump.output_type()
    }
}

impl Drop for MonitorCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
        log::info!(
            "monitor capture stopped after {} frames",
            self.frames.load(Ordering::Relaxed)
        );
    }
}

/// Logs the first FrameArrived callback once, to confirm WGC is delivering
/// frames at all (vs. the throttle silently dropping them).
static FIRST_ARRIVAL: AtomicBool = AtomicBool::new(true);

/// Logs each step of the *first* processed frame exactly once, so if the
/// pipeline crashes on frame 1 the log shows the last step that succeeded.
static FIRST_FRAME: AtomicBool = AtomicBool::new(true);

/// Convert one captured frame to NV12 and submit it to the encoder pump.
fn process_frame(
    frame: &Direct3D11CaptureFrame,
    converter: &Nv12Converter,
    tx: &std::sync::mpsc::Sender<FrameMsg>,
    ts: i64,
    dur_100ns: i64,
) -> Result<()> {
    let first = FIRST_FRAME.swap(false, Ordering::Relaxed);
    macro_rules! step {
        ($($a:tt)*) => { if first { log::info!($($a)*); } };
    }

    step!("frame1: begin");
    let bgra = frame_texture(frame)?;
    step!("frame1: got BGRA capture texture");
    // Fresh NV12 texture per frame so in-flight encoder samples don't alias.
    let nv12 = converter.create_nv12_texture()?;
    step!("frame1: NV12 texture allocated");
    converter.convert(&bgra, &nv12)?;
    step!("frame1: converted BGRA->NV12");
    // The pump thread builds the IMFSample; we only send the (Send) texture.
    let _ = tx.send((nv12, ts, dur_100ns));
    step!("frame1: NV12 handed to encoder pump");
    Ok(())
}

/// Extract the `ID3D11Texture2D` backing a captured frame.
fn frame_texture(frame: &Direct3D11CaptureFrame) -> Result<ID3D11Texture2D> {
    let surface = frame.Surface().context("frame surface")?;
    let access: IDirect3DDxgiInterfaceAccess =
        surface.cast().context("surface -> IDirect3DDxgiInterfaceAccess")?;
    let texture: ID3D11Texture2D =
        unsafe { access.GetInterface() }.context("GetInterface ID3D11Texture2D")?;
    Ok(texture)
}

/// Create a hardware D3D11 device (+ immediate context) with BGRA support.
fn create_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("D3D11CreateDevice")?;
    }
    let device = device.context("D3D11CreateDevice returned no device")?;
    let context = context.context("D3D11CreateDevice returned no context")?;

    // REQUIRED for sharing this device with Media Foundation via
    // IMFDXGIDeviceManager: the WGC frame-arrived thread runs VideoProcessorBlt
    // on the immediate context while the NVENC pump thread locks the same device
    // through the device manager. Without multithread protection the contexts
    // race and the process dies with a D3D11 access violation the moment frames
    // start flowing. See ID3D11Multithread::SetMultithreadProtected (MS docs).
    if let Ok(mt) = context.cast::<ID3D11Multithread>() {
        unsafe {
            mt.SetMultithreadProtected(true.into());
        }
    } else {
        log::warn!("ID3D11Multithread unavailable — D3D11 device not thread-protected");
    }

    Ok((device, context))
}

/// Wrap the D3D11 device as a WinRT `IDirect3DDevice` for the frame pool.
fn to_winrt_device(device: &ID3D11Device) -> Result<IDirect3DDevice> {
    let dxgi: IDXGIDevice = device.cast().context("ID3D11Device -> IDXGIDevice")?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .context("CreateDirect3D11DeviceFromDXGIDevice")?;
    inspectable.cast().context("IInspectable -> IDirect3DDevice")
}

/// A `GraphicsCaptureItem` for the primary monitor.
fn primary_monitor_item() -> Result<GraphicsCaptureItem> {
    let hmon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .context("GraphicsCaptureItem interop factory")?;
    let item = unsafe { interop.CreateForMonitor(hmon) }.context("CreateForMonitor")?;
    Ok(item)
}

/// A `GraphicsCaptureItem` for the current foreground window (L5 game-detect
/// will pass a specific game window; this default grabs whatever's focused).
fn foreground_window_item() -> Result<GraphicsCaptureItem> {
    let hwnd: HWND = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        anyhow::bail!("no foreground window to capture");
    }
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .context("GraphicsCaptureItem interop factory")?;
    let item = unsafe { interop.CreateForWindow(hwnd) }.context("CreateForWindow")?;
    Ok(item)
}
