// SPDX-License-Identifier: GPL-3.0-only
//! 长截图拼接引擎（LNG-1/4/5/6/10）：数据面，维护拼接画布、固定页眉/页脚、
//! 高度上限与撤销段历史。纯逻辑，不依赖窗口（滚动由 `scroll` 模块负责）。
//!
//! 帧流时序：`seed(首帧)` 仅缓冲；此后每帧 `push(frame, h)` 内部用上一帧做
//! 固定区域检测与条带定位。首帧不直接入画布——等拿到第二帧才能判断页眉/页脚，
//! 避免「先入画布再纠正」的错位。

use super::rows::RowBuffer;
use super::stitch::{self, Params};

/// 拼接画布：宽度固定为视口宽，高度随帧增长。
pub struct Canvas {
    w: u32,
    /// 内容区画布（不含固定页眉/页脚）。
    rows: RowBuffer,
    /// 首帧缓冲（等待第二帧确定固定区域后才建画布）。
    first: Option<Vec<u8>>,
    /// 上一帧（固定区域检测与定位用）。
    prev: Vec<u8>,
    /// 已确认的固定页眉（首帧顶部行，最终置于画布顶部）。
    header: Vec<u8>,
    header_h: u32,
    /// 已确认的固定页脚（首帧底部行，最终置于画布底部）。
    footer: Vec<u8>,
    footer_h: u32,
    params: Params,
    max_height: u32,
    /// 连续无位移帧数（LNG-6：达到上限即到底）。
    pub still_run: u32,
    /// 已拼内容行数（不含页眉/页脚）。
    pub content_rows: u32,
    /// 段历史（撤销最后一段用）：(是否在顶部插入, 行数)。
    segments: Vec<(bool, u32)>,
}

/// 单步结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 成功拼入一段。`offset` 本帧相对上一帧滚动像素（>0 向下），`rows` 新增行数。
    Appended { offset: i32, rows: u32 },
    /// 无位移（到页底或用户未滚）。
    Still,
    /// 定位失败（画面变化太快 / 内容不连续）。
    Failed,
    /// 已达高度上限。
    Limit,
}

impl Canvas {
    pub fn new(w: u32, params: Params, max_height: u32) -> Self {
        Self {
            w,
            rows: RowBuffer::new(w),
            first: None,
            prev: Vec::new(),
            header: Vec::new(),
            header_h: 0,
            footer: Vec::new(),
            footer_h: 0,
            params,
            max_height,
            still_run: 0,
            content_rows: 0,
            segments: Vec::new(),
        }
    }

    pub fn height(&self) -> u32 {
        self.header_h + self.content_rows + self.footer_h
    }

    /// 缓冲首帧。画布实际在第二次推帧时建立。
    pub fn seed(&mut self, frame: &[u8]) {
        self.first = Some(frame.to_vec());
    }

    /// 处理一帧。`frame` 与上一帧同尺寸（宽 `w`、高 `h`）。
    pub fn push(&mut self, frame: &[u8], h: u32) -> Step {
        // 首帧推入：建立画布。
        if let Some(first) = self.first.take() {
            let (mut t, mut b) = stitch::fixed_regions(&first, frame, self.w, h, &self.params);
            // 完全静止（首帧==本帧）时不算页眉/页脚：整帧内容应入画布。
            if t >= h {
                t = 0;
                b = 0;
            }
            let t = if t >= self.params.min_fixed { t } else { 0 };
            let b = if b >= self.params.min_fixed { b } else { 0 };
            // 首帧页眉/页脚入 header/footer，内容区入画布。
            let stride = self.w as usize * 4;
            if t > 0 {
                self.header = first[..t as usize * stride].to_vec();
                self.header_h = t;
            }
            if b > 0 {
                self.footer = first[first.len() - b as usize * stride..].to_vec();
                self.footer_h = b;
            }
            self.rows.append(&first, t, h - b);
            self.content_rows = self.rows.height();
            self.prev = first.clone();
            // 继续用首帧作上一帧，与本帧做一次正常定位，得到首段位移。
            return self.push_inner(frame, h);
        }
        self.push_inner(frame, h)
    }

