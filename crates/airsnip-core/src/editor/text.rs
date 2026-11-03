// SPDX-License-Identifier: GPL-3.0-only
//! 文本对象辅助（EDT-4）：文本包围盒与内边距。
//! 文本实际尺寸由渲染层测量（`crate::render::text::measure`）后缓存进 `Kind::Text::size`，
//! 本模块只做几何运算，不依赖系统字体 API。

use super::{Bounds, Point};

/// 文本内边距（渲染背景与包围盒共用，随字号缩放）。
pub fn pad(font_size: f32) -> f32 {
    font_size * 0.3
}

/// 文本包围盒：`pos` 为文字左上角（baseline 上方，见 `render::text`），
/// `size` 为测量出的文字尺寸（不含内边距）；外扩内边距 + 1px 容差。
pub fn bounds(pos: Point, size: (f32, f32), font_size: f32) -> Bounds {
    let p = pad(font_size) + 1.0;
    Bounds::around(pos.x, pos.y, pos.x + size.0, pos.y + size.1, p)
}

/// 文字矩形（不含内边距，命中与渲染共用）。
pub fn rect(pos: Point, size: (f32, f32)) -> (Point, Point) {
    (pos, Point::new(pos.x + size.0.max(1.0), pos.y + size.1.max(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_expand_with_font_size() {
        let small = bounds(Point::new(10.0, 20.0), (40.0, 20.0), 12.0);
        let large = bounds(Point::new(10.0, 20.0), (40.0, 20.0), 48.0);
        assert!(small.x0 < 10.0 && small.y0 < 20.0 && small.x1 > 50.0 && small.y1 > 40.0);
        assert!(large.x0 < small.x0, "字号越大留白越多");
    }
}
