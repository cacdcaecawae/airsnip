// SPDX-License-Identifier: GPL-3.0-only
//! 条带拼接（LNG-4）：用「上一帧底部/顶部条带」在「新帧」中做行哈希匹配，求纵向像素偏移，
//! 定位重叠区并去除重复。纯函数、无窗口依赖，便于单测覆盖多种滚动场景。
//!
//! 术语：`offset` > 0 = 内容上移（向下滚动，LNG-2）；< 0 = 内容下移（向上滚动，LNG-10）。
//! 固定页眉/页脚（LNG-5）由调用方跨帧检测后传入，定位时从模板与新增区间中排除。

/// 单行像素的哈希（FNV-1a 变体，逐 8 字节块加速）。
fn row_hash(row: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    let mut chunks = row.chunks_exact(8);
    for c in &mut chunks {
        let v = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        h = (h ^ v).wrapping_mul(PRIME);
    }
    for &b in chunks.remainder() {
        h = (h ^ b as u64).wrapping_mul(PRIME);
    }
    h
}

/// 计算每行哈希（长度 = 行数）。
fn row_hashes(frame: &[u8], w: u32, h: u32) -> Vec<u64> {
    let stride = w as usize * 4;
    (0..h as usize)
        .map(|y| row_hash(&frame[y * stride..(y + 1) * stride]))
        .collect()
}

/// 条带匹配参数。
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// 判定「重叠区一致」的最低匹配行占比（0.0~1.0）。
    pub min_ratio: f32,
    /// 固定页眉 / 页脚的最小高度（像素），低于此值不认定为固定区域。
    pub min_fixed: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            min_ratio: 0.9,
            min_fixed: 8,
        }
    }
}

/// 拼接结果。
#[derive(Debug, Clone, Copy)]
pub struct Match {
    /// 新帧相对上一帧的滚动像素数：>0 内容上移（向下滚动），<0 内容下移（向上滚动），0 无位移。
    pub offset: i32,
    /// 新帧中应拼入画布的行区间 `[start, end)`（全天坐标，已排除固定区域与重叠部分）。
    pub start: u32,
    /// 行区间结束（不含）。
    pub end: u32,
    /// 定位是否可信（匹配行占比达标）。
    pub confident: bool,
}

/// 在两帧间定位唯一滚动偏移：分别尝试「底部条带在新帧中的位置」与「顶部条带的位置」，
/// 在两帧间定位滚动偏移：对每个候选位移 `s` 检验重叠区是否一致，
/// 取满足匹配度的**最小** `s`（最小位移 = 最保守、不会凭空冒出新内容）。
///
/// 相比「取一条条带在新帧中找位置」，全重叠检验能抵抗重复内容：
/// 重复文本会让条带在多个位置命中，但只有真实偏移能让整段重叠区全部对上。
///
/// `fixed_top` / `fixed_bottom` 为已确认的固定页眉/页脚高度，匹配只在内容区进行。
pub fn locate(prev: &[u8], new: &[u8], w: u32, h: u32, fixed_top: u32, fixed_bottom: u32, params: &Params) -> Match {
    let ph = row_hashes(prev, w, h);
    let nh = row_hashes(new, w, h);
    let ct = fixed_top;
    let content_h = h.saturating_sub(fixed_top + fixed_bottom);
    // 内容区太小则不做拼接（避免零碎结果）。
    if content_h < 8 {
        return no_match(h);
    }
    // 最多允许半个内容区的位移（超过即不可能通过重叠检验）。
    let max_shift = (content_h / 2).max(1);

    // s=0（无位移）：整个内容区逐行一致。
    let same = (0..content_h)
        .filter(|i| ph[(ct + i) as usize] == nh[(ct + i) as usize])
        .count() as u32;
    if same as f32 / content_h as f32 >= params.min_ratio {
        return Match {
            offset: 0,
            start: h,
            end: h,
            confident: true,
        };
    }

    // 向下滚（内容上移）：prev[ct+s..ct+ch) == new[ct..ct+ch-s)
    let down = min_consistent_shift(&ph, &nh, ct, content_h, max_shift, params.min_ratio, false);
    // 向上滚（内容下移）：prev[ct..ct+ch-s) == new[ct+s..ct+ch)
    let up = min_consistent_shift(&ph, &nh, ct, content_h, max_shift, params.min_ratio, true);

    match (down, up) {
        (None, None) => no_match(h),
        (Some((s, _)), None) => down_match(ct, content_h, s),
        (None, Some((s, _))) => up_match(ct, s),
        // 两侧都成立：取位移更小的一侧（更保守）；相等取向下。
        (Some((sd, _)), Some((su, _))) => {
            if sd <= su {
                down_match(ct, content_h, sd)
            } else {
                up_match(ct, su)
            }
        }
    }
}

fn no_match(h: u32) -> Match {
    Match {
        offset: 0,
        start: h,
        end: h,
        confident: false,
    }
}

fn down_match(ct: u32, content_h: u32, s: u32) -> Match {
    Match {
        offset: s as i32,
        start: ct + content_h - s,
        end: ct + content_h,
        confident: true,
    }
}

fn up_match(ct: u32, s: u32) -> Match {
    Match {
        offset: -(s as i32),
        start: ct,
        end: ct + s,
        confident: true,
    }
}

