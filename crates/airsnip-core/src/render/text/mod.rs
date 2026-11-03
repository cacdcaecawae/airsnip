// SPDX-License-Identifier: GPL-3.0-only
//! 文本测量与栅格化（EDT-4）：`measure` 得到文字尺寸（写入对象测量缓存），
//! `draw` 把文本栅格化进 tiny_skia 像素图。
//!
//! 平台后端只做两件事：测量（`backend::measure`）与把文字栅格化成单通道覆盖度
//! （`backend::rasterize`，每像素 1 字节，0 = 无字形）；覆盖度与目标色的混合在本模块统一完成。
//! Windows 用 DirectWrite（`dwrite`），macOS 用 Core Text（`coretext`）。
//! 后端不可用时返回估算测量并静默跳过绘制（不崩溃）。

use std::sync::Arc;

use tiny_skia::{Pixmap, Transform};

use crate::editor::{Kind, Point};

#[cfg(target_os = "macos")]
pub(crate) mod coretext;
#[cfg(target_os = "macos")]
mod coretext_ffi;
#[cfg(target_os = "macos")]
pub(crate) use coretext as backend;
#[cfg(windows)]
pub(crate) mod dwrite;
#[cfg(windows)]
mod dwrite_dib;
#[cfg(windows)]
pub(crate) use dwrite as backend;

/// 默认字体族（与对象模型同源，按平台取系统自带中文字体）。
pub const DEFAULT_FAMILY: &str = crate::editor::DEFAULT_FONT_FAMILY;

/// 字体规格（EDT-4）：族名 + 字号 + 粗体/斜体。
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub family: Arc<str>,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
}

impl FontSpec {
    /// 默认字体族、常规字重。
    pub fn new(size: f32) -> Self {
        Self {
            family: Arc::from(DEFAULT_FAMILY),
            size,
            bold: false,
            italic: false,
        }
    }

    /// 由文本对象的字段构造。
    pub fn from_parts(family: &Arc<str>, size: f32, bold: bool, italic: bool) -> Self {
        Self {
            family: family.clone(),
            size,
            bold,
            italic,
        }
    }
}

/// 后端不可用时的估算（供 bounds 使用，保证不崩溃）。
fn approx(text: &str, font_size: f32) -> (f32, f32) {
    let n = text.encode_utf16().count() as f32;
    ((n * font_size * 0.9 + font_size).max(1.0), font_size * 1.4)
}

/// 测量文本尺寸（物理像素；`\n` 换行时宽为最长行、高为总行高）。失败时给估算值。
pub fn measure(text: &str, spec: &FontSpec) -> (f32, f32) {
    backend::measure(text, spec).unwrap_or_else(|| approx(text, spec.size))
}

/// 重测文本对象的尺寸缓存（EDT-4：字号 / 字体族 / 粗斜体变化后调用）；非文本对象无操作。
pub fn remeasure(kind: &mut Kind) {
    if let Kind::Text {
        text,
        font_size,
        font_family,
        bold,
        italic,
        size,
        ..
    } = kind
    {
        *size = measure(text, &FontSpec::from_parts(font_family, *font_size, *bold, *italic));
    }
}

/// 栅格化文本并混合进 `pixmap`（`tr` 用于导出烘焙的坐标平移）。
/// `pos` 为文字左上角。
pub fn draw(pixmap: &mut Pixmap, pos: Point, text: &str, spec: &FontSpec, color: [u8; 4], tr: Transform) {
    let (w, h) = measure(text, spec);
    let pad = crate::editor::text::pad(spec.size).ceil() as u32;
    let cw = (w.ceil() as u32 + pad * 2).max(4);
    let ch = (h.ceil() as u32 + pad * 2).max(4);
    // 字形以 (pad, pad) 为布局左上角栅格化，混合时映射回 pos
    let Some(cov) = backend::rasterize(text, spec, pad, cw, ch) else {
        return;
    };
    blend(pixmap, tr, pos, pad, cw, ch, &cov, color);
}

/// 按覆盖度（cw×ch，每像素 1 字节）把目标色混合进像素图。
#[allow(clippy::too_many_arguments)]
fn blend(pixmap: &mut Pixmap, tr: Transform, pos: Point, pad: u32, cw: u32, ch: u32, cov: &[u8], color: [u8; 4]) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let ox = pos.x - pad as f32;
    let oy = pos.y - pad as f32;
    let data = pixmap.data_mut();
    for y in 0..ch {
        let sy = oy + y as f32;
        for x in 0..cw {
            let c = cov[(y * cw + x) as usize];
            if c == 0 {
                continue;
            }
            let sx = ox + x as f32;
            let tx = (tr.sx * sx + tr.kx * sy + tr.tx).round() as i32;
            let ty = (tr.ky * sx + tr.sy * sy + tr.ty).round() as i32;
            if tx < 0 || ty < 0 || tx >= pw || ty >= ph {
                continue;
            }
            let off = (ty as usize * pw as usize + tx as usize) * 4;
            blend_one(&mut data[off..off + 4], color, c);
        }
    }
}

/// 单像素混合：`cov`（0-255）为字形覆盖度；目标 alpha 恒为不透明。
fn blend_one(d: &mut [u8], color: [u8; 4], cov: u8) {
    let a = color[3] as u32 * cov as u32 / 255;
    let inv = 255 - a;
    for c in 0..3 {
        d[c] = ((d[c] as u32 * inv + color[c] as u32 * a) / 255) as u8;
    }
    d[3] = 255;
}

#[cfg(test)]
mod tests;
