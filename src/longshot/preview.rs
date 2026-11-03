// SPDX-License-Identifier: GPL-3.0-only
//! 长截图预览窗（LNG-3）：在视口左侧或右侧的屏幕剩余空间里显示拼接结果的
//! 等比缩略图 —— 拼接越长图越窄，用户能一眼看到拼到了哪里、有没有错位。
//!
//! 固定尺寸的分层窗口（`WS_EX_TRANSPARENT` 点击/滚轮穿透，不挡目标页面），
//! 窗口底边与视口底边对齐；背景完全透明，只绘制缩略图本身（不铺底板）。
//! 内部按目标像素抽样绘制缩略图（成本与拼接图高度无关，见 `Canvas::thumbnail`）。

use tiny_skia::{Pixmap, Transform};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject, HBITMAP, HDC};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::overlay::dib::create_dib_surface;
use crate::overlay::geometry::SelRect;
use crate::overlay::shape::{paint_of, rounded_path};
use crate::overlay::ui::Ui;
use crate::theme::Palette;

use super::present::{premul_bgra, present};
use super::session::Session;

/// 预览窗与屏幕边缘 / 视口之间的间距基准值。
const MARGIN_BASE: f32 = 10.0;
/// 预览窗最大宽度基准值。
const MAX_W_BASE: f32 = 240.0;
/// 预览窗最大高度基准值。
const MAX_H_BASE: f32 = 560.0;
/// 最小可用宽度基准值（低于此值不显示预览）。
const MIN_W_BASE: f32 = 120.0;
/// 预览窗最小高度基准值。
const MIN_H_BASE: f32 = 120.0;
/// 内容内边距基准值。
const PAD_BASE: f32 = 6.0;

/// 窗口类名。
const CLASS: windows::core::PCWSTR = w!("airsnip.longshot.preview");

/// 预览窗。
pub struct Preview {
    hwnd: HWND,
    w: i32,
    h: i32,
    /// 内容内边距（按 DPI 缩放）。
    pad: i32,
    mem_dc: HDC,
    dib_bmp: HBITMAP,
    dib_bits: *mut u8,
    palette: &'static Palette,
    /// 上次绘制时的拼接高度（未变化则跳过重绘）。
    last_h: u32,
}

impl Preview {
    /// 计算预览窗矩形（屏幕坐标）：优先视口右侧，其次左侧；剩余宽度不足返回 None。
    pub fn layout(vp: SelRect, screen: SelRect, ui: Ui) -> Option<SelRect> {
        let margin = ui.px(MARGIN_BASE);
        let (max_w, max_h, min_w, min_h) = (
            ui.px(MAX_W_BASE),
            ui.px(MAX_H_BASE),
            ui.px(MIN_W_BASE),
            ui.px(MIN_H_BASE),
        );
        let right = screen.x + screen.w - (vp.x + vp.w) - margin * 2;
        let left = vp.x - screen.x - margin * 2;
        // 宽度已定，再算左边位置：左侧时右缘贴视口左边缘
        let w = right.max(left).min(max_w);
        let (x, w) = if right >= left {
            (vp.x + vp.w + margin, w)
        } else {
            (vp.x - margin - w, w)
        };
        if w < min_w {
            return None;
        }
        let screen_h = (screen.h - margin * 2).max(min_h);
        let h = (vp.h as i32).clamp(min_h, max_h).min(screen_h);
        // 底边与视口底边对齐；超出屏幕时夹回屏幕内
        let y = (vp.y + vp.h - h)
            .min(screen.y + screen.h - margin - h)
            .max(screen.y + margin);
        Some(SelRect { x, y, w, h })
    }

