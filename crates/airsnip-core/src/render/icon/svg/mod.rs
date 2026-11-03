// SPDX-License-Identifier: GPL-3.0-only
//! 极简 SVG 解析（UI 图标专用）：只处理 `assets/icons/ui/` 下的受控格式。
//! 支持 `<path d>`（`M L H V C S Q T A Z`，绝对与相对命令）与 `fill-rule="evenodd"`；
//! 不支持 `<g>`、`transform`、渐变、`style` —— 规格见 `assets/icons/ui/README.md`。
//! 不做通用 SVG 兼容，遇到不认识的命令直接丢弃该 path。

mod arc;
mod lex;

use tiny_skia::{FillRule, Path, PathBuilder};

use arc::arc_to;
use lex::Lexer;

/// 一个图标：若干 path（含填充规则）与合并后的包围盒（viewBox 单位）。
pub struct SvgIcon {
    pub paths: Vec<(Path, FillRule)>,
    /// `[x0, y0, x1, y1]`
    pub bounds: [f32; 4],
}

/// 解析 SVG 文档，返回路径与包围盒。无有效 path 时返回单位包围盒。
pub fn parse(src: &str) -> SvgIcon {
    let mut paths: Vec<(Path, FillRule)> = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("<path") {
        let tail = &rest[at..];
        let end = tail
            .find("/>")
            .map_or_else(|| tail.find('>').map_or(tail.len(), |i| i + 1), |i| i + 2);
        let tag = &tail[..end];
        rest = &tail[end..];
        if tag.contains("fill=\"none\"") {
            continue;
        }
        let Some(d) = attr(tag, "d") else { continue };
        let rule = if tag.contains("fill-rule=\"evenodd\"") {
            FillRule::EvenOdd
        } else {
            FillRule::Winding
        };
        if let Some(path) = build_path(d) {
            paths.push((path, rule));
        }
    }

    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for (path, _) in &paths {
        let r = path.bounds();
        b[0] = b[0].min(r.left());
        b[1] = b[1].min(r.top());
        b[2] = b[2].max(r.right());
        b[3] = b[3].max(r.bottom());
    }
    if !b[0].is_finite() || !b[2].is_finite() {
        b = [0.0, 0.0, 1.0, 1.0];
    }
    SvgIcon { paths, bounds: b }
}

/// 取标签内某个属性的原始值。
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let pat = format!("{name}=\"");
    let start = tag.find(&pat)? + pat.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

/// 把 `d` 属性解析成路径；格式异常时返回 None。
fn build_path(d: &str) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut lx = Lexer::new(d);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut last_cubic: Option<(f32, f32)> = None;
    let mut last_quad: Option<(f32, f32)> = None;
    let mut cmd = 0u8;

    loop {
        if lx.peek_cmd() {
            cmd = lx.take_cmd()?;
        } else if cmd == 0 {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };

        match cmd.to_ascii_uppercase() {
            b'M' => {
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                sx = x;
                sy = y;
                pb.move_to(x, y);
                // 后续坐标对按 L 处理（SVG 规范）
                cmd = if rel { b'l' } else { b'L' };
                last_cubic = None;
                last_quad = None;
            }
            b'L' => {
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                pb.line_to(x, y);
                last_cubic = None;
                last_quad = None;
            }
            b'H' => {
                x = lx.num()? + if rel { x } else { 0.0 };
                pb.line_to(x, y);
                last_cubic = None;
                last_quad = None;
            }
            b'V' => {
                y = lx.num()? + if rel { y } else { 0.0 };
                pb.line_to(x, y);
                last_cubic = None;
                last_quad = None;
            }
            b'C' => {
                let c1 = (lx.num()? + ox, lx.num()? + oy);
                let c2 = (lx.num()? + ox, lx.num()? + oy);
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                pb.cubic_to(c1.0, c1.1, c2.0, c2.1, x, y);
                last_cubic = Some(c2);
                last_quad = None;
            }
            b'S' => {
                let c1 = last_cubic.map_or((x, y), |c| (2.0 * x - c.0, 2.0 * y - c.1));
                let c2 = (lx.num()? + ox, lx.num()? + oy);
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                pb.cubic_to(c1.0, c1.1, c2.0, c2.1, x, y);
                last_cubic = Some(c2);
                last_quad = None;
            }
            b'Q' => {
                let q = (lx.num()? + ox, lx.num()? + oy);
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                pb.quad_to(q.0, q.1, x, y);
                last_quad = Some(q);
                last_cubic = None;
            }
            b'T' => {
                let q = last_quad.map_or((x, y), |c| (2.0 * x - c.0, 2.0 * y - c.1));
                x = lx.num()? + ox;
                y = lx.num()? + oy;
                pb.quad_to(q.0, q.1, x, y);
                last_quad = Some(q);
                last_cubic = None;
            }
            b'A' => {
                let radii = (lx.num()?, lx.num()?);
                let rot = lx.num()?;
                let large = lx.num()? != 0.0;
                let sweep = lx.num()? != 0.0;
                let nx = lx.num()? + ox;
                let ny = lx.num()? + oy;
                arc_to(&mut pb, (x, y), radii, rot, large, sweep, (nx, ny));
                x = nx;
                y = ny;
                last_cubic = None;
                last_quad = None;
            }
            b'Z' => {
                pb.close();
                x = sx;
                y = sy;
                last_cubic = None;
                last_quad = None;
            }
            _ => break,
        }
        if lx.at_end() {
            break;
        }
    }
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds_of(d: &str) -> [f32; 4] {
        parse(&format!("<svg><path fill=\"currentColor\" d=\"{d}\"/></svg>")).bounds
    }

    /// 绝对与相对命令得到同一形状。
    #[test]
    fn absolute_and_relative_match() {
        let a = bounds_of("M10 20L110 20L110 70Z");
        let b = bounds_of("m10 20l100 0l0 50z");
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 0.01, "第 {i} 项不一致：{a:?} / {b:?}");
        }
    }

    /// 圆弧端点必须落在目标坐标上（半径过小时按规范放大）。
    #[test]
    fn arc_reaches_endpoint() {
        let b = bounds_of("M0 100A100 100 0 0 1 200 100");
        assert!((b[0] - 0.0).abs() < 0.5, "左边界 {b:?}");
        assert!((b[2] - 200.0).abs() < 0.5, "右边界 {b:?}");
        assert!(b[1] < 5.0, "应向上拱起：{b:?}");
    }

    /// `fill-rule="evenodd"` 被识别为偶数填充。
    #[test]
    fn evenodd_is_detected() {
        let icon = parse("<svg><path fill=\"currentColor\" fill-rule=\"evenodd\" d=\"M0 0H10V10H0Z\"/></svg>");
        assert_eq!(icon.paths.len(), 1);
        assert_eq!(icon.paths[0].1, FillRule::EvenOdd);
    }

    /// `fill="none"` 的 path 应被跳过。
    #[test]
    fn fill_none_is_skipped() {
        let src = "<svg><path fill=\"none\" d=\"M0 0H10\"/><path fill=\"currentColor\" d=\"M0 0H5V5Z\"/></svg>";
        assert_eq!(parse(src).paths.len(), 1);
    }

    /// 指数与无前导零的小数能被正确解析。
    #[test]
    fn exotic_numbers_parse() {
        let b = bounds_of("M.5 .5L1e2 .5L1e2 1E2Z");
        assert!((b[2] - 100.0).abs() < 0.01, "右边界 {b:?}");
        assert!((b[3] - 100.0).abs() < 0.01, "下边界 {b:?}");
    }
}
