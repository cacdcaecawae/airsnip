// SPDX-License-Identifier: GPL-3.0-only
//! 自绘光标（CAP-2）：白十字（默认）与抓手（按住空格移动选区）。
//! 方向 / 移动光标用系统内置（见 `super`），这里只画系统没有的形状。
//! 画布尺寸随显示器 DPI 缩放（CAP-5）：4K / 200% 下十字与抓手不再只有一半大。
//! 圆角光标（EDT-1）见 `super::radius`。

use std::mem::size_of;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyCursor, HCURSOR, ICONINFO};

use super::super::ui::Ui;
use super::CANVAS_BASE;

/// 创建白十字 + 黑描边彩色光标（32bpp alpha），保证在暗化画面上清晰可见。
/// 结果缓存进 static：窗口类可多次注册（截图每次重注册），直接新建会泄漏 GDI 图标句柄。
/// 尺寸随 DPI 变化（换屏后再次截图）时销毁旧句柄重建，不累积泄漏。
pub(in crate::overlay) fn cross_cursor(ui: Ui) -> HCURSOR {
    let size = ui.px(CANVAS_BASE);
    let cached = CROSS_CURSOR.load(Ordering::Relaxed);
    if cached != 0 && CROSS_SIZE.load(Ordering::Relaxed) == size {
        return HCURSOR(cached as *mut core::ffi::c_void);
    }
    if cached != 0 {
        // 旧句柄只被本缓存持有（窗口类用的是系统十字，见 `super::class_cursor`），销毁安全
        unsafe {
            let _ = DestroyCursor(HCURSOR(cached as *mut core::ffi::c_void));
        }
    }
    let cur = build_cross_cursor(size);
    CROSS_CURSOR.store(cur.0 as isize, Ordering::Relaxed);
    CROSS_SIZE.store(size, Ordering::Relaxed);
    cur
}

/// 十字光标句柄缓存（进程内按尺寸只保留一个）。
static CROSS_CURSOR: AtomicIsize = AtomicIsize::new(0);
/// 缓存的十字光标画布边长。
static CROSS_SIZE: AtomicI32 = AtomicI32::new(0);

