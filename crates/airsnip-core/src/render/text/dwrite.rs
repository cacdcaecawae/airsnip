// SPDX-License-Identifier: GPL-3.0-only
//! DirectWrite 文本后端（Windows，EDT-4）。
//!
//! 渲染路径：IDWriteTextLayout 布局 → 自定义 `IDWriteTextRenderer` 接收字形 →
//! `IDWriteBitmapRenderTarget` 栅格化进内存 DIB（黑字白底）→ 反色得到覆盖度。
//! 全程纯 CPU、无窗口依赖。

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject, GdiFlush, HDC, PatBlt, WHITENESS};
use windows::core::{BOOL, IUnknown, PCWSTR, Ref, Result, implement, w};

use super::FontSpec;
use super::dwrite_dib::{create_dib, read_coverage};

/// 布局构造的宽高上限：禁止自动换行。
const MAX_LAYOUT: f32 = 100_000.0;

/// 共享工厂缓存（DWrite SHARED 工厂线程安全，`IDWriteFactory` 实现了 Send+Sync）。
pub(crate) fn factory() -> Option<&'static IDWriteFactory> {
    static F: std::sync::OnceLock<Option<IDWriteFactory>> = std::sync::OnceLock::new();
    F.get_or_init(|| unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok() })
        .as_ref()
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// 宽字符串（尾部补 NUL）。
/// **必须**用于传给 `CreateTextFormat` 的字体族名：DirectWrite 不接收长度参数，
/// 只按 NUL 结尾读取；缺 NUL 会读到缓冲区之后的堆内存，导致字体名解析到垃圾
/// （表现为「换了字体却渲染成默认字体」），空串还会直接访问越界崩溃。
fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_format(f: &IDWriteFactory, spec: &FontSpec) -> Option<IDWriteTextFormat> {
    let family = wide_z(&spec.family);
    let weight = if spec.bold {
        DWRITE_FONT_WEIGHT_BOLD
    } else {
        DWRITE_FONT_WEIGHT_NORMAL
    };
    let style = if spec.italic {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    unsafe {
        f.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None,
            weight,
            style,
            DWRITE_FONT_STRETCH_NORMAL,
            spec.size,
            w!("zh-CN"),
        )
    }
    .ok()
}

fn layout(f: &IDWriteFactory, text: &str, spec: &FontSpec) -> Option<IDWriteTextLayout> {
    let format = create_format(f, spec)?;
    unsafe { f.CreateTextLayout(&utf16(text), &format, MAX_LAYOUT, MAX_LAYOUT) }.ok()
}

/// 测量文本尺寸；DirectWrite 不可用时返回 None（调用方给估算值）。
pub(crate) fn measure(text: &str, spec: &FontSpec) -> Option<(f32, f32)> {
    let layout = layout(factory()?, text, spec)?;
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut m) }.ok()?;
    // 多行（\n 换行）时 width 为布局宽度（最长行），height 为总行高
    Some((m.width.max(1.0), m.height.max(1.0)))
}

/// 自定义文字器：只把字形转发给位图渲染目标（黑字白底），忽略下划线/删除线。
// 实现目标为 `Renderer_Impl`（windows-implement 0.60 约定），字段经 `this` 访问。
#[implement(IDWriteTextRenderer)]
struct Renderer {
    target: IDWriteBitmapRenderTarget,
    params: IDWriteRenderingParams,
}

impl IDWritePixelSnapping_Impl for Renderer_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const core::ffi::c_void) -> Result<BOOL> {
        Ok(false.into())
    }

    fn GetCurrentTransform(&self, _: *const core::ffi::c_void, transform: *mut DWRITE_MATRIX) -> Result<()> {
        unsafe {
            *transform = DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _: *const core::ffi::c_void) -> Result<f32> {
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for Renderer_Impl {
    fn DrawGlyphRun(
        &self,
        _ctx: *const core::ffi::c_void,
        x: f32,
        y: f32,
        mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        _desc: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        unsafe {
            self.this
                .target
                .DrawGlyphRun(x, y, mode, run, &self.this.params, COLORREF(0), None)
        }?;
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_UNDERLINE,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_STRIKETHROUGH,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: Ref<IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

/// 栅格化为覆盖度（cw×ch，每像素 1 字节）；布局左上角位于 (pad, pad)。
pub(crate) fn rasterize(text: &str, spec: &FontSpec, pad: u32, cw: u32, ch: u32) -> Option<Vec<u8>> {
    let f = factory()?;
    // 内存 DIB（top-down BGRA），DirectWrite 栅格化画布
    let (dc, bmp) = create_dib(cw, ch)?;
    let out = rasterize_into(f, dc, text, spec, pad, cw, ch);
    unsafe {
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
    }
    out
}

fn rasterize_into(
    f: &IDWriteFactory,
    dc: HDC,
    text: &str,
    spec: &FontSpec,
    pad: u32,
    cw: u32,
    ch: u32,
) -> Option<Vec<u8>> {
    let target = unsafe {
        f.GetGdiInterop()
            .and_then(|i| i.CreateBitmapRenderTarget(Some(dc), cw, ch))
    }
    .ok()?;
    unsafe { target.SetPixelsPerDip(1.0) }.ok()?;
    // 渲染目标的内存 DC：白化 + 读回共用这一个句柄
    let rt_dc = unsafe { target.GetMemoryDC() };
    unsafe {
        let _ = PatBlt(rt_dc, 0, 0, cw as i32, ch as i32, WHITENESS);
    }
    let params = unsafe { f.CreateRenderingParams() }.ok()?;
    let layout = layout(f, text, spec)?;
    let renderer: IDWriteTextRenderer = Renderer { target, params }.into();
    // origin 取 (pad, pad)：字形在 DIB 中相对四周内边距居中，混合时映射回 pos 即为文字左上角
    unsafe { layout.Draw(None, &renderer, pad as f32, pad as f32) }.ok()?;
    let _ = unsafe { GdiFlush() };
    // 字形绘制在 render target 内部位图上，用 GetDIBits 读回像素
    read_coverage(rt_dc, cw, ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_write_factory_available() {
        let f = unsafe { DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED) };
        assert!(f.is_ok(), "桌面会话应能创建 DirectWrite 工厂");
    }
}
