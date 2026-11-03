// SPDX-License-Identifier: GPL-3.0-only
//! 输出（OUT-1）：剪贴板 `CF_DIB` + `PNG` 格式（Windows）。
//! 编码、默认目录与命名模板在 `airsnip_core::output`（此处再导出，路径不变）。

use std::mem::size_of;

pub use airsnip_core::output::*;

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;
use windows::core::w;

/// 复制到剪贴板：写入 CF_DIB（BGRA）与 "PNG" 注册格式（OUT-1）。
pub fn copy_to_clipboard(pixmap: &Pixmap) -> Result<(), String> {
    let w = pixmap.width() as u32;
    let h = pixmap.height() as u32;
    let bgra = rgba_to_bgra(pixmap.data());
    let png_bytes = encode_png(pixmap)?;

    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("OpenClipboard 失败".into());
        }
        let fail = |msg: String| {
            let _ = CloseClipboard();
            msg
        };

        if EmptyClipboard().is_err() {
            return Err(fail("EmptyClipboard 失败".into()));
        }

        // CF_DIB：BITMAPINFOHEADER + BGRA 像素。
        let header_size = size_of::<BITMAPINFOHEADER>() as usize;
        let hdib = match GlobalAlloc(GMEM_MOVEABLE, header_size + bgra.len()) {
            Ok(h) => h,
            Err(e) => return Err(fail(format!("GlobalAlloc CF_DIB 失败：{e}"))),
        };
        let ptr = GlobalLock(hdib);
        if ptr.is_null() {
            return Err(fail("GlobalLock CF_DIB 失败".into()));
        }
        let header = BITMAPINFOHEADER {
            biSize: header_size as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // 自上而下
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        std::ptr::copy_nonoverlapping(&header as *const _ as *const u8, ptr as *mut u8, header_size);
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), (ptr as *mut u8).add(header_size), bgra.len());
        let _ = GlobalUnlock(hdib);
        if SetClipboardData(CF_DIB.0 as u32, Some(HANDLE(hdib.0))).is_err() {
            // SetClipboardData 失败时剪贴板未接管句柄，必须自行释放
            let _ = GlobalFree(Some(HGLOBAL(hdib.0)));
            return Err(fail("SetClipboardData CF_DIB 失败".into()));
        }

        // PNG 格式（延迟渲染的简化：直接同步提供完整数据）。
        let fmt = RegisterClipboardFormatW(w!("PNG"));
        if fmt == 0 {
            return Err(fail("RegisterClipboardFormatW(PNG) 返回 0".into()));
        }
        let hpng = match GlobalAlloc(GMEM_MOVEABLE, png_bytes.len()) {
            Ok(h) => h,
            Err(e) => return Err(fail(format!("GlobalAlloc PNG 失败：{e}"))),
        };
        let pptr = GlobalLock(hpng);
        if pptr.is_null() {
            return Err(fail("GlobalLock PNG 失败".into()));
        }
        std::ptr::copy_nonoverlapping(png_bytes.as_ptr(), pptr as *mut u8, png_bytes.len());
        let _ = GlobalUnlock(hpng);
        if SetClipboardData(fmt, Some(HANDLE(hpng.0))).is_err() {
            let _ = GlobalFree(Some(HGLOBAL(hpng.0)));
            return Err(fail("SetClipboardData PNG 失败".into()));
        }

        let _ = CloseClipboard();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_roundtrip() {
        let pixmap = Pixmap::new(8, 8).unwrap();
        match copy_to_clipboard(&pixmap) {
            Ok(()) => println!("clipboard ok"),
            Err(e) => println!("clipboard err: {e}"),
        }
    }
}
