// SPDX-License-Identifier: GPL-3.0-only
//! 文本编辑状态（EDT-4）：「输入中」的纯数据与字符级编辑逻辑。
//! IME 消息的读取与提交见 `textinput`（涉及 HWND/消息循环）。

use crate::editor::Point;
use crate::render;

use super::geometry::SelRect;

/// 输入中状态（EDT-4）。
pub(super) struct TextEdit {
    /// 文字左上角（覆盖层坐标，不含内边距）。
    pub pos: Point,
    /// 编辑已有对象的索引；None 表示新建。
    pub obj_index: Option<usize>,
    /// 已确认文本（字符单位）。
    pub chars: Vec<char>,
    /// IME 组合串（候选预览，显示在光标处）。
    pub comp: Vec<char>,
    /// 光标位置（`chars` 中的索引）。
    pub caret: usize,
    /// 正在积攒的高位代理项（WM_CHAR 补充平面字符）。
    pub pending_high: Option<u16>,
    /// 字号（物理像素）。
    pub font_size: f32,
    /// 字体族名、粗体、斜体（EDT-4）。
    pub font_family: std::sync::Arc<str>,
    pub bold: bool,
    pub italic: bool,
}

impl TextEdit {
    /// 渲染用字体规格（EDT-4）。
    pub fn spec(&self) -> render::text::FontSpec {
        render::text::FontSpec::from_parts(&self.font_family, self.font_size, self.bold, self.italic)
    }

    /// 完整显示文本（已确认 + 组合串）。
    pub fn display(&self) -> String {
        self.chars.iter().chain(self.comp.iter()).collect()
    }

    /// 光标所在 x 偏移（相对文字左上角）：光标行内的前缀宽度；
    /// IME 组合时光标贴在组合串末尾（组合仅在光标行内）。
    pub fn caret_x(&self) -> f32 {
        let pre: String = self.chars[..self.caret].iter().collect();
        // 多行：只取光标所在行（最后一个换行符之后）的前缀
        let line = match pre.rsplit_once('\n') {
            Some((_, after)) => after,
            None => pre.as_str(),
        };
        // 空行行首 x 精确为 0（measure 对空串有最小宽度保护）
        let (w, _) = if line.is_empty() {
            (0.0, 0.0)
        } else {
            render::text::measure(line, &self.spec())
        };
        let comp_w = if self.caret == self.chars.len() {
            let (cw, _) = render::text::measure(&self.comp.iter().collect::<String>(), &self.spec());
            cw
        } else {
            0.0
        };
        w + comp_w
    }

    /// 光标竖线 (x, y, 高)，像素：绘制光标与 macOS 输入法候选框定位共用。
    pub fn caret_rect(&self) -> (f32, f32, f32) {
        let pad = crate::editor::text::pad(self.font_size);
        let x = (self.pos.x + self.caret_x()).round();
        let y = self.pos.y + self.caret_row() as f32 * self.line_height() + pad * 0.4;
        (x, y, self.font_size * 1.1)
    }

    /// 光标所在行号（chars[..caret] 中的换行次数）。
    pub fn caret_row(&self) -> usize {
        self.chars[..self.caret].iter().filter(|&&c| c == '\n').count()
    }

    /// 单行高度（测量空串，渲染与光标共用）。
    pub fn line_height(&self) -> f32 {
        let (_, h) = render::text::measure("", &self.spec());
        h.max(self.font_size * 1.2)
    }

    /// 输入框占据的像素区域（含内边距，供脏区与绘制使用）。
    pub fn region(&self) -> SelRect {
        let (w, h) = render::text::measure(&self.display(), &self.spec());
        let h = h.max(self.line_height());
        let pad = crate::editor::text::pad(self.font_size);
        let x0 = (self.pos.x - pad).floor() as i32;
        let y0 = (self.pos.y - pad).floor() as i32;
        SelRect {
            x: x0,
            y: y0,
            w: (self.pos.x + w + pad).ceil() as i32 - x0,
            h: (self.pos.y + h + pad).ceil() as i32 - y0,
        }
    }

    /// WM_CHAR：单个 UTF-16 码元入缓冲（补充平面字符按代理对组合）。
    pub fn push_code_unit(&mut self, unit: u16) {
        if let Some(high) = self.pending_high.take() {
            if let Some(c) = char::from_u32(((high as u32) << 10) + (unit as u32) + 0x10000 - (0xD800 << 10) - 0xDC00) {
                self.insert_char(c);
                return;
            }
        }
        if (0xD800..=0xDBFF).contains(&unit) {
            self.pending_high = Some(unit);
            return;
        }
        if let Some(c) = char::from_u32(unit as u32) {
            self.insert_char(c);
        }
    }

