// SPDX-License-Identifier: GPL-3.0-only
//! 弹出菜单上屏绘制：内容 → premultiplied BGRA → UpdateLayeredWindow。

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{COLORREF, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, GetDC, ReleaseDC};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, ULW_ALPHA, UpdateLayeredWindow};

use super::super::super::overlay::geometry::SelRect;
use crate::editor::Point as FPoint;
use crate::overlay::dib::convert_rgba_to_bgra;
use crate::overlay::shape::rounded_rect;
use crate::render::text as dwrite;

use super::{CORNER, Ctx, ITEM_H, PAD_X, SEP_H};

/// 全量重绘：内容 → premultiplied BGRA → UpdateLayeredWindow 上屏。
pub(super) fn present(ctx: &Ctx) {
    let Some(mut pix) = Pixmap::new(ctx.w as u32, ctx.h as u32) else {
        return;
    };
    let p = ctx.palette;
    let s = ctx.scale;
    let item_h = (ITEM_H as f32 * s) as i32;
    let pad_x = (PAD_X as f32 * s) as i32;
    let font_size = 14.0 * s;
    let text_h = (font_size * 1.2) as i32;

    // 内容背景（圆角矩形，窗口四角透明）+ 1px 细边框提供对比度（替代阴影）
    rounded_rect(
        &mut pix,
        SelRect {
            x: 0,
            y: 0,
            w: ctx.w,
            h: ctx.h,
        },
        CORNER,
        p.bg,
        Some(p.border),
    );

    for (i, it) in ctx.items.iter().enumerate() {
        let y = ctx.ys[i];
        if it.separator_before {
            let line_y = y - (SEP_H as f32 * s) as i32 / 2;
            let rect = SelRect {
                x: pad_x,
                y: line_y,
                w: ctx.w - pad_x * 2,
                h: 1,
            };
            rounded_rect(&mut pix, rect, 0.5, p.divider, None);
        }
        // 悬停高亮整行占满（不留左右空隙）
        let rect = SelRect {
            x: 0,
            y,
            w: ctx.w,
            h: item_h,
        };
        if ctx.hover == Some(i) {
            rounded_rect(&mut pix, rect, 0.0, p.row_hover, None);
        }
        let color = p.text;
        let ty = y + (item_h - text_h) / 2;
        let spec = dwrite::FontSpec::new(font_size);
        dwrite::draw(
            &mut pix,
            FPoint::new(pad_x as f32, ty as f32),
            it.text,
            &spec,
            color,
            tiny_skia::Transform::identity(),
        );
    }

    // premultiplied RGBA → BGRA 上屏
    convert_rgba_to_bgra(pix.data(), ctx.dib_bits as usize, ctx.w, 0, 0, ctx.w, ctx.h, 1);
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.0.is_null() {
            return;
        }
        // 窗口当前位置（UpdateLayeredWindow 的 pptdst 会把窗口移动过去，必须传真实坐标）
        let mut wr = RECT::default();
        let _ = GetWindowRect(ctx.hwnd, &mut wr);
        let pt = POINT { x: wr.left, y: wr.top };
        let size = SIZE { cx: ctx.w, cy: ctx.h };
        let pt_src = POINT::default();
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = UpdateLayeredWindow(
            ctx.hwnd,
            Some(hdc_screen),
            Some(&pt as *const POINT),
            Some(&size as *const SIZE),
            Some(ctx.mem_dc),
            Some(&pt_src as *const POINT),
            COLORREF(0),
            Some(&blend as *const BLENDFUNCTION),
            ULW_ALPHA,
        );
        let _ = ReleaseDC(None, hdc_screen);
    }
}
