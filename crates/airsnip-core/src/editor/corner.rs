// SPDX-License-Identifier: GPL-3.0-only
//! 圆角半径的读写与收敛（EDT-1/EDT-5/EDT-6/EDT-9）：矩形与三类区域对象共用同一套规则。
//!
//! 圆角不再由工具栏开关控制，而是选中对象后拖内侧角的**圆角控制柄**直接调
//! （见 `overlay::objedit`）。上限取短边一半 —— 与 `render` 里圆角路径的收敛规则一致，
//! 拖过头也不会把图形拖崩。
//!
//! 注意 `Kind::Blur` 的 `radius` 是**模糊强度**，圆角字段一律叫 `corner`，两者不可混用
//! （滚轮调强度、手柄调圆角，各改各的）。

use super::Kind;

impl Kind {
    /// 当前圆角半径（已夹到上限）；矩形/高亮/模糊/聚光以外的对象返回 None。
    ///
    /// 对象被缩小时存储值可能超出上限（`remap` 不缩放圆角，见 `transform`），
    /// 因此这里统一返回夹紧后的值，渲染与手柄位置都直接用，不会画出畸形。
    pub fn corner(&self) -> Option<f32> {
        let raw = match self {
            Kind::Rect { radius, .. } => *radius,
            Kind::Highlight { corner, .. } | Kind::Blur { corner, .. } | Kind::Glow { corner, .. } => *corner,
            _ => return None,
        };
        let limit = self.corner_limit().unwrap_or(0.0);
        Some(raw.clamp(0.0, limit))
    }

    /// 圆角半径上限：短边一半；无圆角概念的对象返回 None。
    pub fn corner_limit(&self) -> Option<f32> {
        let (a, b) = match self {
            Kind::Rect { a, b, .. }
            | Kind::Highlight { a, b, .. }
            | Kind::Blur { a, b, .. }
            | Kind::Glow { a, b, .. } => (a, b),
            _ => return None,
        };
        Some(((a.x - b.x).abs().min((a.y - b.y).abs())) * 0.5)
    }

    /// 设置圆角半径，夹到 `[0, 短边一半]`；返回是否真的发生了变化。
    pub fn set_corner(&mut self, r: f32) -> bool {
        let Some(limit) = self.corner_limit() else {
            return false;
        };
        let v = r.clamp(0.0, limit);
        let slot = match self {
            Kind::Rect { radius, .. } => radius,
            Kind::Highlight { corner, .. } | Kind::Blur { corner, .. } | Kind::Glow { corner, .. } => corner,
            _ => return false,
        };
        if (v - *slot).abs() < 0.01 {
            return false;
        }
        *slot = v;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Bounds, Point};

    fn rect(a: (f32, f32), b: (f32, f32), radius: f32) -> Kind {
        Kind::Rect {
            a: Point::new(a.0, a.1),
            b: Point::new(b.0, b.1),
            radius,
            filled: false,
        }
    }

    /// 四类共用同一套读写（矩形 → radius，其余 → corner）。
    fn area_kinds(a: (f32, f32), b: (f32, f32), corner: f32) -> Vec<(&'static str, Kind)> {
        let (pa, pb) = (Point::new(a.0, a.1), Point::new(b.0, b.1));
        vec![
            ("矩形", rect(a, b, corner)),
            ("高亮", Kind::Highlight { a: pa, b: pb, corner }),
            (
                "模糊",
                Kind::Blur {
                    a: pa,
                    b: pb,
                    radius: 16.0,
                    corner,
                },
            ),
            ("聚光", Kind::Glow { a: pa, b: pb, corner }),
        ]
    }

    /// 上限取短边一半（此处短边是高度 100）。
    #[test]
    fn limit_is_half_of_shorter_side() {
        for (name, k) in area_kinds((0.0, 0.0), (400.0, 100.0), 0.0) {
            assert_eq!(k.corner_limit(), Some(50.0), "{name}");
        }
        for (name, k) in area_kinds((0.0, 0.0), (60.0, 300.0), 0.0) {
            assert_eq!(k.corner_limit(), Some(30.0), "{name}");
        }
    }

    /// 设置时夹在上限内，且不接受负值。
    #[test]
    fn set_clamps_to_limit_and_zero() {
        for (name, mut k) in area_kinds((0.0, 0.0), (400.0, 100.0), 0.0) {
            assert!(k.set_corner(500.0), "{name}");
            assert_eq!(k.corner(), Some(50.0), "{name}：超出上限应夹到短边一半");
            assert!(k.set_corner(-20.0), "{name}");
            assert_eq!(k.corner(), Some(0.0), "{name}：负值应夹到 0");
        }
    }

    /// 值没变化时返回 false（拖动时避免空撤销步）。
    #[test]
    fn set_reports_no_change_when_identical() {
        for (name, mut k) in area_kinds((0.0, 0.0), (400.0, 100.0), 20.0) {
            assert!(!k.set_corner(20.0), "{name}");
            assert!(k.set_corner(21.0), "{name}");
        }
    }

    /// 存进去的超限值读回时收敛到当前上限（`remap` 不缩放圆角，对象缩小后靠这里收）。
    #[test]
    fn stored_value_converges_when_object_shrinks() {
        let mut k = Kind::Highlight {
            a: Point::new(0.0, 0.0),
            b: Point::new(400.0, 100.0),
            corner: 0.0,
        };
        assert!(k.set_corner(50.0));
        // 高度缩到 40 → 上限变成 20
        k.remap(
            Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 400.0,
                y1: 100.0,
            },
            Bounds {
                x0: 0.0,
                y0: 0.0,
                x1: 400.0,
                y1: 40.0,
            },
        );
        assert_eq!(k.corner(), Some(20.0), "超限的存储值应读回为当前上限");
    }

    /// 没有圆角概念的对象：读返回 None、写无效。
    #[test]
    fn non_area_kinds_have_no_corner() {
        let mut arrow = Kind::arrow(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        assert_eq!(arrow.corner(), None);
        assert_eq!(arrow.corner_limit(), None);
        assert!(!arrow.set_corner(5.0));
    }
}
