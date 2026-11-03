// SPDX-License-Identifier: GPL-3.0-only
//! 界面绘制的共用小工具（覆盖层工具栏、长截图状态条、设置面板共用）：
//! 纯色画刷与圆角矩形（填充 / 描边 / 路径）。
//! 图标本身由 `crate::render::icon` 绘制，这里只提供容器与背景。

use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::overlay::geometry::SelRect;

/// 由 RGBA 构造纯色画刷。
pub fn paint_of(color: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(color[0], color[1], color[2], color[3]);
    p
}

/// 填充圆角矩形（可选描边）。
pub fn rounded_rect(pixmap: &mut Pixmap, r: SelRect, radius: f32, fill: [u8; 4], border: Option<[u8; 4]>) {
    let Some(path) = rounded_path(r, radius) else {
        return;
    };
    pixmap.fill_path(
        &path,
        &paint_of(fill),
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    if let Some(bc) = border {
        let stroke = tiny_skia::Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_of(bc), &stroke, Transform::identity(), None);
    }
}

/// 仅描边的圆角矩形（选中态外框）。
pub fn rounded_rect_outline(pixmap: &mut Pixmap, r: SelRect, radius: f32, color: [u8; 4]) {
    let Some(path) = rounded_path(r, radius) else {
        return;
    };
    let stroke = tiny_skia::Stroke {
        width: 2.0,
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint_of(color), &stroke, Transform::identity(), None);
}

/// 圆角矩形路径（半径自动收敛到短边一半以内）。
pub fn rounded_path(r: SelRect, radius: f32) -> Option<tiny_skia::Path> {
    let rect = Rect::from_ltrb(
        r.x as f32 + 0.5,
        r.y as f32 + 0.5,
        (r.x + r.w) as f32 - 0.5,
        (r.y + r.h) as f32 - 0.5,
    )?;
    let rad = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let mut pb = PathBuilder::new();
    if rad < 0.5 {
        pb.push_rect(rect);
        return pb.finish();
    }
    let k = rad * 0.552_284_8;
    let (x0, y0, x1, y1) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    pb.move_to(x0 + rad, y0);
    pb.line_to(x1 - rad, y0);
    pb.cubic_to(x1 - rad + k, y0, x1, y0 + rad - k, x1, y0 + rad);
    pb.line_to(x1, y1 - rad);
    pb.cubic_to(x1, y1 - rad + k, x1 - rad + k, y1, x1 - rad, y1);
    pb.line_to(x0 + rad, y1);
    pb.cubic_to(x0 + rad - k, y1, x0, y1 - rad + k, x0, y1 - rad);
    pb.line_to(x0, y0 + rad);
    pb.cubic_to(x0, y0 + rad - k, x0 + rad - k, y0, x0 + rad, y0);
    pb.close();
    pb.finish()
}
