// SPDX-License-Identifier: GPL-3.0-only
//! 对象拖动、缩放、删除与层级调整（EDT-7）：鼠标位置 → 目标包围盒的换算、
//! 撤销栈边界（按下 `begin` / 松手 `commit`，一次拖拽 = 一步）与脏区重绘。

use crate::editor::{ArrowEnd, Bounds, Kind, Point};

use super::super::geometry::{SelRect, expand_rect, union_rect};
use super::{ObjDrag, ObjHandle, lock_axis, radius_delta, resized, resized_aspect};

impl super::super::Overlay {
    /// 按下选中对象的控制柄（或对象本体）：进入拖动状态并建立撤销快照。
    pub(in crate::overlay) fn begin_obj_drag(&mut self, index: usize, handle: ObjHandle, x: i32, y: i32) {
        let Some(obj) = self.doc.objects().get(index).cloned() else {
            return;
        };
        self.set_selected(Some(index));
        self.doc.begin();
        self.obj_drag = Some(ObjDrag {
            index,
            handle,
            frame: obj.bounds(),
            orig: obj,
            px: x,
            py: y,
            moved: false,
        });
    }

    /// 拖动中：把鼠标位置换算成新几何，再从按下时的对象重算（避免逐帧累加误差）。
    /// `shift` 为真时施加约束（与新建一致）：箭头端点→水平/垂直/45° 吸附、移动→锁定轴向、
    /// 角手柄缩放→等比。
    pub(in crate::overlay) fn drag_obj(&mut self, x: i32, y: i32, shift: bool) {
        let Some(d) = self.obj_drag.as_ref() else {
            return;
        };
        let (index, handle, frame, orig, px, py, moved) =
            (d.index, d.handle, d.frame, d.orig.clone(), d.px, d.py, d.moved);
        let Some(sel) = self.selection else {
            return;
        };
        let before = self.selected_clip();
        {
            let obj = &mut self.doc.objects_mut()[index];
            match handle {
                // 箭头端点（EDT-2）：端点跟随鼠标（限制在选区内），控制点由相似变换跟随
                ObjHandle::ArrowFrom | ObjHandle::ArrowTo => {
                    let end = if handle == ObjHandle::ArrowFrom {
                        ArrowEnd::From
                    } else {
                        ArrowEnd::To
                    };
                    let mut target = Point::new(x as f32, y as f32);
                    // Shift：吸附为水平 / 垂直 / 45°（与新建箭头一致，EDT-2）
                    if shift && let Kind::Arrow { from, to, .. } = &obj.kind {
                        let anchor = if handle == ObjHandle::ArrowFrom { *to } else { *from };
                        target = crate::editor::snap_direction(anchor, target);
                    }
                    obj.kind.move_arrow_end(end, clamp_point(sel, target));
                }
                // 整体移动：整框跟随鼠标增量，并限制在选区内（标注不出选区，CAP-2 约定）
                ObjHandle::Move => {
                    let (dx, dy) = (x - px, y - py);
                    // Shift：锁定到位移较大的轴
                    let (dx, dy) = if shift { lock_axis(dx, dy) } else { (dx, dy) };
                    obj.kind = orig.kind.clone();
                    obj.kind.remap(frame, shifted_frame(frame, sel, dx, dy));
                }
                // 圆角控制柄（EDT-1）：朝矩形中心拖变大、朝角拖变小（控制柄代表圆弧本身）。
                // 每帧从按下时的半径重算，避免逐帧增量累积误差。
                ObjHandle::Radius => {
                    let base = orig.kind.corner().unwrap_or(0.0);
                    obj.kind = orig.kind.clone();
                    obj.kind.set_corner(base + radius_delta(x - px, y - py));
                }
                // 八向缩放（Shift + 角手柄 = 等比）
                _ => {
                    let target = if shift {
                        resized_aspect(frame, handle, x, y)
                    } else {
                        resized(frame, handle, x, y)
                    };
                    obj.kind = orig.kind.clone();
                    obj.kind.remap(frame, target);
                    // 文本：几何缩放只给出字号比例，尺寸缓存用渲染层重新测量（EDT-4）
                    crate::render::text::remeasure(&mut obj.kind);
                }
            }
        }
        let changed = self.doc.objects().get(index) != Some(&orig);
        if let Some(d) = self.obj_drag.as_mut() {
            d.moved = moved || changed;
        }
        self.repaint_selected(before);
    }
    /// 松开：有变化才进撤销栈（单击选中不留历史）。
    pub(in crate::overlay) fn end_obj_drag(&mut self) {
        let Some(d) = self.obj_drag.take() else {
            return;
        };
        self.doc.commit(d.moved);
        if d.moved {
            self.repaint_toolbar();
        }
    }

