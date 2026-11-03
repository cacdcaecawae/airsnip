// SPDX-License-Identifier: GPL-3.0-only
//! 命中测试与选中状态（EDT-3/EDT-7）：选区八向手柄、已有对象、曲线控制柄。
//! 控制柄位置即曲线控制点 P1/P2（直线箭头时落在 1/3、2/3 处）。

use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::editor::{Kind, Object, Point};

use super::geometry::{SelRect, expand_rect, union_rect};
use super::ui::Ui;

/// 曲线控制柄半径基准值（含描边，需 ≤ 对象包围盒的余量）。
const CTRL_RADIUS_BASE: f32 = 4.5;
const ACCENT: u8 = 26; // 主题蓝
const ACCENT_G: u8 = 115;
const ACCENT_B: u8 = 232;

/// 手柄命中半径基准值（物理像素，经 `Ui` 缩放）。
pub(super) const PICK_RADIUS_BASE: f32 = 6.0;

/// 按界面缩放后的手柄命中半径。
pub(super) fn pick_radius(ui: Ui) -> i32 {
    ui.px(PICK_RADIUS_BASE)
}

/// 选区手柄类型（八向 + 移动）；对象手柄见 `objedit::ObjHandle`。
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Move,
}

/// 八向手柄/移动命中（CAP-2）。
pub(super) fn hit_handle(sel: SelRect, x: i32, y: i32, ui: Ui) -> Option<Handle> {
    let pick = pick_radius(ui);
    let (x0, y0, x1, y1) = (sel.x, sel.y, sel.x + sel.w, sel.y + sel.h);
    let cx = x0 + sel.w / 2;
    let cy = y0 + sel.h / 2;
    for (hx, hy, h) in [
        (x0, y0, Handle::NW),
        (cx, y0, Handle::N),
        (x1, y0, Handle::NE),
        (x0, cy, Handle::W),
        (x1, cy, Handle::E),
        (x0, y1, Handle::SW),
        (cx, y1, Handle::S),
        (x1, y1, Handle::SE),
    ] {
        if (x - hx).abs() <= pick && (y - hy).abs() <= pick {
            return Some(h);
        }
    }
    None
}