    fn push_inner(&mut self, frame: &[u8], h: u32) -> Step {
        let params = self.params;
        let m = stitch::locate(&self.prev, frame, self.w, h, self.header_h, self.footer_h, &params);
        super::debug::log(&format!(
            "push: offset={} start={} end={} confident={} fixed_top={} fixed_bottom={}",
            m.offset, m.start, m.end, m.confident, self.header_h, self.footer_h
        ));
        if !m.confident {
            self.still_run += 1;
            self.prev = frame.to_vec();
            return Step::Failed;
        }
        if m.offset == 0 {
            self.still_run += 1;
            self.prev = frame.to_vec();
            return Step::Still;
        }
        self.still_run = 0;

        // 追加/前置新增内容（m.start/m.end 为全天坐标，已剔除固定区域）。
        let rows = (m.end - m.start) as u32;
        if rows == 0 {
            self.prev = frame.to_vec();
            return Step::Still;
        }
        if self.content_rows + rows > self.max_height {
            self.prev = frame.to_vec();
            return Step::Limit;
        }
        let prepend = m.offset < 0;
        if prepend {
            self.rows.prepend(frame, m.start, m.end);
        } else {
            self.rows.append(frame, m.start, m.end);
        }
        self.content_rows = self.rows.height();
        self.segments.push((prepend, rows));
        self.prev = frame.to_vec();
        Step::Appended { offset: m.offset, rows }
    }

    /// 是否有可撤销的段。
    pub fn can_undo(&self) -> bool {
        !self.segments.is_empty()
    }

    /// 撤销最后一段内容（LNG-10 失败修正入口）。
    pub fn undo_last(&mut self) -> bool {
        let Some((prepend, rows)) = self.segments.pop() else {
            return false;
        };
        if prepend {
            self.rows.remove_top(rows);
        } else {
            self.rows.remove_bottom(rows);
        }
        self.content_rows = self.rows.height();
        true
    }

    /// 取整图第 `y` 行（含页眉与页脚）。
    fn row_at(&self, y: u32) -> Option<&[u8]> {
        let stride = self.w as usize * 4;
        if y < self.header_h {
            let o = y as usize * stride;
            return self.header.get(o..o + stride);
        }
        let y = y - self.header_h;
        if y < self.content_rows {
            return Some(self.rows.row(y));
        }
        let y = y - self.content_rows;
        if y < self.footer_h {
            let o = y as usize * stride;
            return self.footer.get(o..o + stride);
        }
        None
    }

    /// 生成整图缩略图（LNG-3 预览）：最近邻抽样，等比缩放到 `max_w × max_h` 内。
    /// 成本只与目标尺寸相关，与拼接图高度无关（不先合成整张大图）。
    pub fn thumbnail(&self, max_w: u32, max_h: u32) -> Option<tiny_skia::Pixmap> {
        let (sw, sh) = (self.w, self.height());
        if sw == 0 || sh == 0 || max_w == 0 || max_h == 0 {
            return None;
        }
        // 等比缩放：宽高都不超过上限
        let scale = (max_w as f32 / sw as f32).min(max_h as f32 / sh as f32);
        let tw = ((sw as f32 * scale) as u32).clamp(1, max_w);
        let th = ((sh as f32 * scale) as u32).clamp(1, max_h);
        let mut out = vec![0u8; (tw * th * 4) as usize];
        let dst_stride = tw as usize * 4;
        for y in 0..th {
            let sy = (y as u64 * sh as u64 / th as u64) as u32;
            let Some(row) = self.row_at(sy) else { continue };
            let dst = y as usize * dst_stride;
            for x in 0..tw {
                let sx = (x as u64 * sw as u64 / tw as u64) as usize * 4;
                let d = dst + x as usize * 4;
                if let Some(px) = row.get(sx..sx + 4) {
                    out[d..d + 4].copy_from_slice(px);
                }
            }
        }
        tiny_skia::Pixmap::from_vec(out, tiny_skia::IntSize::from_wh(tw, th)?)
    }

