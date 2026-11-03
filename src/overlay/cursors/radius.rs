// SPDX-License-Identifier: GPL-3.0-only
//! 圆角光标（EDT-1）：圆弧 + 拖拽提示点，Win32 没有「调圆角」的内置光标。
//! 画布尺寸随显示器 DPI 缩放（CAP-5）。

use std::mem::size_of;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyCursor, HCURSOR, ICONINFO};

use super::super::ui::Ui;
use super::CANVAS_BASE;

/// 圆角光标（EDT-1）：圆弧 + 拖拽提示点，Win32 没有「调圆角」的内置光标。
pub(super) fn radius_cursor(ui: Ui) -> HCURSOR {
    let size = ui.px(CANVAS_BASE);
    let cached = RADIUS_CURSOR.load(Ordering::Relaxed);
    if cached != 0 && RADIUS_SIZE.load(Ordering::Relaxed) == size {
        return HCURSOR(cached as *mut core::ffi::c_void);
    }
    if cached != 0 {
        unsafe {
            let _ = DestroyCursor(HCURSOR(cached as *mut core::ffi::c_void));
        }
    }
    let cur = create_radius_cursor(size);
    RADIUS_CURSOR.store(cur.0 as isize, Ordering::Relaxed);
    RADIUS_SIZE.store(size, Ordering::Relaxed);
    cur
}

/// 圆角光标句柄缓存（进程内按尺寸只保留一个，避免每次截图泄漏 GDI 句柄）。
static RADIUS_CURSOR: AtomicIsize = AtomicIsize::new(0);
/// 缓存的圆角光标画布边长。
static RADIUS_SIZE: AtomicI32 = AtomicI32::new(0);

/// 圆角光标的形状：左下象限的圆弧 + 弧中点的小圆点（`(x, y)` 为像素坐标，
/// 按 `k = s/32` 缩放到基准画布上判定）。
fn radius_shape(x: i32, y: i32, k: f32) -> bool {
    let (fx, fy) = ((x as f32 + 0.5) / k, (y as f32 + 0.5) / k);
    // 以 (22, 10) 为圆心、半径 14 的圆弧，只取左下象限（形状像一个圆角）
    let (dx, dy) = (fx - 22.0, fy - 10.0);
    let on_arc = ((dx * dx + dy * dy).sqrt() - 14.0).abs() <= 1.2 && dx <= 0.5 && dy >= -0.5;
    // 弧中点的小圆点：暗示「这里可以拖」
    let dot = (fx - 12.1).powi(2) + (fy - 19.9).powi(2) <= 9.0;
    on_arc || dot
}

/// 把圆角形状绘制到 BGRA 缓冲区：形状白色 + 外侧 1px 黑边（暗/浅背景都看得见）。
fn paint_radius(px: &mut [u8], s: i32) {
    let k = s as f32 / CANVAS_BASE;
    for y in 0..s {
        for x in 0..s {
            let inside = radius_shape(x, y, k);
            let halo = !inside && (-1..=1).any(|dy| (-1..=1).any(|dx| radius_shape(x + dx, y + dy, k)));
            if !inside && !halo {
                continue;
            }
            let i = ((y * s + x) * 4) as usize;
            let (r, g, b) = if inside { (255, 255, 255) } else { (0, 0, 0) };
            px[i] = b;
            px[i + 1] = g;
            px[i + 2] = r;
            px[i + 3] = 255;
        }
    }
}

/// 自绘圆角光标。
fn create_radius_cursor(s: i32) -> HCURSOR {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: s,
                biHeight: -s,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [Default::default()],
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let hbm_color = match CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) => b,
            Err(_) => return HCURSOR::default(),
        };
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (s * s * 4) as usize);
        paint_radius(px, s);

        let mask = CreateBitmap(s, s, 1, 1, None);
        let info = ICONINFO {
            fIcon: false.into(),
            xHotspot: (s / 2) as u32,
            yHotspot: (s / 2) as u32,
            hbmMask: mask,
            hbmColor: hbm_color,
        };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(hbm_color.into());
        if !mask.0.is_null() {
            let _ = DeleteObject(mask.into());
        }
        HCURSOR(icon.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Ui {
        Ui::from_dpi(96)
    }

    /// 圆角光标创建并缓存（拖圆角控制柄时用）。
    #[test]
    fn radius_cursor_is_cached() {
        let a = radius_cursor(base());
        let b = radius_cursor(base());
        assert!(!a.0.is_null(), "圆角光标创建失败");
        assert_eq!(a.0, b.0, "圆角光标应复用缓存句柄");
    }

    /// 圆角光标要有白色形状与黑色轮廓，且尺寸不能填满整个画布。
    /// 形状是细弧 + 小圆点（面积随周长增长，非平方），高 DPI 下限只要求翻倍。
    #[test]
    fn radius_shape_has_white_and_outline() {
        for (ui, min_px) in [(base(), 40), (Ui::from_dpi(192), 80)] {
            let s = ui.px(CANVAS_BASE);
            let mut px = vec![0u8; (s * s * 4) as usize];
            paint_radius(&mut px, s);
            let count = |rgb: [u8; 3]| {
                px.chunks_exact(4)
                    .filter(|p| p[0] == rgb[2] && p[1] == rgb[1] && p[2] == rgb[0] && p[3] == 255)
                    .count()
            };
            let max_px = 400 * (ui.scale() * ui.scale()) as usize;
            let white = count([255, 255, 255]);
            let black = count([0, 0, 0]);
            assert!(white > min_px, "白色形状过小（{s}px）：{white}");
            assert!(black > min_px, "黑色轮廓过小（{s}px）：{black}");
            assert!(
                white < max_px && black < max_px,
                "形状过大（{s}px）：white={white} black={black}"
            );
        }
    }
}