    /// 在光标处插入字符。
    pub fn insert_char(&mut self, c: char) {
        self.chars.insert(self.caret, c);
        self.caret += 1;
    }

    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.caret < self.chars.len() {
            self.chars.remove(self.caret);
        }
    }

    pub fn move_caret(&mut self, delta: i32) {
        self.caret = (self.caret as i32 + delta).clamp(0, self.chars.len() as i32) as usize;
    }

    /// IME 组合串更新（GCS_COMPSTR）。
    pub fn set_composition(&mut self, comp: String) {
        self.comp = comp.chars().collect();
    }

    /// IME 结果串（GCS_RESULTSTR）：在光标处插入结果字符并清空组合预览。
    pub fn commit_composition(&mut self, result: String) {
        let out: Vec<char> = result.chars().collect();
        let start = self.caret;
        self.chars.splice(start..start, out.iter().copied());
        self.caret = start + out.len();
        self.comp.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit() -> TextEdit {
        TextEdit {
            pos: Point::new(10.0, 10.0),
            obj_index: None,
            chars: "你好".chars().collect(),
            comp: Vec::new(),
            caret: 2,
            pending_high: None,
            font_size: 24.0,
            font_family: std::sync::Arc::from(crate::editor::DEFAULT_FONT_FAMILY),
            bold: false,
            italic: false,
        }
    }

    #[test]
    fn insert_and_move_caret() {
        let mut e = edit();
        e.insert_char('a');
        assert_eq!(e.chars.iter().collect::<String>(), "你好a");
        assert_eq!(e.caret, 3);
        e.move_caret(-2);
        e.insert_char('b');
        assert_eq!(e.chars.iter().collect::<String>(), "你b好a");
        e.move_caret(99);
        assert_eq!(e.caret, e.chars.len());
        e.move_caret(-99);
        assert_eq!(e.caret, 0);
    }

    #[test]
    fn backspace_and_delete() {
        let mut e = edit();
        e.caret = 1;
        e.backspace();
        assert_eq!(e.chars.iter().collect::<String>(), "好");
        assert_eq!(e.caret, 0);
        e.insert_char('x');
        e.delete_forward();
        assert_eq!(e.chars.iter().collect::<String>(), "x");
    }

    #[test]
    fn composition_commit_inserts_result_at_caret() {
        let mut e = edit();
        e.caret = 2;
        e.set_composition("nihao".to_string());
        assert_eq!(e.display(), "你好nihao");
        e.commit_composition("你".to_string());
        assert_eq!(e.chars.iter().collect::<String>(), "你好你");
        assert!(e.comp.is_empty());
        assert_eq!(e.caret, 3);
    }

    #[test]
    fn surrogate_pair_assembles_emoji() {
        let mut e = edit();
        e.push_code_unit(0xD83D); // 🙏 高位代理
        e.push_code_unit(0xDE4F); // 低位代理
        assert_eq!(e.chars.last(), Some(&'🙏'));
        assert_eq!(e.caret, 3);
    }

    #[test]
    fn region_covers_text_with_padding() {
        let r = edit().region();
        assert!(r.w >= 24 && r.h >= 24, "输入框区域应覆盖文字：{r:?}");
        assert!(r.x <= 10 && r.y <= 10);
    }

    #[test]
    fn multiline_insert_and_caret_positioning() {
        let mut e = edit(); // "你好" caret=2
        e.insert_char('\n'); // "你好\n" caret=3
        assert_eq!(e.caret_row(), 1, "换行后光标应在第二行");
        e.insert_char('x'); // caret=4
        e.insert_char('y'); // caret=5
        e.move_caret(-2); // caret=3，位于换行符之后（第二行行首）
        assert_eq!(e.caret_row(), 1);
        e.move_caret(-1); // caret=2（跨过换行符回到第一行末尾）
        assert_eq!(e.caret_row(), 0);
        e.move_caret(1); // caret=3（回到换行符之后），退格删除换行符
        e.backspace();
        assert_eq!(e.chars.iter().collect::<String>(), "你好xy");
        assert_eq!(e.caret_row(), 0, "删除换行后回到第一行");
    }

    #[test]
    fn multiline_region_is_taller() {
        let single = edit();
        let mut multi = edit();
        multi.insert_char('\n');
        multi.insert_char('a');
        multi.insert_char('b');
        let h_single = single.region().h;
        let h_multi = multi.region().h;
        assert!(h_multi > h_single, "多行输入框应更高：{h_single} vs {h_multi}");
    }

    #[test]
    fn multiline_caret_x_resets_at_line_start() {
        let mut multi = edit();
        multi.insert_char('\n'); // caret=3
        multi.insert_char('a'); // caret=4
        multi.insert_char('b'); // caret=5
        let x_mid = multi.caret_x();
        assert!(x_mid > 5.0, "第二行有字符时光标 x 应大于 0：{x_mid}");
        multi.move_caret(-2); // 回到第二行行首
        assert!(
            multi.caret_x() < 1.0,
            "第二行行首 x 应≈0（不累计第一行宽度）：{}",
            multi.caret_x()
        );
    }
}
