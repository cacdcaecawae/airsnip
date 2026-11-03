// SPDX-License-Identifier: GPL-3.0-only
//! 对象几何变换与绘制约束（EDT-7 / EDT-1 / EDT-2）：
//! 平移、按包围盒映射缩放（八向手柄），以及 Shift 绘制约束（正方形 / 方向吸附）。
//! 纯几何运算，不依赖渲染层；文本的测量缓存由调用方用渲染层结果刷新。

use super::{Bounds, Kind, Point};

/// 缩放后允许的文本字号范围（滚轮调节范围为 10–96，缩放另留余量）。
const FONT_RANGE: (f32, f32) = (8.0, 160.0);

/// 箭头端点（EDT-7 二次编辑：拖端点拉长 / 改向）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ArrowEnd {
    From,
    To,
}

impl Kind {
    /// 平移全部几何点（EDT-7 对象移动）。
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Kind::Rect { a, b, .. }
            | Kind::Highlight { a, b, .. }
            | Kind::Blur { a, b, .. }
            | Kind::Glow { a, b, .. } => {
                *a = Point::new(a.x + dx, a.y + dy);
                *b = Point::new(b.x + dx, b.y + dy);
            }
            Kind::Arrow { from, c1, c2, to } => {
                for p in [from, c1, c2, to] {
                    *p = Point::new(p.x + dx, p.y + dy);
                }
            }
            Kind::Text { pos, .. } => *pos = Point::new(pos.x + dx, pos.y + dy),
        }
    }

    /// 拖箭头端点（EDT-2/EDT-3 二次编辑）：端点移到 `p`，控制点按「以另一端为不动点的
    /// 相似变换」（旋转 + 等比缩放）跟随。直线箭头保持直线（控制点仍在 1/3、2/3 处），
    /// 弯曲箭头随主轴旋转缩放，不会出现剪切变形。
    pub fn move_arrow_end(&mut self, end: ArrowEnd, p: Point) {
        let Kind::Arrow { from, c1, c2, to } = self else {
            return;
        };
        let (old_from, old_to) = (*from, *to);
        let (new_from, new_to) = match end {
            ArrowEnd::From => (p, old_to),
            ArrowEnd::To => (old_from, p),
        };
        // 主轴向量：新 / 旧（复数比）给出旋转 + 等比缩放
        let (ax, ay) = (old_to.x - old_from.x, old_to.y - old_from.y);
        let (bx, by) = (new_to.x - new_from.x, new_to.y - new_from.y);
        let la2 = ax * ax + ay * ay;
        let anchor = match end {
            ArrowEnd::From => old_to,
            ArrowEnd::To => old_from,
        };
        if la2 < 1e-6 {
            // 原箭头退化（两端重合）：无旋转可定义，控制点随端点平移
            let (dx, dy) = (new_from.x - old_from.x, new_from.y - old_from.y);
            *c1 = Point::new(c1.x + dx, c1.y + dy);
            *c2 = Point::new(c2.x + dx, c2.y + dy);
        } else {
            let re = (bx * ax + by * ay) / la2;
            let im = (by * ax - bx * ay) / la2;
            let xform = |q: Point| {
                let (dx, dy) = (q.x - anchor.x, q.y - anchor.y);
                Point::new(anchor.x + re * dx - im * dy, anchor.y + im * dx + re * dy)
            };
            *c1 = xform(*c1);
            *c2 = xform(*c2);
        }
        *from = new_from;
        *to = new_to;
    }

    /// 按包围盒映射缩放（EDT-7 对象八向缩放）：把 `from` → `to` 的仿射映射施加到几何点。
    /// 两点式（矩形/高亮/模糊/聚光）两个对角点各自映射，两点顺序自然保留；
    /// 箭头四个点（含控制点）整体映射，曲线形状随包围盒同比例伸缩；
    /// 文本位置映射并按纵向比例缩放字号，`size` 缓存由调用方重测。
    pub fn remap(&mut self, from: Bounds, to: Bounds) {
        let (sx, sy) = scale_factors(from, to);
        let map = |p: Point| {
            Point::new(
                axis(p.x, from.x0, from.x1, to.x0, to.x1),
                axis(p.y, from.y0, from.y1, to.y0, to.y1),
            )
        };
        match self {
            Kind::Rect { a, b, .. }
            | Kind::Highlight { a, b, .. }
            | Kind::Blur { a, b, .. }
            | Kind::Glow { a, b, .. } => {
                *a = map(*a);
                *b = map(*b);
            }
            Kind::Arrow { from, c1, c2, to } => {
                *from = map(*from);
                *c1 = map(*c1);
                *c2 = map(*c2);
                *to = map(*to);
            }
            Kind::Text {
                pos, font_size, size, ..
            } => {
                *pos = map(*pos);
                // 字号只跟纵向比例走（文字不做横向拉伸）
                *font_size = (*font_size * sy).clamp(FONT_RANGE.0, FONT_RANGE.1);
                *size = ((size.0 * sx).max(1.0), (size.1 * sy).max(1.0));
            }
        }
    }
}