    /// 导出画布像素（页眉 + 内容 + 页脚）。
    pub fn to_pixmap(&self) -> Option<tiny_skia::Pixmap> {
        let mut out = RowBuffer::new(self.w);
        if self.header_h > 0 {
            let mut hdr = RowBuffer::new(self.w);
            hdr.append(&self.header, 0, self.header_h);
            out.append_buffer(&hdr);
        }
        out.append_buffer(&self.rows);
        if self.footer_h > 0 {
            let mut ftr = RowBuffer::new(self.w);
            ftr.append(&self.footer, 0, self.footer_h);
            out.append_buffer(&ftr);
        }
        out.into_pixmap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Params {
        Params {
            min_ratio: 0.6,
            min_fixed: 8,
        }
    }

    /// 造完整页面：固定页眉 hdr 行 + 内容 total 行 + 固定页脚 ftr 行。
    /// 行内容按全局行号着色，页眉/页脚用特殊色便于断言。
    fn make_page(w: u32, hdr: u32, content: u32, ftr: u32) -> Vec<u8> {
        let h = hdr + content + ftr;
        let mut data = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            let is_hdr = y < hdr;
            let is_ftr = y >= hdr + content;
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                if is_hdr {
                    data[o] = 200;
                    data[o + 1] = 201;
                    data[o + 2] = (x * 5) as u8;
                } else if is_ftr {
                    data[o] = 202;
                    data[o + 1] = 203;
                    data[o + 2] = (x * 7) as u8;
                } else {
                    data[o] = (y & 0xff) as u8;
                    data[o + 1] = ((y >> 8) & 0xff) as u8;
                    data[o + 2] = (x * 3) as u8;
                }
                data[o + 3] = 255;
            }
        }
        data
    }

    /// 简化：把「视口内、页眉页脚固定的完整页面窗口」抽成页眉+内容窗口+页脚。
    fn frame_simple(page: &[u8], w: u32, hdr: u32, ftr: u32, content_y0: u32, view_h: u32) -> Vec<u8> {
        let stride = w as usize * 4;
        let content_win = (view_h - hdr - ftr) as usize;
        let mut out = vec![0u8; (w * view_h) as usize * 4];
        // 页眉
        out[..hdr as usize * stride].copy_from_slice(&page[..hdr as usize * stride]);
        // 内容窗口
        let c_start = (hdr + content_y0) as usize * stride;
        let c_len = content_win * stride;
        out[hdr as usize * stride..(hdr as usize + content_win) * stride]
            .copy_from_slice(&page[c_start..c_start + c_len]);
        // 页脚
        let f_start = (page.len() / stride - ftr as usize) * stride;
        out[(view_h - ftr) as usize * stride..].copy_from_slice(&page[f_start..]);
        out
    }

    #[test]
    fn canvas_stitches_down_scroll_with_fixed_header_footer() {
        let w = 64u32;
        let hdr = 10u32;
        let ftr = 8u32;
        let content_total = 120u32;
        let view_h = hdr + 60 + ftr; // 内容视口 60 行
        let page = make_page(w, hdr, content_total, ftr);

        let mut c = Canvas::new(w, params(), 30000);
        // 首帧：内容窗口从 0 开始
        let f0 = frame_simple(&page, w, hdr, ftr, 0, view_h);
        c.seed(&f0);
        // 第二帧：内容下滚 10 行
        let f1 = frame_simple(&page, w, hdr, ftr, 10, view_h);
        let s1 = c.push(&f1, view_h);
        assert_eq!(s1, Step::Appended { offset: 10, rows: 10 });
        // 第三帧：再多滚 20 行
        let f2 = frame_simple(&page, w, hdr, ftr, 30, view_h);
        let s2 = c.push(&f2, view_h);
        assert_eq!(s2, Step::Appended { offset: 20, rows: 20 });

        // 最终高度 = 页眉 10 + 内容 90 + 页脚 8
        assert_eq!(c.height(), 10 + 60 + 30 + 8);
        let img = c.to_pixmap().expect("画布可转像素图");
        assert_eq!(img.height(), 10 + 60 + 30 + 8);
        // 分段比对：页眉 = 页面 0..10，内容 = 页面 10..100，页脚 = 页面末尾 130..138
        for y in 0..10 {
            let o = y as usize * w as usize * 4;
            assert_eq!(
                &img.data()[o..o + w as usize * 4],
                &page[o..o + w as usize * 4],
                "页眉第 {y} 行"
            );
        }
        for y in 10..100 {
            let o = y as usize * w as usize * 4;
            assert_eq!(
                &img.data()[o..o + w as usize * 4],
                &page[o..o + w as usize * 4],
                "内容第 {y} 行"
            );
        }
        for y in 0..8 {
            let o = (100 + y) as usize * w as usize * 4;
            let po = (130 + y) as usize * w as usize * 4;
            assert_eq!(
                &img.data()[o..o + w as usize * 4],
                &page[po..po + w as usize * 4],
                "页脚第 {y} 行"
            );
        }
    }

    #[test]
    fn canvas_undo_removes_last_segment() {
        let w = 64u32;
        let hdr = 0u32;
        let ftr = 0u32;
        let content_total = 200u32;
        let view_h = 60u32;
        let page = make_page(w, hdr, content_total, ftr);

        let mut c = Canvas::new(w, params(), 30000);
        c.seed(&view(&page, w, 0, view_h));
        let s1 = c.push(&view(&page, w, 10, view_h), view_h);
        assert_eq!(s1, Step::Appended { offset: 10, rows: 10 });
        assert_eq!(c.height(), 70);
        assert!(c.undo_last());
        assert_eq!(c.height(), 60);
        // 无可撤销段时返回 false
        assert!(!c.undo_last());
    }

    #[test]
    fn canvas_prepends_on_up_scroll() {
        let w = 64u32;
        let hdr = 0u32;
        let ftr = 0u32;
        let content_total = 200u32;
        let view_h = 60u32;
        let page = make_page(w, hdr, content_total, ftr);

        let mut c = Canvas::new(w, params(), 30000);
        // 起点在第 90 行；向上滚 10 行 → 内容窗口 [80,140)
        c.seed(&view(&page, w, 90, view_h));
        let s1 = c.push(&view(&page, w, 80, view_h), view_h);
        assert_eq!(s1, Step::Appended { offset: -10, rows: 10 });
        // 画布 = 上滚前移：起点 80，共 10 + 60 = 70 行
        assert_eq!(c.height(), 70);
        let img = c.to_pixmap().unwrap();
        for y in 0..img.height() {
            let o = y as usize * w as usize * 4;
            let po = (80 + y) as usize * w as usize * 4;
            assert_eq!(
                &img.data()[o..o + w as usize * 4],
                &page[po..po + w as usize * 4],
                "第 {y} 行"
            );
        }
    }

    #[test]
    fn canvas_respects_height_limit() {
        let w = 64u32;
        let hdr = 0u32;
        let ftr = 0u32;
        let content_total = 200u32;
        let view_h = 60u32;
        let page = make_page(w, hdr, content_total, ftr);
        let mut c = Canvas::new(w, params(), 85); // 上限 85
        c.seed(&view(&page, w, 0, view_h));
        assert_eq!(
            c.push(&view(&page, w, 10, view_h), view_h),
            Step::Appended { offset: 10, rows: 10 }
        ); // 70
        assert_eq!(
            c.push(&view(&page, w, 20, view_h), view_h),
            Step::Appended { offset: 10, rows: 10 }
        ); // 80
        assert_eq!(c.push(&view(&page, w, 30, view_h), view_h), Step::Limit); // 90 > 85
        assert_eq!(c.height(), 80);
    }

    fn view(page: &[u8], w: u32, y0: u32, h: u32) -> Vec<u8> {
        let stride = w as usize * 4;
        page[y0 as usize * stride..(y0 + h) as usize * stride].to_vec()
    }

    /// 预览缩略图（LNG-3）：尺寸不超上限、保持长宽比、内容可区分。
    #[test]
    fn thumbnail_scales_within_bounds() {
        let w = 64u32;
        let view_h = 60u32;
        let page = make_page(w, 0, 200, 0);
        let mut c = Canvas::new(w, params(), 30000);
        c.seed(&view(&page, w, 0, view_h));
        // 每次滚 20 行（不超过内容区一半，locate 才能定位）
        c.push(&view(&page, w, 20, view_h), view_h);
        c.push(&view(&page, w, 40, view_h), view_h);
        assert_eq!(c.height(), 100);

        // 高度上限 90 → 按高度等比缩（64×100 → 57×90）
        let t = c.thumbnail(1000, 90).expect("可生成缩略图");
        assert!(t.width() <= 1000 && t.height() <= 90, "不得超出上限");
        assert_eq!(t.height(), 90, "应缩到高度上限");
        assert!(
            (t.width() as i32 - 57).abs() <= 1,
            "宽度应保持长宽比，实际 {}",
            t.width()
        );

        // 宽度受限时按宽度缩（64×100 → 16×25）
        let t2 = c.thumbnail(16, 1000).expect("可生成缩略图");
        assert_eq!((t2.width(), t2.height()), (16, 25), "应等比缩放到宽度上限");

        // 内容不透明（页面每行 alpha=255）
        assert_eq!(t.data()[3], 255, "缩略图像素应不透明");
    }
}
