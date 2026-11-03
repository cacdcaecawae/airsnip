// SPDX-License-Identifier: GPL-3.0-only
//! 曲线控制柄交互（EDT-3）：拖动控制点弯曲箭头、双击恢复直线。
//! 命中与绘制装饰见 `pick`；其余对象交互见 `interaction`。

use crate::editor::Point;

use super::geometry::{expand_rect, union_rect};
use super::pick::pick_radius;

impl super::Overlay {
    /// 命中已选箭头的控制柄（EDT-3），返回 0（尾部侧）或 1（头部侧）。
    pub(super) fn hit_control(&self, x: i32, y: i32) -> Option<usize> {
        let cps = self.selected_object()?.kind.control_points()?;
        let pick = pick_radius(self.ui) as f32;
        cps.iter()
            .position(|p| (p.x - x as f32).abs() <= pick && (p.y - y as f32).abs() <= pick)
    }

    /// 按下控制柄：记录起点（用于「未移动则不进撤销栈」）。
    pub(super) fn begin_control_drag(&mut self, index: usize) {
        let Some(p) = self
            .selected_object()
            .and_then(|o| o.kind.control_points())
            .map(|c| c[index])
        else {
            return;
        };
        self.doc.begin();
        self.ctrl_drag = Some((index, p));
    }

    /// 拖动控制柄：控制点跟随鼠标（限制在选区内），重绘受影响区域。
    pub(super) fn drag_control(&mut self, x: i32, y: i32) {
        let Some((index, _)) = self.ctrl_drag else {
            return;
        };
        let Some(sel) = self.selection else {
            return;
        };
        let Some(i) = self.selected else {
            return;
        };
        let before = self.selected_clip();
        let cx = x.clamp(sel.x, (sel.x + sel.w - 1).max(sel.x)) as f32;
        let cy = y.clamp(sel.y, (sel.y + sel.h - 1).max(sel.y)) as f32;
        self.doc.objects_mut()[i].kind.set_control(index, Point::new(cx, cy));
        if let Some(d) = union_rect(before, self.selected_clip()) {
            self.repaint(expand_rect(d, 2));
        }
    }

    /// 松开控制柄：未移动则丢弃本次历史快照。
    pub(super) fn end_control_drag(&mut self) {
        let Some((index, orig)) = self.ctrl_drag.take() else {
            return;
        };
        let moved = self
            .selected_object()
            .and_then(|o| o.kind.control_points())
            .map(|c| c[index] != orig)
            .unwrap_or(false);
        self.doc.commit(moved);
        if moved {
            // 撤销按钮可用性可能变化
            self.repaint_toolbar_chrome();
        }
    }

    /// 双击控制柄恢复直线（EDT-3）；命中并处理返回 true。
    pub fn reset_curve_at(&mut self, x: i32, y: i32) -> bool {
        if self.hit_control(x, y).is_none() {
            return false;
        }
        self.ctrl_drag = None; // 双击前那次按下启动的拖动作废
        let Some(i) = self.selected else {
            return true;
        };
        let before = self.selected_clip();
        self.doc.begin();
        let changed = self.doc.objects_mut()[i].kind.reset_curve();
        self.doc.commit(changed);
        if changed {
            if let Some(d) = union_rect(before, self.selected_clip()) {
                self.repaint(expand_rect(d, 2));
            }
            if let Some(bar) = &self.bar {
                self.repaint(expand_rect(bar.rect, 2));
            }
        }
        true
    }
}
