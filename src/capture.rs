//! Screen capture via Windows.Graphics.Capture (WGC).
//!
//! Layer 2a: capture the **primary monitor** as GPU BGRA textures.
//! Layer 2b: convert each captured frame to NV12 on the GPU (encoder input).
//! Encoding (L2c) attaches next; window capture + fps throttle (L2e) after.
//!
//! Frames never leave the GPU: WGC texture -> NV12 texture, no CPU copy.
//! `UNVERIFIED` until it builds/runs on Windows.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::{IInspectable, Interface, Ref};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::convert::Nv12Converter;

/// A running WGC capture of the primary monitor. Holds every COM object alive
/// for the lifetime of the capture; dropping it stops the session.
pub struct MonitorCapture {
    session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
    _device: ID3D11Device,
    _context: ID3D11DeviceContext,
    frames: Arc<AtomicU64>,
}

impl MonitorCapture {
    /// Start capturing the primary monitor. Each frame is converted to NV12;
    /// arrivals are logged periodically.
    pub fn start() -> Result<Self> {
        let (device, context) = create_d3d11_device()?;
        let winrt_device = to_winrt_device(&device)?;
        let item = primary_monitor_item()?;
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

        // Converter + reusable NV12 target texture, moved into the callback.
        let converter = Nv12Converter::new(&device, &context, width, height)?;
        let nv12 = converter.create_nv12_texture()?;

        let frames = Arc::new(AtomicU64::new(0));
        let frames_cb = frames.clone();

        // FrameArrived fires on a background MTA thread. L2c will hand the NV12
        // texture to the encoder here instead of just logging.
        let handler = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
            move |pool: Ref<Direct3D11CaptureFramePool>, _args: Ref<IInspectable>| {
                if let Some(pool) = pool.as_ref() {
                    if let Ok(frame) = pool.TryGetNextFrame() {
                        let n = frames_cb.fetch_add(1, Ordering::Relaxed) + 1;
                        match frame_texture(&frame) {
                            Ok(bgra) => match converter.convert(&bgra, &nv12) {
                                Ok(()) => {
                                    if n % 120 == 1 {
                                        log::info!(
                                            "capture+convert ok: {n} frames -> NV12 {width}x{height}"
                                        );
                                    }
                                }
                                Err(e) => {
                                    if n % 120 == 1 {
                                        log::warn!("nv12 convert failed at frame {n}: {e:#}");
                                    }
                                }
                            },
                            Err(e) => {
                                if n % 120 == 1 {
                                    log::warn!("frame texture unavailable at {n}: {e:#}");
                                }
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
    Ok((
        device.context("D3D11CreateDevice returned no device")?,
        context.context("D3D11CreateDevice returned no context")?,
    ))
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
