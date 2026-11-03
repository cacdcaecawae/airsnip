// SPDX-License-Identifier: GPL-3.0-only
//! 三次贝塞尔几何（EDT-3/EDT-3a）：展平、切线、分割、包围盒与点到折线距离。
//! 纯计算，不依赖渲染库；渲染与命中测试共用同一套展平参数，保证「看到的」与「点得到的」一致。

use super::Point;

/// 展平分段数（EDT-3a：命中测试把曲线展平为折线）。
/// 屏幕尺度（≤4K 单屏对角线约 4400px）下最大偏差约 0.5px，弧长近似误差 < 0.1%。
pub const SEGMENTS: usize = 24;

/// 曲线上的点（t ∈ [0,1]）。
pub fn point_at(p0: Point, c1: Point, c2: Point, p3: Point, t: f32) -> Point {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Point::new(
        a * p0.x + b * c1.x + c * c2.x + d * p3.x,
        a * p0.y + b * c1.y + c * c2.y + d * p3.y,
    )
}

/// 终点切线方向（未归一化，EDT-3：箭头头部沿 P3−P2；P2 与 P3 重合时回退到 P3−P1）。
pub fn end_tangent(c1: Point, c2: Point, p3: Point) -> Point {
    let main = Point::new(p3.x - c2.x, p3.y - c2.y);
    if main.x * main.x + main.y * main.y > 1e-6 {
        return main;
    }
    Point::new(p3.x - c1.x, p3.y - c1.y)
}

/// 展平为折线（含首尾，共 SEGMENTS + 1 个点）。
pub fn flatten(p0: Point, c1: Point, c2: Point, p3: Point) -> Vec<Point> {
    flatten_n(p0, c1, c2, p3, SEGMENTS)
}

/// 指定分段数展平。
pub fn flatten_n(p0: Point, c1: Point, c2: Point, p3: Point, segments: usize) -> Vec<Point> {
    (0..=segments)
        .map(|i| point_at(p0, c1, c2, p3, i as f32 / segments as f32))
        .collect()
}

/// 头部三角内的参数搜索精度（EDT-3a：头部只有几像素长，需比弧长近似更细）。
pub const HEAD_STEPS: usize = 96;

