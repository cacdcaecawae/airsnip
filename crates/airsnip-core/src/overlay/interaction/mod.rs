// SPDX-License-Identifier: GPL-3.0-only
//! 鼠标与绘制交互（CAP-2、EDT-1/2/3/7）：框选、八向缩放、空格移动选区、选区内绘制标注、
//! 选中对象的拖动与缩放。命中优先级：
//! 选中对象的手柄 > 曲线控制柄 > 选区手柄 > 已有对象（选中并可拖动）> 空白处绘制。
//! 对象手柄与几何变换见 `objedit`；手柄/对象命中判定与选中状态见 `pick`，绘制草稿见 `draft`。

mod draft;

use super::geometry::SelRect;
use super::objedit::{self, ObjHandle};
use super::pick::{Handle, hit_handle};

/// 交互阶段。
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Phase {
    /// 拖框选择中
    Selecting,
    /// 选区确认后，可移动/缩放/输出
    Adjusting,
}

/// 拖拽中的上下文（选区拖动；对象拖动见 `objedit::ObjDrag`）。
#[derive(Clone, Copy)]
pub(super) struct Drag {
    pub(super) handle: Handle,
    /// 按下时的选区（用于计算增量）。
    orig: SelRect,
    /// 按下时的鼠标位置。
    px: i32,
    py: i32,
}

impl super::Overlay {
    /// 方向键微调（EDT-7 / CAP-2）：有选中对象 → 移动该对象（限制在选区内）；
    /// 否则 → 移动选区框（标注不跟随，允许落到选区外，导出以选区为准）。
    /// 每次按键 = 一步撤销。
    pub fn nudge(&mut self, dx: i32, dy: i32) {
        // 拖动/绘制中忽略：这些流程每帧从按下时的几何重算，微调会被覆盖
        if self.obj_drag.is_some() || self.ctrl_drag.is_some() || self.drag.is_some() || self.draft.is_some() {
            return;
        }
        if self.selected.is_some() {
            self.nudge_object(dx, dy);
            return;
        }
        self.nudge_selection(dx, dy);
    }

    /// 微调选区框位置（CAP-2）：限制在虚拟屏幕内，工具栏随之重排。
    fn nudge_selection(&mut self, dx: i32, dy: i32) {
        let Some(sel) = self.selection else {
            return;
        };
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        let x = (sel.x + dx).clamp(0, (vw - sel.w).max(0));
        let y = (sel.y + dy).clamp(0, (vh - sel.h).max(0));
        if x == sel.x && y == sel.y {
            return;
        }
        self.set_selection(Some(SelRect {
            x,
            y,
            w: sel.w,
            h: sel.h,
        }));
    }

    pub fn on_lbutton_down(&mut self, x: i32, y: i32) {
        // 工具栏优先（EDT-7）：点在工具栏上只切换状态，不开始绘制
        let bar = self.bar.clone();
        if let Some(bar) = &bar {
            if super::geometry::in_rect(bar.rect, x, y) {
                if let Some(action) = bar.hit(x, y) {
                    self.apply_action(action);
                }
                return;
            }
        }
        // 字体面板（EDT-4）：不覆盖工具栏，故排在工具栏之后；命中行选字体，
        // 点面板之外则收起面板并退回常规流程
        if self.on_font_list_down(x, y) {
            return;
        }
        // 文本输入中：任何左键先提交当前输入（EDT-4），再按常规流程（Text 工具点击空白会开启新输入）
        if self.text_edit.is_some() {
            self.commit_text();
        }
        self.down_started_draw = false;
        let new = match self.selection {
            // 窗口识别（CAP-6）：先不建选区，拖出容差才开始框选，没拖动则弹起时选中悬停的窗口
            None if self.snapping() => {
                self.update_hover(x, y);
                self.begin_select(x, y);
                return;
            }
            None => Some(self.begin_select(x, y)),
            Some(sel) => {
                // 1) 选中对象的八向手柄（EDT-7）：可能压在选区装饰或对象描边上，最优先
                if let Some(i) = self.selected
                    && let Some(h) = self
                        .doc
                        .objects()
                        .get(i)
                        .and_then(|o| objedit::hit_handle(o, x, y, self.ui))
                {
                    self.begin_obj_drag(i, h, x, y);
                    return;
                }
                // 2) 曲线控制柄（EDT-3）：控制点常落在自己线身上，需先于「选中对象」判定
                if let Some(index) = self.hit_control(x, y) {
                    self.begin_control_drag(index);
                    return;
                }
                if let Some(handle) = hit_handle(sel, x, y, self.ui) {
                    self.drag = Some(Drag {
                        handle,
                        orig: sel,
                        px: x,
                        py: y,
                    });
                    self.phase = Phase::Adjusting;
                    Some(sel)
                } else if sel.contains(x, y) {
                    if self.mods.space {
                        // 按住空格拖动：移动整个选区（CAP-2）
                        self.drag = Some(Drag {
                            handle: Handle::Move,
                            orig: sel,
                            px: x,
                            py: y,
                        });
                        self.phase = Phase::Adjusting;
                        Some(sel)
                    } else if let Some(i) = self.hit_object(x, y) {
                        // 点中已有对象：选中并可直接拖动（EDT-7）；
                        // 无位移的按下退化为单击选中，只有真正移动才进撤销栈
                        self.begin_obj_drag(i, ObjHandle::Move, x, y);
                        return;
                    } else {
                        // 空白处按下：用当前工具绘制标注（EDT-1/EDT-2/EDT-3）
                        self.set_selected(None);
                        self.down_started_draw = true;
                        self.start_draw(x, y);
                        return;
                    }
                } else {
                    // 远处点击：开始新选区，并取消选中
                    self.set_selected(None);
                    Some(self.begin_select(x, y))
                }
            }
        };
        self.set_selection(new);
    }

