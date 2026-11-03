// SPDX-License-Identifier: GPL-3.0-only
//! 抓屏（CAP-1）：一次性抓取全部虚拟屏幕（含负坐标/混合 DPI 布局，CAP-5）。
//! 以 `BitBlt`（`CAPTUREBLT`）为主，输出 BGRA 像素与 RGBA 预乘副本供 tiny-skia 使用。

use std::mem::size_of;
use std::ptr::null_mut;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub use airsnip_core::capture::CapturedScreen;

/// 按 u32 批量交换 R/B 通道（BGRA ↔ RGBA），比逐字节快数倍。
fn swap_rb(buf: &mut [u8]) {
    let n = buf.len() / 4;
    unsafe {
        let p = buf.as_mut_ptr() as *mut u32;
        for i in 0..n {
            let v = *p.add(i);
            *p.add(i) = (v & 0xFF00_FF00) | ((v & 0x00FF_0000) >> 16) | ((v & 0x0000_00FF) << 16);
        }
    }
}

/// 大缓冲区多线程交换 R/B（目标地址用 usize 传递，裸指针不满足 Send）。
fn swap_rb_parallel(buf: &mut [u8]) {
    let n = buf.len() / 4;
    let workers = if buf.len() >= 1 << 20 {
        std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(4)
            .clamp(1, 8)
    } else {
        1
    };
    if workers <= 1 {
        swap_rb(buf);
        return;
    }
    let addr = buf.as_mut_ptr() as usize;
    let chunk = n.div_ceil(workers);
    std::thread::scope(|s| {
        for start in (0..n).step_by(chunk.max(1)) {
            let count = (n - start).min(chunk);
            s.spawn(move || {
                let p = addr as *mut u32;
                for i in start..start + count {
                    let v = unsafe { *p.add(i) };
                    unsafe {
                        *p.add(i) = (v & 0xFF00_FF00) | ((v & 0x00FF_0000) >> 16) | ((v & 0x0000_00FF) << 16);
                    }
                }
            });
        }
    });
}

/// 抓取屏幕指定区域（长截图逐帧采集视口用，LNG-2/3）。
/// 屏幕坐标为物理像素；区域越界部分自动裁切（BitBlt 源在屏幕外返回黑像素）。
pub fn capture_region(x: i32, y: i32, w: u32, h: u32) -> Result<CapturedScreen, String> {
    unsafe {
        if w == 0 || h == 0 {
            return Err("区域尺寸为 0，无法抓屏".into());
        }
        let hdc_screen = GetDC(None);
        if hdc_screen.0.is_null() {
            return Err("GetDC 失败".into());
        }
        let hdc_mem = CreateCompatibleDC(None);
        if hdc_mem.0.is_null() {
            let _ = ReleaseDC(None, hdc_screen);
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
        let mut bits: *mut core::ffi::c_void = null_mut();
        let hbmp = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(hbmp) => hbmp,
            Err(e) => {
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(None, hdc_screen);
                return Err(format!("CreateDIBSection 失败：{e}"));
            }
        };

        let _ = SelectObject(hdc_mem, hbmp.into());
        // 长截图不需要 CAPTUREBLT：抓取普通窗口内容即可（分层窗口内容由目标页面自绘）。
        if BitBlt(hdc_mem, 0, 0, w as i32, h as i32, Some(hdc_screen), x, y, SRCCOPY).is_err() {
            let _ = DeleteObject(hbmp.into());
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
            return Err("BitBlt 失败".into());
        }

        let len = (w * h * 4) as usize;
        let mut rgba = vec![0u8; len];
        std::ptr::copy_nonoverlapping(bits as *const u8, rgba.as_mut_ptr(), len);
        swap_rb_parallel(&mut rgba);

        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);

        Ok(CapturedScreen {
            width: w,
            height: h,
            origin_x: x,
            origin_y: y,
            rgba,
        })
    }
}

