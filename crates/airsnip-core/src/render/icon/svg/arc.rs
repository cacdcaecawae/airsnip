// SPDX-License-Identifier: GPL-3.0-only
//! SVG 椭圆弧（`A` 命令）转三次贝塞尔：端点参数化 → 中心参数化，按 ≤90° 分段。

use std::f32::consts::{FRAC_PI_2, TAU};

use tiny_skia::PathBuilder;

/// 从 `from` 到 `to` 画一段椭圆弧，追加到 `pb`。半径不足时按 SVG 规范等比放大。
pub(super) fn arc_to(
    pb: &mut PathBuilder,
    from: (f32, f32),
    radii: (f32, f32),
    rot_deg: f32,
    large: bool,
    sweep: bool,
    to: (f32, f32),
) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    if radii.0 == 0.0 || radii.1 == 0.0 {
        pb.line_to(x1, y1);
        return;
    }
    let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
    let (sin_p, cos_p) = rot_deg.to_radians().sin_cos();
    let dx2 = (x0 - x1) / 2.0;
    let dy2 = (y0 - y1) / 2.0;
    let x1p = cos_p * dx2 + sin_p * dy2;
    let y1p = -sin_p * dx2 + cos_p * dy2;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    // 圆心：两个候选由 large/sweep 的组合决定
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let co = if den > 0.0 {
        sign * (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    let cxp = co * rx * y1p / ry;
    let cyp = -co * ry * x1p / rx;
    let cx = cos_p * cxp - sin_p * cyp + (x0 + x1) / 2.0;
    let cy = sin_p * cxp + cos_p * cyp + (y0 + y1) / 2.0;

    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let a = if len > 0.0 {
            (dot / len).clamp(-1.0, 1.0).acos()
        } else {
            0.0
        };
        if ux * vy - uy * vx < 0.0 { -a } else { a }
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let start = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= TAU;
    }
    if sweep && delta < 0.0 {
        delta += TAU;
    }

    let segs = (delta.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / segs as f32;
    // 单段 ≤90° 的圆弧用三次贝塞尔近似的控制点比例
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let (mut a, mut px, mut py) = (start, x0, y0);
    for _ in 0..segs {
        let b = a + step;
        let (sa, ca) = a.sin_cos();
        let (sb, cb) = b.sin_cos();
        let ex = cx + rx * cb * cos_p - ry * sb * sin_p;
        let ey = cy + rx * cb * sin_p + ry * sb * cos_p;
        let c1x = px + k * (-rx * sa * cos_p - ry * ca * sin_p);
        let c1y = py + k * (-rx * sa * sin_p + ry * ca * cos_p);
        let c2x = ex - k * (-rx * sb * cos_p - ry * cb * sin_p);
        let c2y = ey - k * (-rx * sb * sin_p + ry * cb * cos_p);
        pb.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
        a = b;
        px = ex;
        py = ey;
    }
}
