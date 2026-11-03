// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层的 Win32 宿主（`airsnip_core::overlay::Host`）：DIB 局部上屏、光标、关闭、提示框、
//! 剪贴板、另存为对话框、后台保存、贴图载荷与输入法开关。

use std::path::{Path, PathBuf};

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{GlobalFree, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{BitBlt, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, ReleaseDC, SRCCOPY};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext, ImmCreateContext};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, PostMessageW, SetCursor, WM_CLOSE};

use super::dib::{convert_rgba_to_bgra, create_dib_surface};
use super::geometry::SelRect;
use super::ui::Ui;
use super::{Host, Shape};
use crate::app::message_box;

pub(super) struct WinHost {
    hwnd: HWND,
    /// 主消息窗口（贴图载荷交给它创建贴图窗口，PIN-1）。
    main_hwnd: HWND,
    /// 禁用输入法前的 IME 上下文（文本输入时恢复，EDT-4）。
    ime_old: HIMC,
    /// 上屏用内存 DC（持有 DIB section，BGRA 像素直接写入 `dib_bits`）。
    mem_dc: HDC,
    dib_bmp: HBITMAP,
    /// DIB section 像素指针（BGRA，top-down）。
    dib_bits: *mut u8,
}

impl WinHost {
    /// 为覆盖窗口创建与虚拟屏同尺寸的 DIB 上屏面。
    pub(super) fn new(hwnd: HWND, main_hwnd: HWND, w: u32, h: u32) -> Result<Self, String> {
        let (mem_dc, dib_bmp, dib_bits) = create_dib_surface(w, h)?;
        Ok(Self {
            hwnd,
            main_hwnd,
            ime_old: HIMC(std::ptr::null_mut()),
            mem_dc,
            dib_bmp,
            dib_bits,
        })
    }
}

impl Drop for WinHost {
    /// 释放上屏用 GDI 资源（DIB section 位图 + 内存 DC）。
    fn drop(&mut self) {
        unsafe {
            if !self.dib_bmp.0.is_null() {
                let _ = DeleteObject(self.dib_bmp.into());
            }
            if !self.mem_dc.0.is_null() {
                let _ = DeleteDC(self.mem_dc);
            }
        }
    }
}

impl Host for WinHost {
    /// 局部 RGBA → BGRA 写入 DIB section，再用 BitBlt 刷到窗口。
    /// 用 BitBlt 而非 SetDIBitsToDevice：后者对 top-down DIB 的源行语义易出错。
    fn present(&mut self, display: &Pixmap, r: SelRect) {
        let w = display.width() as i32;
        let h = display.height() as i32;
        let x0 = r.x.clamp(0, w);
        let y0 = r.y.clamp(0, h);
        let x1 = (r.x + r.w).clamp(0, w);
        let y1 = (r.y + r.h).clamp(0, h);
        let (rw, rh) = (x1 - x0, y1 - y0);
        if rw <= 0 || rh <= 0 {
            return;
        }
        // 大面积多线程、小面积串行
        let workers = if rw as usize * rh as usize >= 1 << 18 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .clamp(1, 8)
        } else {
            1
        };
        convert_rgba_to_bgra(display.data(), self.dib_bits as usize, w, x0, y0, rw, rh, workers);
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            if hdc.0.is_null() {
                return;
            }
            let _ = BitBlt(hdc, x0, y0, rw, rh, Some(self.mem_dc), x0, y0, SRCCOPY);
            let _ = ReleaseDC(Some(self.hwnd), hdc);
        }
    }

    fn set_cursor(&mut self, shape: Shape, ui: Ui) {
        unsafe {
            let _ = SetCursor(Some(super::cursors::hcursor(shape, ui)));
        }
    }

    fn close(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }

    fn alert(&mut self, msg: &str) {
        message_box(None, msg, MB_OK | MB_ICONERROR);
    }

    fn copy_image(&mut self, img: Pixmap) {
        std::thread::spawn(move || {
            if let Err(e) = crate::output::copy_to_clipboard(&img) {
                message_box(None, &format!("复制失败：{e}"), MB_OK | MB_ICONERROR);
            }
        });
    }

    /// 跨线程交给主窗口贴图。载荷：GlobalAlloc[u32 w][u32 h][i32 x][i32 y][RGBA]，
    /// 主窗口创建贴图窗口后释放（app/mod.rs WM_PIN_FROM_EDITOR）。
    fn pin(&mut self, img: Pixmap, x: i32, y: i32) -> bool {
        let (w, h) = (img.width(), img.height());
        let px_total = w as usize * h as usize * 4;
        unsafe {
            let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, 16 + px_total) else {
                // 分配失败：提示并保留覆盖层（用户可重按 F3 重试，审查 P2-1）
                message_box(None, "贴图内存分配失败，请关闭部分贴图后重试。", MB_OK | MB_ICONERROR);
                return false;
            };
            let ptr = GlobalLock(hg);
            if ptr.is_null() {
                let _ = GlobalFree(Some(hg));
                message_box(None, "贴图内存锁失败。", MB_OK | MB_ICONERROR);
                return false;
            }
            let hp = ptr as *mut u32;
            *hp = w;
            *hp.add(1) = h;
            let sp = hp.add(2) as *mut i32;
            *sp = x;
            *sp.add(1) = y;
            std::ptr::copy_nonoverlapping(img.data().as_ptr(), (ptr as *mut u8).add(16), px_total);
            let _ = GlobalUnlock(hg);
            let _ = PostMessageW(
                Some(self.main_hwnd),
                crate::app::WM_PIN_FROM_EDITOR,
                WPARAM(hg.0 as usize),
                LPARAM(0),
            );
        }
        true
    }

    fn pick_save_file(&mut self, dir: Option<&Path>, name: &str, ext: &str) -> Option<PathBuf> {
        crate::app::filedlg::pick_save_file(self.hwnd, dir, name, ext)
    }

    fn save_image(&mut self, img: Pixmap, path: PathBuf) {
        std::thread::spawn(move || {
            if let Err(e) = crate::output::save_file(&img, &path) {
                // 保存失败必须提示：静默吞掉会让用户以为截图已保存（OUT-2）
                message_box(None, &format!("保存失败：{e}"), MB_OK | MB_ICONERROR);
            }
        });
    }

    /// 输入法开关：`true` 恢复关联的 IME 上下文（文本输入），`false` 断开（默认态）。
    /// 不禁用的话，中文输入法活动时按 `S`（长截图）/ 回车等键会被 IME 截走
    /// （转为 `VK_PROCESSKEY` 并启动拼音组合），覆盖层快捷键全部失效。
    fn set_ime(&mut self, on: bool) {
        unsafe {
            if on {
                // 窗口原本就没关联上下文时新建一个，否则文本工具无法输入中文
                let ctx = if self.ime_old.0.is_null() {
                    ImmCreateContext()
                } else {
                    self.ime_old
                };
                if !ctx.0.is_null() {
                    let _ = ImmAssociateContext(self.hwnd, ctx);
                }
            } else {
                let old = ImmAssociateContext(self.hwnd, HIMC(std::ptr::null_mut()));
                if !old.0.is_null() {
                    self.ime_old = old;
                }
            }
        }
    }
}
