// SPDX-License-Identifier: GPL-3.0-only
//! 矩形几何（CAP-2）：`SelRect` 与交集/并集/外扩等运算，供选区、脏区与工具栏共用。
//! 界面度量（信息文字块与间隔）以 1080p / 100% DPI 为基准，经 `Ui` 缩放（CAP-5）。

use super::ui::Ui;

/// 信息文字高度（`text_rect` 与工具栏布局共用）。
/// 字号 16px + 上下各 6px 内缩；工具栏单行是 34，两者不强求等高。
pub(super) fn info_text_h(ui: Ui) -> i32 {
    ui.px(28.0)
}

/// 信息文字字号（DIP，`render::text` 的 `FontSpec::size`；`SetPixelsPerDip(1.0)` 下 1 DIP = 1 物理像素）。
/// 原 `DEFAULT_GUI_FONT` 只有 11px，2560×1440 下偏小；22px 又太大。
pub(super) fn info_font_size(ui: Ui) -> f32 {
    ui.f(16.0)
}

/// 信息文字相对信息块的左侧内缩。
pub(super) fn info_pad_x(ui: Ui) -> i32 {
    ui.px(6.0)
}

/// 信息文字/工具栏与选区的间隔。
pub(super) fn info_offset(ui: Ui) -> i32 {
    ui.px(8.0)
}

/// 矩形选区（相对虚拟屏幕左上角的物理像素坐标）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl SelRect {
    /// 保证 w/h 非负。
    pub(super) fn normalized(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        let x = x0.min(x1);
        let y = y0.min(y1);
        SelRect {
            x,
            y,
            w: x0.max(x1) - x,
            h: y0.max(y1) - y,
        }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// 点是否在矩形内（半开区间）。
pub fn in_rect(r: SelRect, x: i32, y: i32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

/// 两矩形交集；不相交时返回 None。
pub(super) fn intersect_rect(a: SelRect, b: SelRect) -> Option<SelRect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        None
    } else {
        Some(SelRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }
}

/// 两可选矩形的并集。
pub(super) fn union_rect(a: Option<SelRect>, b: Option<SelRect>) -> Option<SelRect> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let x0 = a.x.min(b.x);
            let y0 = a.y.min(b.y);
            let x1 = (a.x + a.w).max(b.x + b.w);
            let y1 = (a.y + a.h).max(b.y + b.h);
            Some(SelRect {
                x: x0,
                y: y0,
                w: x1 - x0,
                h: y1 - y0,
            })
        }
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// 矩形外扩 pad 像素。
pub(super) fn expand_rect(r: SelRect, pad: i32) -> SelRect {
    SelRect {
        x: r.x - pad,
        y: r.y - pad,
        w: r.w + pad * 2,
        h: r.h + pad * 2,
    }
}

/// 两个可选矩形是否相同。
pub(super) fn same_rect(a: Option<SelRect>, b: Option<SelRect>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.x == y.x && x.y == y.y && x.w == y.w && x.h == y.h,
        _ => false,
    }
}

/// 信息文字是否放在选区下方（工具栏与文字共用同一决策，避免两者重叠）。
///
/// 默认沿用旧规则：选区上方容得下一个文字块就放上方，否则放下方。
/// 唯一例外：文字在上方、但工具栏下方放不下又无法翻到文字之上时，把文字让到下方，
/// 让工具栏贴屏幕顶部——否则两者会叠在一起（小屏 + 大选区 + 两行工具栏）。
pub(super) fn info_text_below(sel: SelRect, screen_h: i32, toolbar_h: i32, ui: Ui) -> bool {
    let (text_h, offset) = (info_text_h(ui), info_offset(ui));
    if sel.y <= text_h + offset {
        return true;
    }
    let bar_below_fits = sel.y + sel.h + offset + toolbar_h + 4 <= screen_h;
    let bar_above_text_fits = sel.y - text_h - offset - offset - toolbar_h >= 2;
    !(bar_below_fits || bar_above_text_fits)
}

/// 估算信息文本占位矩形（供脏区域与工具栏避让使用，宽度保守）。
pub(super) fn text_rect(sel: SelRect, screen_w: i32, screen_h: i32, toolbar_h: i32, ui: Ui) -> SelRect {
    let (text_h, offset, pad_x) = (info_text_h(ui), info_offset(ui), info_pad_x(ui));
    let len = format!("{} × {}", sel.w, sel.h).chars().count() as i32;
    // 16px Segoe UI Semibold 下「1173 × 801」实测 92px，按 10px/字符估仍偏保守。
    let tw = (len * ui.px(10.0) + pad_x * 2 + 4).min(screen_w.max(0));
    let ty = if info_text_below(sel, screen_h, toolbar_h, ui) {
        sel.y + sel.h + offset
    } else {
        sel.y - text_h - offset
    };
    SelRect {
        x: sel.x.min((screen_w - tw - 8).max(4)).max(4),
        y: ty,
        w: tw,
        h: text_h,
    }
}
