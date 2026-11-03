// SPDX-License-Identifier: GPL-3.0-only
//! 字体列表面板的交互（EDT-4）：展开/收起、点击选字、滚轮滚动、悬停高亮。
//! 面板的布局、命中与绘制见 `overlay::toolbar::fontlist`；字体变更落在 `super::apply_font`。

use std::sync::Arc;

use crate::overlay::Overlay;
use crate::overlay::geometry::{expand_rect, union_rect};
use crate::overlay::toolbar::{self, fontlist::FontPanel};

use super::FontChange;

impl Overlay {
    /// 展开 / 收起字体面板。
    pub(in crate::overlay) fn toggle_font_list(&mut self) {
        let old = self.font_list.as_ref().map(|p| p.rect);
        self.font_list = if self.font_list.is_some() {
            None
        } else {
            self.open_font_list()
        };
        if let Some(dirty) = union_rect(old, self.font_list.as_ref().map(|p| p.rect)) {
            self.repaint(expand_rect(dirty, 2));
        }
    }

    /// 从第二行字体按钮位置弹出面板（按钮不存在或缺工具栏时不展开）。
    fn open_font_list(&self) -> Option<FontPanel> {
        let bar = self.bar.as_ref()?;
        let btn = bar
            .hits
            .iter()
            .find(|(_, a)| *a == toolbar::Action::FontPick)
            .map(|(r, _)| *r)?;
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        let (family, ..) = self.font_state();
        Some(FontPanel::open(btn, bar.rect, &family, vw, vh, self.ui))
    }

    /// 面板上的左键：命中某行即选中该字体并收起面板。
    /// 返回 true 表示点击已被面板消费（不再走选区/绘制流程）。
    pub(in crate::overlay) fn on_font_list_down(&mut self, x: i32, y: i32) -> bool {
        let Some(panel) = &self.font_list else {
            return false;
        };
        if !panel.contains(x, y) {
            // 点面板之外：收起面板，并把这次点击交给正常流程
            self.toggle_font_list();
            return false;
        }
        let hit = panel.hit(x, y);
        let rect = panel.rect;
        self.font_list = None;
        if let Some(index) = hit
            && let Some(name) = crate::render::fonts::families().get(index)
        {
            self.apply_font(FontChange::Family(Arc::from(name.as_str())));
        }
        self.repaint(expand_rect(rect, 2));
        true
    }

    /// 面板上的滚轮：滚动列表并局部重绘。返回 true 表示已消费。
    pub fn on_font_list_wheel(&mut self, x: i32, y: i32, delta: i16) -> bool {
        let Some(panel) = self.font_list.as_mut() else {
            return false;
        };
        if !panel.contains(x, y) {
            return false;
        }
        let rect = panel.rect;
        if panel.scroll(delta) {
            self.repaint(expand_rect(rect, 2));
        }
        true
    }

    /// 面板悬停高亮（仅悬停行变化时重绘）。
    pub fn on_font_list_hover(&mut self, x: i32, y: i32) {
        let Some(panel) = self.font_list.as_mut() else {
            return;
        };
        if panel.hover_at(x, y) {
            let rect = panel.rect;
            self.repaint(expand_rect(rect, 2));
        }
    }

    /// 字体面板是否展开（Esc 分流用）。
    pub fn font_list_open(&self) -> bool {
        self.font_list.is_some()
    }

    /// 鼠标是否在字体面板上（光标形状与滚轮分流用）。
    pub(in crate::overlay) fn over_font_list(&self, x: i32, y: i32) -> bool {
        self.font_list.as_ref().is_some_and(|p| p.contains(x, y))
    }

    /// 收起面板（工具栏其它按钮、Esc）。
    pub fn close_font_list(&mut self) {
        if let Some(panel) = self.font_list.take() {
            self.repaint(expand_rect(panel.rect, 2));
        }
    }
}
