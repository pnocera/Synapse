use std::{cell::RefCell, ffi::c_void, slice};

use synapse_core::Rect;
use windows::{
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Storage::Streams::DataWriter,
    Win32::Graphics::{
        Direct3D11::{
            D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
            D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, ID3D11Resource, ID3D11Texture2D,
        },
        Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
            SRCCOPY, SelectObject,
        },
    },
    core::Interface as _,
};

use crate::{CaptureError, CapturedFrame, CapturedSoftwareBitmap, DxgiFormat};

use super::common::capture_unsupported;

thread_local! {
    static SCREEN_CAPTURE_SCRATCH: RefCell<Option<GdiCaptureScratch>> = const { RefCell::new(None) };
}

pub fn captured_frame_region_to_software_bitmap(
    frame: &CapturedFrame,
    region: Rect,
) -> Result<CapturedSoftwareBitmap, CaptureError> {
    let region = clamp_region_to_frame(frame, region)?;
    let bytes = copy_region_bgra(frame, region)?;
    let bitmap = software_bitmap_from_bgra(&bytes, region.w, region.h)?;
    Ok(CapturedSoftwareBitmap { region, bitmap })
}

pub fn screen_region_to_software_bitmap(
    region: Rect,
) -> Result<CapturedSoftwareBitmap, CaptureError> {
    validate_bitmap_region(region)?;
    let bytes = copy_screen_region_bgra(region)?;
    let bitmap = software_bitmap_from_bgra(&bytes, region.w, region.h)?;
    Ok(CapturedSoftwareBitmap { region, bitmap })
}

fn software_bitmap_from_bgra(
    bytes: &[u8],
    width: i32,
    height: i32,
) -> Result<SoftwareBitmap, CaptureError> {
    let writer = DataWriter::new().map_err(capture_unsupported)?;
    writer.WriteBytes(bytes).map_err(capture_unsupported)?;
    let buffer = writer.DetachBuffer().map_err(capture_unsupported)?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width,
        height,
        BitmapAlphaMode::Premultiplied,
    )
    .map_err(capture_unsupported)
}
fn copy_region_bgra(frame: &CapturedFrame, region: Rect) -> Result<Vec<u8>, CaptureError> {
    let convert_rgba_to_bgra = match frame.format {
        DxgiFormat::Bgra8 | DxgiFormat::Bgra8Srgb => false,
        DxgiFormat::Rgba8 | DxgiFormat::Rgba8Srgb => true,
        other => {
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: format!("OCR bitmap copy does not support frame format {other:?}"),
            });
        }
    };

    let width = u32::try_from(region.w).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let height = u32::try_from(region.h).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let texture = frame.texture.get();
    let staging = create_staging_texture(texture, width, height)?;
    let context = unsafe { texture.GetDevice() }
        .and_then(|device| unsafe { device.GetImmediateContext() })
        .map_err(capture_unsupported)?;
    let source: ID3D11Resource = texture.cast().map_err(capture_unsupported)?;
    let target: ID3D11Resource = staging.cast().map_err(capture_unsupported)?;
    let source_box = D3D11_BOX {
        left: u32::try_from(region.x).unwrap_or(0),
        top: u32::try_from(region.y).unwrap_or(0),
        front: 0,
        right: u32::try_from(region.x.saturating_add(region.w)).unwrap_or(width),
        bottom: u32::try_from(region.y.saturating_add(region.h)).unwrap_or(height),
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(&target, 0, 0, 0, 0, &source, 0, Some(&raw const source_box));
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&target, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }
        .map_err(capture_unsupported)?;
    let bytes = copy_mapped_rows(&mapped, width, height, convert_rgba_to_bgra);
    unsafe {
        context.Unmap(&target, 0);
    }
    bytes
}

fn create_staging_texture(
    texture: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, CaptureError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        texture.GetDesc(&raw mut desc);
    }
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
    desc.MiscFlags = 0;
    desc.SampleDesc.Count = 1;
    desc.SampleDesc.Quality = 0;

    let device = unsafe { texture.GetDevice() }.map_err(capture_unsupported)?;
    let mut staging = None;
    unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut staging)) }
        .map_err(capture_unsupported)?;
    staging.ok_or_else(|| CaptureError::GraphicsApiUnsupported {
        detail: "CreateTexture2D returned no staging texture".to_owned(),
    })
}

fn copy_mapped_rows(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: u32,
    height: u32,
    convert_rgba_to_bgra: bool,
) -> Result<Vec<u8>, CaptureError> {
    let row_len = usize::try_from(width)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid OCR bitmap width {width}"),
        })?;
    let height = usize::try_from(height).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let row_pitch =
        usize::try_from(mapped.RowPitch).map_err(|err| CaptureError::GraphicsApiUnsupported {
            detail: err.to_string(),
        })?;
    let mut output = vec![0_u8; row_len.saturating_mul(height)];
    for row in 0..height {
        let source = unsafe {
            slice::from_raw_parts((mapped.pData as *const u8).add(row * row_pitch), row_len)
        };
        let start = row.saturating_mul(row_len);
        output[start..start + row_len].copy_from_slice(source);
    }
    if convert_rgba_to_bgra {
        for pixel in output.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    Ok(output)
}

