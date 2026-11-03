// SPDX-License-Identifier: GPL-3.0-only
//! 编辑对象模型（EDT-1/EDT-2/EDT-3/EDT-7，PRD 6.3.3）：
//! 底图保持不变，标注以矢量对象列表叠加；列表顺序即 z 序（越靠后越上层）。
//! 坐标统一为覆盖层坐标系下的物理像素。

mod corner;
mod curve;
mod document;
mod hit;
pub mod text;
mod transform;

pub use curve::{
    SEGMENTS as CURVE_SEGMENTS, dist_to_polyline, end_tangent, flatten, head_entry_t, point_at, polyline_len,
    split_left, t_at_arc_from_end,
};
pub use document::{Document, MAX_HISTORY};
pub use transform::{ArrowEnd, scale_factors, snap_direction, square_corner};

/// 坐标点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 对象的外接矩形（已含线宽/箭头头部的余量）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bounds {
    fn around(x0: f32, y0: f32, x1: f32, y1: f32, pad: f32) -> Self {
        Self {
            x0: x0 - pad,
            y0: y0 - pad,
            x1: x1 + pad,
            y1: y1 + pad,
        }
    }
}

/// 对象样式（EDT-7：颜色、线宽）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// 描边/填充色（RGBA，alpha 支持矩形透明度，EDT-1）。
    pub color: [u8; 4],
    /// 线宽（物理像素）。
    pub width: f32,
}

impl Style {
    pub fn new(color: [u8; 4], width: f32) -> Self {
        Self { color, width }
    }
}

/// 对象几何（EDT-1 矩形 / EDT-2 直线箭头 / EDT-3 曲线箭头 / EDT-4 文本 / EDT-5 模糊 / EDT-6 高亮）。
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// EDT-1 矩形：对角两点；`radius` 圆角半径；`filled` 是否填充（描边始终绘制）。
    Rect {
        a: Point,
        b: Point,
        radius: f32,
        filled: bool,
    },
    /// EDT-2/EDT-3 箭头：尾部 `from` → 尖端 `to`，三次贝塞尔控制点 `c1`/`c2`。
    Arrow {
        from: Point,
        c1: Point,
        c2: Point,
        to: Point,
    },
    /// EDT-4 文本：`pos` 为文字左上角；`text` 为内容（共享字符串，保持对象 Clone）；
    /// `font_size` 字号（物理像素）；`font_family` 字体族名；`bold`/`italic` 粗体与斜体；
    /// `size` 为测量缓存（文字宽高，不含内边距），由渲染层测量后写入。
    Text {
        pos: Point,
        text: std::sync::Arc<str>,
        font_size: f32,
        font_family: std::sync::Arc<str>,
        bold: bool,
        italic: bool,
        size: (f32, f32),
    },
    /// EDT-6 高亮：对角两点，荧光笔效果（半透明色块 + Multiply 混合，不遮挡文字）；
    /// `corner` 圆角半径（与矩形同一套手柄，0 = 直角）。
    Highlight { a: Point, b: Point, corner: f32 },
    /// EDT-5 高斯模糊：对角两点，`radius` 为模糊**强度**（由滚轮调），
    /// `corner` 为圆角半径（勿混用命名，见 `editor::corner`）。
    Blur {
        a: Point,
        b: Point,
        radius: f32,
        corner: f32,
    },
    /// 高亮（聚光效果，与荧光笔是两种功能）：对角两点 + `corner` 圆角半径。
    /// 选区其余部分暗化、该区域显示原图亮度，详见渲染层说明。
    Glow { a: Point, b: Point, corner: f32 },
}

impl Kind {
    /// 直线箭头：控制点取起点→终点的 1/3、2/3 处（EDT-3 控制柄初始位置）。
    pub fn arrow(from: Point, to: Point) -> Self {
        Kind::Arrow {
            from,
            c1: lerp(from, to, 1.0 / 3.0),
            c2: lerp(from, to, 2.0 / 3.0),
            to,
        }
    }

    /// 两个控制点（非箭头返回 None）。
    pub fn control_points(&self) -> Option<[Point; 2]> {
        match *self {
            Kind::Arrow { c1, c2, .. } => Some([c1, c2]),
            Kind::Rect { .. } | Kind::Text { .. } | Kind::Highlight { .. } | Kind::Blur { .. } | Kind::Glow { .. } => {
                None
            }
        }
    }

    /// 拖动控制柄（EDT-3）；`index` 为 0（尾部侧）或 1（头部侧）。
    pub fn set_control(&mut self, index: usize, p: Point) {
        if let Kind::Arrow { c1, c2, .. } = self {
            match index {
                0 => *c1 = p,
                _ => *c2 = p,
            }
        }
    }