/// 求曲线自终点往回「从头部三角外跨入三角形内」的边界参数 t（EDT-3a：线身在头部边界处截断）。
/// 三角形以 `base` 为底边中心、`dir` 为指向尖端的方向：
/// `s = (p − base)·dir ∈ [0, head]`，横向偏移 `≤ half·(1 − s/head)`。
/// 返回 None 表示曲线未进入三角形（调用方回退到弧长截断）。
pub fn head_entry_t(
    p0: Point,
    c1: Point,
    c2: Point,
    p3: Point,
    base: Point,
    dir: (f32, f32),
    head: f32,
    half: f32,
) -> Option<f32> {
    let perp = (-dir.1, dir.0);
    let inside = |t: f32| {
        let p = point_at(p0, c1, c2, p3, t);
        let (dx, dy) = (p.x - base.x, p.y - base.y);
        let s = dx * dir.0 + dy * dir.1;
        if s < -1e-3 || s > head + 1e-3 {
            return false;
        }
        (dx * perp.0 + dy * perp.1).abs() <= half * (1.0 - s / head) + 1e-3
    };

    // 从终点往回找最后一个仍在三角形内的采样点
    let mut t_in = f32::NAN;
    for i in (0..=HEAD_STEPS).rev() {
        let t = i as f32 / HEAD_STEPS as f32;
        if inside(t) {
            t_in = t;
        } else {
            break;
        }
    }
    if t_in.is_nan() || t_in <= 0.0 {
        return None;
    }
    // 在「外部」与「内部」之间二分到 1/64 采样精度
    let (mut lo, mut hi) = ((t_in - 1.0 / HEAD_STEPS as f32).max(0.0), t_in);
    for _ in 0..6 {
        let mid = (lo + hi) * 0.5;
        if inside(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(hi)
}

/// 折线总长度。
pub fn polyline_len(pts: &[Point]) -> f32 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// 距终点弧长为 `back` 处的曲线参数 t（基于折线弧长近似，用于头部截断）。
pub fn t_at_arc_from_end(pts: &[Point], back: f32) -> f32 {
    let mut acc = 0.0;
    for i in (1..pts.len()).rev() {
        let seg = dist(pts[i - 1], pts[i]);
        if acc + seg >= back {
            let ratio = if seg > 1e-6 { (back - acc) / seg } else { 0.0 };
            let idx = i as f32 - ratio;
            return (idx / (pts.len() - 1) as f32).clamp(0.0, 1.0);
        }
        acc += seg;
    }
    0.0
}

/// de Casteljau 在 t 处分割，返回左段（t=0..t）的四个控制点。
pub fn split_left(p0: Point, c1: Point, c2: Point, p3: Point, t: f32) -> [Point; 4] {
    let a = lerp(p0, c1, t);
    let b = lerp(c1, c2, t);
    let c = lerp(c2, p3, t);
    let d = lerp(a, b, t);
    let e = lerp(b, c, t);
    [p0, a, d, lerp(d, e, t)]
}

/// 控制多边形（凸包）包围盒：贝塞尔曲线必落在凸包内，故可作为精确上界（EDT-3）。
pub fn hull_bounds(p0: Point, c1: Point, c2: Point, p3: Point) -> (f32, f32, f32, f32) {
    let xs = [p0.x, c1.x, c2.x, p3.x];
    let ys = [p0.y, c1.y, c2.y, p3.y];
    (
        xs.iter().copied().fold(f32::INFINITY, f32::min),
        ys.iter().copied().fold(f32::INFINITY, f32::min),
        xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    )
}

/// 点到折线的最短距离（EDT-3a 命中测试）。
pub fn dist_to_polyline(pts: &[Point], q: Point) -> f32 {
    pts.windows(2)
        .map(|w| dist_to_segment(q, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn dist(a: Point, b: Point) -> f32 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt()
}

fn dist_to_segment(q: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 1e-6 {
        (((q.x - a.x) * dx + (q.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(q, Point::new(a.x + dx * t, a.y + dy * t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> (Point, Point, Point, Point) {
        (
            Point::new(0.0, 0.0),
            Point::new(30.0, 0.0),
            Point::new(60.0, 0.0),
            Point::new(90.0, 0.0),
        )
    }

    #[test]
    fn straight_controls_keep_curve_on_the_line() {
        let (p0, c1, c2, p3) = line();
        let mid = point_at(p0, c1, c2, p3, 0.5);
        assert!((mid.x - 45.0).abs() < 0.01 && mid.y.abs() < 0.01, "{mid:?}");
        assert!((polyline_len(&flatten(p0, c1, c2, p3)) - 90.0).abs() < 0.05);
    }

    #[test]
    fn tangent_falls_back_when_control_meets_end() {
        let (_, c1, _, p3) = line();
        let t = end_tangent(c1, p3, p3);
        assert!(t.x > 0.0 && t.y == 0.0, "P2=P3 时应回退到 P3−P1：{t:?}");
        let t2 = end_tangent(c1, Point::new(80.0, 10.0), p3);
        assert!((t2.x - 10.0).abs() < 1e-3 && (t2.y + 10.0).abs() < 1e-3);
    }

    #[test]
    fn arc_lookup_matches_length() {
        let (p0, c1, c2, p3) = line();
        let pts = flatten(p0, c1, c2, p3);
        // 距终点 30px（总长 90）→ t ≈ 2/3
        let t = t_at_arc_from_end(&pts, 30.0);
        assert!((t - 2.0 / 3.0).abs() < 0.02, "t={t}");
    }

    #[test]
    fn split_left_preserves_endpoint() {
        let (p0, _, _, p3) = line();
        let (c1, c2) = (Point::new(10.0, 40.0), Point::new(60.0, -30.0));
        let seg = split_left(p0, c1, c2, p3, 0.4);
        let end = point_at(p0, c1, c2, p3, 0.4);
        assert!((seg[3].x - end.x).abs() < 1e-3 && (seg[3].y - end.y).abs() < 1e-3);
    }

    #[test]
    fn head_entry_stops_at_triangle_boundary() {
        // P2 靠近尖端：按弧长截断会与头部错开几像素（线身脱开、穿出三角侧面）
        let (p0, p3) = (Point::new(0.0, 0.0), Point::new(240.0, 0.0));
        let (c1, c2) = (Point::new(80.0, 0.0), Point::new(235.0, 15.0));
        let (head, half) = (16.0f32, 7.2f32);
        let t = end_tangent(c1, c2, p3);
        let len = (t.x * t.x + t.y * t.y).sqrt();
        let dir = (t.x / len, t.y / len);
        let base = Point::new(p3.x - dir.0 * head, p3.y - dir.1 * head);
        let inside = |t: f32| {
            let p = point_at(p0, c1, c2, p3, t);
            let (dx, dy) = (p.x - base.x, p.y - base.y);
            let s = dx * dir.0 + dy * dir.1;
            s >= 0.0 && s <= head && (dx * -dir.1 + dy * dir.0).abs() <= half * (1.0 - s / head)
        };

        let t0 = head_entry_t(p0, c1, c2, p3, base, dir, head, half).expect("应能求出进入点");
        assert!(inside(t0), "截断点必须在头部三角内：t={t0}");
        assert!(!inside(t0 - 0.01), "截断点必须是进入三角的边界（再往前应在三角外）");
    }

    #[test]
    fn hull_covers_curve_and_distance_hits_stroke() {
        let (p0, c1, c2, p3) = (
            Point::new(0.0, 0.0),
            Point::new(0.0, 100.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
        );
        let (x0, y0, x1, y1) = hull_bounds(p0, c1, c2, p3);
        assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 100.0, 100.0));

        let pts = flatten(p0, c1, c2, p3);
        let on = point_at(p0, c1, c2, p3, 0.5);
        assert!(dist_to_polyline(&pts, on) < 0.1, "曲线上的点距离应≈0");
        assert!(dist_to_polyline(&pts, Point::new(on.x, on.y + 40.0)) > 30.0);
    }
}
