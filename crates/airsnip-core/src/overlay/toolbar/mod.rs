// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层工具栏（EDT-7）：布局与命中测试；绘制见 `draw` 子模块。
//! 所有尺寸以 1080p / 100% DPI 为基准，经 `Ui` 按显示器 DPI 缩放（CAP-5）。

mod draw;
pub(crate) mod fontlist;
mod label;

use std::sync::Arc;

use tiny_skia::Pixmap;

use crate::editor::{PALETTE, Tool};
use crate::overlay::geometry::{SelRect, info_offset, info_text_below, info_text_h};
use crate::overlay::ui::Ui;
use crate::theme::Palette;

/// 第一行高度（工具 / 颜色 / 开关 / 输出按钮）。
const H_BASE: f32 = 34.0;
/// 文本第二行高度（EDT-4：字体族 / 粗体 / 斜体 / 字号）。
const H2_BASE: f32 = 30.0;
/// 两行之间的间距。
const ROW_GAP_BASE: f32 = 3.0;
/// 字体族按钮宽度（超出时截断加省略号）。
pub(super) const FONT_BTN_W_BASE: f32 = 132.0;
/// 字号显示格宽度。
const SIZE_W_BASE: f32 = 34.0;
/// 方形按钮边长（工具、撤销、开关）。
const BTN_BASE: f32 = 26.0;
/// 色块边长。
const SW_BASE: f32 = 22.0;
/// 元素间距。
const GAP_BASE: f32 = 4.0;
/// 组分隔区宽度。
const SEP_BASE: f32 = 9.0;
/// 内边距。
const PAD_BASE: f32 = 5.0;

/// 工具栏当前状态（决定高亮与可用性）。
#[derive(Clone, PartialEq)]
pub struct State {
    pub tool: Tool,
    pub color: usize,
    /// 矩形是否填充（EDT-1）。
    pub filled: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// 第二行（EDT-4）：当前字体族、粗体/斜体、字号与字体面板是否展开。
    pub font_family: Arc<str>,
    pub bold: bool,
    pub italic: bool,
    pub font_size: f32,
    pub font_open: bool,
}

/// 工具栏动作。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Tool(Tool),
    Color(usize),
    ToggleFill,
    Undo,
    Redo,
    /// PIN-1：把选区（含标注）贴为置顶窗口。
    Pin,
    /// M4：把当前选区作为滚动视口，进入长截图拼接（长截图键同效，默认 S）。
    Longshot,
    /// OUT-1：复制选区到剪贴板（Enter / Ctrl+C 同效）。
    Copy,
    /// OUT-2：按配置的目录与格式保存（Ctrl+S 同效）。
    Save,
    /// EDT-4：展开/收起字体列表面板。
    FontPick,
    /// EDT-4：粗体开关。
    ToggleBold,
    /// EDT-4：斜体开关。
    ToggleItalic,
    /// EDT-4：字号减小 / 增大（每步 2px）。
    FontSmaller,
    FontLarger,
    /// EDT-4：字号数值格（仅展示，不响应点击）。
    FontSize,
}

/// 工具栏布局：整体矩形（含第二行）+ 各元素命中区。
#[derive(Clone, Debug)]
pub struct Toolbar {
    /// 整个工具栏（两行的并集，绘制背景与脏区共用）。
    pub rect: SelRect,
    /// 第一行矩形。
    pub first: SelRect,
    /// 第二行矩形（仅文本工具 / 选中文本对象时存在）。
    pub second: Option<SelRect>,
    /// 界面缩放：绘制时的圆角、内缩、分隔线与字号按它缩放（CAP-5）。
    pub ui: Ui,
    /// 元素间距（分隔线定位用）。
    pub(crate) gap: i32,
    /// 命中区（draw 子模块读取所有元素）。
    pub(crate) hits: Vec<(SelRect, Action)>,
}

impl Toolbar {
    /// 命中测试：返回落在哪个元素上。
    pub fn hit(&self, x: i32, y: i32) -> Option<Action> {
        self.hits
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|(_, a)| *a)
    }
}

