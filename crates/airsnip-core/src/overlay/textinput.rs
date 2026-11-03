// SPDX-License-Identifier: GPL-3.0-only
//! 文本输入接线（EDT-4）：「输入中」状态的开始/提交/取消、IME 消息读取与输入框绘制。
//! 编辑状态数据与字符级逻辑见 `text_edit_state`；绘制在覆盖层像素图上，不创建系统控件（ADR 0004）。

use tiny_skia::Pixmap;

use crate::editor::{Kind, Object, Point};
use crate::render;

use super::geometry::expand_rect;
use super::text_edit_state::TextEdit;

impl super::Overlay {
    /// 进入文本输入（Text 工具下点击选区内空白，EDT-4）。
    /// 撤销快照不在此时建立：输入期间其它操作（如滚轮微调对象）可能抢先 begin，
    /// 覆盖 pending 导致提交时快照丢失，故统一在 `commit_text` 修改文档前建立。
    pub(super) fn begin_text_input(&mut self, x: i32, y: i32) {
        let edit = TextEdit {
            pos: Point::new(x as f32, y as f32),
            obj_index: None,
            chars: Vec::new(),
            comp: Vec::new(),
            caret: 0,
            pending_high: None,
            font_size: self.font_size,
            font_family: self.font_family.clone(),
            bold: self.bold,
            italic: self.italic,
        };
        self.text_edit = Some(edit);
        self.host.set_ime(true);
        self.repaint_text_edit();
    }

    /// 双击已提交的文本对象进入再编辑（EDT-4）；非文本对象时返回 false。
    pub fn begin_edit_text(&mut self, i: usize) -> bool {
        let obj = self.doc.objects().get(i).cloned();
        let Some(obj) = obj else { return false };
        let Kind::Text {
            pos,
            text,
            font_size,
            font_family,
            bold,
            italic,
            ..
        } = obj.kind
        else {
            return false;
        };
        // 先收尾上一次输入（提交，不回滚）
        if self.text_edit.is_some() {
            self.commit_text();
        }
        self.selected = Some(i);
        self.text_edit = Some(TextEdit {
            pos,
            obj_index: Some(i),
            chars: text.chars().collect(),
            comp: Vec::new(),
            caret: text.chars().count(),
            pending_high: None,
            font_size,
            font_family,
            bold,
            italic,
        });
        self.host.set_ime(true);
        // 原对象隐藏、输入框登场，立即重绘
        self.repaint_text_edit();
        true
    }

    /// 是否正在文本输入。
    pub fn is_editing_text(&self) -> bool {
        self.text_edit.is_some()
    }

    /// 输入中的光标竖线 (x, y, 高)，像素（macOS 输入法候选框跟随光标）。
    pub fn text_caret(&self) -> Option<(f32, f32, f32)> {
        self.text_edit.as_ref().map(TextEdit::caret_rect)
    }

    /// 提交当前输入（EDT-4）：空文本不创建（编辑时清空 = 删除对象）；Enter / 点击别处触发。
    /// 修改文档前才建撤销快照（begin），避免输入期间其它 begin 覆盖；结束后重绘整个选区。
    pub(super) fn commit_text(&mut self) {
        let Some(edit) = self.text_edit.take() else { return };
        self.host.set_ime(false);
        if edit.chars.is_empty() {
            // 编辑已有对象时全部删掉 = 删除该对象；新建时空文本直接丢弃
            if let Some(i) = edit.obj_index {
                self.doc.begin();
                self.doc.remove(i);
                self.doc.commit(true);
                self.selected = None;
            }
            self.repaint_whole_selection();
            return;
        }
        let text: String = edit.chars.iter().collect();
        let size = render::text::measure(&text, &edit.spec());
        let kind = Kind::Text {
            pos: edit.pos,
            text: std::sync::Arc::from(text),
            font_size: edit.font_size,
            font_family: edit.font_family,
            bold: edit.bold,
            italic: edit.italic,
            size,
        };
        match edit.obj_index {
            None => {
                self.doc.begin();
                self.doc.push(Object {
                    kind,
                    style: self.current_style(),
                });
                self.doc.commit(true);
                self.selected = Some(self.doc.objects().len() - 1);
            }
            Some(i) => {
                self.doc.begin();
                self.doc.objects_mut()[i].kind = kind;
                self.doc.commit(true);
            }
        }
        self.repaint_whole_selection();
    }

    /// 取消当前输入（Esc，EDT-4）：编辑已有对象时不入撤销栈，原对象恢复显示。
    pub(super) fn cancel_text(&mut self) {
        let Some(_edit) = self.text_edit.take() else { return };
        self.host.set_ime(false);
        self.doc.commit(false);
        self.repaint_whole_selection();
    }

    /// 文本输入变化后的重绘：覆盖整个选区，彻底避免文本变短/光标位移时的尾部残留。
    fn repaint_whole_selection(&mut self) {
        // 输入态的开始/结束会影响第二行显隐（EDT-4）
        self.refresh_toolbar();
        if let Some(sel) = self.selection {
            self.repaint(expand_rect(sel, 2));
        }
    }

    /// WM_CHAR：普通字符（含标点/数字），跳过控制字符。
    pub fn on_text_char(&mut self, unit: u16) {
        if (unit as u32) < 0x20 {
            return;
        }
        let Some(edit) = self.text_edit.as_mut() else { return };
        edit.push_code_unit(unit);
        self.repaint_text_edit();
    }

