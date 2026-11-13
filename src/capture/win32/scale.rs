//! GPU downscale (mip chain + small CPU box filter) and readback.
//!
//! WGC hands us a BGRA8 texture at the window's native size. Generating a mip
//! chain on the GPU gives a box-filtered image down to (typically) 240x135;
//! only that small image is read back, then a CPU box filter produces the
//! detector's 160-pixel-short-side frame. This avoids both a naive bilinear
//! downscale and a full-resolution CPU readback.

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11ShaderResourceView, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_RESOURCE_MISC_GENERATE_MIPS,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_B8G8R8X8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};

pub const DETECTOR_SHORT_SIDE: u32 = 160;

pub struct ScaledFrame<'a> {
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub all_black: bool,
}

pub struct Downscaler {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    mip: Option<MipChain>,
    staging: Option<StagingTexture>,
    output: Vec<u8>,
}

struct MipChain {
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    levels: u32,
}

struct StagingTexture {
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    texture: ID3D11Texture2D,
}

impl Downscaler {
    pub fn new(device: &ID3D11Device) -> windows::core::Result<Self> {
        let context = unsafe { device.GetImmediateContext() }?;
        Ok(Self {
            device: device.clone(),
            context,
            mip: None,
            staging: None,
            output: Vec::new(),
        })
    }

    pub fn downscale<'a>(
        &'a mut self,
        source: &ID3D11Texture2D,
    ) -> Result<ScaledFrame<'a>, String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut desc) };
        if desc.Width == 0 || desc.Height == 0 {
            return Err("captured texture has zero size".into());
        }

        let (target_width, target_height) = target_dimensions(desc.Width, desc.Height);
        let (mip_level, mip_width, mip_height) =
            self.prepare_mip_chain(source, &desc, target_width, target_height)?;

        self.copy_to_staging(mip_level, mip_width, mip_height, desc.Format)?;
        self.readback_and_box_filter(
            mip_width,
            mip_height,
            target_width,
            target_height,
            desc.Format,
        )
    }

    fn prepare_mip_chain(
        &mut self,
        source: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
        target_width: u32,
        target_height: u32,
    ) -> Result<(u32, u32, u32), String> {
        let levels = mip_level_count(desc.Width, desc.Height);
        let needs_recreate = self
            .mip
            .as_ref()
            .map(|mip| {
                mip.width != desc.Width || mip.height != desc.Height || mip.format != desc.Format
            })
            .unwrap_or(true);

        if needs_recreate {
            let texture = create_texture(
                &self.device,
                desc.Width,
                desc.Height,
                desc.Format,
                levels,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
                0,
                D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
            )
            .map_err(|error| format!("create mip texture: {error}"))?;
            let resource: ID3D11Resource = texture
                .cast()
                .map_err(|error| format!("cast mip resource: {error}"))?;
            let mut view = None;
            unsafe {
                self.device
                    .CreateShaderResourceView(&resource, None, Some(&mut view))
            }
            .map_err(|error| format!("create mip SRV: {error}"))?;
            let view = view.ok_or("mip SRV was not created")?;

            self.mip = Some(MipChain {
                width: desc.Width,
                height: desc.Height,
                format: desc.Format,
                texture,
                view,
                levels,
            });
            self.staging = None;
        }

        let mip = self.mip.as_ref().expect("mip chain created");
        let source_resource: ID3D11Resource = source
            .cast()
            .map_err(|error| format!("cast source resource: {error}"))?;
        let dest_resource: ID3D11Resource = mip
            .texture
            .cast()
            .map_err(|error| format!("cast mip destination: {error}"))?;

        unsafe {
            self.context.CopySubresourceRegion(
                &dest_resource,
                0,
                0,
                0,
                0,
                &source_resource,
                0,
                None,
            );
            self.context.GenerateMips(&mip.view);
        }

        let mut level = 0;
        loop {
            let width = (mip.width >> level).max(1);
            let height = (mip.height >> level).max(1);
            let next_width = (mip.width >> (level + 1)).max(1);
            let next_height = (mip.height >> (level + 1)).max(1);

            if next_width < target_width || next_height < target_height || level + 1 >= mip.levels {
                return Ok((level, width, height));
            }
            level += 1;
        }
    }

    fn copy_to_staging(
        &mut self,
        mip_level: u32,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
    ) -> Result<(), String> {
        let needs_recreate = self
            .staging
            .as_ref()
            .map(|staging| {
                staging.width != width || staging.height != height || staging.format != format
            })
            .unwrap_or(true);

        if needs_recreate {
            let texture = create_texture(
                &self.device,
                width,
                height,
                format,
                1,
                D3D11_USAGE_STAGING,
                windows::Win32::Graphics::Direct3D11::D3D11_BIND_FLAG(0).0 as u32,
                D3D11_CPU_ACCESS_READ.0 as u32,
                0,
            )
            .map_err(|error| format!("create staging texture: {error}"))?;
            self.staging = Some(StagingTexture {
                width,
                height,
                format,
                texture,
            });
        }

        let staging = self.staging.as_ref().expect("staging created");
        let mip = self.mip.as_ref().ok_or("mip chain missing")?;
        let source_resource: ID3D11Resource = mip
            .texture
            .cast()
            .map_err(|error| format!("cast mip resource: {error}"))?;
        let staging_resource: ID3D11Resource = staging
            .texture
            .cast()
            .map_err(|error| format!("cast staging resource: {error}"))?;

        unsafe {
            self.context.CopySubresourceRegion(
                &staging_resource,
                0,
                0,
                0,
                0,
                &source_resource,
                mip_level,
                None,
            );
        }
        Ok(())
    }

    fn readback_and_box_filter(
        &mut self,
        source_width: u32,
        source_height: u32,
        target_width: u32,
        target_height: u32,
        format: DXGI_FORMAT,
    ) -> Result<ScaledFrame<'_>, String> {
        let staging = self.staging.as_ref().ok_or("staging missing")?;
        let resource: ID3D11Resource = staging
            .texture
            .cast()
            .map_err(|error| format!("cast staging resource: {error}"))?;

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(|error| format!("map staging texture: {error}"))?;

        if mapped.pData.is_null() {
            unsafe {
                self.context.Unmap(&resource, 0);
            }
            return Err("staging texture mapped to null".to_string());
        }

        let context = self.context.clone();
        let result = box_filter(
            mapped.pData as *const u8,
            mapped.RowPitch as usize,
            source_width as usize,
            source_height as usize,
            target_width as usize,
            target_height as usize,
            format,
            &mut self.output,
        );

        unsafe {
            context.Unmap(&resource, 0);
        }

        result
    }
}

