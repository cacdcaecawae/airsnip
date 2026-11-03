// SPDX-License-Identifier: GPL-3.0-only
//! 窗口识别与全屏（CAP-6 / CAP-7）：还没有选区时，鼠标下的窗口（不在任何窗口上则为整屏）
//! 恢复原图亮度、描边并显示尺寸，单击即选中；按下后拖出 `CLICK_SLOP` 以上才开始手动框选。
//! 选中后在选区内双击即复制，所以「双击窗口 / 双击桌面」= 一步截窗口 / 截全屏。
//! 窗口矩形由平台层在截图时取得（`set_windows`）；未提供时功能关闭，交互与之前一致。
//! 全选整屏（⌘A / Ctrl+A，`select_all`）不依赖窗口列表。高亮的绘制在 `selection::repaint`。

use super::geometry::{SelRect, expand_rect, intersect_rect, text_rect, union_rect};
use super::interaction::Phase;

/// 单击判定的位移容差（像素）：按下后移动不超过它仍算单击，选中识别到的窗口。
pub(super) const CLICK_SLOP: i32 = 4;

impl super::Overlay {
    /// 截图时屏幕上各窗口的矩形（覆盖层像素，前到后），开启窗口识别；超出屏幕的部分裁掉。
    pub fn set_windows(&mut self, rects: Vec<SelRect>) {
        let screen = self.screen_rect();
        self.windows = Some(rects.into_iter().filter_map(|r| intersect_rect(r, screen)).collect());
    }

    fn screen_rect(&self) -> SelRect {
        SelRect {
            x: 0,
            y: 0,
            w: self.display.width() as i32,
            h: self.display.height() as i32,
        }
    }

    /// 窗口识别开启且还没有选区：悬停高亮与单击选中只在此时生效。
    pub(super) fn snapping(&self) -> bool {
        self.windows.is_some() && self.selection.is_none()
    }

    /// 鼠标移动：更新悬停识别的窗口（最前面那个包含鼠标的；都不包含时为整屏），变化时重绘新旧区域。
    pub(super) fn update_hover(&mut self, x: i32, y: i32) {
        if !self.snapping() {
            return;
        }
        let screen = self.screen_rect();
        let found = self
            .windows
            .as_ref()
            .and_then(|ws| ws.iter().copied().find(|r| r.contains(x, y)));
        let new = found.unwrap_or(screen);
        if self.hover == Some(new) {
            return;
        }
        let old = self.hover.replace(new);
        let dirty = [old, Some(new)].into_iter().flatten().fold(None, |acc, r| {
            union_rect(union_rect(acc, Some(self.hover_dirty(r))), Some(self.hover_text(r)))
        });
        if let Some(d) = dirty {
            self.repaint(d);
        }
    }

    /// 鼠标离开本屏：撤掉悬停高亮（多屏时只有鼠标所在的屏显示高亮）。
    pub fn clear_hover(&mut self) {
        if let Some(old) = self.hover.take()
            && let Some(d) = union_rect(Some(self.hover_dirty(old)), Some(self.hover_text(old)))
        {
            self.repaint(d);
        }
    }

    /// 悬停高亮的边框区域（边框画在矩形外沿）。
    pub(super) fn hover_dirty(&self, r: SelRect) -> SelRect {
        expand_rect(r, self.dirty_pad())
    }

    /// 悬停高亮的尺寸提示区域（没有工具栏）。
    pub(super) fn hover_text(&self, r: SelRect) -> SelRect {
        text_rect(r, self.display.width() as i32, self.display.height() as i32, 0, self.ui)
    }

    /// 全选整屏（CAP-7）：已有选区时替换为整屏，已画的标注保留。
    pub fn select_all(&mut self) {
        self.drag = None;
        self.phase = Phase::Adjusting;
        let screen = self.screen_rect();
        self.set_selection(Some(screen));
    }
}
