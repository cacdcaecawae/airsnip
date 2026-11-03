// SPDX-License-Identifier: GPL-3.0-only
//! 区域类标注的逐像素渲染：EDT-6 高亮（荧光笔）与 EDT-5 高斯模糊。
//!
//! 两者都是整块区域的像素运算，圆角通过 `mask::corner_coverage` 施加；
//! **`corner == 0` 走原有快速路径**，不引入任何逐像素距离计算（性能红线，见 `docs/perf-m5.md`）。

use tiny_skia::{Pixmap, Transform};

use super::blur;
use super::mask::{self, FRect, Region};
use crate::editor::{Point, Style};

/// 荧光笔透明度（EDT-6：半透明块 + 正片叠底，透出底图文字）。
const HIGHLIGHT_ALPHA: u8 = 150;

/// 对象矩形 →（裁剪到画布的像素区域，完整对象矩形）。
/// 两者必须分开：脏区重绘时区域会被裁剪，但圆角要按**完整**对象矩形算。
fn mapped_rect(pixmap: &Pixmap, a: Point, b: Point, tr: Transform) -> Option<(Region, FRect)> {
    // 变换目前只有导出烘焙的纯平移；写成通用仿射，避免依赖 tiny-skia 的 Point 类型
    let map = |p: Point| Point::new(tr.sx * p.x + tr.kx * p.y + tr.tx, tr.ky * p.x + tr.sy * p.y + tr.ty);
    let (pa, pb) = (map(a), map(b));
    let full = (pa.x.min(pb.x), pa.y.min(pb.y), pa.x.max(pb.x), pa.y.max(pb.y));
    let (gx0, gy0) = (full.0.floor() as i32, full.1.floor() as i32);
    let (gx1, gy1) = (full.2.ceil() as i32, full.3.ceil() as i32);
    if gx1 - gx0 < 1 || gy1 - gy0 < 1 {
        return None;
    }
    Some((mask::clip_region(pixmap, (gx0, gy0, gx1, gy1))?, full))
}

/// EDT-6 高亮：只取调色板 RGB（alpha 固定为荧光笔透明度）。
/// 逐像素混合 `dst × (1-a + a×src/255)`（正片叠底的半透明形式）：
/// 白底被染成淡黄、深色文字几乎不变，不遮挡底图。纯 CPU 整数运算。
/// `corner > 0` 时四角按覆盖率在「原像素」与「染色像素」之间插值。
pub(super) fn draw_highlight(pixmap: &mut Pixmap, a: Point, b: Point, corner: f32, style: &Style, tr: Transform) {
    let Some((region, full)) = mapped_rect(pixmap, a, b, tr) else {
        return;
    };
    let (x0, y0, x1, y1) = region;
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let stride = pixmap.width() as usize;
    // 预计算各通道系数：coef = 255 - a + a×src/255
    let alpha = HIGHLIGHT_ALPHA as u32;
    let inv = 255 - alpha;
    let coef = [
        inv + alpha * style.color[0] as u32 / 255,
        inv + alpha * style.color[1] as u32 / 255,
        inv + alpha * style.color[2] as u32 / 255,
    ];
    let r = mask::clamp_radius(full, corner);
    let data = pixmap.data_mut();

    if r < 0.5 {
        // 直角快速路径：与加圆角前逐字节一致，不做任何逐像素判定
        for row in 0..rh {
            let base = ((y0 as usize + row) * stride + x0 as usize) * 4;
            for col in 0..rw {
                tint_px(data, base + col * 4, &coef);
            }
        }
        return;
    }
    // 圆角：只有四角方块内的像素才逐像素算覆盖率，直边行整行走快速路径
    let bands = mask::CornerBands::new(region, full, r);
    for row in 0..rh {
        let base = ((y0 as usize + row) * stride + x0 as usize) * 4;
        if !bands.row_cornered(row) {
            for col in 0..rw {
                tint_px(data, base + col * 4, &coef);
            }
            continue;
        }
        let py = (y0 + row as i32) as f32 + 0.5;
        for col in 0..rw {
            if col >= bands.n_left && col < bands.r_start {
                tint_px(data, base + col * 4, &coef);
                continue;
            }
            let px = (x0 + col as i32) as f32 + 0.5;
            let cov = mask::corner_coverage(px, py, full, r);
            if cov <= 0.0 {
                continue;
            }
            let off = base + col * 4;
            for c in 0..3 {
                let d = data[off + c] as u32;
                let tinted = (d * coef[c] / 255) as f32;
                data[off + c] = (d as f32 + (tinted - d as f32) * cov) as u8;
            }
        }
    }
}