    /// 双击控制柄恢复直线（EDT-3）；返回是否发生变化。
    pub fn reset_curve(&mut self) -> bool {
        let Kind::Arrow { from, to, .. } = self else {
            return false;
        };
        let straight = Kind::arrow(*from, *to);
        if &straight == self {
            return false;
        }
        *self = straight;
        true
    }

    /// 几何包围盒，含线宽一半与箭头头部的余量（供脏区计算使用）。
    pub fn bounds(&self, style: &Style) -> Bounds {
        let half = style.width * 0.5 + 1.0;
        match self {
            Kind::Rect { a, b, .. } => Bounds::around(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y), half),
            Kind::Arrow { from, c1, c2, to } => {
                // 曲线必落在控制点凸包内，故取凸包包围盒（精确上界）再补头部尺寸
                let pad = half.max(arrow_head_len(style.width) * 0.5);
                let (x0, y0, x1, y1) = curve::hull_bounds(*from, *c1, *c2, *to);
                Bounds::around(x0, y0, x1, y1, pad)
            }
            // 区域类（高亮/模糊/聚光）：整块都是绘制内容，外扩 1px 保证脏区取整
            Kind::Highlight { a, b, .. } | Kind::Blur { a, b, .. } | Kind::Glow { a, b, .. } => {
                Bounds::around(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y), 1.0)
            }
            // 文本：范围由测量缓存决定（EDT-4），padding 随字号缩放
            Kind::Text {
                pos, size, font_size, ..
            } => text::bounds(*pos, *size, *font_size),
        }
    }
}

fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// 箭头头部长度（EDT-2：随线宽缩放，并保证最小可见尺寸）。
pub fn arrow_head_len(width: f32) -> f32 {
    (width * 4.0).max(10.0)
}

/// 一个标注对象。
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub kind: Kind,
    pub style: Style,
}

impl Object {
    /// 一个标注对象是否命中点 `q`（容差含半个线宽，EDT-3a/EDT-7）。
    pub fn hit(&self, q: Point, tol: f32) -> bool {
        self.kind.hit(q, tol + self.style.width * 0.5)
    }

    pub fn bounds(&self) -> Bounds {
        self.kind.bounds(&self.style)
    }
}

/// 当前绘制工具（EDT-1/EDT-2/EDT-4/EDT-5/EDT-6/聚光）。
/// `#[repr(u8)]`：`settings::last_tool` 用 `as u8` 持久化/还原判别值，需保证布局稳定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Tool {
    Rect,
    Arrow,
    /// EDT-4 文本：点击放置，输入后提交。
    Text,
    Highlight,
    Blur,
    /// 聚光高亮：选区其余暗化、该区域恢复原图亮度。
    Glow,
}

impl Tool {
    /// 工具栏顺序。
    pub const ALL: [Tool; 6] = [
        Tool::Rect,
        Tool::Arrow,
        Tool::Text,
        Tool::Highlight,
        Tool::Blur,
        Tool::Glow,
    ];
}

/// 新建对象默认线宽（物理像素，连续可调：滚轮 / 二级工具栏）。
pub const DEFAULT_LINE_WIDTH: f32 = 4.0;

/// 模糊默认强度（EDT-5：半径，连续可调）。
pub const DEFAULT_BLUR_RADIUS: f32 = 16.0;

/// 文本默认字号（EDT-4：物理像素，滚轮连续可调）。
pub const DEFAULT_FONT_SIZE: f32 = 24.0;

/// 文本默认字体族（系统自带中文字体；字体面板可改，EDT-4）：Windows 微软雅黑 UI，macOS 苹方。
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_FONT_FAMILY: &str = "Microsoft YaHei UI";
#[cfg(target_os = "macos")]
pub const DEFAULT_FONT_FAMILY: &str = "PingFang SC";

/// 字号可调范围（滚轮 / 第二行 −+ 共用）。
pub const FONT_SIZE_RANGE: (f32, f32) = (10.0, 96.0);