fn create_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    mip_levels: u32,
    usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE,
    bind_flags: u32,
    cpu_flags: u32,
    misc_flags: u32,
) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mip_levels,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind_flags,
        CPUAccessFlags: cpu_flags,
        MiscFlags: misc_flags,
    };

    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
    texture.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER))
}

fn mip_level_count(width: u32, height: u32) -> u32 {
    let mut size = width.max(height);
    let mut levels = 1;
    while size > 1 {
        size >>= 1;
        levels += 1;
    }
    levels
}

fn target_dimensions(width: u32, height: u32) -> (u32, u32) {
    let short = DETECTOR_SHORT_SIDE as f64;
    let (mut target_width, mut target_height) = if width >= height {
        let height = (short * height as f64 / width as f64).round() as u32;
        (DETECTOR_SHORT_SIDE, height)
    } else {
        let width = (short * width as f64 / height as f64).round() as u32;
        (width, DETECTOR_SHORT_SIDE)
    };

    target_width = (target_width.max(2) / 2) * 2;
    target_height = (target_height.max(2) / 2) * 2;
    (target_width, target_height)
}

#[allow(clippy::too_many_arguments)]
fn box_filter<'a>(
    source: *const u8,
    row_pitch: usize,
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
    format: DXGI_FORMAT,
    output: &'a mut Vec<u8>,
) -> Result<ScaledFrame<'a>, String> {
    let kind = PixelKind::from_format(format)
        .ok_or_else(|| format!("unsupported capture format {format:?}"))?;
    let bytes_per_pixel = kind.bytes_per_pixel();

    output.clear();
    output.resize(target_width * target_height * 4, 0);
    let mut max_channel = 0u8;

    for target_y in 0..target_height {
        let source_y0 = target_y * source_height / target_height;
        let source_y1 = (((target_y + 1) * source_height) / target_height)
            .max(source_y0 + 1)
            .min(source_height);

        for target_x in 0..target_width {
            let source_x0 = target_x * source_width / target_width;
            let source_x1 = (((target_x + 1) * source_width) / target_width)
                .max(source_x0 + 1)
                .min(source_width);

            let (red, green, blue) = kind.average_region(
                source, row_pitch, source_x0, source_y0, source_x1, source_y1,
            );

            let output_index = (target_y * target_width + target_x) * 4;
            output[output_index] = red;
            output[output_index + 1] = green;
            output[output_index + 2] = blue;
            output[output_index + 3] = 255;
            max_channel = max_channel.max(red).max(green).max(blue);
        }
    }

    let _ = bytes_per_pixel;
    Ok(ScaledFrame {
        data: output.as_slice(),
        width: target_width,
        height: target_height,
        all_black: max_channel <= 20,
    })
}