    pub fn on_mouse_move(&mut self, x: i32, y: i32) {
        // Shift 约束（EDT-1/EDT-2/EDT-7）：新建与二次编辑共用同一状态
        let shift = self.mods.shift;
        // 拖动选中对象（EDT-7）
        if self.obj_drag.is_some() {
            self.drag_obj(x, y, shift);
            return;
        }
        // 拖动曲线控制柄（EDT-3）
        if self.ctrl_drag.is_some() {
            self.drag_control(x, y);
            return;
        }
        // 绘制中：草稿终点跟随鼠标（限制在选区内）；Shift 施加正方形/方向约束（EDT-1/EDT-2）
        if self.draft.is_some() {
            self.update_draft(x, y, shift);
            return;
        }
        let Some(drag) = &self.drag else {
            self.update_hover(x, y);
            return;
        };
        let (dx, dy) = (x - drag.px, y - drag.py);
        let handle = drag.handle;
        let orig = drag.orig;
        let phase = self.phase;
        // 按下后还在单击容差内：仍是单击，保留悬停高亮（CAP-6）
        if phase == Phase::Selecting
            && self.snapping()
            && dx.abs() <= super::snap::CLICK_SLOP
            && dy.abs() <= super::snap::CLICK_SLOP
        {
            return;
        }
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        let max_x = vw.saturating_sub(1);
        let max_y = vh.saturating_sub(1);
        let x = x.clamp(0, max_x);
        let y = y.clamp(0, max_y);

        let new = if phase == Phase::Selecting {
            // 框选阶段：以按下点为锚点拉伸矩形
            SelRect::normalized(orig.x, orig.y, x, y)
        } else {
            match handle {
                Handle::Move => {
                    let lim_x = (max_x - orig.w).max(0);
                    let lim_y = (max_y - orig.h).max(0);
                    SelRect {
                        x: (orig.x + dx).clamp(0, lim_x),
                        y: (orig.y + dy).clamp(0, lim_y),
                        w: orig.w,
                        h: orig.h,
                    }
                }
                Handle::N => SelRect::normalized(orig.x, y, orig.x + orig.w, orig.y + orig.h),
                Handle::S => SelRect {
                    x: orig.x,
                    y: orig.y,
                    w: orig.w,
                    h: (y - orig.y).max(1),
                },
                Handle::W => SelRect::normalized(x, orig.y, orig.x + orig.w, orig.y + orig.h),
                Handle::E => SelRect {
                    x: orig.x,
                    y: orig.y,
                    w: (x - orig.x).max(1),
                    h: orig.h,
                },
                Handle::NE => SelRect::normalized(orig.x, y, orig.x + orig.w, orig.y + orig.h),
                Handle::NW => SelRect::normalized(x, y, orig.x + orig.w, orig.y + orig.h),
                Handle::SE => SelRect::normalized(orig.x, orig.y, x.max(orig.x + 1), y.max(orig.y + 1)),
                Handle::SW => SelRect::normalized(x, orig.y, orig.x + orig.w, orig.y + orig.h),
            }
        };
        self.set_selection(Some(new));
    }

    pub fn on_lbutton_up(&mut self) {
        // 结束对象拖动（EDT-7）：无位移视为单击（仅选中），不进撤销栈
        if self.obj_drag.is_some() {
            self.end_obj_drag();
            return;
        }
        // 结束控制柄拖动（EDT-3）
        if self.ctrl_drag.is_some() {
            self.end_control_drag();
            return;
        }
        // 结束绘制：过短视为误触，丢弃；提交后进入撤销栈
        if self.draft.is_some() {
            self.commit_draft();
            return;
        }
        self.drag = None;
        let new = match self.selection {
            Some(sel) if sel.w < 2 || sel.h < 2 => None, // 误触，放弃
            // 单击（没拖出选区）：选中悬停识别的窗口或整屏（CAP-6/7；功能关闭时 hover 恒为 None）
            None => self.hover,
            other => other,
        };
        if new.is_some() {
            self.phase = Phase::Adjusting;
        }
        self.set_selection(new);
    }

    /// 开始新的框选（返回初始 1px 选区）。
    fn begin_select(&mut self, x: i32, y: i32) -> SelRect {
        let sel = SelRect { x, y, w: 1, h: 1 };
        self.phase = Phase::Selecting;
        self.drag = Some(Drag {
            handle: Handle::Move,
            orig: sel,
            px: x,
            py: y,
        });
        sel
    }
}
