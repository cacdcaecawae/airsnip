// SPDX-License-Identifier: GPL-3.0-only
//! GDI 上屏缓冲（CAP-3）：DIB section 创建与 RGBA → BGRA 局部转换。
//! 覆盖层、长截图窗口、设置面板与弹出菜单共用。

use std::mem::size_of;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, HBITMAP, HDC,
    SelectObject,
};

/// 创建上屏用 DIB section（BGRA、top-down），返回内存 DC、位图句柄与像素指针。
pub(crate) fn create_dib_surface(w: u32, h: u32) -> Result<(HDC, HBITMAP, *mut u8), String> {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        if hdc.0.is_null() {
            return Err("CreateCompatibleDC 失败".into());
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
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
        let hbmp = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(e) => {
                let _ = DeleteDC(hdc);
                return Err(format!("CreateDIBSection 失败：{e}"));
            }
        };
        let _ = SelectObject(hdc, hbmp.into());
        Ok((hdc, hbmp, bits as *mut u8))
    }
}

/// RGBA → BGRA 写入目标缓冲区：转换 (x0,y0,rw,rh) 区域，按行多线程分块。
/// 目标地址用 usize 传递（裸指针不满足 Send）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_rgba_to_bgra(
    src: &[u8],
    dst_addr: usize,
    w: i32,
    x0: i32,
    y0: i32,
    rw: i32,
    rh: i32,
    workers: usize,
) {
    let stride = w as usize * 4;
    let total_rows = rh as usize;
    let convert_row = move |row: usize| {
        let off = (y0 as usize + row) * stride + x0 as usize * 4;
        unsafe {
            let sp = src.as_ptr().add(off) as *const u32;
            let dp = (dst_addr as *mut u8).add(off) as *mut u32;
            for i in 0..rw as usize {
                let v = *sp.add(i);
                *dp.add(i) = (v & 0xFF00_FF00) | ((v & 0x00FF_0000) >> 16) | ((v & 0x0000_00FF) << 16);
            }
        }
    };

    if workers <= 1 {
        for row in 0..total_rows {
            convert_row(row);
        }
        return;
    }
    let row_chunk = total_rows.div_ceil(workers);
    std::thread::scope(|s| {
        for start in (0..total_rows).step_by(row_chunk.max(1)) {
            let rows = (total_rows - start).min(row_chunk);
            s.spawn(move || {
                for row in start..start + rows {
                    convert_row(row);
                }
            });
        }
    });
}
