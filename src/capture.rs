//! Screen capture via Windows.Graphics.Capture (WGC).
//!
//! Layer 2a: create a D3D11 device and capture the **primary monitor**,
//! logging that frames are arriving. Frames stay on the GPU as
//! `ID3D11Texture2D` (BGRA) — no CPU-side pixel copy. Encoding (L2c) and
//! window capture + fps throttling (L2e) come next; for now this proves the
//! capture path works end to end.
//!
//! `UNVERIFIED` until it builds/runs on Windows.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::{IInspectable, Interface, Ref};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// A running WGC capture of the primary monitor. Holds every COM object alive
/// for the lifetime of the capture; dropping it stops the session.
pub struct MonitorCapture {
    session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
    // Keep the D3D11 device alive as long as capture runs.
    _device: ID3D11Device,
    frames: Arc<AtomicU64>,
}

impl MonitorCapture {
    /// Start capturing the primary monitor. Frame arrivals are logged.
    pub fn start() -> Result<Self> {
        let device = create_d3d11_device()?;
        let winrt_device = to_winrt_device(&device)?;
        let item = primary_monitor_item()?;
        let size = item.Size().context("capture item size")?;

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

        let frames = Arc::new(AtomicU64::new(0));
        let frames_cb = frames.clone();

        // FrameArrived fires on a background MTA thread. For L2a we just count
        // and periodically log; L2c will hand the texture to the encoder here.
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _args: Ref<IInspectable>| {
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        let ts = frame
                            .SystemRelativeTime()
                            .map(|t| t.Duration)
                            .unwrap_or(0);
                        let n = frames_cb.fetch_add(1, Ordering::Relaxed) + 1;
                        // ~ every 2 seconds at 60fps.
                        if n % 120 == 1 {
                            log::info!("capture: {n} frames (last ts {} ms)", ts / 10_000);
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
        log::info!(
            "monitor capture started: {}x{}",
            size.Width,
            size.Height
        );

        Ok(Self {
            session,
            frame_pool,
            _item: item,
            _device: device,
            frames,
        })
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

/// Create a hardware D3D11 device with BGRA support (required by WGC).
fn create_d3d11_device() -> Result<ID3D11Device> {
    let mut device: Option<ID3D11Device> = None;
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
            None,
        )
        .context("D3D11CreateDevice")?;
    }
    device.context("D3D11CreateDevice returned no device")
}

/// Wrap the D3D11 device as a WinRT `IDirect3DDevice` for the frame pool.
fn to_winrt_device(device: &ID3D11Device) -> Result<IDirect3DDevice> {
    let dxgi: IDXGIDevice = device.cast().context("ID3D11Device -> IDXGIDevice")?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .context("CreateDirect3D11DeviceFromDXGIDevice")?;
    inspectable
        .cast()
        .context("IInspectable -> IDirect3DDevice")
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

// D3D_FEATURE_LEVEL is referenced only to keep the import meaningful if we pin
// feature levels later; silence unused warnings for now.
#[allow(dead_code)]
const _FEATURE_LEVEL_MARKER: Option<D3D_FEATURE_LEVEL> = None;