    /// 输入法组合更新：有结果串则提交，否则以组合串（候选）替换当前组合。
    /// 平台层负责读出字符串（Windows `WM_IME_COMPOSITION`，macOS `NSTextInputClient`）。
    pub fn on_text_composition(&mut self, result: Option<String>, comp: Option<String>) {
        let Some(edit) = self.text_edit.as_mut() else { return };
        let changed = if let Some(s) = result {
            edit.commit_composition(s);
            true
        } else if let Some(s) = comp {
            edit.set_composition(s);
            true
        } else {
            false
        };
        if changed {
            self.repaint_text_edit();
        }
    }

    /// IME 组合结束：清空候选。
    pub fn on_text_composition_end(&mut self) {
        let Some(edit) = self.text_edit.as_mut() else { return };
        if !edit.comp.is_empty() {
            edit.comp.clear();
            self.repaint_text_edit();
        }
    }

    /// 输入中的按键（WM_KEYDOWN）；返回是否已处理。`shift`/`ctrl`：
    /// Enter + Shift/Ctrl 换行（EDT-4），单独 Enter 提交。
    pub fn on_text_key(&mut self, vk: u32, shift: bool, ctrl: bool) -> bool {
        let Some(edit) = self.text_edit.as_mut() else {
            return false;
        };
        let mut handled = true;
        match vk {
            0x08 => edit.backspace(),              // Backspace
            0x2E => edit.delete_forward(),         // Delete
            0x25 => edit.move_caret(-1),           // ←
            0x27 => edit.move_caret(1),            // →
            0x24 => edit.caret = 0,                // Home
            0x23 => edit.caret = edit.chars.len(), // End
            // Enter：Shift/Ctrl 组合换行，单独 Enter 提交
            0x0D if shift || ctrl => edit.insert_char('\n'),
            0x0D => self.commit_text(), // Enter：提交
            // Esc：字体面板展开时先收面板（保留输入态），否则取消输入（EDT-4）
            0x1B => {
                if self.font_list_open() {
                    self.close_font_list();
                } else {
                    self.cancel_text();
                }
            }
            // Ctrl+Z：放弃本次未提交输入（已提交文本走全局撤销 Ctrl+Z）
            0x5A if ctrl => self.cancel_text(),
            _ => handled = false,
        }
        if handled {
            self.repaint_text_edit();
        }
        handled
    }

    /// 输入框重绘：整个选区重绘（文本变短/光标位移时旧像素可能落在新 region 外，
    /// 局部重绘必然残留；文本输入是低频按键事件，选区级重绘成本可接受）。
    pub(super) fn repaint_text_edit(&mut self) {
        self.repaint_whole_selection();
    }
}

/// 输入框绘制（selection::repaint 调用于对象之上）：文字 + 带描边的光标 + 组合串下划线。
pub(super) fn draw_edit(pixmap: &mut Pixmap, edit: &TextEdit, color: [u8; 4]) {
    let pad = crate::editor::text::pad(edit.font_size);

    // 文本（已确认 + 组合串）
    render::text::draw(
        pixmap,
        edit.pos,
        &edit.display(),
        &edit.spec(),
        color,
        tiny_skia::Transform::identity(),
    );

    // 光标（竖线，白底 + 黑描边，任意底色下可见）；多行时光标按行定位
    let (cx, cy, ch) = edit.caret_rect();
    if let Some(rect) = tiny_skia::Rect::from_xywh(cx, cy, 2.0, ch)
        && let Some(path) = caret_path(rect)
    {
        let mut black = tiny_skia::Paint::default();
        black.set_color_rgba8(0, 0, 0, 255);
        let edge = tiny_skia::Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &black, &edge, tiny_skia::Transform::identity(), None);
        let mut white = tiny_skia::Paint::default();
        white.set_color_rgba8(255, 255, 255, 255);
        pixmap.fill_path(
            &path,
            &white,
            tiny_skia::FillRule::Winding,
            tiny_skia::Transform::identity(),
            None,
        );
    }

    // 组合串下划线（IME 候选标记，贴在光标行文本下方）
    if !edit.comp.is_empty() {
        let (cw, _) = render::text::measure(&edit.comp.iter().collect::<String>(), &edit.spec());
        let x0 = edit.pos.x + edit.caret_x() - cw;
        let y = edit.pos.y + (edit.caret_row() + 1) as f32 * edit.line_height() + pad * 0.3;
        if let Some(rect) = tiny_skia::Rect::from_xywh(x0, y, cw.max(1.0), 1.0) {
            let mut pb = tiny_skia::PathBuilder::new();
            pb.push_rect(rect);
            if let Some(path) = pb.finish() {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(255, 255, 255, 180);
                pixmap.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    tiny_skia::Transform::identity(),
                    None,
                );
            }
        }
    }
}

/// 光标竖线路径（四周外扩 1px 供描边）。
fn caret_path(rect: tiny_skia::Rect) -> Option<tiny_skia::Path> {
    let mut pb = tiny_skia::PathBuilder::new();
    pb.push_rect(tiny_skia::Rect::from_ltrb(
        rect.left() - 1.0,
        rect.top() - 1.0,
        rect.right() + 1.0,
        rect.bottom() + 1.0,
    )?);
    pb.finish()
}