/// 包围盒缩放的横纵比例（跨度为零时按 1 处理）。
pub fn scale_factors(from: Bounds, to: Bounds) -> (f32, f32) {
    let ratio = |a: f32, b: f32, c: f32, d: f32| {
        let span = c - a;
        if span.abs() < 1e-6 { 1.0 } else { (d - b) / span }
    };
    (
        ratio(from.x0, to.x0, from.x1, to.x1),
        ratio(from.y0, to.y0, from.y1, to.y1),
    )
}

/// 把 `v` 从 `[f0, f1]` 线性映射到 `[t0, t1]`（区间退化时落到 `t0`）。
fn axis(v: f32, f0: f32, f1: f32, t0: f32, t1: f32) -> f32 {
    let span = f1 - f0;
    let t = if span.abs() < 1e-6 { 0.0 } else { (v - f0) / span };
    t0 + t * (t1 - t0)
}

/// 符号：把 0 归为 +1，保证「零位移方向」仍给出确定的正方形/吸附结果。
fn sign(v: f32) -> f32 {
    if v < 0.0 { -1.0 } else { 1.0 }
}

/// EDT-1：Shift 约束的矩形对角点——边长取两轴位移的较大者，方向沿用鼠标所在象限。
pub fn square_corner(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let side = dx.abs().max(dy.abs());
    Point::new(a.x + side * sign(dx), a.y + side * sign(dy))
}