/// 布局游标：依次排布元素，返回元素左边界。
struct Cursor {
    x: i32,
    gap: i32,
    sep: i32,
}

impl Cursor {
    fn next(&mut self, w: i32) -> i32 {
        let x = self.x;
        self.x += w + self.gap;
        x
    }

    fn sep(&mut self) {
        self.x += self.sep - self.gap;
    }
}

/// 计算工具栏布局：优先放在选区下方，空间不足则放上方。
/// `text_row` 为真时附上第二行（文本工具或选中文本对象，EDT-4）。
pub fn layout(sel: SelRect, screen_w: i32, screen_h: i32, text_row: bool, ui: Ui) -> Toolbar {
    let (row_h, h2) = (ui.px(H_BASE), ui.px(H2_BASE));
    let (btn, sw) = (ui.px(BTN_BASE), ui.px(SW_BASE));
    let (gap, sep, pad) = (ui.px(GAP_BASE), ui.px(SEP_BASE), ui.px(PAD_BASE));
    let row_gap = ui.px(ROW_GAP_BASE);
    let margin = ui.px(4.0);

    let mut c = Cursor { x: pad, gap, sep };
    let mut hits: Vec<(SelRect, Action)> = Vec::new();
    let mut item = |c: &mut Cursor, w: i32, item_h: i32, action: Action| {
        let x = c.next(w);
        let y = (row_h - item_h) / 2;
        hits.push((SelRect { x, y, w, h: item_h }, action));
    };

    for tool in Tool::ALL {
        item(&mut c, btn, btn, Action::Tool(tool));
    }
    c.sep();
    for i in 0..PALETTE.len() {
        item(&mut c, sw, sw, Action::Color(i));
    }
    c.sep();
    item(&mut c, btn, btn, Action::ToggleFill);
    c.sep();
    item(&mut c, btn, btn, Action::Undo);
    item(&mut c, btn, btn, Action::Redo);
    c.sep();
    item(&mut c, btn, btn, Action::Pin);
    item(&mut c, btn, btn, Action::Longshot);
    c.sep();
    item(&mut c, btn, btn, Action::Copy);
    item(&mut c, btn, btn, Action::Save);
    let w1 = c.x - gap + pad;

    // 第二行（EDT-4）：字体族按钮 + B / I + 字号 − 数值 +
    let mut c2 = Cursor { x: pad, gap, sep };
    let mut hits2: Vec<(SelRect, Action)> = Vec::new();
    let mut item2 = |c: &mut Cursor, w: i32, action: Action| {
        let x = c.next(w);
        let y = (h2 - btn) / 2;
        hits2.push((SelRect { x, y, w, h: btn }, action));
    };
    if text_row {
        item2(&mut c2, ui.px(FONT_BTN_W_BASE), Action::FontPick);
        c2.sep();
        item2(&mut c2, btn, Action::ToggleBold);
        item2(&mut c2, btn, Action::ToggleItalic);
        c2.sep();
        item2(&mut c2, btn, Action::FontSmaller);
        item2(&mut c2, ui.px(SIZE_W_BASE), Action::FontSize);
        item2(&mut c2, btn, Action::FontLarger);
    }
    let w2 = if text_row { c2.x - gap + pad } else { 0 };

    let w = w1.max(w2);
    let total_h = if text_row { row_h + row_gap + h2 } else { row_h };
    let x = sel.x.clamp(margin, (screen_w - w - margin).max(margin));
    // 信息文字占位（见 overlay::text_rect）：默认贴选区下方，与文字同侧时再让出一个文字高度；
    // 下方塞不下就翻到上方，且避开文字块（见 geometry::info_text_below）。
    let text_below = info_text_below(sel, screen_h, total_h, ui);
    let offset = info_offset(ui);
    let below = sel.y + sel.h + offset + if text_below { info_text_h(ui) } else { 0 };
    let y = if below + total_h + margin <= screen_h {
        below
    } else if text_below {
        (sel.y - offset - total_h).max(ui.px(2.0))
    } else {
        (sel.y - info_text_h(ui) - offset - offset - total_h).max(ui.px(2.0))
    };
    let first = SelRect { x, y, w: w1, h: row_h };
    let second = text_row.then(|| SelRect {
        x,
        y: y + row_h + row_gap,
        w: w2,
        h: h2,
    });
    let rect = SelRect { x, y, w, h: total_h };

    // 相对坐标 → 屏幕坐标
    let offset = |r: SelRect, dy: i32| SelRect {
        x: r.x + x,
        y: r.y + y + dy,
        w: r.w,
        h: r.h,
    };
    let hits = hits
        .into_iter()
        .map(|(r, a)| (offset(r, 0), a))
        .chain(hits2.into_iter().map(|(r, a)| (offset(r, row_h + row_gap), a)));
    Toolbar {
        rect,
        first,
        second,
        ui,
        gap,
        hits: hits.collect(),
    }
}

