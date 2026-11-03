// SPDX-License-Identifier: GPL-3.0-only
//! 区域盒式模糊（EDT-5）：水平+垂直滑动窗口近似高斯，纯 CPU 整数运算。

use tiny_skia::Pixmap;

/// 区域盒式模糊：`passes` 次水平+垂直滑动窗口（边缘复制），就地修改像素图指定区域。
/// 区域坐标已在目标坐标系；半径过小或区域越界时安全返回。
pub(super) fn box_blur_region(pixmap: &mut Pixmap, x0: i32, y0: i32, rw: i32, rh: i32, radius: f32, passes: u32) {
    let r = radius.round().max(1.0) as i32;
    if rw < 1 || rh < 1 || r < 1 {
        return;
    }
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (x1, y1) = ((x0 + rw).min(w), (y0 + rh).min(h));
    let (rw, rh) = (x1 - x0, y1 - y0);
    if rw < 1 || rh < 1 {
        return;
    }
    let (rw, rh) = (rw as usize, rh as usize);
    let stride = pixmap.width() as usize;

    // 独立区域缓冲：多 pass 之间只在本区域读写，不污染区域外像素
    let mut buf = vec![0u8; rw * rh * 4];
    for row in 0..rh {
        let src = (y0 as usize + row) * stride + x0 as usize;
        buf[row * rw * 4..(row + 1) * rw * 4].copy_from_slice(&pixmap.data()[src * 4..(src + rw) * 4]);
    }
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..passes {
        box_blur_hv(&mut buf, &mut tmp, rw, rh, r as usize);
    }
    for row in 0..rh {
        let dst = (y0 as usize + row) * stride + x0 as usize;
        pixmap.data_mut()[dst * 4..(dst + rw) * 4].copy_from_slice(&buf[row * rw * 4..(row + 1) * rw * 4]);
    }
}

/// 一次水平 + 垂直盒式模糊：水平结果写入 `out`，垂直结果写回 `buf`。
/// 滑动窗口 O(n)，边缘 clamp 到边界像素（等同于复制延伸）。
fn box_blur_hv(buf: &mut [u8], out: &mut [u8], w: usize, h: usize, r: usize) {
    box_blur_h(buf, out, w, h, r);
    box_blur_v(out, buf, w, h, r);
}

/// 水平滑动窗口模糊：`src` → `dst`（每行独立窗口）。
fn box_blur_h(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let win = (r * 2 + 1) as u32;
    for y in 0..h {
        let base = y * w;
        for c in 0..4 {
            let mut sum: u32 = 0;
            for x in -(r as i32)..=(r as i32) {
                let px = x.clamp(0, w as i32 - 1) as usize;
                sum += src[(base + px) * 4 + c] as u32;
            }
            for x in 0..w {
                dst[(base + x) * 4 + c] = (sum / win) as u8;
                let rm = (x as i32 - r as i32).clamp(0, w as i32 - 1) as usize;
                let add = (x as i32 + r as i32 + 1).clamp(0, w as i32 - 1) as usize;
                sum += src[(base + add) * 4 + c] as u32;
                sum -= src[(base + rm) * 4 + c] as u32;
            }
        }
    }
}

/// 垂直滑动窗口模糊：`src` → `dst`（每列独立窗口）。
fn box_blur_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let win = (r * 2 + 1) as u32;
    for x in 0..w {
        for c in 0..4 {
            let mut sum: u32 = 0;
            for y in -(r as i32)..=(r as i32) {
                let py = y.clamp(0, h as i32 - 1) as usize;
                sum += src[(py * w + x) * 4 + c] as u32;
            }
            for y in 0..h {
                dst[(y * w + x) * 4 + c] = (sum / win) as u8;
                let rm = (y as i32 - r as i32).clamp(0, h as i32 - 1) as usize;
                let add = (y as i32 + r as i32 + 1).clamp(0, h as i32 - 1) as usize;
                sum += src[(add * w + x) * 4 + c] as u32;
                sum -= src[(rm * w + x) * 4 + c] as u32;
            }
        }
    }
}