/// 误触判定：长宽均不足 2px 的对象丢弃（EDT-1/EDT-2）。
pub(super) fn is_degenerate(obj: &Object) -> bool {
    match &obj.kind {
        Kind::Rect { a, b, .. } => (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0,
        Kind::Arrow { from, to, .. } => (from.x - to.x).abs() < 2.0 && (from.y - to.y).abs() < 2.0,
        // 区域类（高亮/模糊）：过小视为误触
        Kind::Highlight { a, b, .. } => (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0,
        Kind::Blur { a, b, .. } => (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0,
        Kind::Glow { a, b, .. } => (a.x - b.x).abs() < 2.0 && (a.y - b.y).abs() < 2.0,
        // 文本：空文本在提交时已拦截，进入文档的均为有效对象
        Kind::Text { .. } => false,
    }
}

impl super::Overlay {
    /// 已选对象。
    pub(super) fn selected_object(&self) -> Option<&Object> {
        self.doc.objects().get(self.selected?)
    }

    /// 切换选中对象；控制柄与辅助线随之变化，故重绘整个选区（只在点选时触发）。
    pub(super) fn set_selected(&mut self, new: Option<usize>) {
        if self.selected == new {
            return;
        }
        let had = self.selected.is_some();
        self.selected = new;
        // 选中文本对象时第二行才出现（EDT-4）
        self.refresh_toolbar();
        if (had || new.is_some())
            && let Some(sel) = self.selection
        {
            self.repaint(expand_rect(sel, self.dirty_pad()));
        }
    }

    /// 命中已有对象：从上层往下找（列表顺序即 z 序，EDT-7）。
    pub fn hit_object(&self, x: i32, y: i32) -> Option<usize> {
        let q = Point::new(x as f32, y as f32);
        let tol = pick_radius(self.ui) as f32;
        self.doc.objects().iter().rposition(|o| o.hit(q, tol))
    }

    /// 选中对象占据的像素区域。
    pub(super) fn selected_clip(&self) -> Option<SelRect> {
        let c = crate::render::bounds_to_clip(&self.selected_object()?.bounds());
        Some(SelRect {
            x: c.0,
            y: c.1,
            w: c.2 - c.0,
            h: c.3 - c.1,
        })
    }

    /// 鼠标滚轮（EDT-7/EDT-4）：悬停在线条/矩形边框 / 模糊区域 / 文本上时，
    /// 滚轮连续调整线宽、模糊半径或字号（Ctrl 3 倍步进）；无悬停对象时调整当前工具默认值。
    /// `sx`/`sy` 为屏幕坐标，需减虚拟屏幕原点转覆盖层坐标。
    pub fn on_mouse_wheel(&mut self, sx: i32, sy: i32, delta: i16, ctrl: bool) {
        let (x, y) = (sx - self.capture.origin_x, sy - self.capture.origin_y);
        let step = if ctrl { 3.0 } else { 1.0 } * if delta > 0 { 1.0 } else { -1.0 };
        // 文本输入中：滚轮调整输入字号（EDT-4）
        if let Some(edit) = self.text_edit.as_mut() {
            edit.font_size = (edit.font_size + step).clamp(10.0, 96.0);
            self.font_size = edit.font_size;
            self.repaint_text_edit();
            return;
        }
        if let Some(i) = self.hit_object(x, y) {
            self.adjust_object(i, step);
        } else {
            // 空白处滚动：只改默认值，供下一次绘制沿用
            match self.tool {
                crate::editor::Tool::Rect | crate::editor::Tool::Arrow => {
                    self.line_width = (self.line_width + step).clamp(1.0, 32.0);
                }
                crate::editor::Tool::Text => {
                    self.font_size = (self.font_size + step).clamp(10.0, 96.0);
                }
                crate::editor::Tool::Blur => self.blur_radius = (self.blur_radius + step).clamp(4.0, 64.0),
                crate::editor::Tool::Highlight | crate::editor::Tool::Glow => {}
            }
        }
    }

    /// 调整指定对象的线宽 / 模糊半径 / 字号（差分入撤销栈），并同步全局默认值。
    fn adjust_object(&mut self, i: usize, step: f32) {
        let before = self.selected_clip();
        self.doc.begin();
        let changed = {
            let obj = &mut self.doc.objects_mut()[i];
            let changed = match &mut obj.kind {
                Kind::Rect { .. } | Kind::Arrow { .. } => {
                    let w = (obj.style.width + step).clamp(1.0, 32.0);
                    let changed = (w - obj.style.width).abs() > 0.01;
                    if changed {
                        obj.style.width = w;
                        self.line_width = w;
                    }
                    changed
                }
                Kind::Blur { radius, .. } => {
                    let r = (*radius + step).clamp(4.0, 64.0);
                    let changed = (r - *radius).abs() > 0.01;
                    if changed {
                        *radius = r;
                        self.blur_radius = r;
                    }
                    changed
                }
                // 高亮/聚光：无可调属性
                Kind::Highlight { .. } | Kind::Glow { .. } => false,
                // 文本：字号连续可调，同步重建测量缓存（EDT-4）
                Kind::Text { font_size, .. } => {
                    let fs =
                        (*font_size + step).clamp(crate::editor::FONT_SIZE_RANGE.0, crate::editor::FONT_SIZE_RANGE.1);
                    let changed = (*font_size - fs).abs() > 0.01;
                    if changed {
                        *font_size = fs;
                        self.font_size = fs;
                    }
                    changed
                }
            };
            // 非文本对象与未变字号下 remeasure 无操作
            if changed {
                crate::render::text::remeasure(&mut obj.kind);
            }
            changed
        };
        self.doc.commit(changed);
        if !changed {
            return;
        }
        let after = self.selected_clip();
        let dirty = union_rect(before, after);
        if let Some(d) = dirty {
            self.repaint(expand_rect(d, 4));
        }
        if let Some(bar) = &self.bar {
            // 撤销可用性与字号显示可能变化
            self.repaint(expand_rect(bar.rect, 2));
        }
    }
}
/// 选中箭头的控制柄（EDT-3）：控制点处画圆点，与端点之间用虚线辅助线连接。
pub(super) fn draw_curve_handles(pixmap: &mut Pixmap, obj: &Object, ui: Ui) {
    let (Some(cps), Kind::Arrow { from, to, .. }) = (obj.kind.control_points(), &obj.kind) else {
        return;
    };
    let [c1, c2] = cps;
    dashed_line(pixmap, *from, c1, ui);
    dashed_line(pixmap, c2, *to, ui);
    for p in [c1, c2] {
        let mut pb = PathBuilder::new();
        pb.push_circle(p.x, p.y, ui.f(CTRL_RADIUS_BASE));
        let Some(path) = pb.finish() else {
            continue;
        };
        let mut fill = Paint::default();
        fill.set_color_rgba8(255, 255, 255, 255);
        pixmap.fill_path(&path, &fill, tiny_skia::FillRule::Winding, Transform::identity(), None);
        let mut edge = Paint::default();
        edge.set_color_rgba8(ACCENT, ACCENT_G, ACCENT_B, 255);
        let stroke = Stroke {
            width: ui.f(1.5),
            ..Default::default()
        };
        pixmap.stroke_path(&path, &edge, &stroke, Transform::identity(), None);
    }
}

/// 虚线辅助线（5px 实 / 4px 虚）：tiny-skia 无 dash，手动拆成小段。
pub(super) fn dashed_line(pixmap: &mut Pixmap, a: Point, b: Point, ui: Ui) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    // 实线段与周期按界面缩放，避免 4K 下虚线看起来是实线
    let (dash, period) = (ui.f(5.0), ui.f(9.0));
    let mut pb = PathBuilder::new();
    let mut t = 0.0;
    while t < len {
        let end = (t + dash).min(len);
        pb.move_to(a.x + ux * t, a.y + uy * t);
        pb.line_to(a.x + ux * end, a.y + uy * end);
        t += period;
    }
    let Some(path) = pb.finish() else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 200);
    let stroke = Stroke {
        width: ui.f(1.0),
        ..Default::default()
    };
    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}
