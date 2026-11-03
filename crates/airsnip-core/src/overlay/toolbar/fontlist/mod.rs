// SPDX-License-Identifier: GPL-3.0-only
//! 字体列表面板（EDT-4）：工具栏第二行字体按钮展开的自绘浮层。
//! 每行用该字体自身渲染（所见即所选），当前字体高亮；滚轮滚动。
//! 布局/命中/滚动在本模块，绘制见 `paint`。尺寸基准为 100% DPI，经 `Ui` 缩放（CAP-5）。

mod paint;
pub(in crate::overlay) use paint::render;

use crate::overlay::geometry::{SelRect, in_rect};
use crate::overlay::ui::Ui;
use crate::render::fonts;

/// 行高基准值。
const ROW_H_BASE: f32 = 22.0;
/// 面板宽度基准值。
const W_BASE: f32 = 200.0;
/// 面板最大高度基准值（超出部分靠滚动）。
const MAX_H_BASE: f32 = 240.0;
/// 上下内边距基准值。
const PAD_BASE: f32 = 4.0;
/// 滚轮一次滚动的行数。
const SCROLL_ROWS: usize = 3;

/// 打开中的字体面板状态。
#[derive(Clone)]
pub struct FontPanel {
    pub rect: SelRect,
    /// 界面缩放（行高、内缩与字号随之缩放）。
    ui: Ui,
    /// 首个可见字体在 `fonts::families()` 中的下标。
    top: usize,
    /// 鼠标悬停的字体下标。
    hover: Option<usize>,
}

impl FontPanel {
    /// 从字体按钮与工具栏位置展开：优先显示在工具栏上方，空间不足时翻到下方；
    /// 并把当前字体滚到可见区中部。
    pub fn open(btn: SelRect, bar: SelRect, current: &str, screen_w: i32, screen_h: i32, ui: Ui) -> Self {
        let list = fonts::families();
        let (row_h, w, pad) = (ui.px(ROW_H_BASE), ui.px(W_BASE), ui.px(PAD_BASE));
        // 高度按整行取整（不留底部空白死区，命中/绘制严格对齐）
        let max_rows = ((ui.px(MAX_H_BASE) - pad * 2) / row_h).max(1) as usize;
        let content = list.len().min(max_rows) as i32 * row_h + pad * 2;
        let margin = ui.px(4.0);
        let gap = ui.px(2.0);
        let x = btn.x.clamp(margin, (screen_w - w - margin).max(margin));
        let above = bar.y - content - gap;
        let y = if above >= margin {
            above
        } else {
            (bar.y + bar.h + gap).min((screen_h - content - margin).max(margin))
        };
        let mut panel = Self {
            rect: SelRect { x, y, w, h: content },
            ui,
            top: 0,
            hover: None,
        };
        let index = list.iter().position(|f| f == current).unwrap_or(0);
        panel.top = index.saturating_sub(panel.rows() / 2);
        panel.clamp_top();
        panel
    }

    /// 界面缩放（绘制子模块读取字号与圆角）。
    pub(in crate::overlay) fn ui(&self) -> Ui {
        self.ui
    }

    /// 行高。
    fn row_h(&self) -> i32 {
        self.ui.px(ROW_H_BASE)
    }

    /// 上下内边距。
    fn pad(&self) -> i32 {
        self.ui.px(PAD_BASE)
    }

    /// 可见行数。
    pub fn rows(&self) -> usize {
        ((self.rect.h - self.pad() * 2).max(self.row_h()) / self.row_h()) as usize
    }

    /// 最大首行下标（列表尾部对齐）。
    fn max_top(&self) -> usize {
        fonts::families().len().saturating_sub(self.rows())
    }

    fn clamp_top(&mut self) {
        self.top = self.top.min(self.max_top());
    }

    /// 第 `row` 个可见行：字体下标、族名与行矩形；超出列表返回 None。
    pub fn row(&self, row: usize) -> Option<(usize, &'static str, SelRect)> {
        let index = self.top + row;
        let name = fonts::families().get(index)?;
        Some((index, name.as_str(), self.row_rect(row)))
    }

    fn row_rect(&self, row: usize) -> SelRect {
        SelRect {
            x: self.rect.x + 1,
            y: self.rect.y + self.pad() + row as i32 * self.row_h(),
            w: self.rect.w - 2,
            h: self.row_h(),
        }
    }

