//! Windows Graphics Capture session for the D2R window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows::core::Interface;
use windows::Foundation::TimeSpan;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::capture::{CaptureError, CapturedTexture};

type Inbox = Arc<(Mutex<Option<Direct3D11CaptureFrame>>, Condvar)>;

pub struct WgcCapture {
    session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
    inbox: Inbox,
    closed: Arc<AtomicBool>,
    width: u32,
    height: u32,
    _frame_handler: TypedEventHandler<Direct3D11CaptureFramePool, windows::core::IInspectable>,
    _item_closed_handler: TypedEventHandler<GraphicsCaptureItem, windows::core::IInspectable>,
}

impl WgcCapture {
    pub fn start(hwnd: HWND, device: &ID3D11Device, fps: u32) -> Result<Self, CaptureError> {
        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .map_err(|error| CaptureError::Recoverable(format!("WGC factory: {error}")))?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd) }
            .map_err(|error| CaptureError::Recoverable(format!("WGC item: {error}")))?;

        let size = item
            .Size()
            .map_err(|error| CaptureError::Recoverable(format!("WGC item size: {error}")))?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(CaptureError::Recoverable("WGC item has zero size".into()));
        }

        let dxgi_device: IDXGIDevice = device
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI device: {error}")))?;
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
            .map_err(|error| CaptureError::Recoverable(format!("WGC device: {error}")))?;
        let d3d_device: IDirect3DDevice = inspectable
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("WGC D3D device: {error}")))?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|error| CaptureError::Recoverable(format!("WGC frame pool: {error}")))?;

        let inbox: Inbox = Arc::new((Mutex::new(None), Condvar::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let frame_handler = {
            let inbox = inbox.clone();
            let closed = closed.clone();
            TypedEventHandler::new(
                move |sender: windows::core::Ref<'_, Direct3D11CaptureFramePool>,
                      _args: windows::core::Ref<'_, windows::core::IInspectable>| {
                    if closed.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    if let Some(pool) = sender.as_ref() {
                        if let Ok(frame) = pool.TryGetNextFrame() {
                            let (lock, condvar) = &*inbox;
                            let mut slot =
                                lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                            *slot = Some(frame);
                            condvar.notify_one();
                        }
                    }
                    Ok(())
                },
            )
        };

        let item_closed_handler = {
            let closed = closed.clone();
            let inbox = inbox.clone();
            TypedEventHandler::new(
                move |_sender: windows::core::Ref<'_, GraphicsCaptureItem>,
                      _args: windows::core::Ref<'_, windows::core::IInspectable>| {
                    closed.store(true, Ordering::SeqCst);
                    let (_, condvar) = &*inbox;
                    condvar.notify_all();
                    Ok(())
                },
            )
        };

        frame_pool
            .FrameArrived(&frame_handler)
            .map_err(|error| CaptureError::Recoverable(format!("WGC frame event: {error}")))?;
        item.Closed(&item_closed_handler)
            .map_err(|error| CaptureError::Recoverable(format!("WGC item event: {error}")))?;

        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(|error| CaptureError::Recoverable(format!("WGC session: {error}")))?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        if fps > 0 {
            let interval = TimeSpan {
                Duration: (10_000_000i64 / i64::from(fps)).max(1),
            };
            let _ = session.SetMinUpdateInterval(interval);
        }
        session
            .StartCapture()
            .map_err(|error| CaptureError::Recoverable(format!("WGC start: {error}")))?;

        Ok(Self {
            session,
            frame_pool,
            _item: item,
            inbox,
            closed,
            width: size.Width as u32,
            height: size.Height as u32,
            _frame_handler: frame_handler,
            _item_closed_handler: item_closed_handler,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn next_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedTexture>, CaptureError> {
        let (lock, condvar) = &*self.inbox;
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut guard, _) = condvar
            .wait_timeout_while(guard, timeout, |slot| {
                slot.is_none() && !self.closed.load(Ordering::SeqCst)
            })
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let frame = guard.take();
        drop(guard);

        let Some(frame) = frame else {
            if self.closed.load(Ordering::SeqCst) {
                return Err(CaptureError::Recoverable("WGC capture closed".into()));
            }
            return Ok(None);
        };

        let surface = frame
            .Surface()
            .map_err(|error| CaptureError::Recoverable(format!("WGC surface: {error}")))?;
        let access: windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess =
            surface.cast().map_err(|error| {
                CaptureError::Recoverable(format!("WGC surface access: {error}"))
            })?;
        let texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D =
            unsafe { access.GetInterface() }
                .map_err(|error| CaptureError::Recoverable(format!("WGC texture: {error}")))?;
        let content = frame
            .ContentSize()
            .map_err(|error| CaptureError::Recoverable(format!("WGC content size: {error}")))?;
        let timestamp = frame
            .SystemRelativeTime()
            .map(|time| time.Duration)
            .unwrap_or(0);

        let width = content.Width.max(0) as u32;
        let height = content.Height.max(0) as u32;
        if width == 0 || height == 0 {
            return Ok(None);
        }

        Ok(Some(CapturedTexture {
            texture,
            timestamp,
            size: (width, height),
            _keepalive: Some(frame),
        }))
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
        let (_, condvar) = &*self.inbox;
        condvar.notify_all();
    }
}