/// 把工具栏绘制到覆盖层像素图（委托给 draw 子模块）。
pub fn draw(pixmap: &mut Pixmap, bar: &Toolbar, state: &State, palette: &Palette) {
    draw::render(pixmap, bar, state, palette);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::overlay::geometry::{intersect_rect, text_rect};

    /// 基准缩放（100% DPI）：尺寸与改动前的硬编码值一致。
    fn base() -> Ui {
        Ui::from_dpi(96)
    }

    fn sel(x: i32, y: i32, w: i32, h: i32) -> SelRect {
        SelRect { x, y, w, h }
    }

    /// 工具栏默认贴在选区下方（上下都有空间时不能跑到上方）。
    #[test]
    fn toolbar_defaults_below_selection() {
        let (vw, vh) = (2560, 1440);
        for text_row in [false, true] {
            for s in [sel(600, 500, 800, 400), sel(600, 60, 800, 400)] {
                let bar = layout(s, vw, vh, text_row, base());
                assert!(
                    bar.rect.y >= s.y + s.h,
                    "工具栏应在选区下方（text_row={text_row}, 选区 {s:?}）：{bar:?}"
                );
            }
        }
    }

    /// 工具栏不能与信息文字重叠，否则文字会盖在工具栏上（见 overlay::text_rect）。
    #[test]
    fn toolbar_never_overlaps_info_text() {
        let (vw, vh) = (2560, 1440);
        let cases = [
            sel(100, 100, 400, 300),   // 常规：文字在上、工具栏在下
            sel(100, 4, 400, 300),     // 贴顶：文字在下，工具栏再往下让一个文字高度
            sel(100, 100, 900, 1300),  // 贴底：工具栏翻到文字上方
            sel(100, 4, 900, 1420),    // 贴顶贴底：工具栏翻到上方、文字在下方
            sel(2400, 100, 150, 1200), // 靠右：工具栏贴右边
        ];
        for ui in [base(), Ui::from_dpi(192)] {
            for s in cases {
                let bar = layout(s, vw, vh, false, ui).rect;
                let text = text_rect(s, vw, vh, bar.h, ui);
                assert!(
                    intersect_rect(bar, text).is_none(),
                    "工具栏 {bar:?} 与信息文字 {text:?} 重叠（选区 {s:?}）"
                );
                assert!(bar.y >= 0 && bar.y + bar.h <= vh, "工具栏应在屏幕内：{bar:?}");
                assert!(bar.x >= 0 && bar.x + bar.w <= vw, "工具栏应在屏幕内：{bar:?}");
            }
        }
    }

    /// 第二行（EDT-4）同样不能与信息文字重叠，且两行不越出屏幕。
    #[test]
    fn text_row_stays_inside_screen() {
        let (vw, vh) = (1366, 768);
        let cases = [
            sel(100, 100, 400, 300),
            sel(100, 4, 400, 300),
            sel(100, 100, 900, 600),
            sel(100, 4, 900, 700),
            sel(1000, 300, 300, 400),
        ];
        for ui in [base(), Ui::from_dpi(192)] {
            for s in cases {
                let bar = layout(s, vw, vh, true, ui);
                let text = text_rect(s, vw, vh, bar.rect.h, ui);
                assert!(
                    intersect_rect(bar.rect, text).is_none(),
                    "工具栏 {bar:?} 与信息文字 {text:?} 重叠"
                );
                let second = bar.second.expect("文本工具应有第二行");
                assert_eq!(second.y, bar.first.y + bar.first.h + ui.px(ROW_GAP_BASE));
                assert!(
                    bar.rect.y >= 0 && bar.rect.y + bar.rect.h <= vh,
                    "工具栏应在屏幕内：{bar:?}"
                );
                assert!(
                    bar.rect.x >= 0 && bar.rect.x + bar.rect.w <= vw,
                    "工具栏应在屏幕内：{bar:?}"
                );
                // 第二行元素全在第二行矩形内
                for (r, _) in bar.hits.iter().filter(|(r, _)| r.y >= second.y) {
                    assert!(
                        r.x >= second.x && r.x + r.w <= second.x + second.w,
                        "第二行元素 {r:?} 超出第二行 {second:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn toolbar_hit_maps_each_element() {
        let bar = layout(sel(100, 100, 400, 300), 2560, 1440, true, base());
        // 每个元素的中心点必须命中它自己
        for (r, action) in &bar.hits {
            let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
            assert_eq!(bar.hit(cx, cy), Some(*action), "元素 {action:?} 中心未命中自己");
        }
        // 元素不重叠（命中区连续递增；第二行另起一排列，只比较同行）
        let second_y = bar.second.expect("应有第二行").y;
        for w in bar.hits.windows(2) {
            if w[0].0.y >= second_y && w[1].0.y >= second_y {
                assert!(w[0].0.x + w[0].0.w <= w[1].0.x, "元素重叠：{:?} / {:?}", w[0].0, w[1].0);
            }
        }
        // 元素顺序与数量（EDT-7 工具栏结构 + EDT-4 第二行）
        let actions: Vec<Action> = bar.hits.iter().map(|(_, a)| *a).collect();
        assert_eq!(actions.len(), 24);
        assert_eq!(actions[..6], Tool::ALL.map(Action::Tool));
        assert_eq!(actions[12], Action::Undo);
        assert_eq!(actions[13], Action::Redo);
        assert_eq!(actions[14], Action::Pin);
        assert_eq!(actions[15], Action::Longshot);
        assert_eq!(actions[16], Action::Copy);
        assert_eq!(actions[17], Action::Save);
        assert_eq!(
            actions[18..],
            [
                Action::FontPick,
                Action::ToggleBold,
                Action::ToggleItalic,
                Action::FontSmaller,
                Action::FontSize,
                Action::FontLarger
            ]
        );
    }

    /// 非文本场景第二行不应出现（避免占地与多余命中区）。
    #[test]
    fn second_row_only_when_text_row_requested() {
        let bar = layout(sel(100, 100, 400, 300), 2560, 1440, false, base());
        assert!(bar.second.is_none());
        assert_eq!(bar.rect.h, base().px(H_BASE));
        assert_eq!(bar.hits.len(), 18);
    }

    /// 高 DPI（200%）下工具栏整体尺寸翻倍：4K 屏上控件不再只有一半大（CAP-5）。
    #[test]
    fn toolbar_scales_with_dpi() {
        let s = sel(600, 500, 800, 400);
        let base_bar = layout(s, 2560, 1440, true, base());
        let hidpi = layout(s, 3840, 2160, true, Ui::from_dpi(192));
        assert_eq!(hidpi.first.h, base_bar.first.h * 2, "第一行高度应翻倍");
        assert_eq!(hidpi.rect.h, base_bar.rect.h * 2, "整体高度应翻倍");
        assert_eq!(hidpi.rect.w, base_bar.rect.w * 2, "整体宽度应翻倍");
        // 每个命中区同样翻倍（按钮仍可点，且命中范围与绘制一致）
        for (a, b) in base_bar.hits.iter().zip(hidpi.hits.iter()) {
            assert_eq!(a.1, b.1, "元素顺序应一致");
            assert_eq!(a.0.w * 2, b.0.w, "元素宽度应翻倍：{:?}", a.1);
            assert_eq!(a.0.h * 2, b.0.h, "元素高度应翻倍：{:?}", a.1);
        }
    }
}