    /// 命中行：返回字体下标（面板外或空白处返回 None）。
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        if !in_rect(self.rect, x, y) {
            return None;
        }
        let row = ((y - self.rect.y - self.pad()).max(0) / self.row_h()) as usize;
        // 不用 `row()` 的越界保护：面板内但落在已绘制行之外的空白也要返回 None
        if row >= self.rows() {
            return None;
        }
        self.row(row).map(|(i, _, _)| i)
    }

    /// 滚轮滚动（`delta` 为 ±120 倍数，向上为正向内容上移）；返回是否变化。
    pub fn scroll(&mut self, delta: i16) -> bool {
        let step = SCROLL_ROWS;
        let before = self.top;
        let dir = if delta > 0 { -1 } else { 1 };
        let next = self.top as i32 + dir * step as i32;
        self.top = next.clamp(0, self.max_top() as i32) as usize;
        self.top != before
    }

    /// 更新悬停行；返回是否变化（供局部重绘）。
    pub fn hover_at(&mut self, x: i32, y: i32) -> bool {
        let hover = self.hit(x, y);
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    /// 鼠标是否在面板内（用于滚轮与点击的分流）。
    pub fn contains(&self, x: i32, y: i32) -> bool {
        in_rect(self.rect, x, y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Ui {
        Ui::from_dpi(96)
    }

    fn btn(x: i32, y: i32) -> SelRect {
        SelRect { x, y, w: 132, h: 26 }
    }

    /// 工具栏整体（两行）：第二行底边在 y + 64。
    fn bar(x: i32, y: i32) -> SelRect {
        SelRect { x, y, w: 560, h: 64 }
    }

    #[test]
    fn panel_stays_inside_screen_and_prefers_above() {
        let (sw, sh) = (1366, 768);
        let above = FontPanel::open(
            btn(200, 700 + 34),
            bar(200, 700),
            crate::editor::DEFAULT_FONT_FAMILY,
            sw,
            sh,
            base(),
        );
        assert!(
            above.rect.y + above.rect.h <= 700,
            "应展开在工具栏上方：{:?}",
            above.rect
        );
        assert!(above.rect.x >= 0 && above.rect.x + above.rect.w <= sw);
        assert!(above.rect.y >= 0 && above.rect.y + above.rect.h <= sh);

        // 工具栏贴近顶部：翻到下方，且不与工具栏重叠
        let below = FontPanel::open(
            btn(200, 34),
            bar(200, 0),
            crate::editor::DEFAULT_FONT_FAMILY,
            sw,
            sh,
            base(),
        );
        assert!(below.rect.y >= 64, "应展开在工具栏下方：{:?}", below.rect);
        assert!(below.rect.y + below.rect.h <= sh);
    }

    #[test]
    fn hit_maps_row_to_font_index() {
        let panel = FontPanel::open(
            btn(200, 400 + 34),
            bar(200, 400),
            crate::editor::DEFAULT_FONT_FAMILY,
            1366,
            768,
            base(),
        );
        let (index, _, r) = panel.row(0).expect("至少一行");
        assert_eq!(panel.hit(r.x + 4, r.y + 4), Some(index));
        assert_eq!(panel.hit(panel.rect.x - 10, panel.rect.y + 4), None, "面板外不命中");
        assert_eq!(
            panel.hit(panel.rect.x + 4, panel.rect.y + panel.rect.h - 1),
            None,
            "最后一行下方的空白不应命中字体"
        );
    }

    #[test]
    fn current_font_is_visible_after_open() {
        let list = fonts::families();
        let last = list.last().expect("至少一个字体");
        let panel = FontPanel::open(btn(200, 434), bar(200, 400), last, 1366, 768, base());
        let visible = (0..panel.rows())
            .filter_map(|r| panel.row(r))
            .any(|(_, n, _)| n == last);
        assert!(visible, "当前字体应滚入可见区（列表 {last}）");
    }

    #[test]
    fn scroll_clamps_to_list_bounds() {
        let mut panel = FontPanel::open(
            btn(200, 434),
            bar(200, 400),
            crate::editor::DEFAULT_FONT_FAMILY,
            1366,
            768,
            base(),
        );
        for _ in 0..200 {
            panel.scroll(120);
        }
        assert_eq!(panel.top, 0, "上滚到底应停在首行");
        for _ in 0..200 {
            panel.scroll(-120);
        }
        assert_eq!(panel.top, panel.max_top(), "下滚到底应停在末行");
    }

    /// 高 DPI 下面板整体放大，行高与命中区同步（CAP-5）。
    #[test]
    fn panel_scales_with_dpi() {
        let base_panel = FontPanel::open(
            btn(200, 434),
            bar(200, 400),
            crate::editor::DEFAULT_FONT_FAMILY,
            1366,
            768,
            base(),
        );
        let hidpi = FontPanel::open(
            btn(400, 868),
            bar(400, 800),
            crate::editor::DEFAULT_FONT_FAMILY,
            2732,
            1536,
            Ui::from_dpi(192),
        );
        assert_eq!(hidpi.rect.w, base_panel.rect.w * 2, "面板宽度应翻倍");
        let (_, _, r0) = base_panel.row(0).expect("至少一行");
        let (_, _, r1) = hidpi.row(0).expect("至少一行");
        assert_eq!(r1.h, r0.h * 2, "行高应翻倍");
    }
}
