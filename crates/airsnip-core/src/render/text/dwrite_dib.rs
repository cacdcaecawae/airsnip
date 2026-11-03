// SPDX-License-Identifier: GPL-3.0-only
//! 文本位图缓冲（EDT-4）：DirectWrite 栅格化用的内存 DIB，以及把黑字白底像素读回为覆盖度。

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
    GetCurrentObject, GetDIBits, HBITMAP, HDC, OBJ_BITMAP, SelectObject,
};

/// 创建位图渲染目标使用的（w×h 32bpp top-down）内存 DIB 的 DC 与位图。
pub(super) fn create_dib(w: u32, h: u32) -> Option<(HDC, HBITMAP)> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return None;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [Default::default()],
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) => {
                let _ = SelectObject(dc, bmp.into());
                Some((dc, bmp))
            }
            Err(_) => {
                let _ = DeleteDC(dc);
                None
            }
        }
    }
}

/// 从位图渲染目标的内存 DC 读回覆盖度（w×h，每像素 1 字节）：黑字白底取 B 通道反色。
pub(super) fn read_coverage(dc: HDC, w: u32, h: u32) -> Option<Vec<u8>> {
    let px = read_glyphs(dc, w, h)?;
    Some(
        px.chunks_exact(4)
            .take(w as usize * h as usize)
            .map(|p| 255 - p[0])
            .collect(),
    )
}

/// 读回 32bpp 像素（BGRX，黑色字形）。
fn read_glyphs(dc: HDC, w: u32, h: u32) -> Option<Vec<u8>> {
    unsafe {
        let bmp = GetCurrentObject(dc, OBJ_BITMAP);
        if bmp.0.is_null() {
            return None;
        }
        let hbmp = HBITMAP(bmp.0);
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [Default::default()],
        };
        let mut out = vec![0u8; (w as usize * h as usize * 4).max(4)];
        let n = GetDIBits(
            dc,
            hbmp,
            0,
            h,
            Some(out.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        );
        if n == 0 {
            return None;
        }
        Some(out)
    }
}
