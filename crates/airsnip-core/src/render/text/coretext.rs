// SPDX-License-Identifier: GPL-3.0-only
//! Core Text 文本后端（macOS，EDT-4）。
//!
//! 与 DirectWrite 后端同一思路：按 `\n` 分行建 `CTLine`，在白底灰度位图上画黑字，反色得到覆盖度。
//! 族名不存在或为空时回退默认字体；字体族缺粗体/斜体变体时合成（描边加粗、倾斜矩阵），
//! 与 DirectWrite 的模拟粗斜体行为对应。全程纯 CPU、线程安全、无窗口依赖。

use std::collections::BTreeSet;
use std::ffi::c_void;
use std::ptr::null;

use super::coretext_ffi::*;
use super::{DEFAULT_FAMILY, FontSpec};

/// 合成斜体的倾斜量（tan ≈ 12°）。
const ITALIC_SKEW: CGAffineTransform = CGAffineTransform {
    a: 1.0,
    b: 0.0,
    c: 0.21,
    d: 1.0,
    tx: 0.0,
    ty: 0.0,
};
/// 合成粗体的描边宽度（字号百分比；负值 = 描边并填充）。
const FAKE_BOLD_STROKE: f64 = -4.0;
const IDENTITY: CGAffineTransform = CGAffineTransform {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    tx: 0.0,
    ty: 0.0,
};

/// CF 对象所有权：Drop 时 `CFRelease`。
struct Cf(CFTypeRef);

impl Cf {
    fn new(p: CFTypeRef) -> Option<Self> {
        // 必须惰性构造：`then_some(Cf(p))` 会先造出 Cf，空指针时随即 Drop → CFRelease(NULL) 崩溃
        (!p.is_null()).then(|| Cf(p))
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

fn cf_string(s: &str) -> Option<Cf> {
    Cf::new(unsafe { CFStringCreateWithBytes(null(), s.as_ptr(), s.len() as CFIndex, kCFStringEncodingUTF8, 0) })
}

fn cf_dict(keys: &[CFTypeRef], values: &[CFTypeRef]) -> Option<Cf> {
    Cf::new(unsafe {
        CFDictionaryCreate(
            null(),
            keys.as_ptr(),
            values.as_ptr(),
            keys.len() as CFIndex,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    })
}

fn rust_string(s: CFStringRef) -> Option<String> {
    let max = unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(s), kCFStringEncodingUTF8) } + 1;
    let mut buf = vec![0u8; max as usize];
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr().cast(), max, kCFStringEncodingUTF8) } == 0 {
        return None;
    }
    let n = buf.iter().position(|&b| b == 0)?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}

/// 按族名建字体；描述符匹配不到时 Core Text 会给替代字体，族名不符即视为不存在。
fn font_of_family(family: &str, size: f64) -> Option<Cf> {
    let name = cf_string(family)?;
    let attrs = cf_dict(&[unsafe { kCTFontFamilyNameAttribute }], &[name.0])?;
    let desc = Cf::new(unsafe { CTFontDescriptorCreateWithAttributes(attrs.0) })?;
    let font = Cf::new(unsafe { CTFontCreateWithFontDescriptor(desc.0, size, null()) })?;
    let actual = Cf::new(unsafe { CTFontCopyFamilyName(font.0) })?;
    rust_string(actual.0)?.eq_ignore_ascii_case(family).then_some(font)
}

fn with_traits(font: &Cf, traits: u32) -> Option<Cf> {
    Cf::new(unsafe { CTFontCreateCopyWithSymbolicTraits(font.0, 0.0, null(), traits, traits) })
}

/// 规格对应的字体；第二项为是否需要合成粗体（字体族无粗体字重）。
fn styled_font(spec: &FontSpec) -> Option<(Cf, bool)> {
    let size = spec.size as f64;
    let mut font = Some(spec.family.as_ref())
        .filter(|f| !f.is_empty())
        .and_then(|f| font_of_family(f, size))
        .or_else(|| font_of_family(DEFAULT_FAMILY, size))?;
    let mut fake_bold = false;
    if spec.bold {
        match with_traits(&font, kCTFontBoldTrait) {
            Some(f) => font = f,
            None => fake_bold = true,
        }
    }
    if spec.italic {
        font = match with_traits(&font, kCTFontItalicTrait) {
            Some(f) => f,
            None => Cf::new(unsafe { CTFontCreateCopyWithAttributes(font.0, 0.0, &ITALIC_SKEW, null()) })?,
        };
    }
    Some((font, fake_bold))
}

