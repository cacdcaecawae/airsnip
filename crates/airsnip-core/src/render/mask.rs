// SPDX-License-Identifier: GPL-3.0-only
//! 圆角遮罩（EDT-1/EDT-5/EDT-6/EDT-9）：区域类对象（高亮 / 高斯模糊 / 聚光）共用的
//! 圆角覆盖率计算与按遮罩回贴像素。
//!
//! 性能约定（见 `docs/perf-m5.md`）：`r < 0.5`（直角）时调用方一律走原有快速路径，
//! 不进这里的逐像素距离计算；即便走遮罩，也只有四角的 `r × r` 方块内才做一次开方，
//! 直边与中心区域直接返回满覆盖。

use tiny_skia::Pixmap;

/// 像素区域（x0, y0, x1, y1 半开区间，物理像素）。
pub type Region = (i32, i32, i32, i32);

/// 浮点矩形（x0, y0, x1, y1）：**完整**对象矩形，圆角一律按它算（不随脏区裁剪）。
pub type FRect = (f32, f32, f32, f32);

/// 圆角半径夹到 `[0, 短边一半]`（与 `editor::Kind::corner_limit` 同一规则）。
/// `rect` 为**完整**对象矩形 `(x0, y0, x1, y1)`。
pub fn clamp_radius(rect: FRect, r: f32) -> f32 {
    let (x0, y0, x1, y1) = rect;
    let limit = (x1 - x0).abs().min((y1 - y0).abs()) * 0.5;
    r.clamp(0.0, limit)
}

/// 圆角矩形在像素中心 `(px, py)` 处的覆盖率（0..=1）。
///
/// `rect` 必须是**完整**对象矩形，不能是裁剪后的脏区：聚光 / 模糊回贴时区域会被裁到脏区，
/// 但圆角要按完整矩形算，否则靠角的那次局部重绘会把角画成直角。
pub(crate) fn corner_coverage(px: f32, py: f32, rect: FRect, r: f32) -> f32 {
    if r < 0.5 {
        return 1.0;
    }
    let (x0, y0, x1, y1) = rect;
    let (left, right) = (px < x0 + r, px > x1 - r);
    let (top, bottom) = (py < y0 + r, py > y1 - r);
    // 直边与中心：满覆盖（不做开方）
    if !(left || right) || !(top || bottom) {
        return 1.0;
    }
    // 四角方块内：按到圆弧圆心的距离做 1px 抗锯齿
    let cx = if left { x0 + r } else { x1 - r };
    let cy = if top { y0 + r } else { y1 - r };
    let (dx, dy) = (px - cx, py - cy);
    (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0)
}

/// 区域内的四角方块行列范围：不在这些行列上的像素一律满覆盖（走快速路径）。
///
/// 性能意义：整行直边可以按块拷贝或走原循环，只有角的 `r × r` 方块内才逐像素算覆盖率。
pub(crate) struct CornerBands {
    /// `[0, n_top)` 行落在上角方块内
    n_top: usize,
    /// `[b_start, rh)` 行落在下角方块内
    b_start: usize,
    /// `[0, n_left)` 列落在左角方块内
    pub n_left: usize,
    /// `[r_start, rw)` 列落在右角方块内
    pub r_start: usize,
}

impl CornerBands {
    /// `region` 已裁剪到画布，`rect` 为完整对象矩形（同坐标系）。
    pub fn new(region: Region, rect: FRect, r: f32) -> Self {
        let (x0, y0, x1, y1) = region;
        let (fx0, fy0, fx1, fy1) = rect;
        let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        // 像素中心 = 索引 + 0.5，与 `corner_coverage` 的判定一致：
        // 前 n 个索引落于 `start` 之前，后段从 `[first, len)` 落于 `end` 之后
        let span = |start: f32, end: f32, origin: i32, len: usize| {
            let head = ((start - origin as f32 - 0.5).ceil().max(0.0) as usize).min(len);
            let tail = (((end - origin as f32 - 0.5).floor() + 1.0).max(0.0) as usize).min(len);
            (head, tail)
        };
        let (n_top, b_start) = span(fy0 + r, fy1 - r, y0, rh);
        let (n_left, r_start) = span(fx0 + r, fx1 - r, x0, rw);
        Self {
            n_top,
            b_start,
            n_left,
            r_start,
        }
    }

    /// 该行是否与四角方块相交（否则整行满覆盖）。
    #[inline]
    pub fn row_cornered(&self, row: usize) -> bool {
        row < self.n_top || row >= self.b_start
    }
}