/// 单像素染色（`coef = 255 - a + a×src/255`）；直角快速路径与圆角中间段共用。
#[inline]
fn tint_px(data: &mut [u8], off: usize, coef: &[u32; 3]) {
    for c in 0..3 {
        let d = data[off + c] as u32;
        data[off + c] = (d * coef[c] / 255) as u8;
    }
}

/// EDT-5 高斯模糊：从像素图自身抠出区域做盒式模糊后贴回。
/// 重绘路径上底图已复位（见 `overlay::selection::repaint`），此处模糊的即原图内容；
/// z 序在其下的对象也会一并模糊（与真实模糊工具盖在上层的效果一致）。
/// 盒式模糊（Box Blur）迭代近似高斯，纯 CPU 友好；`tr` 用于导出烘焙的坐标平移。
/// `corner > 0` 时四角按遮罩恢复模糊前的原像素。
pub(super) fn draw_blur(pixmap: &mut Pixmap, a: Point, b: Point, radius: f32, corner: f32, tr: Transform) {
    let Some((region, full)) = mapped_rect(pixmap, a, b, tr) else {
        return;
    };
    let (x0, y0, x1, y1) = region;
    let r = mask::clamp_radius(full, corner);
    if r < 0.5 {
        // 直角快速路径：原逻辑不动
        blur::box_blur_region(pixmap, x0, y0, x1 - x0, y1 - y0, radius, 2);
        return;
    }
    // 模糊是就地修改，圆角处要恢复的「原像素」必须在模糊前备份
    let backup = mask::copy_region(pixmap, region);
    blur::box_blur_region(pixmap, x0, y0, x1 - x0, y1 - y0, radius, 2);
    mask::restore_compact(pixmap, &backup, region, full, r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Kind, Object};
    use tiny_skia::Color;

    fn pixel(p: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let off = ((y * p.width() + x) * 4) as usize;
        (p.data()[off], p.data()[off + 1], p.data()[off + 2], p.data()[off + 3])
    }

    fn highlight(a: (f32, f32), b: (f32, f32), corner: f32) -> Object {
        Object {
            kind: Kind::Highlight {
                a: Point::new(a.0, a.1),
                b: Point::new(b.0, b.1),
                corner,
            },
            style: Style::new([255, 240, 0, 255], 2.0),
        }
    }

    fn blank(w: u32, h: u32) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(Color::from_rgba8(255, 255, 255, 255));
        p
    }

    fn white_block(p: &mut Pixmap, x: i32, y: i32, size: i32) {
        let w = p.width() as i32;
        for dy in y..y + size {
            for dx in x..x + size {
                let off = ((dy * w + dx) * 4) as usize;
                p.data_mut()[off..off + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }

    /// 圆角高亮：四角不被染色、直边与中心被染色（与直角版对比）。
    #[test]
    fn highlight_corner_leaves_corners_untouched() {
        let mut p = blank(80, 60);
        crate::render::draw_object(&mut p, &highlight((10.0, 10.0), (70.0, 50.0), 12.0));

        assert_eq!(pixel(&p, 5, 5), (255, 255, 255, 255), "对象外应保持原样");
        assert_eq!(pixel(&p, 10, 10), (255, 255, 255, 255), "圆角处应未被染色");
        assert_eq!(pixel(&p, 69, 49), (255, 255, 255, 255), "右下圆角也应未被染色");
        let (_, g, b, _) = pixel(&p, 40, 30);
        assert!(g < 255 && b < 160, "中心应被染黄：g={g} b={b}");
        let (_, g, b, _) = pixel(&p, 40, 10);
        assert!(g < 255 && b < 160, "上直边应被染色：g={g} b={b}");
    }

    /// 圆角模糊：四角恢复成模糊前的原像素（含未被模糊的亮块），中心被平滑。
    #[test]
    fn blur_corner_keeps_original_pixels_at_corners() {
        let mut p = Pixmap::new(60, 60).unwrap();
        p.fill(Color::from_rgba8(0, 0, 0, 255));
        white_block(&mut p, 6, 6, 4); // 落在左上圆角的切角区内
        white_block(&mut p, 27, 27, 5); // 落在中心，会被模糊
        let obj = Object {
            kind: Kind::Blur {
                a: Point::new(5.0, 5.0),
                b: Point::new(55.0, 55.0),
                radius: 6.0,
                corner: 20.0,
            },
            style: Style::new([0, 0, 0, 255], 2.0),
        };
        crate::render::draw_object(&mut p, &obj);

        assert_eq!(
            pixel(&p, 7, 7),
            (255, 255, 255, 255),
            "圆角处应恢复模糊前的原像素（亮块未被模糊）"
        );
        let (center, _, _, _) = pixel(&p, 29, 29);
        assert!(center > 0 && center < 200, "中心亮块应被模糊：r={center}");
        assert_eq!(pixel(&p, 0, 0), (0, 0, 0, 255), "模糊区域外不变");
    }
}