/// 在 `1..=max_shift` 中找最小位移 `s`，使重叠区匹配行占比 ≥ `min_ratio`。
/// `up` 为 true 时检验向上滚动方向。
fn min_consistent_shift(
    ph: &[u64],
    nh: &[u64],
    ct: u32,
    ch: u32,
    max_shift: u32,
    min_ratio: f32,
    up: bool,
) -> Option<(u32, f32)> {
    for s in 1..=max_shift {
        let overlap = ch - s;
        if overlap == 0 {
            continue;
        }
        let mut matches = 0u32;
        for i in 0..overlap {
            let (a, b) = if up {
                // prev[ct+i] vs new[ct+i+s]
                (ph[(ct + i) as usize], nh[(ct + i + s) as usize])
            } else {
                // prev[ct+i+s] vs new[ct+i]
                (ph[(ct + i + s) as usize], nh[(ct + i) as usize])
            };
            if a == b {
                matches += 1;
            }
        }
        let ratio = matches as f32 / overlap as f32;
        if ratio >= min_ratio {
            return Some((s, ratio));
        }
    }
    None
}

/// 检测帧顶部 / 底部的「固定区域」高度（LNG-5 页眉 / 页脚）：
/// 与上一帧逐行比较，返回（顶部静止行数, 底部静止行数）。
/// 静止行可能是页面固定头部 / 底部工具栏；拆分时据此避免重复拼入。
pub fn fixed_regions(prev: &[u8], new: &[u8], w: u32, h: u32, params: &Params) -> (u32, u32) {
    let stride = w as usize * 4;
    // 顶部：从上往下数，行相等则计入；遇到首个不同行停止。
    let mut top = 0u32;
    while top < h {
        let o = top as usize * stride;
        if prev.get(o..o + stride) == new.get(o..o + stride) {
            top += 1;
        } else {
            break;
        }
    }
    if top < params.min_fixed {
        top = 0;
    }
    // 底部：从下往上数。
    let mut bottom = 0u32;
    while bottom < h {
        let o = (h - 1 - bottom) as usize * stride;
        if prev.get(o..o + stride) == new.get(o..o + stride) {
            bottom += 1;
        } else {
            break;
        }
    }
    // 顶部与底部不允许重叠。
    if bottom + top > h {
        bottom = h - top;
    }
    if bottom < params.min_fixed {
        bottom = 0;
    }
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一页：每行按行号着色（R=低8位、G=高8位、B=横向渐变保证行内变化），行间可区分。
    fn make_page(w: u32, page_h: u32) -> Vec<u8> {
        let mut data = vec![0u8; (w * page_h * 4) as usize];
        for y in 0..page_h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                data[o] = (y & 0xff) as u8;
                data[o + 1] = ((y >> 8) & 0xff) as u8;
                data[o + 2] = ((y * 7 + x * 13) & 0xff) as u8;
                data[o + 3] = 255;
            }
        }
        data
    }

    /// 截取页面的视口窗口 [y0, y0+h)。
    fn viewport(page: &[u8], w: u32, y0: u32, h: u32) -> Vec<u8> {
        let stride = w as usize * 4;
        page[y0 as usize * stride..(y0 + h) as usize * stride].to_vec()
    }

    fn params() -> Params {
        Params {
            min_ratio: 0.6,
            min_fixed: 8,
        }
    }

    #[test]
    fn locate_down_scroll_finds_offset() {
        let w = 64;
        let h = 60u32;
        let page = make_page(w, 400);
        let prev = viewport(&page, w, 0, h);
        let new = viewport(&page, w, 10, h);
        let m = locate(&prev, &new, w, h, 0, 0, &params());
        assert!(m.confident);
        assert_eq!(m.offset, 10);
        // 新增内容为 new 底部 [50,60) = page[60..70)
        assert_eq!(m.start, 50);
        assert_eq!(m.end, 60);
    }

    #[test]
    fn locate_up_scroll_finds_offset() {
        let w = 64;
        let h = 60u32;
        let page = make_page(w, 400);
        let prev = viewport(&page, w, 60, h);
        let new = viewport(&page, w, 50, h);
        let m = locate(&prev, &new, w, h, 0, 0, &params());
        assert!(m.confident);
        assert_eq!(m.offset, -10);
        // 新增内容为 new 顶部 [0,10) = page[50..60)
        assert_eq!(m.start, 0);
        assert_eq!(m.end, 10);
    }

    #[test]
    fn locate_no_move_returns_zero() {
        let w = 64;
        let h = 60u32;
        let page = make_page(w, 200);
        let f = viewport(&page, w, 20, h);
        let m = locate(&f, &f, w, h, 0, 0, &params());
        assert!(m.confident);
        assert_eq!(m.offset, 0);
        assert_eq!(m.start, h);
        assert_eq!(m.end, h);
    }

    #[test]
    fn fixed_regions_detects_header_footer() {
        let w = 64;
        let h = 80u32;
        // 页面：10 行固定页眉(值=900)、60 行内容、10 行固定页脚(值=901)
        let mut page = vec![0u8; (w * h * 4) as usize];
        for y in 0..10 {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                page[o] = 200;
                page[o + 1] = 201;
                page[o + 2] = (x * 5) as u8;
                page[o + 3] = 255;
            }
        }
        for y in 10..70 {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                page[o] = (y) as u8;
                page[o + 1] = (y >> 8) as u8;
                page[o + 2] = (x * 3) as u8;
                page[o + 3] = 255;
            }
        }
        for y in 70..80 {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                page[o] = 202;
                page[o + 1] = 203;
                page[o + 2] = (x * 7) as u8;
                page[o + 3] = 255;
            }
        }
        // 第二帧整体下移 5 行（但页眉/页脚保留在原位）
        let mut frame2 = page.clone();
        for y in 10..70 {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                frame2[o] = (y + 5) as u8;
                frame2[o + 1] = ((y + 5) >> 8) as u8;
                frame2[o + 2] = (x * 3) as u8;
                frame2[o + 3] = 255;
            }
        }
        let (t, b) = fixed_regions(&page, &frame2, w, h, &params());
        assert_eq!(t, 10);
        assert_eq!(b, 10);
    }
}