#[derive(Clone, Copy)]
enum PixelKind {
    Bgra8,
    Rgba8,
    Bgrx8,
    Rgba16Float,
    Rgb10a2,
}

impl PixelKind {
    fn from_format(format: DXGI_FORMAT) -> Option<Self> {
        match format {
            DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Some(Self::Bgra8),
            DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => Some(Self::Rgba8),
            DXGI_FORMAT_B8G8R8X8_UNORM => Some(Self::Bgrx8),
            DXGI_FORMAT_R16G16B16A16_FLOAT => Some(Self::Rgba16Float),
            DXGI_FORMAT_R10G10B10A2_UNORM => Some(Self::Rgb10a2),
            _ => None,
        }
    }

    fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra8 | Self::Rgba8 | Self::Bgrx8 | Self::Rgb10a2 => 4,
            Self::Rgba16Float => 8,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn average_region(
        self,
        source: *const u8,
        row_pitch: usize,
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
    ) -> (u8, u8, u8) {
        let count = ((x1 - x0) * (y1 - y0)).max(1) as f32;

        match self {
            Self::Bgra8 | Self::Bgrx8 => {
                let mut red = 0u32;
                let mut green = 0u32;
                let mut blue = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let index = y * row_pitch + x * 4;
                        let pixel = unsafe { std::slice::from_raw_parts(source.add(index), 4) };
                        blue += u32::from(pixel[0]);
                        green += u32::from(pixel[1]);
                        red += u32::from(pixel[2]);
                    }
                }
                (
                    (red as f32 / count) as u8,
                    (green as f32 / count) as u8,
                    (blue as f32 / count) as u8,
                )
            }
            Self::Rgba8 => {
                let mut red = 0u32;
                let mut green = 0u32;
                let mut blue = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let index = y * row_pitch + x * 4;
                        let pixel = unsafe { std::slice::from_raw_parts(source.add(index), 4) };
                        red += u32::from(pixel[0]);
                        green += u32::from(pixel[1]);
                        blue += u32::from(pixel[2]);
                    }
                }
                (
                    (red as f32 / count) as u8,
                    (green as f32 / count) as u8,
                    (blue as f32 / count) as u8,
                )
            }
            Self::Rgba16Float => {
                let mut red = 0f32;
                let mut green = 0f32;
                let mut blue = 0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let index = y * row_pitch + x * 8;
                        let pixel = unsafe { std::slice::from_raw_parts(source.add(index), 8) };
                        red += linear_to_srgb(f16_to_f32(u16::from_le_bytes([pixel[0], pixel[1]])));
                        green +=
                            linear_to_srgb(f16_to_f32(u16::from_le_bytes([pixel[2], pixel[3]])));
                        blue +=
                            linear_to_srgb(f16_to_f32(u16::from_le_bytes([pixel[4], pixel[5]])));
                    }
                }
                (
                    (red / count * 255.0).clamp(0.0, 255.0) as u8,
                    (green / count * 255.0).clamp(0.0, 255.0) as u8,
                    (blue / count * 255.0).clamp(0.0, 255.0) as u8,
                )
            }
            Self::Rgb10a2 => {
                let mut red = 0u32;
                let mut green = 0u32;
                let mut blue = 0u32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let index = y * row_pitch + x * 4;
                        let pixel = unsafe { std::slice::from_raw_parts(source.add(index), 4) };
                        let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                        red += (value & 0x3FF) >> 2;
                        green += ((value >> 10) & 0x3FF) >> 2;
                        blue += ((value >> 20) & 0x3FF) >> 2;
                    }
                }
                (
                    (red as f32 / count) as u8,
                    (green as f32 / count) as u8,
                    (blue as f32 / count) as u8,
                )
            }
        }
    }
}