/// 把像素区域与画布求交；空区域返回 None。
pub(crate) fn clip_region(pixmap: &Pixmap, region: Region) -> Option<Region> {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (x0, y0, x1, y1) = region;
    let c = (x0.max(0), y0.max(0), x1.min(w), y1.min(h));
    (c.2 > c.0 && c.3 > c.1).then_some(c)
}

/// 拷贝 `region` 范围内的 RGBA 像素为紧凑缓冲（行优先，行宽 = 区域宽）。
/// 区域裁剪到画布内；空区域返回空缓冲。
pub fn copy_region(pixmap: &Pixmap, region: Region) -> Vec<u8> {
    let Some((x0, y0, x1, y1)) = clip_region(pixmap, region) else {
        return Vec::new();
    };
    let stride = pixmap.width() as usize;
    let row = (x1 - x0) as usize * 4;
    let mut buf = Vec::with_capacity(row * (y1 - y0) as usize);
    for y in y0..y1 {
        let off = (y as usize * stride + x0 as usize) * 4;
        buf.extend_from_slice(&pixmap.data()[off..off + row]);
    }
    buf
}

/// 按圆角遮罩把紧凑缓冲 `src` 混入 `dst` 的同一区域：`dst = dst + (src − dst) × coverage`。
/// `src` 必须来自 `copy_region(dst, region)`（同尺寸、同裁剪），`corner_rect` 为完整对象矩形。
/// 用于聚光：`src` 是原图、`dst` 是暗图，圆角内换成原图亮度。
pub fn blend_compact(dst: &mut Pixmap, src: &[u8], region: Region, corner_rect: FRect, r: f32) {
    blend_impl(dst, src, region, corner_rect, r, false);
}

/// 按圆角遮罩把紧凑缓冲 `src` 恢复回来：`dst = src + (dst − src) × coverage`。
/// 用于模糊：`dst` 已经是模糊结果，圆角处要换回模糊前的 `src`。
pub(crate) fn restore_compact(dst: &mut Pixmap, src: &[u8], region: Region, corner_rect: FRect, r: f32) {
    blend_impl(dst, src, region, corner_rect, r, true);
}

