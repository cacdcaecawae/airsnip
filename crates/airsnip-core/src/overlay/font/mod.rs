// SPDX-License-Identifier: GPL-3.0-only
//! 字体设置（EDT-4）：字体族 / 粗体 / 斜体 / 字号的目标与默认值管理。
//! 作用目标优先级：正在输入的文本 > 选中的文本对象 > 新建文本的默认值
//! （前两者即时生效并进撤销栈，默认值供下一次放置文本沿用）。
//! 工具栏与字体面板的显隐、脏区合并与面板交互见 `panel`。

mod panel;

use std::sync::Arc;

use super::geometry::{SelRect, expand_rect, union_rect};
use super::toolbar;
use crate::editor::{self, Kind};

/// 字号每步增量（第二行 − / + 按钮；滚轮仍为 1px 连续调节）。
pub(super) const SIZE_STEP: f32 = 2.0;

/// 一次字体变更（绝对值，非增量）。
pub(super) enum FontChange {
    Family(Arc<str>),
    Bold(bool),
    Italic(bool),
    Size(f32),
}

/// 把变更写入一组字体字段；返回是否真的改变。
fn apply_fields(
    change: &FontChange,
    family: &mut Arc<str>,
    size: &mut f32,
    bold: &mut bool,
    italic: &mut bool,
) -> bool {
    match change {
        FontChange::Family(new) => {
            if family.as_ref() == new.as_ref() {
                return false;
            }
            *family = new.clone();
            true
        }
        FontChange::Bold(v) => std::mem::replace(bold, *v) != *v,
        FontChange::Italic(v) => std::mem::replace(italic, *v) != *v,
        FontChange::Size(v) => {
            let v = v.clamp(editor::FONT_SIZE_RANGE.0, editor::FONT_SIZE_RANGE.1);
            if (*size - v).abs() < 0.01 {
                return false;
            }
            *size = v;
            true
        }
    }
}

impl super::Overlay {
    /// 工具栏与字体面板的合并脏区（两者可能分离，重绘需一并覆盖）。
    pub(super) fn chrome_rect(&self) -> Option<SelRect> {
        union_rect(
            self.bar.as_ref().map(|b| b.rect),
            self.font_list.as_ref().map(|p| p.rect),
        )
    }

    /// 重绘工具栏与字体面板所在区域。
    pub(super) fn repaint_toolbar_chrome(&mut self) {
        if let Some(rect) = self.chrome_rect() {
            self.repaint(expand_rect(rect, 2));
        }
    }

    /// 第二行是否展开（EDT-4）：文本工具、选中文本对象或正在输入文本。
    pub(super) fn text_row_visible(&self) -> bool {
        self.text_edit.is_some()
            || self.tool == editor::Tool::Text
            || self
                .selected_object()
                .is_some_and(|o| matches!(o.kind, Kind::Text { .. }))
    }

    /// 第二行展示的字体状态：输入态 > 选中文本对象 > 默认值。
    pub(super) fn font_state(&self) -> (Arc<str>, f32, bool, bool) {
        if let Some(edit) = &self.text_edit {
            return (edit.font_family.clone(), edit.font_size, edit.bold, edit.italic);
        }
        if let Some(Kind::Text {
            font_family,
            font_size,
            bold,
            italic,
            ..
        }) = self.selected_object().map(|o| &o.kind)
        {
            return (font_family.clone(), *font_size, *bold, *italic);
        }
        (self.font_family.clone(), self.font_size, self.bold, self.italic)
    }

    /// 第二行显隐可能与当前状态不符时重建布局（并合并旧/新脏区重绘）。
    pub(super) fn refresh_toolbar(&mut self) {
        let Some(sel) = self.selection else {
            return;
        };
        let want = self.text_row_visible();
        if self.bar.as_ref().is_some_and(|b| b.second.is_some() == want) {
            return;
        }
        let old = self.chrome_rect();
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        self.bar = Some(toolbar::layout(sel, vw, vh, want, self.ui));
        // 第二行显隐会改变工具栏高度，面板位置随之失效
        self.font_list = None;
        if let Some(dirty) = union_rect(old, self.chrome_rect()) {
            self.repaint(expand_rect(dirty, 2));
        }
    }

