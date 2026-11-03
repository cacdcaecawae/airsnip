// SPDX-License-Identifier: GPL-3.0-only
//! 工具栏绘制：把工具栏渲染到覆盖层像素图。
//! 按钮图标统一由 `crate::render::icon` 绘制（资源见 `assets/icons/ui/`），
//! 这里只负责背景、选中态与文字类控件。

use tiny_skia::Pixmap;

use crate::editor::{PALETTE, Tool};
use crate::overlay::SelRect;
use crate::overlay::shape::{rounded_rect, rounded_rect_outline};
use crate::overlay::toolbar::{Action, State, Toolbar};
use crate::overlay::ui::Ui;
use crate::render::icon::{self, Icon};
use crate::render::text::FontSpec;
use crate::theme::Palette;

use super::label::{LABEL_SIZE_BASE, centered_text, divider, fit_name, left_text};

/// 绘制工具栏到像素图。
pub fn render(pixmap: &mut Pixmap, bar: &Toolbar, state: &State, p: &Palette) {
    let ui = bar.ui;
    let gap = bar.gap;
    let first_h = bar.first.h;
    rounded_rect(pixmap, bar.rect, ui.f(7.0), p.bg, Some(p.border));

    // 组分隔线：画在每个组首个元素左侧的空白处
    // 组划分（hits 索引）：[工具 0-5][颜色 6-10][填充 11][撤销 12 重做 13]
    // [贴图 14 长截图 15][复制 16 保存 17]
    for &index in &[6usize, 11, 12, 14, 16] {
        let Some((first, _)) = bar.hits.get(index) else {
            continue;
        };
        divider(
            pixmap,
            first.x - gap / 2 - 1,
            bar.first.y + ui.px(7.0),
            first_h - ui.px(14.0),
            p,
        );
    }
    // 第二行分隔线（EDT-4）：字体按钮之后、斜体重之后
    if let Some(second) = bar.second {
        for action in [Action::ToggleBold, Action::FontSmaller] {
            if let Some((r, _)) = bar.hits.iter().find(|(_, a)| *a == action) {
                divider(
                    pixmap,
                    r.x - gap / 2 - 1,
                    second.y + ui.px(7.0),
                    second.h - ui.px(14.0),
                    p,
                );
            }
        }
    }

    for (r, action) in &bar.hits {
        match *action {
            Action::Tool(tool) => {
                let active = tool == state.tool;
                if active {
                    rounded_rect(pixmap, *r, 5.0, p.accent_soft, None);
                }
                let color = if tool == Tool::Highlight {
                    // 荧光笔图标：颜色跟主题（浅色主题用暗金，否则看不清楚），激活时更浓
                    let c = p.highlight_ink;
                    [c[0], c[1], c[2], if active { 255 } else { c[3] }]
                } else if active {
                    p.icon
                } else {
                    p.icon_dim
                };
                draw_icon(pixmap, tool_icon(tool), *r, color);
            }
            Action::Color(i) => {
                let inset = ui.px(2.0);
                let swatch = SelRect {
                    x: r.x + inset,
                    y: r.y + inset,
                    w: r.w - inset * 2,
                    h: r.h - inset * 2,
                };
                rounded_rect(pixmap, swatch, ui.f(4.0), PALETTE[i], Some(p.swatch_border));
                if i == state.color {
                    rounded_rect_outline(pixmap, *r, ui.f(4.0), p.accent);
                }
            }
            // 文本工具无背景功能：填充开关置灰禁用
            Action::ToggleFill => {
                let enabled = state.tool != Tool::Text;
                if enabled && state.filled {
                    rounded_rect(pixmap, *r, ui.f(5.0), p.accent_soft, None);
                }
                draw_icon(pixmap, Icon::Fill, *r, toggle_color(enabled, state.filled, p));
            }
            Action::Undo => draw_icon(pixmap, Icon::Undo, *r, if state.can_undo { p.icon } else { p.icon_dim }),
            Action::Redo => draw_icon(pixmap, Icon::Redo, *r, if state.can_redo { p.icon } else { p.icon_dim }),
            Action::Pin => draw_icon(pixmap, Icon::Pin, *r, p.icon),
            Action::Longshot => draw_icon(pixmap, Icon::Longshot, *r, p.icon),
            Action::Copy => draw_icon(pixmap, Icon::Copy, *r, p.icon),
            Action::Save => draw_icon(pixmap, Icon::Save, *r, p.icon),
            // EDT-4：字体族按钮（展开/收起字体列表），名字过长截断加省略号
            Action::FontPick => {
                if state.font_open {
                    rounded_rect(pixmap, *r, ui.f(5.0), p.accent_soft, None);
                }
                let spec = FontSpec::new(ui.f(LABEL_SIZE_BASE));
                let color = if state.font_open { p.accent } else { p.text };
                let name = fit_name(&state.font_family, (r.w - ui.px(12.0)) as f32, &spec);
                left_text(pixmap, *r, ui.px(6.0), &name, &spec, color);
            }
            Action::ToggleBold => toggle_icon(pixmap, Icon::Bold, *r, state.bold, p, ui),
            Action::ToggleItalic => toggle_icon(pixmap, Icon::Italic, *r, state.italic, p, ui),
            Action::FontSmaller => draw_icon(pixmap, Icon::Minus, *r, p.icon),
            Action::FontLarger => draw_icon(pixmap, Icon::Plus, *r, p.icon),
            Action::FontSize => {
                let spec = FontSpec::new(ui.f(LABEL_SIZE_BASE));
                let text = format!("{}", state.font_size.round() as i32);
                centered_text(pixmap, *r, &text, &spec, p.text);
            }
        }
    }
}

/// 工具 → 图标。
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Rect => Icon::Rect,
        Tool::Arrow => Icon::Arrow,
        Tool::Text => Icon::Text,
        Tool::Highlight => Icon::Highlight,
        Tool::Blur => Icon::Blur,
        Tool::Glow => Icon::Glow,
    }
}

/// 在矩形中心按边长绘制图标（活动区由 `render::icon` 统一归一化）。
fn draw_icon(pixmap: &mut Pixmap, icon: Icon, r: SelRect, color: [u8; 4]) {
    let cx = r.x as f32 + r.w as f32 / 2.0;
    let cy = r.y as f32 + r.h as f32 / 2.0;
    icon::draw(pixmap, icon, cx, cy, r.w.min(r.h) as f32, color);
}

/// 开关类按钮的图标颜色：禁用置灰、开启用强调色、关闭用主色。
fn toggle_color(enabled: bool, on: bool, p: &Palette) -> [u8; 4] {
    if !enabled {
        p.icon_dim
    } else if on {
        p.accent
    } else {
        p.icon
    }
}

/// B / I 开关按钮：开启时强调底色 + 强调色图标。
fn toggle_icon(pixmap: &mut Pixmap, icon: Icon, r: SelRect, on: bool, p: &Palette, ui: Ui) {
    if on {
        rounded_rect(pixmap, r, ui.f(5.0), p.accent_soft, None);
    }
    draw_icon(pixmap, icon, r, if on { p.accent } else { p.icon });
}