/// 实际创建十字光标（`s` 为画布边长，几何按 `s/32` 等比缩放）。
fn build_cross_cursor(s: i32) -> HCURSOR {
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
        let mut set = |x: i32, y: i32, r: u8, g: u8, b: u8, a: u8| {
            if x < 0 || y < 0 || x >= s || y >= s {
                return;
            }
            let i = ((y * s + x) * 4) as usize;
            px[i] = b;
            px[i + 1] = g;
            px[i + 2] = r;
            px[i + 3] = a;
        };
        let k = s as f32 / CANVAS_BASE;
        let (cx, cy) = (s / 2, s / 2);
        let arm = (14.0 * k).round() as i32;
        // 线宽随缩放增加（100% 下 1px，200% 下 2px），否则 4K 上细得看不见
        let lw = (k.round() as i32).max(1);
        // 黑色描边（白线两侧各 lw 像素）
        for d in -arm..=arm {
            for w in -lw..=lw {
                set(cx + d, cy + w, 0, 0, 0, 255);
                set(cx + w, cy + d, 0, 0, 0, 255);
            }
        }
        // 白色细十字（lw 像素宽，居中）
        for d in -arm..=arm {
            for o in 0..lw {
                let off = o - lw / 2;
                set(cx + d, cy + off, 255, 255, 255, 255);
                set(cx + off, cy + d, 255, 255, 255, 255);
            }
        }

        let mask = CreateBitmap(s, s, 1, 1, None);
        let info = ICONINFO {
            fIcon: false.into(),
            xHotspot: cx as u32,
            yHotspot: cy as u32,
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

/// 抓手光标句柄缓存（进程内按尺寸只保留一个，避免每次截图泄漏 GDI 句柄）。
static HAND_CURSOR: AtomicIsize = AtomicIsize::new(0);
/// 缓存的抓手光标画布边长。
static HAND_SIZE: AtomicI32 = AtomicI32::new(0);

/// 手掌形状的字符画掩码，`#` 为实心（三指 + 拇指 + 掌心）。
const HAND_MASK: [&str; 16] = [
    ".....##........",
    ".....##........",
    ".....##..##....",
    ".....##..##....",
    ".....##..##.##.",
    ".....##..##.##.",
    "..#..##..##.##.",
    "..#####..##.##.",
    "..############.",
    ".#############.",
    "..############.",
    "..############.",
    "...###########.",
    "...##########..",
    "....#########..",
    ".....#######...",
];

/// 掩码在光标画布中的偏移（32px 基准画布）。
const HAND_OFFSET: (i32, i32) = (8, 6);
/// 热点在基准画布中的位置。
const HAND_HOTSPOT: (f32, f32) = (15.0, 17.0);

/// 把抓手掩码绘制到 BGRA 缓冲区：形状白色 + 外侧 1px 黑边（不侵入形状，细手指也保持白色）。
/// `s` 为画布边长，按 `s/32` 最近邻放大掩码。
fn paint_hand(px: &mut [u8], s: i32) {
    let k = s as f32 / CANVAS_BASE;
    // 目标像素 → 基准画布坐标
    let base = |v: i32| ((v as f32 + 0.5) / k).floor() as i32;
    let filled = |gx: i32, gy: i32| -> bool {
        if gx < 0 || gy < 0 || gy >= HAND_MASK.len() as i32 {
            return false;
        }
        let row = HAND_MASK[gy as usize].as_bytes();
        gx < row.len() as i32 && row[gx as usize] == b'#'
    };
    for y in 0..s {
        for x in 0..s {
            let (gx, gy) = (base(x) - HAND_OFFSET.0, base(y) - HAND_OFFSET.1);
            let inside = filled(gx, gy);
            let halo = !inside && (-1..=1).any(|dy| (-1..=1).any(|dx| filled(gx + dx, gy + dy)));
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

/// 抓手光标（空格移动选区时使用）。
pub(super) fn hand_cursor(ui: Ui) -> HCURSOR {
    let size = ui.px(CANVAS_BASE);
    let cached = HAND_CURSOR.load(Ordering::Relaxed);
    if cached != 0 && HAND_SIZE.load(Ordering::Relaxed) == size {
        return HCURSOR(cached as *mut core::ffi::c_void);
    }
    if cached != 0 {
        unsafe {
            let _ = DestroyCursor(HCURSOR(cached as *mut core::ffi::c_void));
        }
    }
    let cur = create_hand_cursor(size);
    HAND_CURSOR.store(cur.0 as isize, Ordering::Relaxed);
    HAND_SIZE.store(size, Ordering::Relaxed);
    cur
}

/// 自绘抓手光标：Win32 无内置抓手，用字符画掩码 + 外侧 1px 黑边生成 32bpp 彩色光标。
fn create_hand_cursor(s: i32) -> HCURSOR {
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
        paint_hand(px, s);

        let k = s as f32 / CANVAS_BASE;
        let mask = CreateBitmap(s, s, 1, 1, None);
        let info = ICONINFO {
            fIcon: false.into(),
            xHotspot: (HAND_HOTSPOT.0 * k) as u32,
            yHotspot: (HAND_HOTSPOT.1 * k) as u32,
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

    /// 抓手光标创建并缓存（空格移动选区用）。
    #[test]
    fn hand_cursor_is_cached() {
        let a = hand_cursor(base());
        let b = hand_cursor(base());
        assert!(!a.0.is_null(), "抓手光标创建失败");
        assert_eq!(a.0, b.0, "抓手光标应复用缓存句柄");
    }

    /// 抓手掩码应同时有白色掌心/手指与黑色轮廓（否则在浅色或深色背景上会看不见）。
    #[test]
    fn hand_mask_has_white_shape_and_black_outline() {
        for ui in [base(), Ui::from_dpi(192)] {
            let s = ui.px(CANVAS_BASE);
            let mut px = vec![0u8; (s * s * 4) as usize];
            paint_hand(&mut px, s);
            let count = |rgb: [u8; 3]| {
                px.chunks_exact(4)
                    .filter(|p| p[0] == rgb[2] && p[1] == rgb[1] && p[2] == rgb[0] && p[3] == 255)
                    .count()
            };
            let k = (ui.scale() * ui.scale()) as usize;
            let white = count([255, 255, 255]);
            let black = count([0, 0, 0]);
            assert!(white > 60 * k, "白色形状过小（{s}px）：{white}");
            assert!(black > 60 * k, "黑色轮廓过小（{s}px）：{black}");
            assert!(
                white < 400 * k && black < 400 * k,
                "形状过大（{s}px）：white={white} black={black}"
            );
        }
    }
}
