//! Windows native icon extraction — Shell Image Factory → PNG data URL + accent.
//!
//! Primary path requests the icon at `export_px` via `IShellItemImageFactory`
//! (jumbo / multi-res assets). `SHGFI_LARGEICON` (~32px) + `DrawIconEx` upscale
//! is only a fallback — that path looks soft/jagged on HiDPI.

use std::path::Path;

use base64::Engine as _;
use image::{ImageBuffer, Rgba};
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHGetFileInfoW, SHFILEINFOW,
    SHGFI_ICON, SHGFI_LARGEICON, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetIconInfo, DI_NORMAL, ICONINFO,
};

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

fn ensure_com() {
    // Idempotent — S_FALSE / RPC_E_CHANGED_MODE are fine if already initialized.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

fn shell_path_wide(path: &Path) -> Vec<u16> {
    let lossy = path.to_string_lossy();
    let normalized = lossy
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .unwrap_or_else(|| {
            lossy
                .strip_prefix(r"\\?\")
                .map(|rest| rest.to_string())
                .unwrap_or_else(|| lossy.into_owned())
        });
    normalized.encode_utf16().chain([0]).collect()
}

fn icon_to_png_and_accent(path: &Path, export_px: u32) -> Result<IconResolveResult, String> {
    ensure_com();
    match shell_item_image(path, export_px) {
        Ok(result) => Ok(result),
        Err(err) => {
            log::warn!(
                "IShellItemImageFactory failed ({err}), falling back to SHGFI_LARGEICON"
            );
            shgfi_large_icon(path, export_px)
        }
    }
}

fn shell_item_image(path: &Path, export_px: u32) -> Result<IconResolveResult, String> {
    let path_wide = shell_path_wide(path);
    let factory: IShellItemImageFactory = unsafe {
        SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)
            .map_err(|e| format!("SHCreateItemFromParsingName: {e}"))?
    };

    let size = SIZE {
        cx: export_px as i32,
        cy: export_px as i32,
    };
    let flags = SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK | SIIGBF_RESIZETOFIT;
    let hbmp = unsafe {
        factory
            .GetImage(size, flags)
            .map_err(|e| format!("GetImage: {e}"))?
    };

    let result = hbitmap_to_icon_result(hbmp);
    unsafe {
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
    }
    result
}

fn shgfi_large_icon(path: &Path, export_px: u32) -> Result<IconResolveResult, String> {
    let path_wide = shell_path_wide(path);

    let mut shfi = SHFILEINFOW::default();
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if result == 0 {
        return Err("SHGetFileInfoW returned no icon".to_string());
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

fn hbitmap_to_icon_result(hbmp: HBITMAP) -> Result<IconResolveResult, String> {
    let mut bm = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(hbmp.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        )
    };
    if got == 0 {
        return Err("GetObjectW failed for HBITMAP".to_string());
    }

    let width = bm.bmWidth.unsigned_abs();
    let height = bm.bmHeight.unsigned_abs();
    if width == 0 || height == 0 {
        return Err("HBITMAP has empty dimensions".to_string());
    }

    let hdc_screen = unsafe { GetDC(None) };
    if hdc_screen.is_invalid() {
        return Err("GetDC failed".to_string());
    }

    let result = (|| {
        let hdc_mem = unsafe { CreateCompatibleDC(Some(hdc_screen)) };
        if hdc_mem.is_invalid() {
            return Err("CreateCompatibleDC failed".to_string());
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let byte_len = (width * height * 4) as usize;
        let mut rgba = vec![0u8; byte_len];
        let lines = unsafe {
            GetDIBits(
                hdc_mem,
                hbmp,
                0,
                height,
                Some(rgba.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        unsafe {
            let _ = DeleteDC(hdc_mem);
        }
        if lines == 0 {
            return Err("GetDIBits failed".to_string());
        }

        // BGRA → RGBA, then un-premultiply. IShellItemImageFactory returns
        // PARGB; the DrawIconEx fallback must NOT use this path (straight alpha).
        for chunk in rgba.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }
        unpremultiply_rgba(&mut rgba);

        rgba_to_icon_result(width, height, rgba)
    })();

    unsafe {
        let _ = ReleaseDC(None, hdc_screen);
    }
    result
}

fn unpremultiply_rgba(rgba: &mut [u8]) {
    for chunk in rgba.chunks_exact_mut(4) {
        let a = chunk[3] as u16;
        if a == 0 {
            chunk[0] = 0;
            chunk[1] = 0;
            chunk[2] = 0;
        } else if a < 255 {
            chunk[0] = ((chunk[0] as u16 * 255) / a).min(255) as u8;
            chunk[1] = ((chunk[1] as u16 * 255) / a).min(255) as u8;
            chunk[2] = ((chunk[2] as u16 * 255) / a).min(255) as u8;
        }
    }
}

fn rgba_to_icon_result(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> Result<IconResolveResult, String> {
    let accent = accent_color_from_rgba(width, height, &rgba);
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, rgba).ok_or("invalid image buffer")?;
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
}

fn rasterize_icon(
    hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
    size: u32,
) -> Result<IconResolveResult, String> {
    let mut icon_info = ICONINFO::default();
    unsafe {
        GetIconInfo(hicon, &mut icon_info).map_err(|e| e.to_string())?;
    }

    let hdc_screen = unsafe { GetDC(None) };
    if hdc_screen.is_invalid() {
        return Err("GetDC failed".to_string());
    }

    let result = (|| {
        let hdc_mem = unsafe { CreateCompatibleDC(Some(hdc_screen)) };
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
                Some(hdc_mem),
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
            DrawIconEx(
                hdc_mem,
                0,
                0,
                hicon,
                size as i32,
                size as i32,
                0,
                None,
                DI_NORMAL,
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

        // BGRA → RGBA only. Do NOT un-premultiply: DrawIconEx into a fresh
        // DIB is typically straight (or mixed) alpha — blind unpremultiply
        // brightens fringes into white halos.
        for chunk in rgba.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }

        rgba_to_icon_result(size, size, rgba)
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