/// 颜色调色板（RGBA）。透明度由工具栏后续提供，当前固定 255。
pub const PALETTE: [[u8; 4]; 5] = [
    [255, 255, 255, 255],
    [229, 57, 53, 255],
    [251, 140, 0, 255],
    [67, 160, 71, 255],
    [30, 136, 229, 255],
];

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(a: (f32, f32), b: (f32, f32)) -> Object {
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

    fn arrow(a: (f32, f32), b: (f32, f32)) -> Object {
        Object {
            kind: Kind::arrow(Point::new(a.0, a.1), Point::new(b.0, b.1)),
            style: Style::new([255, 0, 0, 255], 2.0),
        }
    }

    #[test]
    fn add_undo_redo_roundtrip() {
        let mut doc = Document::new();
        doc.begin();
        doc.push(rect((0.0, 0.0), (10.0, 10.0)));
        doc.commit(true);

        assert_eq!(doc.objects().len(), 1);
        assert!(doc.can_undo());

        assert!(doc.undo());
        assert!(doc.objects().is_empty());
        assert!(doc.can_redo());

        assert!(doc.redo());
        assert_eq!(doc.objects().len(), 1);
    }

    #[test]
    fn commit_false_keeps_history_clean() {
        let mut doc = Document::new();
        doc.begin();
        doc.push(rect((0.0, 0.0), (1.0, 1.0)));
        doc.commit(false); // 误触，丢弃

        assert!(!doc.can_undo());
        assert_eq!(doc.objects().len(), 1);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut doc = Document::new();
        doc.begin();
        doc.push(rect((0.0, 0.0), (5.0, 5.0)));
        doc.commit(true);
        doc.undo();
        assert!(doc.can_redo());

        doc.begin();
        doc.push(arrow((0.0, 0.0), (9.0, 9.0)));
        doc.commit(true);
        assert!(!doc.can_redo());

        assert!(doc.undo());
        assert!(doc.objects().is_empty());
        assert!(doc.redo());
        assert!(matches!(&doc.objects()[0].kind, Kind::Arrow { .. }));
    }

    #[test]
    fn undoing_past_empty_is_noop() {
        let mut doc = Document::new();
        assert!(!doc.undo());
        assert!(!doc.redo());
    }

    #[test]
    fn history_is_bounded() {
        let mut doc = Document::new();
        for i in 0..(MAX_HISTORY + 20) {
            doc.begin();
            doc.push(rect((i as f32, 0.0), (i as f32 + 1.0, 1.0)));
            doc.commit(true);
        }
        let mut pops = 0;
        while doc.undo() {
            pops += 1;
        }
        assert!(pops <= MAX_HISTORY);
    }

    #[test]
    fn bounds_include_stroke_and_arrow_head() {
        let thin = rect((10.0, 20.0), (30.0, 40.0));
        let b = thin.bounds();
        assert!(b.x0 < 10.0 && b.y0 < 20.0 && b.x1 > 30.0 && b.y1 > 40.0);

        // 水平箭头：头部沿 x 方向外扩
        let a = arrow((0.0, 0.0), (100.0, 0.0));
        let ab = a.bounds();
        assert!(ab.x1 > 100.0, "箭头头部应超出尖端：{ab:?}");
        assert!(ab.x0 <= 0.0);
    }

    #[test]
    fn new_arrow_is_straight_with_third_point_handles() {
        let a = arrow((0.0, 0.0), (90.0, 0.0));
        let [c1, c2] = a.kind.control_points().unwrap();
        assert!(
            (c1.x - 30.0).abs() < 1e-3 && (c2.x - 60.0).abs() < 1e-3,
            "控制柄应在 1/3、2/3 处"
        );
        // 直线：中点在两点连线上
        let mid = point_at(Point::new(0.0, 0.0), c1, c2, Point::new(90.0, 0.0), 0.5);
        assert!((mid.x - 45.0).abs() < 0.01 && mid.y.abs() < 0.01);
    }

    #[test]
    fn control_drag_bends_then_reset_restores_straight() {
        let mut obj = arrow((0.0, 0.0), (90.0, 0.0));
        obj.kind.set_control(0, Point::new(30.0, 60.0));
        let [c1, _] = obj.kind.control_points().unwrap();
        assert_eq!(c1, Point::new(30.0, 60.0));
        // 弯曲后包围盒应向上扩张
        assert!(obj.bounds().y1 > 40.0);

        obj.kind.reset_curve();
        let [c1, c2] = obj.kind.control_points().unwrap();
        assert!((c1.x - 30.0).abs() < 1e-3 && c1.y.abs() < 1e-3);
        assert!((c2.x - 60.0).abs() < 1e-3 && c2.y.abs() < 1e-3);
    }

    #[test]
    fn hit_test_rect_and_arrow() {
        let r = rect((10.0, 10.0), (60.0, 60.0));
        assert!(r.hit(Point::new(10.0, 30.0), 3.0), "左边框应命中");
        assert!(!r.hit(Point::new(35.0, 35.0), 3.0), "未填充矩形内部不命中");

        let filled = Object {
            kind: Kind::Rect {
                a: Point::new(10.0, 10.0),
                b: Point::new(60.0, 60.0),
                radius: 0.0,
                filled: true,
            },
            style: Style::new([0, 0, 0, 255], 2.0),
        };
        assert!(filled.hit(Point::new(35.0, 35.0), 3.0), "填充矩形内部应命中");

        // 弯曲箭头：曲线经过 (45, 45) 附近，而非直线中点
        let mut a = arrow((0.0, 0.0), (90.0, 0.0));
        a.kind.set_control(0, Point::new(0.0, 60.0));
        a.kind.set_control(1, Point::new(90.0, 60.0));
        let on_curve = point_at(
            Point::new(0.0, 0.0),
            Point::new(0.0, 60.0),
            Point::new(90.0, 60.0),
            Point::new(90.0, 0.0),
            0.5,
        );
        assert!(a.hit(on_curve, 3.0), "曲线上的点应命中：{on_curve:?}");
        assert!(!a.hit(Point::new(45.0, 0.0), 3.0), "直线位置不应命中弯曲箭头");
    }

    #[test]
    fn text_object_hit_and_bounds_with_measure_cache() {
        let mut obj = Object {
            kind: Kind::Text {
                pos: Point::new(100.0, 50.0),
                text: std::sync::Arc::from("测试"),
                font_size: 24.0,
                font_family: std::sync::Arc::from(DEFAULT_FONT_FAMILY),
                bold: false,
                italic: false,
                size: (48.0, 33.0), // 渲染层测量后的缓存
            },
            style: Style::new([255, 255, 255, 255], 2.0),
        };
        assert!(obj.hit(Point::new(110.0, 60.0), 3.0), "文字矩形内应命中");
        assert!(!obj.hit(Point::new(200.0, 90.0), 3.0), "文字矩形外不命中");

        let b = obj.bounds();
        assert!(
            b.x0 < 100.0 && b.y0 < 50.0 && b.x1 > 148.0 && b.y1 > 83.0,
            "包围盒应含文字范围与内边距：{b:?}"
        );
        assert!(obj.kind.control_points().is_none(), "文本无控制柄");

        // 对象保持 Copy 语义（Arc 引用计数，快照克隆不拷贝字符串内容）
        let clone = obj.clone();
        assert_eq!(obj, clone, "文本对象应可 Copy");

        // 编辑文本后尺寸缓存由调用方重测
        obj.kind = Kind::Text {
            pos: Point::new(100.0, 50.0),
            text: std::sync::Arc::from("测试"),
            font_size: 32.0,
            font_family: std::sync::Arc::from(DEFAULT_FONT_FAMILY),
            bold: true,
            italic: false,
            size: (64.0, 44.0),
        };
        let b2 = obj.bounds();
        assert!(b2.x1 > b.x1, "字号变大后包围盒应变大");
        assert!(obj.hit(Point::new(120.0, 70.0), 3.0));
    }

    #[test]
    fn tool_all_includes_text() {
        assert!(Tool::ALL.contains(&Tool::Text));
        assert!(Tool::ALL.contains(&Tool::Glow));
        assert_eq!(Tool::ALL.len(), 6);
    }

    #[test]
    fn area_objects_hit_and_bounds() {
        let hl = Object {
            kind: Kind::Highlight {
                a: Point::new(10.0, 10.0),
                b: Point::new(50.0, 40.0),
                corner: 0.0,
            },
            style: Style::new([255, 240, 0, 255], 2.0),
        };
        assert!(hl.hit(Point::new(30.0, 25.0), 3.0), "高亮内部应命中");
        assert!(!hl.hit(Point::new(60.0, 25.0), 3.0), "高亮外部不应命中");
        let b = hl.bounds();
        assert!(b.x0 <= 10.0 && b.x1 >= 50.0 && b.y0 <= 10.0 && b.y1 >= 40.0);

        let blur = Object {
            kind: Kind::Blur {
                a: Point::new(1.0, 2.0),
                b: Point::new(11.0, 22.0),
                radius: 16.0,
                corner: 0.0,
            },
            style: Style::new([0, 0, 0, 255], 2.0),
        };
        assert!(blur.hit(Point::new(5.0, 10.0), 3.0), "模糊内部应命中");
        assert!(!blur.hit(Point::new(30.0, 30.0), 3.0), "模糊外部不应命中");
        assert!(blur.kind.control_points().is_none(), "区域类无控制柄");
        assert!(Tool::ALL.contains(&Tool::Highlight) && Tool::ALL.contains(&Tool::Blur));
    }
}