fn make_line(text: &str, font: &Cf, fake_bold: bool) -> Option<Cf> {
    let s = cf_string(text)?;
    let stroke = if fake_bold {
        let v = FAKE_BOLD_STROKE;
        Some(Cf::new(unsafe {
            CFNumberCreate(null(), kCFNumberDoubleType, (&v as *const f64).cast::<c_void>())
        })?)
    } else {
        None
    };
    let attrs = match &stroke {
        Some(n) => cf_dict(
            &[unsafe { kCTFontAttributeName }, unsafe { kCTStrokeWidthAttributeName }],
            &[font.0, n.0],
        )?,
        None => cf_dict(&[unsafe { kCTFontAttributeName }], &[font.0])?,
    };
    let attributed = Cf::new(unsafe { CFAttributedStringCreate(null(), s.0, attrs.0) })?;
    Cf::new(unsafe { CTLineCreateWithAttributedString(attributed.0) })
}

/// 分行布局结果：每行 `(CTLine, 基线距顶部)`，整体宽（最长行，不含行尾空白）与总高。
struct Layout {
    lines: Vec<(Cf, f64)>,
    width: f64,
    height: f64,
}

impl Layout {
    fn new(text: &str, spec: &FontSpec) -> Option<Self> {
        let (font, fake_bold) = styled_font(spec)?;
        let (fa, fd, fl) = unsafe {
            (
                CTFontGetAscent(font.0),
                CTFontGetDescent(font.0),
                CTFontGetLeading(font.0),
            )
        };
        let (mut lines, mut y, mut width) = (Vec::new(), 0.0f64, 0.0f64);
        for part in text.split('\n') {
            let line = make_line(part, &font, fake_bold)?;
            let (mut a, mut d, mut l) = (0.0, 0.0, 0.0);
            let w = unsafe { CTLineGetTypographicBounds(line.0, &mut a, &mut d, &mut l) }
                - unsafe { CTLineGetTrailingWhitespaceWidth(line.0) };
            // 行高取字体度量与本行实际（含回退字体）的较大者，空行也保持一行高
            let (a, d) = (a.max(fa), d.max(fd));
            lines.push((line, y + a));
            y += a + d + fl;
            width = width.max(w);
        }
        Some(Self {
            lines,
            width,
            height: y,
        })
    }
}

/// 测量文本尺寸（物理像素）。
pub(crate) fn measure(text: &str, spec: &FontSpec) -> Option<(f32, f32)> {
    let l = Layout::new(text, spec)?;
    Some((l.width.max(1.0) as f32, l.height.max(1.0) as f32))
}

/// 栅格化为覆盖度（cw×ch，每像素 1 字节）；布局左上角位于 (pad, pad)。
pub(crate) fn rasterize(text: &str, spec: &FontSpec, pad: u32, cw: u32, ch: u32) -> Option<Vec<u8>> {
    let layout = Layout::new(text, spec)?;
    let mut px = vec![255u8; cw as usize * ch as usize]; // 白底
    unsafe {
        let cs = CGColorSpaceCreateDeviceGray();
        let ctx = CGBitmapContextCreate(
            px.as_mut_ptr().cast(),
            cw as usize,
            ch as usize,
            8,
            cw as usize,
            cs,
            kCGImageAlphaNone,
        );
        CGColorSpaceRelease(cs);
        if ctx.is_null() {
            return None;
        }
        CGContextSetShouldSmoothFonts(ctx, false);
        CGContextSetTextMatrix(ctx, IDENTITY);
        // CG 坐标原点在左下：基线 y 需从顶部距离换算
        for (line, baseline) in &layout.lines {
            CGContextSetTextPosition(ctx, pad as f64, ch as f64 - (pad as f64 + baseline));
            CTLineDraw(line.0, ctx);
        }
        CGContextRelease(ctx);
    }
    Some(px.into_iter().map(|v| 255 - v).collect())
}

/// 系统字体族名（升序去重，过滤 `.` 开头的系统隐藏字体）。
pub(crate) fn families() -> Option<Vec<String>> {
    let arr = Cf::new(unsafe { CTFontManagerCopyAvailableFontFamilyNames() })?;
    let n = unsafe { CFArrayGetCount(arr.0) };
    let names: BTreeSet<String> = (0..n)
        .filter_map(|i| rust_string(unsafe { CFArrayGetValueAtIndex(arr.0, i) }))
        .filter(|s| !s.is_empty() && !s.starts_with('.'))
        .collect();
    (!names.is_empty()).then(|| names.into_iter().collect())
}
