// SPDX-License-Identifier: GPL-3.0-only
//! 命中测试（EDT-3a/EDT-7）：把对象几何转换为「点到图形距离 ≤ 容差」的判定。
//! 曲线箭头复用 `curve` 的展平折线，与渲染使用同一参数，保证所见即所点。

use super::{Kind, Point, curve};

impl Kind {
    /// 点 `q` 是否命中对象（`tol` 为额外视觉容差，不含线宽）。
    pub fn hit(&self, q: Point, tol: f32) -> bool {
        match self {
            Kind::Rect { a, b, filled, .. } => hit_rect(*a, *b, *filled, q, tol),
            Kind::Arrow { from, c1, c2, to } => {
                curve::dist_to_polyline(&curve::flatten(*from, *c1, *c2, *to), q) <= tol
            }
            // 区域类（高亮/模糊/聚光）：整块区域都可命中（同填充矩形），便于选中拖动
            Kind::Highlight { a, b, .. } => hit_rect(*a, *b, true, q, tol),
            Kind::Blur { a, b, .. } => hit_rect(*a, *b, true, q, tol),
            Kind::Glow { a, b, .. } => hit_rect(*a, *b, true, q, tol),
            // 文本：整个文字矩形都可命中（同填充矩形），尺寸来自测量缓存
            Kind::Text { pos, size, .. } => {
                let (a, b) = super::text::rect(*pos, *size);
                hit_rect(a, b, true, q, tol)
            }
        }
    }
}

/// 矩形：填充时内部也算命中，否则只判描边。
fn hit_rect(a: Point, b: Point, filled: bool, q: Point, tol: f32) -> bool {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
    if filled && q.x >= x0 && q.x <= x1 && q.y >= y0 && q.y <= y1 {
        return true;
    }
    let corners = [
        Point::new(x0, y0),
        Point::new(x1, y0),
        Point::new(x1, y1),
        Point::new(x0, y1),
        Point::new(x0, y0),
    ];
    curve::dist_to_polyline(&corners, q) <= tol
}