    /// 应用字体变更（EDT-4）：输入态 / 选中文本对象 / 默认值。
    pub(super) fn apply_font(&mut self, change: FontChange) {
        // 1) 正在输入：改输入态，立即重绘
        if self.text_edit.is_some() {
            let changed = match self.text_edit.as_mut() {
                Some(edit) => apply_fields(
                    &change,
                    &mut edit.font_family,
                    &mut edit.font_size,
                    &mut edit.bold,
                    &mut edit.italic,
                ),
                None => false,
            };
            if changed {
                self.sync_defaults(&change);
                self.repaint_text_edit();
            }
            return;
        }
        // 2) 选中的文本对象：改对象（进撤销栈，尺寸缓存重测）
        if let Some(i) = self
            .selected
            .filter(|i| matches!(self.doc.objects().get(*i).map(|o| &o.kind), Some(Kind::Text { .. })))
        {
            let before = self.selected_clip();
            self.doc.begin();
            let changed = match &mut self.doc.objects_mut()[i].kind {
                Kind::Text {
                    font_family,
                    font_size,
                    bold,
                    italic,
                    ..
                } => apply_fields(&change, font_family, font_size, bold, italic),
                _ => false,
            };
            if changed {
                crate::render::text::remeasure(&mut self.doc.objects_mut()[i].kind);
            }
            self.doc.commit(changed);
            if changed {
                self.sync_defaults(&change);
                if let Some(d) = union_rect(before, self.selected_clip()) {
                    self.repaint(expand_rect(d, 6));
                }
                self.repaint_toolbar_chrome();
            }
            return;
        }
        // 3) 只改默认值（下一次放置文本沿用）
        if apply_fields(
            &change,
            &mut self.font_family,
            &mut self.font_size,
            &mut self.bold,
            &mut self.italic,
        ) {
            self.repaint_toolbar_chrome();
        }
    }

    /// 把本次选择记为新建文本的默认字体（EDT-4）。
    fn sync_defaults(&mut self, change: &FontChange) {
        match change {
            FontChange::Family(f) => self.font_family = f.clone(),
            FontChange::Bold(v) => self.bold = *v,
            FontChange::Italic(v) => self.italic = *v,
            FontChange::Size(v) => self.font_size = v.clamp(editor::FONT_SIZE_RANGE.0, editor::FONT_SIZE_RANGE.1),
        }
    }

    /// 第二行按钮的动作分发（由 `apply_action` 转调，避免标注模块膨胀）。
    pub(super) fn on_font_action(&mut self, action: toolbar::Action) {
        match action {
            toolbar::Action::FontPick => self.toggle_font_list(),
            // 其余按钮操作时收起面板（面板与按钮不同时占用注意力）
            _ => {
                self.close_font_list();
                match action {
                    toolbar::Action::ToggleBold => {
                        let (_, _, bold, _) = self.font_state();
                        self.apply_font(FontChange::Bold(!bold));
                    }
                    toolbar::Action::ToggleItalic => {
                        let (_, _, _, italic) = self.font_state();
                        self.apply_font(FontChange::Italic(!italic));
                    }
                    toolbar::Action::FontSmaller => {
                        let (_, size, _, _) = self.font_state();
                        self.apply_font(FontChange::Size(size - SIZE_STEP));
                    }
                    toolbar::Action::FontLarger => {
                        let (_, size, _, _) = self.font_state();
                        self.apply_font(FontChange::Size(size + SIZE_STEP));
                    }
                    // 字号数值格仅展示，不响应点击
                    toolbar::Action::FontSize => {}
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> (Arc<str>, f32, bool, bool) {
        (Arc::from(editor::DEFAULT_FONT_FAMILY), 24.0, false, false)
    }
    #[test]
    fn family_change_detects_noop() {
        let (mut family, mut size, mut bold, mut italic) = fields();
        let same = family.clone();
        assert!(!apply_fields(
            &FontChange::Family(same),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        let other: Arc<str> = Arc::from("SimSun");
        assert!(apply_fields(
            &FontChange::Family(other.clone()),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert_eq!(&*family, "SimSun");
    }

    #[test]
    fn bold_italic_toggle_and_size_clamp() {
        let (mut family, mut size, mut bold, mut italic) = fields();
        assert!(apply_fields(
            &FontChange::Bold(true),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert!(bold);
        assert!(!apply_fields(
            &FontChange::Bold(true),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert!(apply_fields(
            &FontChange::Italic(true),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert!(italic);

        // 字号：越界被夹住，且不产生伪变化
        assert!(apply_fields(
            &FontChange::Size(1000.0),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert_eq!(size, editor::FONT_SIZE_RANGE.1);
        assert!(!apply_fields(
            &FontChange::Size(1000.0),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert!(apply_fields(
            &FontChange::Size(1.0),
            &mut family,
            &mut size,
            &mut bold,
            &mut italic
        ));
        assert_eq!(size, editor::FONT_SIZE_RANGE.0);
    }
}