/// Convert a scaled-linear (scRGB) channel to sRGB, tone-mapping highlights.
fn linear_to_srgb(value: f32) -> f32 {
    let value = if !value.is_finite() || value <= 0.0 {
        0.0
    } else {
        value
    };
    let value = if value > 1.0 {
        value / (1.0 + value)
    } else {
        value
    };
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x03FF);

    let value = if exponent == 0 {
        if mantissa == 0 {
            0
        } else {
            let mut mantissa = mantissa;
            let mut exponent = 113u32;
            while mantissa & 0x400 == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            sign | (exponent << 23) | ((mantissa & 0x3FF) << 13)
        }
    } else if exponent == 0x1F {
        sign | 0x7F80_0000 | (mantissa << 13)
    } else {
        sign | ((exponent + 112) << 23) | (mantissa << 13)
    };

    f32::from_bits(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::c_void;
    use std::path::PathBuf;

    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
        D3D11_SUBRESOURCE_DATA,
    };

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test")
            .join(name)
    }

    fn load_bgra(name: &str) -> (Vec<u8>, u32, u32) {
        let file = std::fs::File::open(fixture_path(name)).expect("open fixture");
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder.read_info().expect("read png info");
        let mut buffer = vec![0; reader.output_buffer_size().expect("PNG output buffer size")];
        let info = reader.next_frame(&mut buffer).expect("decode png");

        let pixels = info.width as usize * info.height as usize;
        let mut bgra = Vec::with_capacity(pixels * 4);
        match info.color_type {
            png::ColorType::Rgba => {
                for pixel in buffer[..info.buffer_size()].chunks_exact(4) {
                    bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            png::ColorType::Rgb => {
                for pixel in buffer[..info.buffer_size()].chunks_exact(3) {
                    bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
            }
            other => panic!("unsupported PNG color type {other:?}"),
        }
        (bgra, info.width, info.height)
    }

    fn create_device_for_test() -> Option<windows::Win32::Graphics::Direct3D11::ID3D11Device> {
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let mut device = None;
            let mut context = None;
            let result = unsafe {
                D3D11CreateDevice(
                    None,
                    driver,
                    Default::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            };
            if result.is_ok() {
                if let Some(device) = device {
                    return Some(device);
                }
            }
        }
        None
    }

    fn gpu_downscale_matches_fixture(name: &str, expected: bool) {
        let Some(device) = create_device_for_test() else {
            eprintln!("skipping GPU downscale test: no D3D11 device");
            return;
        };
        let (data, width, height) = load_bgra(name);

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const c_void,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        unsafe {
            device
                .CreateTexture2D(&desc, Some(&initial), Some(&mut texture))
                .expect("create source texture");
        }
        let texture = texture.expect("source texture");

        let mut downscaler = Downscaler::new(&device).expect("downscaler");
        let frame = downscaler.downscale(&texture).expect("downscale");
        let detected = crate::detector::detect_loading(frame.data, frame.width, frame.height);
        assert_eq!(
            detected,
            expected,
            "{name} through the GPU downscale should {} loading",
            if expected { "be" } else { "not be" }
        );
    }

    #[test]
    fn gpu_downscale_preserves_door_closed_detection() {
        gpu_downscale_matches_fixture("3840x2160-door-closed.png", true);
    }

    #[test]
    fn gpu_downscale_preserves_logo_detection() {
        gpu_downscale_matches_fixture("3840x2160-logo.png", true);
    }

    #[test]
    fn gpu_downscale_preserves_gameplay_rejection() {
        gpu_downscale_matches_fixture("1280x720-dark.png", false);
    }

    #[test]
    fn detector_targets_short_side_160_with_even_dimensions() {
        assert_eq!(target_dimensions(1920, 1080), (160, 90));
        assert_eq!(target_dimensions(1920, 1200), (160, 100));
        assert_eq!(target_dimensions(2048, 1536), (160, 120));
        assert_eq!(target_dimensions(3840, 2160), (160, 90));
        assert_eq!(target_dimensions(1080, 1920), (90, 160));
    }

    #[test]
    fn converts_half_floats() {
        assert_eq!(f16_to_f32(0), 0.0);
        assert!((f16_to_f32(0x3C00) - 1.0).abs() < 1e-6);
        assert!((f16_to_f32(0x3800) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn box_filters_bgra() {
        let width = 4;
        let height = 4;
        let mut source = vec![0u8; width * height * 4];
        for (index, pixel) in source.chunks_exact_mut(4).enumerate() {
            let value = if index < 8 { 100 } else { 200 };
            pixel.copy_from_slice(&[value, value, value, 255]);
        }

        let mut output = Vec::new();
        let frame = box_filter(
            source.as_ptr(),
            width * 4,
            width,
            height,
            2,
            2,
            DXGI_FORMAT_B8G8R8A8_UNORM,
            &mut output,
        )
        .expect("filter");

        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.data.len(), 16);
        // Top half is darker than the bottom half.
        assert!(frame.data[0] < frame.data[8]);
        assert!(!frame.all_black);
    }
}
