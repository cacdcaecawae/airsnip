// SPDX-License-Identifier: GPL-3.0-only
//! 绘制草稿（EDT-1/2/3）：在选区内拖拽时的临时对象与提交。
//! 文本工具不产生草稿（走输入状态机，见 `textinput`）。

use crate::editor::{Kind, Object, Point, Tool};
use crate::render;

use super::super::geometry::{SelRect, expand_rect, union_rect};
use super::super::pick::is_degenerate;

impl super::super::Overlay {
    /// 在选区内开始拖拽绘制标注（EDT-1/EDT-2）；文本工具转为点击放置（EDT-4）。
    pub(super) fn start_draw(&mut self, x: i32, y: i32) {
        let p = Point::new(x as f32, y as f32);
        let kind = match self.tool {
            Tool::Text => {
                self.begin_text_input(x, y);
                return;
            }
            Tool::Rect => Kind::Rect {
                a: p,
                b: p,
                // 圆角不再由工具栏开关控制：新建为直角，选中后拖内侧角的控制柄调（EDT-1）
                radius: 0.0,
                filled: self.filled,
            },
            Tool::Arrow => Kind::arrow(p, p),
            Tool::Highlight => Kind::Highlight {
                a: p,
                b: p,
                // 新建默认直角，选中后拖内侧角手柄调（EDT-1 同一套）
                corner: 0.0,
            },
            Tool::Blur => Kind::Blur {
                a: p,
                b: p,
                radius: self.blur_radius,
                corner: 0.0,
            },
            Tool::Glow => Kind::Glow {
                a: p,
                b: p,
                corner: 0.0,
            },
        };
        self.doc.begin();
        self.draft = Some(Object {
            kind,
            style: self.current_style(),
        });
        // 聚光灯：按下即进入暗化态（选区其余区域变暗、本区域亮），全选区重绘一次
        if self.tool == Tool::Glow {
            if let Some(sel) = self.selection {
                self.repaint(sel);
            }
        }
    }

    /// 拖动中：草稿终点跟随鼠标（限制在选区内）并重绘。
    /// `shift` 为真时施加绘制约束：矩形 → 正方形（EDT-1），箭头 → 水平 / 垂直 / 45° 吸附（EDT-2）。
    pub(super) fn update_draft(&mut self, x: i32, y: i32, shift: bool) {
        let Some(sel) = self.selection else {
            return;
        };
        let cx = x.clamp(sel.x, (sel.x + sel.w - 1).max(sel.x));
        let cy = y.clamp(sel.y, (sel.y + sel.h - 1).max(sel.y));
        let end = self.constrained_end(cx, cy, shift, sel);
        let before = self.draft_region();
        if let Some(draft) = self.draft.as_mut() {
            draft.kind = match &draft.kind {
                Kind::Rect { a, radius, filled, .. } => Kind::Rect {
                    a: *a,
                    b: end,
                    radius: *radius,
                    filled: *filled,
                },
                // 拖动中重建箭头：控制点保持在 1/3、2/3 处（EDT-3 从直线开始）
                Kind::Arrow { from, .. } => Kind::arrow(*from, end),
                // 区域类（高亮/模糊）：只更新对角点，半径/圆角/样式不变
                Kind::Highlight { a, corner, .. } => Kind::Highlight {
                    a: *a,
                    b: end,
                    corner: *corner,
                },
                Kind::Blur { a, radius, corner, .. } => Kind::Blur {
                    a: *a,
                    b: end,
                    radius: *radius,
                    corner: *corner,
                },
                // 聚光：同区域类，只更新对角点
                Kind::Glow { a, corner, .. } => Kind::Glow {
                    a: *a,
                    b: end,
                    corner: *corner,
                },
                // 文本不经草稿拖拽（Text 工具走输入状态机）
                Kind::Text { .. } => return,
            };
        }
        if let Some(d) = union_rect(before, self.draft_region()) {
            self.repaint(expand_rect(d, 2));
        }
    }

    /// 草稿终点：鼠标点已夹在选区内，再按 Shift 施加几何约束（EDT-1/EDT-2）；
    /// 约束结果仍要落在选区内，故再夹一次（贴边时正方形/45° 会被裁到边界）。
    fn constrained_end(&self, cx: i32, cy: i32, shift: bool, sel: SelRect) -> Point {
        let end = Point::new(cx as f32, cy as f32);
        if !shift {
            return end;
        }
        let snapped = match self.draft.as_ref().map(|d| &d.kind) {
            Some(Kind::Rect { a, .. }) => crate::editor::square_corner(*a, end),
            Some(Kind::Arrow { from, .. }) => crate::editor::snap_direction(*from, end),
            _ => return end,
        };
        Point::new(
            snapped.x.clamp(sel.x as f32, (sel.x + sel.w - 1).max(sel.x) as f32),
            snapped.y.clamp(sel.y as f32, (sel.y + sel.h - 1).max(sel.y) as f32),
        )
    }

    /// 放开：过短视为误触丢弃；保存并选中，入撤销栈。
    pub(super) fn commit_draft(&mut self) {
        let Some(draft) = self.draft.take() else {
            return;
        };
        // 先取几何（对象非 Copy，push 会移动），再决定是否保留
        let draft_bounds = render::bounds_to_clip(&draft.bounds());
        let kept = !is_degenerate(&draft);
        if kept {
            self.doc.push(draft);
            // 新建的箭头随即选中，控制柄落在 1/3、2/3 处（EDT-3）
            self.selected = Some(self.doc.objects().len() - 1);
        }
        self.doc.commit(kept);
        let c = draft_bounds;
        let mut region = SelRect {
            x: c.0,
            y: c.1,
            w: c.2 - c.0,
            h: c.3 - c.1,
        };
        if let Some(bar) = self.bar.as_ref() {
            // 撤销/重做按钮的可用性随之变化
            region = union_rect(Some(region), Some(expand_rect(bar.rect, 2))).unwrap_or(region);
        }
        self.repaint(expand_rect(region, 2));
    }

    /// 草稿对象当前占据的像素区域。
    pub(super) fn draft_region(&self) -> Option<SelRect> {
        let c = render::bounds_to_clip(&self.draft.as_ref()?.bounds());
        Some(SelRect {
            x: c.0,
            y: c.1,
            w: c.2 - c.0,
            h: c.3 - c.1,
        })
    }
}
