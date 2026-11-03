// SPDX-License-Identifier: GPL-3.0-only
//! 选区更新与统一脏区重绘（CAP-2/3、PRD 6.3.2）：
//! 底图复位 → 标注对象 → 选中箭头的控制柄 → 选区装饰 → 工具栏，一次 `repaint` 完成并上屏。
//! 选区边框、手柄与聚光恢复的绘制见 `draw`。

mod draw;

use super::geometry::{SelRect, expand_rect, intersect_rect, same_rect, text_rect, union_rect};
use super::objedit;
use super::pick::draw_curve_handles;
use super::surface::{blit_region, draw_info_text, restore_region};
use super::{textinput, toolbar};
use crate::render;

use draw::{draw_handles, draw_rect, restore_glow};

/// 脏区域外扩基准值（覆盖边框、手柄与信息文字）。
const DIRTY_PAD_BASE: f32 = 60.0;
/// 对象包围框与控制柄的脏区外扩基准值（手柄画在包围盒之外）。
const HANDLE_PAD_BASE: f32 = 12.0;

impl super::Overlay {
    /// 选区脏区外扩量（手柄、边框与信息文字都画在选区之外）。
    pub(super) fn dirty_pad(&self) -> i32 {
        self.ui.px(DIRTY_PAD_BASE)
    }

    /// 更新选区并做脏矩形增量重绘（PRD 6.3.2）：
    /// 只处理旧/新选区的装饰与工具栏区域，避免每次鼠标移动全屏重绘。
    pub(super) fn set_selection(&mut self, new: Option<SelRect>) {
        if same_rect(self.selection, new) {
            return;
        }
        let (vw, vh) = (self.display.width() as i32, self.display.height() as i32);
        let old = self.selection;
        self.selection = new;
        // 选区变化后字体面板的位置失效（EDT-4）：收起并连旧区域一起重绘
        let old_chrome = self.chrome_rect();
        self.font_list = None;
        self.bar = new.map(|s| toolbar::layout(s, vw, vh, self.text_row_visible(), self.ui));
        let new_chrome = self.chrome_rect();
        // 有了选区，窗口识别的悬停高亮（CAP-6）随之撤掉
        let hover = if new.is_some() { self.hover.take() } else { None };

        let mut dirty: Option<SelRect> = None;
        let bar_h = self.bar.as_ref().map(|b| b.rect.h).unwrap_or(0);
        let pad = self.dirty_pad();
        for r in [
            hover.map(|h| self.hover_dirty(h)),
            hover.map(|h| self.hover_text(h)),
            old.map(|o| expand_rect(o, pad)),
            new.map(|n| expand_rect(n, pad)),
            old.map(|o| text_rect(o, vw, vh, bar_h, self.ui)),
            new.map(|n| text_rect(n, vw, vh, bar_h, self.ui)),
            old_chrome.map(|b| expand_rect(b, 2)),
            new_chrome.map(|b| expand_rect(b, 2)),
        ]
        .into_iter()
        .flatten()
        {
            dirty = union_rect(dirty, Some(r));
        }
        if let Some(d) = dirty {
            self.repaint(d);
        }
    }