/// `keep_dst = false` → 遮盖处换成 `src`；`true` → 遮盖处保留 `dst`（圆角处用 `src`）。
fn blend_impl(dst: &mut Pixmap, src: &[u8], region: Region, corner_rect: FRect, r: f32, keep_dst: bool) {
    let Some((x0, y0, x1, y1)) = clip_region(dst, region) else {
        return;
    };
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if src.len() < rw * rh * 4 {
        return;
    }
    let stride = dst.width() as usize;
    let bands = CornerBands::new((x0, y0, x1, y1), corner_rect, r);
    let data = dst.data_mut();
    for row in 0..rh {
        let base = (y0 as usize + row) * stride + x0 as usize;
        let sbase = row * rw;
        // 直边行：整行满覆盖（`!keep_dst` 时整行拷贝，`keep_dst` 时保留 dst 原样）
        if !bands.row_cornered(row) {
            if !keep_dst {
                data[base * 4..(base + rw) * 4].copy_from_slice(&src[sbase * 4..(sbase + rw) * 4]);
            }
            continue;
        }
        let py = (y0 + row as i32) as f32 + 0.5;
        let (a, b) = (bands.n_left.min(rw), bands.r_start.min(rw));
        // 中段直边：满覆盖，同上
        if a < b && !keep_dst {
            data[(base + a) * 4..(base + b) * 4].copy_from_slice(&src[(sbase + a) * 4..(sbase + b) * 4]);
        }
        // 只对残余的两段（角方块内）逐像素算覆盖率
        for col in (0..a).chain(b..rw) {
            let px = (x0 + col as i32) as f32 + 0.5;
            let cov = corner_coverage(px, py, corner_rect, r);
            let w = if keep_dst { 1.0 - cov } else { cov };
            if w <= 0.0 {
                continue;
            }
            let off = (base + col) * 4;
            let s = (sbase + col) * 4;
            if w >= 1.0 {
                data[off..off + 4].copy_from_slice(&src[s..s + 4]);
                continue;
            }
            for c in 0..4 {
                let d = data[off + c] as f32;
                let v = src[s + c] as f32;
                data[off + c] = (d + (v - d) * w).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::Color;

    fn opaque(w: u32, h: u32, c: Color) -> Pixmap {
        let mut p = Pixmap::new(w, h).unwrap();
        p.fill(c);
        p
    }

    /// 直角：任何位置都是满覆盖（不进距离计算）。
    #[test]
    fn straight_corner_is_full_coverage() {
        assert_eq!(corner_coverage(0.5, 0.5, (0.0, 0.0, 10.0, 10.0), 0.0), 1.0);
        assert_eq!(corner_coverage(5.0, 5.0, (0.0, 0.0, 10.0, 10.0), 0.4), 1.0);
    }

    /// 圆角矩形：中心满覆盖、角上 0、直边上满覆盖。
    #[test]
    fn corner_square_is_cut_edges_are_kept() {
        let rect = (0.0, 0.0, 40.0, 40.0);
        assert_eq!(corner_coverage(20.0, 20.0, rect, 10.0), 1.0, "中心满覆盖");
        assert_eq!(corner_coverage(0.5, 0.5, rect, 10.0), 0.0, "左上角被切掉");
        assert_eq!(corner_coverage(39.5, 0.5, rect, 10.0), 0.0, "右上角被切掉");
        assert_eq!(corner_coverage(20.0, 0.5, rect, 10.0), 1.0, "上直边仍是满覆盖");
        assert_eq!(corner_coverage(0.5, 20.0, rect, 10.0), 1.0, "左直边仍是满覆盖");
        // 圆弧上（距圆心 (10,10) 恰好 r=10）→ 半覆盖，形成 1px 抗锯齿
        let on_arc = 10.0 - 10.0 / 2f32.sqrt();
        let cov = corner_coverage(on_arc, on_arc, rect, 10.0);
        assert!((cov - 0.5).abs() < 0.02, "圆弧边界应半覆盖：{cov}");
    }

    /// 按遮罩回贴：圆角处保留目标像素，中心换成源像素。
    #[test]
    fn blend_keeps_corners_and_pastes_center() {
        let mut dst = opaque(40, 40, Color::from_rgba8(0, 0, 0, 255));
        let src = opaque(40, 40, Color::from_rgba8(255, 255, 255, 255));
        let region = (0, 0, 40, 40);
        let buf = copy_region(&src, region);
        blend_compact(&mut dst, &buf, region, (0.0, 0.0, 40.0, 40.0), 12.0);

        let at = |p: &Pixmap, x: u32, y: u32| p.data()[((y * 40 + x) * 4) as usize];
        assert_eq!(at(&dst, 20, 20), 255, "中心应换成源像素");
        assert_eq!(at(&dst, 0, 0), 0, "圆角处应保留原像素");
        assert_eq!(at(&dst, 20, 0), 255, "直边应换成源像素");
    }

    /// `restore_compact`（模糊用）：圆角处换回 `src`，中心与直边保留 `dst`。
    #[test]
    fn restore_keeps_center_and_restores_corners() {
        let mut dst = opaque(40, 40, Color::from_rgba8(255, 255, 255, 255));
        let src = opaque(40, 40, Color::from_rgba8(0, 0, 0, 255));
        let region = (0, 0, 40, 40);
        let buf = copy_region(&src, region);
        restore_compact(&mut dst, &buf, region, (0.0, 0.0, 40.0, 40.0), 12.0);

        let at = |p: &Pixmap, x: u32, y: u32| p.data()[((y * 40 + x) * 4) as usize];
        assert_eq!(at(&dst, 20, 20), 255, "中心应保留 dst（模糊结果）");
        assert_eq!(at(&dst, 20, 0), 255, "直边应保留 dst");
        assert_eq!(at(&dst, 0, 0), 0, "圆角处应换回 src（模糊前）");
    }

    /// 安全不变量：被 `CornerBands` 判为「直边」的像素，覆盖率必须恰好是 1
    /// （否则分段快速路径会漏掉本该抗锯齿的像素）。
    #[test]
    fn bands_never_skip_a_partial_pixel() {
        let rect = (5.0, 5.0, 45.0, 35.0);
        let r = 12.0;
        let (w, h) = (60usize, 60usize);
        let bands = CornerBands::new((0, 0, w as i32, h as i32), rect, r);
        for row in 0..h {
            for col in 0..w {
                let cornered = bands.row_cornered(row) && (col < bands.n_left || col >= bands.r_start);
                let cov = corner_coverage(col as f32 + 0.5, row as f32 + 0.5, rect, r);
                assert!(cornered || cov == 1.0, "({col}, {row}) 被判为直边，却有覆盖率 {cov}");
            }
        }
    }

    /// 半径夹到短边一半。
    #[test]
    fn radius_is_clamped_to_half_of_shorter_side() {
        assert_eq!(clamp_radius((0.0, 0.0, 400.0, 100.0), 500.0), 50.0);
        assert_eq!(clamp_radius((0.0, 0.0, 400.0, 100.0), -3.0), 0.0);
        assert_eq!(clamp_radius((0.0, 0.0, 400.0, 100.0), 12.0), 12.0);
    }
}
