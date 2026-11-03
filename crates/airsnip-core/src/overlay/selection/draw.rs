// SPDX-License-Identifier: GPL-3.0-only
//! 选区装饰绘制（CAP-2/3）：选区边框、八向缩放手柄、聚光区域恢复。
//! 尺寸基准为 100% DPI，经 `Ui` 缩放（CAP-5）。

use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::super::geometry::SelRect;
use super::super::surface::blit_region;
use super::super::ui::Ui;
use crate::render::mask::FRect;

/// 手柄边长基准值。
const HANDLE_SIZE_BASE: f32 = 7.0;
const ACCENT: u8 = 26; // 主题蓝
const ACCENT_G: u8 = 115;
const ACCENT_B: u8 = 232;

/// 聚光区域恢复（EDT-9）：直角直接回贴原图；圆角按遮罩混合（圆周外保持暗化）。
/// `rect` 为**完整**聚光矩形（`part` 可能已被脏区裁剪，不能用它算圆角）。
pub(super) fn restore_glow(display: &mut Pixmap, original: &Pixmap, part: SelRect, rect: FRect, corner: f32) {
    let r = crate::render::mask::clamp_radius(rect, corner);
    if r < 0.5 {
        blit_region(display, original, part);
        return;
    }
    let region = (part.x, part.y, part.x + part.w, part.y + part.h);
    let buf = crate::render::mask::copy_region(original, region);
    crate::render::mask::blend_compact(display, &buf, region, rect, r);
}

/// 画选区边框。
pub(super) fn draw_rect(pixmap: &mut Pixmap, sel: SelRect, ui: Ui) {
    let line = ui.f(2.0);
    let Some(rect) = Rect::from_xywh(
        sel.x as f32 + 0.5,
        sel.y as f32 + 0.5,
        (sel.w as f32 - 1.0).max(1.0),
        (sel.h as f32 - 1.0).max(1.0),
    ) else {
        return;
    };
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let Some(path) = pb.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(ACCENT, ACCENT_G, ACCENT_B, 255);
    let stroke = Stroke {
        width: line,
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    // 外圈白色辅助线
    let mut pb = PathBuilder::new();
    if let Some(inner) = Rect::from_xywh(
        sel.x as f32 + line * 0.75,
        sel.y as f32 + line * 0.75,
        (sel.w as f32 - line * 1.5).max(1.0),
        (sel.h as f32 - line * 1.5).max(1.0),
    ) {
        pb.push_rect(inner);
    }
    if let Some(p) = pb.finish() {
        let mut white = Paint::default();
        white.set_color_rgba8(255, 255, 255, 160);
        let thin = Stroke {
            width: ui.f(1.0),
            ..Default::default()
        };
        pixmap.stroke_path(&p, &white, &thin, Transform::identity(), None);
    }
}

/// 画 8 个缩放手柄。
pub(super) fn draw_handles(pixmap: &mut Pixmap, sel: SelRect, ui: Ui) {
    let size = ui.px(HANDLE_SIZE_BASE);
    let (x0, y0, x1, y1) = (sel.x, sel.y, sel.x + sel.w, sel.y + sel.h);
    let cx = x0 + sel.w / 2;
    let cy = y0 + sel.h / 2;
    for (hx, hy) in [
        (x0, y0),
        (cx, y0),
        (x1, y0),
        (x0, cy),
        (x1, cy),
        (x0, y1),
        (cx, y1),
        (x1, y1),
    ] {
        let r = size / 2;
        let mut paint = Paint::default();
        paint.set_color_rgba8(255, 255, 255, 255);
        if let Some(rect) = Rect::from_xywh((hx - r) as f32, (hy - r) as f32, size as f32, size as f32) {
            let mut pb = PathBuilder::new();
            pb.push_rect(rect);
            if let Some(p) = pb.finish() {
                pixmap.fill_path(&p, &paint, tiny_skia::FillRule::Winding, Transform::identity(), None);
            }
        }
    }
}