    /// 按视口位置创建预览窗（放不下时返回 None，长截图照常进行）。
    pub fn spawn(vp: SelRect, hinstance: HINSTANCE, palette: &'static Palette, ui: Ui) -> Option<Self> {
        let rect = Self::layout(vp, virtual_screen(), ui)?;
        let (mem_dc, dib_bmp, dib_bits) = create_dib_surface(rect.w as u32, rect.h as u32).ok()?;
        unsafe {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(preview_wndproc),
                hInstance: hinstance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                // LAYERED：UpdateLayeredWindow 提交带圆角的像素；
                // TRANSPARENT：点击与滚轮穿透到目标页面，预览窗不挡操作。
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                CLASS,
                w!("长截图预览"),
                WS_POPUP,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                None,
                None,
                Some(hinstance),
                None,
            )
            .ok()?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), rect.x, rect.y, rect.w, rect.h, SWP_SHOWWINDOW);
            Some(Self {
                hwnd,
                w: rect.w,
                h: rect.h,
                pad: ui.px(PAD_BASE),
                mem_dc,
                dib_bmp,
                dib_bits,
                palette,
                // 初值取 MAX：首帧即使高度为 0 也要先提交一次，否则窗口不显示
                last_h: u32::MAX,
            })
        }
    }

    /// 重绘预览：圆角底板 + 居中的等比缩略图，随后提交到分层窗口。
    pub fn update(&mut self, session: &Session) {
        // 拼接高度未变（用户未滚动）时不重算，省掉抽样与上屏
        let h = session.height();
        if h == self.last_h {
            return;
        }
        self.last_h = h;
        // 不铺底板：Pixmap 新分配即为全透明，只画缩略图（LNG-3）
        let Some(mut pixmap) = Pixmap::new(self.w as u32, self.h as u32) else {
            return;
        };
        let inner_w = (self.w - self.pad * 2).max(1) as u32;
        let inner_h = (self.h - self.pad * 2).max(1) as u32;
        if let Some(thumb) = session.thumbnail(inner_w, inner_h) {
            let (tw, th) = (thumb.width() as i32, thumb.height() as i32);
            let dx = (self.w - tw) / 2;
            // 缩略图底部与窗口底部对齐：图的底端即拼接进度末端
            let dy = self.h - self.pad - th;
            pixmap.draw_pixmap(
                dx,
                dy,
                thumb.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            // 1px 边框：围在缩略图外侧，随缩略图尺寸同步变化（图越长越窄）
            let outline = SelRect {
                x: dx - 1,
                y: dy - 1,
                w: tw + 2,
                h: th + 2,
            };
            if let Some(path) = rounded_path(outline, 2.0) {
                let stroke = tiny_skia::Stroke {
                    width: 1.0,
                    ..Default::default()
                };
                pixmap.stroke_path(
                    &path,
                    &paint_of(self.palette.border),
                    &stroke,
                    Transform::identity(),
                    None,
                );
            }
        }
        // RGBA（premultiplied）→ BGRA 写入 DIB，再提交
        let bgra = premul_bgra(pixmap.data());
        unsafe {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), self.dib_bits, bgra.len());
            present(self.hwnd, self.mem_dc);
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        unsafe {
            if !self.dib_bmp.0.is_null() {
                let _ = DeleteObject(self.dib_bmp.into());
            }
            if !self.mem_dc.0.is_null() {
                let _ = DeleteDC(self.mem_dc);
            }
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 预览窗窗口过程：纯展示窗口，全部交给默认处理。
unsafe extern "system" fn preview_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// 虚拟屏幕矩形（物理像素，多屏时为并集）。
fn virtual_screen() -> SelRect {
    unsafe {
        SelRect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Ui {
        Ui::from_dpi(96)
    }

    fn screen() -> SelRect {
        SelRect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }
    }

    #[test]
    fn prefers_right_side_when_wider() {
        let vp = SelRect {
            x: 400,
            y: 100,
            w: 800,
            h: 600,
        };
        let r = Preview::layout(vp, screen(), base()).expect("右侧有空间");
        assert!(r.x >= vp.x + vp.w, "应放在视口右侧");
        assert_eq!(r.w, base().px(MAX_W_BASE));
        assert!(r.x + r.w <= screen().x + screen().w);
    }

    #[test]
    fn falls_back_to_left_side() {
        // 视口贴右边缘，右侧放不下，左侧空间充足
        let vp = SelRect {
            x: 1200,
            y: 100,
            w: 700,
            h: 600,
        };
        let r = Preview::layout(vp, screen(), base()).expect("左侧有空间");
        assert!(r.x + r.w <= vp.x, "应放在视口左侧");
        assert!(r.x >= screen().x);
    }

    #[test]
    fn hides_when_no_room() {
        // 视口几乎占满屏幕宽度，两侧都不足最小宽度
        let vp = SelRect {
            x: 30,
            y: 0,
            w: 1860,
            h: 900,
        };
        assert_eq!(Preview::layout(vp, screen(), base()), None);
    }

    #[test]
    fn stays_inside_screen_vertically() {
        let vp = SelRect {
            x: 400,
            y: 900,
            w: 800,
            h: 170,
        };
        let r = Preview::layout(vp, screen(), base()).expect("有空间");
        assert!(r.y >= 0 && r.y + r.h <= 1080, "预览窗不得超出屏幕：{r:?}");
    }

    #[test]
    fn aligns_to_viewport_bottom() {
        let vp = SelRect {
            x: 400,
            y: 100,
            w: 800,
            h: 600,
        };
        let r = Preview::layout(vp, screen(), base()).expect("有空间");
        assert_eq!(r.y + r.h, vp.y + vp.h, "预览窗底边应与视口底边对齐");
    }

    /// 高 DPI 下预览窗整体放大（CAP-5）。
    #[test]
    fn preview_scales_with_dpi() {
        let vp = SelRect {
            x: 400,
            y: 100,
            w: 800,
            h: 600,
        };
        let hidpi_screen = SelRect {
            x: 0,
            y: 0,
            w: 3840,
            h: 2160,
        };
        let ui = Ui::from_dpi(192);
        let r = Preview::layout(vp, hidpi_screen, ui).expect("有空间");
        assert_eq!(r.w, ui.px(MAX_W_BASE), "预览窗宽度应按 DPI 放大");
        assert_eq!(r.h, 600, "高度跟随视口，不缩放");
    }
}
