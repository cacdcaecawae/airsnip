// SPDX-License-Identifier: GPL-3.0-only
//! 文本测量与栅格化测试（两个平台后端共用同一组断言）。

use super::*;

/// 与默认字体明显不同的系统自带字体（宋体类衬线）。
#[cfg(windows)]
const OTHER_FAMILY: &str = "SimSun";
#[cfg(target_os = "macos")]
const OTHER_FAMILY: &str = "Songti SC";

fn spec(size: f32) -> FontSpec {
    FontSpec::new(size)
}

#[test]
fn measure_handles_chinese_and_latin() {
    let (w1, h1) = measure("挺好", &spec(24.0));
    let (w2, h2) = measure("ab", &spec(24.0));
    assert!(w1 > 0.0 && h1 > 0.0, "中文测量应非零：{w1}x{h1}");
    assert!(w2 > 0.0, "拉丁字符测量应非零");
    assert!(w1 >= w2, "两个全角字应不窄于两个半角字符");
    assert!((h1 - h2).abs() < 8.0, "同行字号高度应接近：{h1} vs {h2}");
}

#[test]
fn larger_font_measures_larger() {
    let (w1, _) = measure("测试", &spec(16.0));
    let (w2, _) = measure("测试", &spec(48.0));
    assert!(w2 > w1 * 2.0, "字号 3 倍宽度应近似 3 倍：{w1} vs {w2}");
}

#[test]
fn bold_and_italic_render_differently() {
    // 中文粗体在部分字体下字宽略窄，故以「实际栅格化像素」为判据：
    // 粗体笔画更粗（着墨像素更多），斜体字形发生倾斜（像素图不同）
    let ink = |spec: &FontSpec| {
        let mut p = Pixmap::new(160, 60).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
        draw(
            &mut p,
            Point::new(5.0, 5.0),
            "bold Wm 粗",
            spec,
            [255, 255, 255, 255],
            Transform::identity(),
        );
        p.data().chunks_exact(4).filter(|px| px[0] > 100).count()
    };
    let regular = FontSpec::new(24.0);
    let bold = FontSpec {
        bold: true,
        ..regular.clone()
    };
    let italic = FontSpec {
        italic: true,
        ..regular.clone()
    };
    let (r, b, i) = (ink(&regular), ink(&bold), ink(&italic));
    assert!(b > r, "粗体着墨应更多：regular={r} bold={b}");
    assert!(i != r, "斜体字形应与常规不同：regular={r} italic={i}");
}

#[test]
fn different_families_measure_differently() {
    let default = FontSpec::new(24.0);
    let other = FontSpec {
        family: Arc::from(OTHER_FAMILY),
        ..default.clone()
    };
    let (w1, _) = measure("字体测量 Family", &default);
    let (w2, _) = measure("字体测量 Family", &other);
    assert!(
        w1 > 0.0 && w2 > 0.0 && (w1 - w2).abs() > 0.5,
        "不同字体族测量应不同：{w1} vs {w2}"
    );
}

#[test]
fn remeasure_updates_text_cache_with_font() {
    let mut kind = Kind::Text {
        pos: Point::new(0.0, 0.0),
        text: Arc::from("测量"),
        font_size: 24.0,
        font_family: Arc::from(DEFAULT_FAMILY),
        bold: false,
        italic: false,
        size: (0.0, 0.0),
    };
    remeasure(&mut kind);
    let Kind::Text { size, .. } = kind else {
        panic!("应仍是文本")
    };
    assert!(size.0 > 0.0 && size.1 > 0.0, "尺寸缓存应被写入：{size:?}");
}

/// 多行文本：高度约为单行的行数倍，宽度取最长行。
#[test]
fn multiline_measures_tallest_and_widest() {
    let (w1, h1) = measure("短", &spec(24.0));
    let (w3, h3) = measure("短\n更长的一行\n短", &spec(24.0));
    assert!(h3 > h1 * 2.5 && h3 < h1 * 3.5, "三行高度应约为单行 3 倍：{h1} vs {h3}");
    assert!(w3 > w1 * 3.0, "宽度应取最长行：{w1} vs {w3}");
}

#[test]
fn draw_produces_white_pixels_on_black() {
    let mut p = Pixmap::new(120, 80).unwrap();
    p.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    draw(
        &mut p,
        Point::new(10.0, 10.0),
        "测试",
        &spec(24.0),
        [255, 255, 255, 255],
        Transform::identity(),
    );
    let any = p.data().chunks_exact(4).any(|px| px[0] > 0);
    assert!(any, "黑底上应绘出白色字形");
}

/// 字形落在 `pos` 起的测量框内（上下不翻转、不越出太多）。
#[test]
fn draw_lands_inside_measured_box() {
    let mut p = Pixmap::new(200, 120).unwrap();
    p.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let (x0, y0) = (20.0, 30.0);
    let (w, h) = measure("字Ag", &spec(32.0));
    draw(
        &mut p,
        Point::new(x0, y0),
        "字Ag",
        &spec(32.0),
        [255, 255, 255, 255],
        Transform::identity(),
    );
    let lit: Vec<(f32, f32)> = p
        .data()
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, px)| px[0] > 128)
        .map(|(i, _)| ((i % 200) as f32, (i / 200) as f32))
        .collect();
    assert!(!lit.is_empty(), "应有字形像素");
    let tol = 4.0;
    for &(x, y) in &lit {
        assert!(x >= x0 - tol && x <= x0 + w + tol, "x={x} 超出 [{x0}, {}]", x0 + w);
        assert!(y >= y0 - tol && y <= y0 + h + tol, "y={y} 超出 [{y0}, {}]", y0 + h);
    }
    // 上半部分应有像素（汉字与大写字母顶部），防止整体上下翻转后只剩底部
    assert!(lit.iter().any(|&(_, y)| y < y0 + h * 0.4), "字形上半部分应有像素");
}

#[test]
fn draw_translates_with_transform() {
    let mut p = Pixmap::new(200, 100).unwrap();
    p.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let tr = Transform::from_translate(50.0, 20.0);
    draw(
        &mut p,
        Point::new(10.0, 10.0),
        "测试",
        &spec(24.0),
        [255, 255, 255, 255],
        tr,
    );
    // 平移后字形应出现在右侧区域（x ≥ 50），原位置（x < 50）应无像素
    let count = |right: bool| {
        p.data()
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, px)| ((i % 200) >= 50) == right && px[0] > 0)
            .count()
    };
    assert_eq!(count(false), 0, "平移后左侧不应有字形");
    assert!(count(true) > 0, "平移后右侧应有字形");
}

/// 未知 / 空字体族名必须回退到默认字体且不得崩溃。
/// 回归（Windows）：`CreateTextFormat` 的族名只能按 NUL 结尾读取，缺 NUL 时会读到缓冲区外的堆内存——
/// 表现为换字体后仍渲染成默认字体，空串还会直接访问越界崩溃。
#[test]
fn unknown_or_empty_family_falls_back_without_crashing() {
    let empty = FontSpec {
        family: Arc::from(""),
        ..FontSpec::new(24.0)
    };
    let unknown = FontSpec {
        family: Arc::from("不存在的字体族名 xyz"),
        ..FontSpec::new(24.0)
    };
    let a = measure("回退测试 Family", &empty);
    let b = measure("回退测试 Family", &unknown);
    assert!(a.0 > 0.0 && b.0 > 0.0);
    assert_eq!(a, b, "未知与空族名应落到同一个默认字体：{a:?} vs {b:?}");
}