/// EDT-2：Shift 约束的箭头终点——吸附到水平 / 垂直 / 45° 三个方向中最近的一个。
/// 判据按两轴位移的比值：明显偏轴则吸到轴，否则吸到 45°（取两轴位移均值作为投影长度）。
pub fn snap_direction(a: Point, b: Point) -> Point {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (ax, ay) = (dx.abs(), dy.abs());
    if ax >= ay * 2.0 {
        Point::new(b.x, a.y)
    } else if ay >= ax * 2.0 {
        Point::new(a.x, b.y)
    } else {
        let t = (ax + ay) * 0.5;
        Point::new(a.x + t * sign(dx), a.y + t * sign(dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Object, Style};

    fn rect_obj(a: (f32, f32), b: (f32, f32)) -> Object {
        Object {
            kind: Kind::Rect {
                a: Point::new(a.0, a.1),
                b: Point::new(b.0, b.1),
                radius: 0.0,
                filled: false,
            },
            style: Style::new([255, 0, 0, 255], 2.0),
        }
    }

    #[test]
    fn translate_moves_every_kind() {
        let mut rect = rect_obj((10.0, 10.0), (40.0, 30.0)).kind;
        rect.translate(5.0, -3.0);
        assert_eq!(
            rect,
            Kind::Rect {
                a: Point::new(15.0, 7.0),
                b: Point::new(45.0, 27.0),
                radius: 0.0,
                filled: false
            }
        );

        let mut arrow = Kind::arrow(Point::new(0.0, 0.0), Point::new(30.0, 0.0));
        arrow.translate(10.0, 10.0);
        let Kind::Arrow { from, c1, c2, to } = arrow else {
            panic!("应仍是箭头");
        };
        assert_eq!(from, Point::new(10.0, 10.0));
        assert_eq!(to, Point::new(40.0, 10.0));
        assert_eq!(c1, Point::new(20.0, 10.0), "控制点应一起平移");
        assert_eq!(c2, Point::new(30.0, 10.0));

        let mut glow = Kind::Glow {
            a: Point::new(1.0, 2.0),
            b: Point::new(3.0, 4.0),
            corner: 0.0,
        };
        glow.translate(-1.0, -2.0);
        assert_eq!(
            glow,
            Kind::Glow {
                a: Point::new(0.0, 0.0),
                b: Point::new(2.0, 2.0),
                corner: 0.0,
            }
        );
    }

    #[test]
    fn remap_scales_rect_by_bounds_ratio() {
        let mut obj = rect_obj((10.0, 20.0), (30.0, 40.0));
        let from = obj.bounds();
        // 左上角固定，宽高都放大一倍
        let to = Bounds {
            x0: from.x0,
            y0: from.y0,
            x1: from.x0 + (from.x1 - from.x0) * 2.0,
            y1: from.y0 + (from.y1 - from.y0) * 2.0,
        };
        obj.kind.remap(from, to);
        let b = obj.bounds();
        // 几何随包围盒一同放大：新包围盒 ≈ to（容差覆盖线宽余量本身不缩放带来的固定偏移）
        assert!((b.x0 - to.x0).abs() < 3.0 && (b.y0 - to.y0).abs() < 3.0, "实际 {b:?}");
        assert!((b.x1 - to.x1).abs() < 3.0 && (b.y1 - to.y1).abs() < 3.0, "实际 {b:?}");
        assert!((b.x1 - b.x0) > (from.x1 - from.x0) * 1.8, "宽度应接近翻倍：{b:?}");
    }

    #[test]
    fn remap_keeps_two_point_order_when_flipped() {
        // 把手柄拖过对角点：a/b 顺序不变（几何仍相同），包围盒翻转
        let mut obj = rect_obj((0.0, 0.0), (20.0, 20.0));
        obj.kind.remap(
            obj.bounds(),
            Bounds {
                x0: 10.0,
                y0: 10.0,
                x1: -10.0,
                y1: 30.0,
            },
        );
        let b = obj.bounds();
        assert!((b.x0 - -10.0).abs() < 3.0 && (b.x1 - 10.0).abs() < 3.0, "{b:?}");
    }

    #[test]
    fn remap_moves_arrow_control_points_and_text_font() {
        let mut arrow = Kind::arrow(Point::new(0.0, 0.0), Point::new(100.0, 0.0));
        let from = crate::editor::Bounds::around(0.0, -5.0, 100.0, 5.0, 0.0);
        let to = crate::editor::Bounds::around(0.0, -10.0, 100.0, 10.0, 0.0);
        arrow.remap(from, to);
        let Kind::Arrow { c1, c2, .. } = arrow else {
            panic!("应仍是箭头");
        };
        assert!((c1.x - 100.0 / 3.0).abs() < 0.01 && (c2.x - 200.0 / 3.0).abs() < 0.01);

        let mut text = Kind::Text {
            pos: Point::new(10.0, 10.0),
            text: std::sync::Arc::from("ab"),
            font_size: 20.0,
            font_family: std::sync::Arc::from(crate::editor::DEFAULT_FONT_FAMILY),
            bold: false,
            italic: false,
            size: (40.0, 28.0),
        };
        let from2 = crate::editor::Bounds::around(0.0, 0.0, 50.0, 40.0, 0.0);
        let to2 = crate::editor::Bounds::around(0.0, 0.0, 100.0, 80.0, 0.0);
        text.remap(from2, to2);
        let Kind::Text {
            font_size, size, pos, ..
        } = text
        else {
            panic!("应仍是文本");
        };
        assert!((font_size - 40.0).abs() < 0.01, "字号应随纵向比例翻倍：{font_size}");
        assert!(size.1 > 28.0 && pos.x > 10.0);
    }

    #[test]
    fn dragging_arrow_end_keeps_straight_arrow_straight() {
        let mut arrow = Kind::arrow(Point::new(0.0, 0.0), Point::new(100.0, 0.0));
        // 拖终点到 (100,100)：主轴变长并旋转 45°
        arrow.move_arrow_end(ArrowEnd::To, Point::new(100.0, 100.0));
        let Kind::Arrow { from, c1, c2, to } = arrow else {
            panic!("应仍是箭头");
        };
        assert_eq!(from, Point::new(0.0, 0.0), "起点不动");
        assert_eq!(to, Point::new(100.0, 100.0));
        // 控制点仍在 1/3、2/3 处（直线箭头拉伸后仍是直线）
        assert!(
            (c1.x - 100.0 / 3.0).abs() < 0.01 && (c1.y - 100.0 / 3.0).abs() < 0.01,
            "{c1:?}"
        );
        assert!(
            (c2.x - 200.0 / 3.0).abs() < 0.01 && (c2.y - 200.0 / 3.0).abs() < 0.01,
            "{c2:?}"
        );
    }

    #[test]
    fn dragging_from_end_keeps_tip_and_scales_curve() {
        let mut arrow = Kind::arrow(Point::new(0.0, 0.0), Point::new(100.0, 0.0));
        arrow.set_control(0, Point::new(30.0, 60.0)); // 先弯一下
        // 拖起点到 (50,0)：主轴长度减半，控制点以尖端为不动点同比缩放
        arrow.move_arrow_end(ArrowEnd::From, Point::new(50.0, 0.0));
        let Kind::Arrow { to, .. } = arrow else {
            panic!("应仍是箭头");
        };
        assert_eq!(to, Point::new(100.0, 0.0), "尖端不动");
        let [c1, _] = arrow.control_points().unwrap();
        // anchor = 尖端 (100,0)，缩放 0.5：(30,60) → (65,30)
        assert!((c1.x - 65.0).abs() < 0.01 && (c1.y - 30.0).abs() < 0.01, "{c1:?}");
    }

    #[test]
    fn dragging_end_of_degenerate_arrow_only_translates() {
        // 两端重合（刚按下未拖动）：无旋转可定义，但不能 panic
        let mut arrow = Kind::arrow(Point::new(10.0, 10.0), Point::new(10.0, 10.0));
        arrow.move_arrow_end(ArrowEnd::To, Point::new(40.0, 10.0));
        let Kind::Arrow { from, to, .. } = arrow else {
            panic!("应仍是箭头");
        };
        assert_eq!(from, Point::new(10.0, 10.0));
        assert_eq!(to, Point::new(40.0, 10.0));
    }

    #[test]
    fn shift_square_uses_longer_side() {
        let a = Point::new(10.0, 10.0);
        // 水平方向更长 → 边长取水平
        let p = square_corner(a, Point::new(50.0, 20.0));
        assert_eq!(p, Point::new(50.0, 50.0));
        // 左上方向拖动（两轴位移各 20，方向均为负/正）
        let p = square_corner(a, Point::new(-10.0, 30.0));
        assert_eq!(p, Point::new(-10.0, 30.0));
    }

    #[test]
    fn shift_snap_picks_nearest_axis_or_diagonal() {
        let a = Point::new(0.0, 0.0);
        // 近水平 → 吸附为水平
        assert_eq!(snap_direction(a, Point::new(100.0, 8.0)), Point::new(100.0, 0.0));
        // 近垂直 → 吸附为垂直
        assert_eq!(snap_direction(a, Point::new(5.0, 90.0)), Point::new(0.0, 90.0));
        // 中间 → 吸附为 45°
        let p = snap_direction(a, Point::new(60.0, 40.0));
        assert_eq!(p, Point::new(50.0, 50.0));
        // 45° 反向
        let p = snap_direction(a, Point::new(-30.0, 30.0));
        assert_eq!(p, Point::new(-30.0, 30.0));
    }
}
