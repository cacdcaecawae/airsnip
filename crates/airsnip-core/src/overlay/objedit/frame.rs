// SPDX-License-Identifier: GPL-3.0-only
//! 选中对象的编辑手柄绘制（EDT-7）：箭头为两端点方柄，其余对象为虚线包围框 + 八向手柄。
//! 仅屏幕显示，不进导出烘焙；手柄位置与命中判定共用 `super::handle_point` / `super::frame_of`。
//! 尺寸基准为 100% DPI，经 `Ui` 缩放（CAP-5）。

use tiny_skia::{FillRule, PathBuilder, Pixmap, Stroke, Transform};

use crate::editor::{Kind, Object, Point};

use super::super::geometry::SelRect;
use super::super::pick::dashed_line;
use super::super::shape::rounded_path;
use super::super::ui::Ui;
use super::{ObjHandle, frame_of, handle_point, radius_handle_point};

/// 手柄边长基准值（物理像素）。
const HANDLE_BASE: f32 = 7.0;
/// 圆角控制柄半径基准值（略小于方柄半边长，避免看起来像个缩放手柄）。
const ROUND_HANDLE_BASE: f32 = 3.5;
const ACCENT: [u8; 4] = [26, 115, 232, 255];

/// 画选中对象的编辑手柄。
/// 箭头（EDT-2/EDT-3）：不画包围框，只在头尾给两个方形端点手柄（拖动拉长 / 改向），
/// 曲率控制点由 `pick::draw_curve_handles` 的 1/3、2/3 圆点负责；
/// 其余对象：虚线包围框 + 八向缩放手柄。
pub(in crate::overlay) fn draw_frame(pixmap: &mut Pixmap, obj: &Object, ui: Ui) {
    let handle = ui.f(HANDLE_BASE);
    if let Kind::Arrow { from, to, .. } = &obj.kind {
        for p in [*from, *to] {
            square_handle(pixmap, p, handle, ui);
        }
        return;
    }
    let f = frame_of(obj);
    let (x0, y0) = (f.x as f32 + 0.5, f.y as f32 + 0.5);
    let (x1, y1) = ((f.x + f.w) as f32 - 0.5, (f.y + f.h) as f32 - 0.5);
    // 包围框：半透明黑打底（浅色底图上可见）+ 白色虚线（深色底图上可见）
    let mut edge = PathBuilder::new();
    edge.move_to(x0, y0);
    edge.line_to(x1, y0);
    edge.line_to(x1, y1);
    edge.line_to(x0, y1);
    edge.close();
    if let Some(path) = edge.finish() {
        let stroke = Stroke {
            width: ui.f(1.0),
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint([0, 0, 0, 120]), &stroke, Transform::identity(), None);
    }
    for (a, b) in [
        ((x0, y0), (x1, y0)),
        ((x1, y0), (x1, y1)),
        ((x1, y1), (x0, y1)),
        ((x0, y1), (x0, y0)),
    ] {
        dashed_line(pixmap, Point::new(a.0, a.1), Point::new(b.0, b.1), ui);
    }
    // 八个控制柄（白底 + 主题蓝描边，与选区手柄区分）
    for h in [
        ObjHandle::NW,
        ObjHandle::N,
        ObjHandle::NE,
        ObjHandle::W,
        ObjHandle::E,
        ObjHandle::SW,
        ObjHandle::S,
        ObjHandle::SE,
    ] {
        let (hx, hy) = handle_point(f, h);
        square_handle(pixmap, Point::new(hx, hy), handle, ui);
    }
    // 圆角控制柄（EDT-1）：内侧右上角的小圆，与方形缩放手柄区分
    if let Some((hx, hy)) = radius_handle_point(obj, ui) {
        circle_handle(pixmap, Point::new(hx, hy), ui);
    }
}

/// 白底蓝框的圆形手柄（圆角控制柄）。
fn circle_handle(pixmap: &mut Pixmap, p: Point, ui: Ui) {
    let mut pb = PathBuilder::new();
    pb.push_circle(p.x, p.y, ui.f(ROUND_HANDLE_BASE));
    let Some(path) = pb.finish() else {
        return;
    };
    pixmap.fill_path(
        &path,
        &paint([255, 255, 255, 255]),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let stroke = Stroke {
        width: ui.f(1.0),
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint(ACCENT), &stroke, Transform::identity(), None);
}

/// 白底蓝框的方形手柄（边长 `size`）。
fn square_handle(pixmap: &mut Pixmap, p: Point, size: f32, ui: Ui) {
    let r = SelRect {
        x: (p.x - size / 2.0).round() as i32,
        y: (p.y - size / 2.0).round() as i32,
        w: size.round() as i32,
        h: size.round() as i32,
    };
    let Some(path) = rounded_path(r, ui.f(1.5)) else {
        return;
    };
    pixmap.fill_path(
        &path,
        &paint([255, 255, 255, 255]),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
    let stroke = Stroke {
        width: ui.f(1.0),
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint(ACCENT), &stroke, Transform::identity(), None);
}

fn paint(color: [u8; 4]) -> tiny_skia::Paint<'static> {
    let mut p = tiny_skia::Paint::default();
    p.set_color_rgba8(color[0], color[1], color[2], color[3]);
    p
}