fn copy_screen_region_bgra(region: Rect) -> Result<Vec<u8>, CaptureError> {
    let width = u32::try_from(region.w).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let height = u32::try_from(region.h).map_err(|err| CaptureError::TargetInvalid {
        detail: err.to_string(),
    })?;
    let byte_len = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CaptureError::TargetInvalid {
            detail: format!("invalid screen capture region {region:?}"),
        })?;
    let screen_dc = unsafe { GetDC(None) };
    if screen_dc.is_invalid() {
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: "GetDC returned null".to_owned(),
        });
    }
    let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
    if memory_dc.is_invalid() {
        let _ = unsafe { ReleaseDC(None, screen_dc) };
        return Err(CaptureError::GraphicsApiUnsupported {
            detail: "CreateCompatibleDC returned null".to_owned(),
        });
    }
    let result = SCREEN_CAPTURE_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let needs_recreate = scratch
            .as_ref()
            .is_none_or(|scratch| !scratch.matches(width, height, byte_len));
        if needs_recreate {
            *scratch = Some(GdiCaptureScratch::new(
                screen_dc, memory_dc, width, height, byte_len,
            )?);
        } else {
            let _ = unsafe { DeleteDC(memory_dc) };
        }
        let scratch = scratch
            .as_ref()
            .ok_or_else(|| CaptureError::GraphicsApiUnsupported {
                detail: "screen capture scratch buffer was not initialized".to_owned(),
            })?;
        let bitblt = unsafe {
            BitBlt(
                scratch.memory_dc,
                0,
                0,
                region.w,
                region.h,
                Some(screen_dc),
                region.x,
                region.y,
                SRCCOPY,
            )
        };
        bitblt.map_err(capture_unsupported)?;
        Ok(unsafe { slice::from_raw_parts(scratch.bits.cast::<u8>(), byte_len) }.to_vec())
    });
    let _ = unsafe { ReleaseDC(None, screen_dc) };
    result
}

struct GdiCaptureScratch {
    width: u32,
    height: u32,
    byte_len: usize,
    memory_dc: HDC,
    bitmap: HBITMAP,
    old_object: HGDIOBJ,
    bits: *mut c_void,
}

impl GdiCaptureScratch {
    fn new(
        screen_dc: HDC,
        memory_dc: HDC,
        width: u32,
        height: u32,
        byte_len: usize,
    ) -> Result<Self, CaptureError> {
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(u32::MAX),
                biWidth: i32::try_from(width).unwrap_or(i32::MAX),
                biHeight: -i32::try_from(height).unwrap_or(i32::MAX),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: u32::try_from(byte_len).unwrap_or(u32::MAX),
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(
                Some(screen_dc),
                &raw const bitmap_info,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            )
        }
        .map_err(capture_unsupported)?;
        if bits.is_null() {
            let _ = unsafe { DeleteObject(HGDIOBJ::from(bitmap)) };
            let _ = unsafe { DeleteDC(memory_dc) };
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: "CreateDIBSection returned no bitmap bits".to_owned(),
            });
        }
        let old_object = unsafe { SelectObject(memory_dc, HGDIOBJ::from(bitmap)) };
        if old_object.is_invalid() {
            let _ = unsafe { DeleteObject(HGDIOBJ::from(bitmap)) };
            let _ = unsafe { DeleteDC(memory_dc) };
            return Err(CaptureError::GraphicsApiUnsupported {
                detail: "SelectObject failed for screen capture bitmap".to_owned(),
            });
        }
        Ok(Self {
            width,
            height,
            byte_len,
            memory_dc,
            bitmap,
            old_object,
            bits,
        })
    }

    const fn matches(&self, width: u32, height: u32, byte_len: usize) -> bool {
        self.width == width && self.height == height && self.byte_len == byte_len
    }
}

impl Drop for GdiCaptureScratch {
    fn drop(&mut self) {
        let _ = unsafe { SelectObject(self.memory_dc, self.old_object) };
        let _ = unsafe { DeleteObject(HGDIOBJ::from(self.bitmap)) };
        let _ = unsafe { DeleteDC(self.memory_dc) };
    }
}

fn validate_bitmap_region(region: Rect) -> Result<(), CaptureError> {
    if region.w <= 0 || region.h <= 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("empty bitmap capture region {region:?}"),
        });
    }
    Ok(())
}

fn clamp_region_to_frame(frame: &CapturedFrame, region: Rect) -> Result<Rect, CaptureError> {
    if region.w <= 0 || region.h <= 0 {
        return Err(CaptureError::TargetInvalid {
            detail: format!("empty OCR capture region {region:?}"),
        });
    }
    let frame_w = i64::from(frame.width);
    let frame_h = i64::from(frame.height);
    let left = i64::from(region.x).clamp(0, frame_w);
    let top = i64::from(region.y).clamp(0, frame_h);
    let right = i64::from(region.x)
        .saturating_add(i64::from(region.w))
        .clamp(0, frame_w);
    let bottom = i64::from(region.y)
        .saturating_add(i64::from(region.h))
        .clamp(0, frame_h);
    if right <= left || bottom <= top {
        return Err(CaptureError::TargetInvalid {
            detail: format!("OCR capture region {region:?} is outside frame bounds"),
        });
    }
    Ok(Rect {
        x: i32::try_from(left).unwrap_or(i32::MAX),
        y: i32::try_from(top).unwrap_or(i32::MAX),
        w: i32::try_from(right - left).unwrap_or(i32::MAX),
        h: i32::try_from(bottom - top).unwrap_or(i32::MAX),
    })
}
