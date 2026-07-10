//! Windows native icon extraction — SHGetFileInfoW → PNG data URL + accent color.

use std::path::Path;

use base64::Engine as _;
use image::{ImageBuffer, Rgba};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHGFI_ICON, SHGFI_LARGEICON, SHFILEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

use crate::platform::icon_accent::{accent_color_from_rgba, icon_export_px};
use crate::platform::IconResolveResult;

pub fn resolve_app_icon(
    app_id: &str,
    icon_size_dip: f64,
    scale_factor: f64,
) -> IconResolveResult {
    let path = Path::new(app_id);
    if !path.is_file() {
        return IconResolveResult::default();
    }

    let export_px = icon_export_px(icon_size_dip, scale_factor);
    match icon_to_png_and_accent(path, export_px) {
        Ok(result) => result,
        Err(err) => {
            log::warn!("resolve_app_icon failed for {app_id}: {err}");
            IconResolveResult::default()
        }
    }
}

fn icon_to_png_and_accent(path: &Path, export_px: u32) -> Result<IconResolveResult, String> {
    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect();

    let mut shfi = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            Default::default(),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
    }

    let hicon = shfi.hIcon;
    if hicon.is_invalid() {
        return Err("SHGetFileInfoW returned no icon".to_string());
    }

    let result = rasterize_icon(hicon, export_px);
    unsafe {
        let _ = DestroyIcon(hicon);
    }
    result
}

fn rasterize_icon(hicon: windows::Win32::UI::WindowsAndMessaging::HICON, size: u32) -> Result<IconResolveResult, String> {
    let mut icon_info = ICONINFO::default();
    unsafe {
        GetIconInfo(hicon, &mut icon_info).map_err(|e| e.to_string())?;
    }

    let hdc_screen = unsafe { GetDC(None) };
    if hdc_screen.is_invalid() {
        return Err("GetDC failed".to_string());
    }

    let result = (|| {
        let hdc_mem = unsafe { CreateCompatibleDC(hdc_screen) };
        if hdc_mem.is_invalid() {
            return Err("CreateCompatibleDC failed".to_string());
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbmp = unsafe {
            windows::Win32::Graphics::Gdi::CreateDIBSection(
                hdc_mem,
                &bmi,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )
        }
        .map_err(|e| e.to_string())?;

        let old = unsafe { SelectObject(hdc_mem, HGDIOBJ(hbmp.0)) };
        unsafe {
            windows::Win32::Graphics::Gdi::DrawIconEx(
                hdc_mem,
                0,
                0,
                hicon,
                size as i32,
                size as i32,
                0,
                None,
                windows::Win32::UI::WindowsAndMessaging::DI_NORMAL,
            )
            .map_err(|e| e.to_string())?;
        }

        let byte_len = (size * size * 4) as usize;
        let mut rgba = vec![0u8; byte_len];
        let lines = unsafe {
            GetDIBits(
                hdc_mem,
                hbmp,
                0,
                size,
                Some(rgba.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        unsafe {
            SelectObject(hdc_mem, old);
            let _ = DeleteObject(HGDIOBJ(hbmp.0));
            let _ = DeleteDC(hdc_mem);
        }

        if lines == 0 {
            return Err("GetDIBits failed".to_string());
        }

        // BGRA → RGBA
        for chunk in rgba.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }

        let accent = accent_color_from_rgba(size, size, &rgba);
        let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_raw(size, size, rgba).ok_or("invalid image buffer")?;
        let mut png_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;

        Ok(IconResolveResult {
            icon_url: Some(format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(png_bytes)
            )),
            accent_color: accent,
        })
    })();

    unsafe {
        let _ = ReleaseDC(None, hdc_screen);
        if !icon_info.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(icon_info.hbmColor.0));
        }
        if !icon_info.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(icon_info.hbmMask.0));
        }
    }

    result
}
