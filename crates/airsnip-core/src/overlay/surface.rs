// SPDX-License-Identifier: GPL-3.0-only
//! 像素缓冲（CAP-3、OUT-2）：暗化、区域回贴、选区裁剪输出与尺寸提示文字；
//! 上屏由平台层完成（`Host::present`）。

use tiny_skia::{Pixmap, Transform};

use super::SelRect;
use super::geometry::{info_font_size, info_pad_x, text_rect};
use super::ui::Ui;
use crate::editor::Point;
use crate::render::text::{self as text, FontSpec};

impl super::Overlay {
    /// 上屏指定区域（`display` 的 r 区域交给平台层刷到窗口）。
    pub fn present_region(&mut self, r: SelRect) {
        self.host.present(&self.display, r);
    }
}

/// 选区尺寸提示（物理像素）：与工具栏文字走同一套文字栅格化。
/// 位置由 `text_rect` 确定，字色纯白；字号随显示器 DPI 缩放（CAP-5）。
pub(super) fn draw_info_text(pixmap: &mut Pixmap, sel: SelRect, screen_w: i32, screen_h: i32, toolbar_h: i32, ui: Ui) {
    let r = text_rect(sel, screen_w, screen_h, toolbar_h, ui);
    let label = format!("{} × {}", sel.w, sel.h);
    // 字重加粗：16px 常规字重在暗底上偏细发灰，加粗后笔画更实且不刺眼。
    let mut spec = FontSpec::new(info_font_size(ui));
    spec.bold = true;
    let (_, th) = text::measure(&label, &spec);
    let x = (r.x + info_pad_x(ui)) as f32;
    let y = r.y as f32 + (r.h as f32 - th) / 2.0;
    text::draw(
        pixmap,
        Point::new(x, y),
        &label,
        &spec,
        [255, 255, 255, 255],
        Transform::identity(),
    );
}

/// 将 src 的 r 区域拷贝到 dst（原地回贴）。
pub(super) fn blit_region(dst: &mut Pixmap, src: &Pixmap, r: SelRect) {
    let w = dst.width() as i32;
    let h = dst.height() as i32;
    let x0 = r.x.clamp(0, w);
    let y0 = r.y.clamp(0, h);
    let x1 = (r.x + r.w).clamp(0, w);
    let y1 = (r.y + r.h).clamp(0, h);
    let rw = (x1 - x0).max(0);
    let rh = (y1 - y0).max(0);
    for row in 0..rh {
        let off = ((y0 + row) as usize * dst.width() as usize + x0 as usize) * 4;
        dst.data_mut()[off..off + rw as usize * 4].copy_from_slice(&src.data()[off..off + rw as usize * 4]);
    }
}

/// 区域从 src 拷贝到 dst（用于从暗图恢复旧选区区域）。
pub(super) fn restore_region(dst: &mut Pixmap, src: &Pixmap, r: SelRect) {
    blit_region(dst, src, r);
}

/// 暗化像素图：逐通道乘以 k/256，多线程分块并行。
pub(super) fn dim_pixmap(pixmap: &mut Pixmap, k: u32) {
    let data = pixmap.data_mut();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    if workers == 1 || data.len() < 1 << 20 {
        dim_chunk(data, k);
        return;
    }
    // 每块按 4 字节对齐切分（保证不跨像素）
    let chunk = ((data.len() / workers) + 3) & !3;
    std::thread::scope(|s| {
        for part in data.chunks_mut(chunk) {
            s.spawn(move || dim_chunk(part, k));
        }
    });
}

/// 对一段 RGBA 像素做暗化。
fn dim_chunk(data: &mut [u8], k: u32) {
    for px in data.chunks_exact_mut(4) {
        px[0] = ((px[0] as u32 * k) >> 8) as u8;
        px[1] = ((px[1] as u32 * k) >> 8) as u8;
        px[2] = ((px[2] as u32 * k) >> 8) as u8;
    }
}

/// 裁剪出选区像素；尺寸非法时返回 None。
pub(super) fn crop_pixmap(src: &Pixmap, r: SelRect) -> Option<Pixmap> {
    let w = (r.w.max(1)) as u32;
    let h = (r.h.max(1)) as u32;
    let mut out = Pixmap::new(w, h)?;
    for (dy, row) in (0..h as i32).enumerate() {
        let sy = r.y + row;
        if sy < 0 || sy >= src.height() as i32 {
            continue;
        }
        let off_src = (sy as usize * src.width() as usize + r.x.max(0) as usize) * 4;
        let off_dst = dy * w as usize * 4;
        out.data_mut()[off_dst..off_dst + w as usize * 4]
            .copy_from_slice(&src.data()[off_src..off_src + w as usize * 4]);
    }
    Some(out)
}
