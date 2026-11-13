//! DXGI Desktop Duplication fallback, cropped to the D2R window.

use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
};

use super::window;
use crate::capture::{CaptureError, CapturedTexture};

pub struct DxgiCapture {
    device: ID3D11Device,
    duplication: IDXGIOutputDuplication,
    hwnd: HWND,
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
    monitor_rect: RECT,
    crop: Option<CropTexture>,
}

struct CropTexture {
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    texture: ID3D11Texture2D,
}

impl DxgiCapture {
    pub fn start(device: &ID3D11Device, hwnd: HWND) -> Result<Self, CaptureError> {
        let (monitor, monitor_rect) = window::monitor_for_window(hwnd)
            .ok_or_else(|| CaptureError::Recoverable("could not find monitor".into()))?;

        let dxgi_device: IDXGIDevice = device
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI device: {error}")))?;
        let adapter: IDXGIAdapter = unsafe { dxgi_device.GetAdapter() }
            .map_err(|error| CaptureError::Recoverable(format!("DXGI adapter: {error}")))?;
        let output = find_output(&adapter, monitor)?;
        let output1: IDXGIOutput1 = output
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI output1: {error}")))?;
        let unknown: windows::core::IUnknown = dxgi_device
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI unknown: {error}")))?;
        let duplication = unsafe { output1.DuplicateOutput(&unknown) }
            .map_err(|error| CaptureError::Recoverable(format!("DXGI duplication: {error}")))?;

        Ok(Self {
            device: device.clone(),
            duplication,
            hwnd,
            monitor,
            monitor_rect,
            crop: None,
        })
    }

    /// Desktop duplication is bound to one monitor. WGC follows the window, but
    /// DXGI must restart when the game moves to a different display.
    pub fn matches_monitor(&self, hwnd: HWND) -> bool {
        window::monitor_for_window(hwnd)
            .map(|(monitor, _)| monitor == self.monitor)
            .unwrap_or(false)
    }

    pub fn next_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<CapturedTexture>, CaptureError> {
        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        match unsafe {
            self.duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        } {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                return Err(CaptureError::Recoverable(
                    "DXGI duplication access lost".into(),
                ));
            }
            Err(error) => {
                return Err(CaptureError::Recoverable(format!(
                    "DXGI acquire frame: {error}"
                )));
            }
        }

        let result = self.copy_frame(resource, frame_info.LastPresentTime);
        let _ = unsafe { self.duplication.ReleaseFrame() };
        result
    }

    fn copy_frame(
        &mut self,
        resource: Option<IDXGIResource>,
        timestamp: i64,
    ) -> Result<Option<CapturedTexture>, CaptureError> {
        let Some(resource) = resource else {
            return Ok(None);
        };
        let source: ID3D11Texture2D = resource
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI texture: {error}")))?;
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut source_desc) };

        let Some(rect) = window::window_rect(self.hwnd) else {
            return Err(CaptureError::Recoverable("D2R window disappeared".into()));
        };

        let monitor_width = (self.monitor_rect.right - self.monitor_rect.left).max(1) as u32;
        let monitor_height = (self.monitor_rect.bottom - self.monitor_rect.top).max(1) as u32;
        let left = (rect.left - self.monitor_rect.left).clamp(0, monitor_width as i32) as u32;
        let top = (rect.top - self.monitor_rect.top).clamp(0, monitor_height as i32) as u32;
        let right = (rect.right - self.monitor_rect.left).clamp(0, monitor_width as i32) as u32;
        let bottom = (rect.bottom - self.monitor_rect.top).clamp(0, monitor_height as i32) as u32;

        let width = right.saturating_sub(left);
        let height = bottom.saturating_sub(top);
        if width == 0 || height == 0 {
            return Err(CaptureError::Recoverable(
                "D2R window is outside its monitor".into(),
            ));
        }

        let needs_crop = self
            .crop
            .as_ref()
            .map(|crop| {
                crop.width != width || crop.height != height || crop.format != source_desc.Format
            })
            .unwrap_or(true);
        if needs_crop {
            let texture = create_crop_texture(&self.device, width, height, source_desc.Format)
                .map_err(|error| {
                    CaptureError::Recoverable(format!("DXGI crop texture: {error}"))
                })?;
            self.crop = Some(CropTexture {
                width,
                height,
                format: source_desc.Format,
                texture,
            });
        }

        let crop = self.crop.as_ref().expect("crop created");
        let source_resource: windows::Win32::Graphics::Direct3D11::ID3D11Resource =
            source
                .cast()
                .map_err(|error| CaptureError::Recoverable(format!("DXGI source: {error}")))?;
        let crop_resource: windows::Win32::Graphics::Direct3D11::ID3D11Resource = crop
            .texture
            .cast()
            .map_err(|error| CaptureError::Recoverable(format!("DXGI crop: {error}")))?;

        let source_box = windows::Win32::Graphics::Direct3D11::D3D11_BOX {
            left,
            top,
            front: 0,
            right,
            bottom,
            back: 1,
        };

        let context = unsafe { self.device.GetImmediateContext() }
            .map_err(|error| CaptureError::Recoverable(format!("DXGI context: {error}")))?;
        unsafe {
            context.CopySubresourceRegion(
                &crop_resource,
                0,
                0,
                0,
                0,
                &source_resource,
                0,
                Some(&source_box),
            );
        }

        Ok(Some(CapturedTexture {
            texture: crop.texture.clone(),
            timestamp,
            size: (width, height),
            _keepalive: None,
        }))
    }
}

fn find_output(
    adapter: &IDXGIAdapter,
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
) -> Result<windows::Win32::Graphics::Dxgi::IDXGIOutput, CaptureError> {
    for index in 0.. {
        match unsafe { adapter.EnumOutputs(index) } {
            Ok(output) => {
                let desc: DXGI_OUTPUT_DESC = unsafe { output.GetDesc() }.map_err(|error| {
                    CaptureError::Recoverable(format!("DXGI output desc: {error}"))
                })?;
                if desc.Monitor == monitor {
                    return Ok(output);
                }
            }
            Err(_) => break,
        }
    }

    Err(CaptureError::Recoverable(
        "no DXGI output matches the D2R monitor".into(),
    ))
}

fn create_crop_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };

    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
    texture.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))
}