    /// 方向键微调选中对象（EDT-7）：`dx`/`dy` 为像素增量（1px，Shift 5px）。
    /// 与拖动一致地限制在选区内；每次按键 = 一步撤销。
    pub(in crate::overlay) fn nudge_object(&mut self, dx: i32, dy: i32) {
        let Some(i) = self.selected else {
            return;
        };
        let Some(sel) = self.selection else {
            return;
        };
        let before = self.selected_clip();
        self.doc.begin();
        let changed = {
            let obj = &mut self.doc.objects_mut()[i];
            let frame = obj.bounds();
            let target = shifted_frame(frame, sel, dx, dy);
            let (mx, my) = (target.x0 - frame.x0, target.y0 - frame.y0);
            let moved = mx.abs() > 0.01 || my.abs() > 0.01;
            if moved {
                obj.kind.translate(mx, my);
            }
            moved
        };
        self.doc.commit(changed);
        if changed {
            self.repaint_selected(before);
        }
    }

    /// 工具栏样式（颜色 / 填充）实时作用于选中对象（EDT-7）。
    /// 线宽 / 字号 / 模糊半径由滚轮直接调整（见 `pick::on_mouse_wheel`）；
    /// 圆角由对象内侧角的控制柄拖动调整（EDT-1，见 `objedit`）。
    pub(in crate::overlay) fn style_selected(&mut self) {
        let Some(i) = self.selected else {
            return;
        };
        let color = crate::editor::PALETTE[self.color_index];
        let filled = self.filled;
        let before = self.selected_clip();
        self.doc.begin();
        let changed = {
            let obj = &mut self.doc.objects_mut()[i];
            let mut changed = false;
            // 模糊（效果）与聚光（亮度还原）不画描边，颜色对其无意义
            if !matches!(obj.kind, Kind::Blur { .. } | Kind::Glow { .. }) && obj.style.color[..3] != color[..3] {
                // 保留原有 alpha（透明度不随颜色选择变化，EDT-1）
                obj.style.color = [color[0], color[1], color[2], obj.style.color[3]];
                changed = true;
            }
            if let Kind::Rect { filled: f, .. } = &mut obj.kind {
                if *f != filled {
                    *f = filled;
                    changed = true;
                }
            }
            changed
        };
        self.doc.commit(changed);
        if changed {
            self.repaint_selected(before);
            self.repaint_toolbar();
        }
    }

    /// 选中对象变化后的重绘：旧区 + 新区（手柄在包围盒外扩范围内）。
    fn repaint_selected(&mut self, before: Option<SelRect>) {
        if let Some(d) = union_rect(before, self.selected_clip()) {
            self.repaint(expand_rect(d, 10));
        }
    }

    /// 重绘工具栏（撤销/重做可用性与第二行字体状态可能变化）。
    fn repaint_toolbar(&mut self) {
        self.repaint_toolbar_chrome();
    }
}

/// 把目标点夹在选区内（拖端点不能把标注拖出选区）。
fn clamp_point(sel: SelRect, p: Point) -> Point {
    Point::new(
        p.x.clamp(sel.x as f32, (sel.x + sel.w - 1).max(sel.x) as f32),
        p.y.clamp(sel.y as f32, (sel.y + sel.h - 1).max(sel.y) as f32),
    )
}

/// 整体移动后的包围盒（整框限制在选区内；对象比选区大时取可动范围的另一端，不报错）。
fn shifted_frame(frame: Bounds, sel: SelRect, dx: i32, dy: i32) -> Bounds {
    let (dx, dy) = (dx as f32, dy as f32);
    let lo_x = sel.x as f32 - frame.x0;
    let hi_x = (sel.x + sel.w) as f32 - frame.x1;
    let lo_y = sel.y as f32 - frame.y0;
    let hi_y = (sel.y + sel.h) as f32 - frame.y1;
    let (dx, dy) = (
        dx.clamp(lo_x.min(hi_x), lo_x.max(hi_x)),
        dy.clamp(lo_y.min(hi_y), lo_y.max(hi_y)),
    );
    Bounds {
        x0: frame.x0 + dx,
        y0: frame.y0 + dy,
        x1: frame.x1 + dx,
        y1: frame.y1 + dy,
    }
}