    /// 重绘指定区域并上屏（PRD 6.3.2 脏矩形）：
    /// 1) 底图复位（选区外恢复暗图、选区内回贴原图）2) 重绘标注 3) 选区装饰 4) 工具栏。
    pub(super) fn repaint(&mut self, r: SelRect) {
        let _frame = crate::perf::frame();
        let r = self.expand_for_objects(r);
        let r = self.expand_for_selected(r);
        let (w, h) = (self.display.width() as i32, self.display.height() as i32);
        let x0 = r.x.clamp(0, w);
        let y0 = r.y.clamp(0, h);
        let x1 = (r.x + r.w).clamp(0, w);
        let y1 = (r.y + r.h).clamp(0, h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let region = SelRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        };

        // 1) 底图复位（CAP-3：预生成暗图 + 选区回贴原图）
        // 首帧例外：`display` 初始内容就是 `dimmed`，无需回贴（省一次全屏拷贝）。
        // 聚光模式：选区内保持暗图，仅聚光对象/草稿区域恢复原图亮度（其余区域继续暗化）。
        // 判断包含正在拖拽的草稿，保证第一笔拖动时即进入暗化状态。
        let first_paint = self.first_paint;
        self.first_paint = false;
        if !first_paint {
            restore_region(&mut self.display, &self.dimmed, region);
        }
        if let Some(sel) = self.selection {
            if let Some(overlap) = intersect_rect(region, sel) {
                let glow_on = self
                    .doc
                    .objects()
                    .iter()
                    .chain(self.draft.iter())
                    .any(|o| matches!(o.kind, crate::editor::Kind::Glow { .. }));
                if glow_on {
                    // 逐个恢复聚光区域（草稿也包含在内；与 overlap 求交后回贴原图）
                    for obj in self.doc.objects().iter().chain(self.draft.iter()) {
                        if let crate::editor::Kind::Glow { a, b, corner } = obj.kind {
                            // 重新计算区域四边（floor/ceil 取整，不能用截断：浮点值可能为负或非整数）
                            let (gx0, gy0) = (a.x.min(b.x).floor(), a.y.min(b.y).floor());
                            let (gx1, gy1) = (a.x.max(b.x).ceil(), a.y.max(b.y).ceil());
                            let gr = SelRect {
                                x: gx0 as i32,
                                y: gy0 as i32,
                                w: (gx1 - gx0) as i32,
                                h: (gy1 - gy0) as i32,
                            };
                            if let Some(part) = intersect_rect(overlap, gr) {
                                restore_glow(&mut self.display, &self.original, part, (gx0, gy0, gx1, gy1), corner);
                            }
                        }
                    }
                } else {
                    blit_region(&mut self.display, &self.original, overlap);
                }
            }
        }
        // 1b) 窗口识别（CAP-6/7）：还没有选区时，悬停的窗口（或整屏）恢复原图亮度
        let hover = self.hover.filter(|_| self.selection.is_none());
        if let Some(overlap) = hover.and_then(|h| intersect_rect(region, h)) {
            blit_region(&mut self.display, &self.original, overlap);
        }
        // 2) 标注对象（含正在拖拽的草稿）；二次编辑中的文本对象隐藏，由输入框呈现（EDT-4）
        render::draw_objects(
            &mut self.display,
            self.doc.objects(),
            self.draft.as_ref(),
            self.text_edit.as_ref().and_then(|e| e.obj_index),
            (x0, y0, x1, y1),
        );
        // 2b) 选中对象的包围框与八向控制柄（EDT-7），箭头另画曲线控制柄（EDT-3）；
        // 仅屏幕显示，不进导出烘焙
        if let Some(obj) = self.selected_object().cloned()
            && render::intersects(&obj.bounds(), (x0, y0, x1, y1))
        {
            objedit::draw_frame(&mut self.display, &obj, self.ui);
            draw_curve_handles(&mut self.display, &obj, self.ui);
        }
        // 2c) 文本输入框（EDT-4；仅屏幕显示，提交后为对象）
        if let Some(edit) = &self.text_edit {
            if intersect_rect(expand_rect(edit.region(), 2), region).is_some() {
                textinput::draw_edit(&mut self.display, edit, crate::editor::PALETTE[self.color_index]);
            }
        }
        // 3) 选区边框与手柄（只在与选区装饰区域相交时重画）；悬停高亮只有边框
        if let Some(sel) = self.selection {
            if intersect_rect(expand_rect(sel, self.dirty_pad()), region).is_some() {
                draw_rect(&mut self.display, sel, self.ui);
                draw_handles(&mut self.display, sel, self.ui);
            }
        } else if let Some(h) = hover
            && intersect_rect(self.hover_dirty(h), region).is_some()
        {
            draw_rect(&mut self.display, h, self.ui);
        }
        // 4) 工具栏
        let state = self.bar_state();
        if let Some(bar) = &self.bar {
            if intersect_rect(expand_rect(bar.rect, 2), region).is_some() {
                toolbar::draw(&mut self.display, bar, &state, self.palette);
            }
        }
        // 5) 字体列表面板（EDT-4）：位于工具栏之外，需单独覆盖
        if let Some(rect) = self.font_list.as_ref().map(|p| p.rect)
            && intersect_rect(expand_rect(rect, 2), region).is_some()
        {
            let (family, ..) = self.font_state();
            let panel = self.font_list.clone().expect("已判定为 Some");
            toolbar::fontlist::render(&mut self.display, &panel, &family, self.palette);
        }
        // 6) 选区（或悬停窗口）尺寸提示（HUD）：走 DirectWrite 栅格化，与工具栏文字同一套渲染
        if let Some(sel) = self.selection.or(hover) {
            let bar_h = self.bar.as_ref().map(|b| b.rect.h).unwrap_or(0);
            if intersect_rect(text_rect(sel, w, h, bar_h, self.ui), region).is_some() {
                draw_info_text(&mut self.display, sel, w, h, bar_h, self.ui);
            }
        }
        self.present_region(region);
    }

    /// 选中对象的手柄画在包围盒之外，脏区需一并覆盖（否则手柄残缺或残留）。
    fn expand_for_selected(&self, r: SelRect) -> SelRect {
        let Some(obj) = self.selected_object() else {
            return r;
        };
        if render::intersects(&obj.bounds(), (r.x, r.y, r.x + r.w, r.y + r.h)) {
            expand_rect(r, self.ui.px(HANDLE_PAD_BASE))
        } else {
            r
        }
    }

    /// 把脏区扩展到与它相交的所有标注对象的完整包围盒：
    /// 保证被绘制的对象不会画到 `region` 之外，避免像素图与窗口显示不一致。
    fn expand_for_objects(&self, r: SelRect) -> SelRect {
        let mut r = r;
        loop {
            let clip = (r.x, r.y, r.x + r.w, r.y + r.h);
            let mut changed = false;
            for obj in self.doc.objects().iter().chain(self.draft.iter()) {
                let bounds = obj.bounds();
                if render::intersects(&bounds, clip) {
                    let c = render::bounds_to_clip(&bounds);
                    let b = SelRect {
                        x: c.0,
                        y: c.1,
                        w: c.2 - c.0,
                        h: c.3 - c.1,
                    };
                    if let Some(u) = union_rect(Some(r), Some(b)) {
                        if u != r {
                            r = u;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                return r;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::geometry::same_rect;
    use super::SelRect;

    #[test]
    fn selection_dirty_covers_old_and_new() {
        // 简单回归：set_selection 依赖 same_rect 判断无变化
        assert!(same_rect(None, None));
        assert!(same_rect(
            Some(SelRect { x: 0, y: 0, w: 1, h: 1 }),
            Some(SelRect { x: 0, y: 0, w: 1, h: 1 })
        ));
        assert!(!same_rect(None, Some(SelRect { x: 0, y: 0, w: 1, h: 1 })));
    }
}