/// 抓取整个虚拟屏幕（一次，冻结画面）。
pub fn capture_virtual_screen() -> Result<CapturedScreen, String> {
    let t0 = std::time::Instant::now();
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32;
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32;
        if vw == 0 || vh == 0 {
            return Err("虚拟屏幕尺寸为 0，无法抓屏".into());
        }

        let hdc_screen = GetDC(None);
        if hdc_screen.0.is_null() {
            return Err("GetDC 失败".into());
        }
        let hdc_mem = CreateCompatibleDC(None);
        if hdc_mem.0.is_null() {
            let _ = ReleaseDC(None, hdc_screen);
            return Err("CreateCompatibleDC 失败".into());
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: vw as i32,
                biHeight: -(vh as i32), // 自上而下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [Default::default()],
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let hbmp = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(hbmp) => hbmp,
            Err(e) => {
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(None, hdc_screen);
                return Err(format!("CreateDIBSection 失败：{e}"));
            }
        };

        let _ = SelectObject(hdc_mem, hbmp.into());
        let timing = std::env::var_os("AIRSNIP_TIMING").is_some();
        let t_blt = std::time::Instant::now();
        if BitBlt(
            hdc_mem,
            0,
            0,
            vw as i32,
            vh as i32,
            Some(hdc_screen),
            vx,
            vy,
            SRCCOPY | CAPTUREBLT,
        )
        .is_err()
        {
            let _ = DeleteObject(hbmp.into());
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
            return Err("BitBlt 失败".into());
        }
        let blt = t_blt.elapsed();

        let t_copy = std::time::Instant::now();
        let len = (vw * vh * 4) as usize;
        let mut rgba = vec![0u8; len];
        std::ptr::copy_nonoverlapping(bits as *const u8, rgba.as_mut_ptr(), len);
        // BGRA（DIB）→ RGBA：批量交换 R/B（截图 alpha 不透明，预乘与直通相同）
        swap_rb_parallel(&mut rgba);
        let copy = t_copy.elapsed();
        if timing {
            eprintln!("[airsnip] BitBlt {blt:?}，拷贝+交换 {copy:?}");
        }

        let _ = DeleteObject(hbmp.into());
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);

        let result = Ok(CapturedScreen {
            width: vw,
            height: vh,
            origin_x: vx,
            origin_y: vy,
            rgba,
        });
        if std::env::var_os("AIRSNIP_TIMING").is_some() {
            eprintln!("[airsnip] 抓屏耗时 {:?}", t0.elapsed());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_screen_works() {
        let screen = capture_virtual_screen().expect("抓屏失败");
        assert!(screen.width > 0 && screen.height > 0);
        assert_eq!(screen.rgba.len(), (screen.width * screen.height * 4) as usize);
    }

    #[test]
    fn swap_rb_swaps_channels() {
        let mut buf = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        swap_rb(&mut buf);
        assert_eq!(buf, vec![3u8, 2, 1, 4, 7, 6, 5, 8]);
    }

    /// 对比 SRCCOPY 与 SRCCOPY|CAPTUREBLT 的全屏抓取耗时，供权衡。
    #[test]
    fn bitblt_rop_timing() {
        use std::time::Instant;
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let hdc_screen = GetDC(None);
            let hdc_mem = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: vw,
                    biHeight: -vh,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                bmiColors: [Default::default()],
            };
            let mut bits: *mut core::ffi::c_void = null_mut();
            let hbmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let _ = SelectObject(hdc_mem, hbmp.into());

            for (name, rop) in [("SRCCOPY", SRCCOPY), ("SRCCOPY|CAPTUREBLT", SRCCOPY | CAPTUREBLT)] {
                let t = Instant::now();
                for _ in 0..5 {
                    let _ = BitBlt(hdc_mem, 0, 0, vw, vh, Some(hdc_screen), vx, vy, rop);
                }
                println!("BitBlt {name}: {:?}/次", t.elapsed() / 5);
            }

            let _ = DeleteObject(hbmp.into());
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(None, hdc_screen);
        }
    }
}
