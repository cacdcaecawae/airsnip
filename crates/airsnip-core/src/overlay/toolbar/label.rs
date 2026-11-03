// SPDX-License-Identifier: GPL-3.0-only
//! 工具栏小部件绘制（EDT-4）：文字标签、截断、竖分隔线。
//! 工具栏第一行与字体列表面板共用，故独立于 `draw`（第二行按钮）与 `fontlist`（列表）。

use tiny_skia::{Pixmap, Rect, Transform};

use crate::editor::Point;
use crate::overlay::SelRect;
use crate::render::text::{self as dwrite, FontSpec};
use crate::theme::Palette;

use crate::overlay::shape::paint_of as paint;

/// 工具栏文字尺寸基准值（字体名 / B / I / 字号），经 `Ui` 缩放。
pub(super) const LABEL_SIZE_BASE: f32 = 13.0;

/// 竖分隔线。
pub(super) fn divider(pixmap: &mut Pixmap, x: i32, y: i32, h: i32, p: &Palette) {
    if let Some(rect) = Rect::from_xywh(x as f32, y as f32, 1.0, h as f32) {
        pixmap.fill_rect(rect, &paint(p.divider), Transform::identity(), None);
    }
}

/// 字体名截断：超过 `max_w` 时逐字符减到刚好放得下，末尾加省略号。
pub(super) fn fit_name(name: &str, max_w: f32, spec: &FontSpec) -> String {
    if dwrite::measure(name, spec).0 <= max_w {
        return name.to_string();
    }
    let mut out = String::new();
    for ch in name.chars() {
        let mut cand = out.clone();
        cand.push(ch);
        cand.push('…');
        if dwrite::measure(&cand, spec).0 > max_w {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// 文字（左对齐，左边距可调）。
pub(super) fn left_text(pixmap: &mut Pixmap, r: SelRect, pad: i32, text: &str, spec: &FontSpec, color: [u8; 4]) {
    let (_, h) = dwrite::measure(text, spec);
    let x = (r.x + pad) as f32;
    let y = r.y as f32 + (r.h as f32 - h) / 2.0;
    dwrite::draw(pixmap, Point::new(x, y), text, spec, color, Transform::identity());
}

/// 文字（在矩形内水平居中）。
pub(super) fn centered_text(pixmap: &mut Pixmap, r: SelRect, text: &str, spec: &FontSpec, color: [u8; 4]) {
    let (w, h) = dwrite::measure(text, spec);
    let x = r.x as f32 + (r.w as f32 - w) / 2.0;
    let y = r.y as f32 + (r.h as f32 - h) / 2.0;
    dwrite::draw(pixmap, Point::new(x, y), text, spec, color, Transform::identity());
}
