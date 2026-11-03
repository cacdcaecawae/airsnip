// SPDX-License-Identifier: GPL-3.0-only
//! macOS 宿主：把核心覆盖层要的平台能力（`airsnip_core::overlay::Host`）转发给 Swift 回调表 `AsHost`。
//!
//! 带 `ctx` 的回调在主线程、在 `airsnip_overlay_*` 调用内同步触发；
//! `clipboard_png` / `notify` 不带 `ctx`，可能在后台线程调用（编码、写文件在后台完成），Swift 侧自行回主线程。

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};

use airsnip_core::overlay::geometry::SelRect;
use airsnip_core::overlay::ui::Ui;
use airsnip_core::overlay::{Host, Shape};
use tiny_skia::Pixmap;

use crate::AsImage;

/// 保存对话框返回路径的缓冲长度（macOS PATH_MAX = 1024，留足 UTF-8 余量）。
const PATH_BUF: usize = 4096;

/// Swift 提供的回调表（见 `include/airsnip.h`）。空指针的回调视为不支持，调用时跳过。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AsHost {
    pub ctx: *mut c_void,
    pub present: Option<unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32)>,
    pub set_cursor: Option<unsafe extern "C" fn(*mut c_void, i32)>,
    pub close: Option<unsafe extern "C" fn(*mut c_void)>,
    pub alert: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    pub pin: Option<unsafe extern "C" fn(*mut c_void, *const AsImage, i32, i32) -> i32>,
    pub pick_save_file: Option<
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char, *mut c_char, usize) -> i32,
    >,
    pub set_ime: Option<unsafe extern "C" fn(*mut c_void, i32)>,
    pub clipboard_png: Option<unsafe extern "C" fn(*const u8, usize, u32, u32)>,
    pub notify: Option<unsafe extern "C" fn(*const c_char)>,
}

/// 字符串转 C 字符串（内含 NUL 时替换为空格，不丢整条消息）。
fn c_string(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

/// 后台线程可用的提示（`notify` 回调）。
fn notify(cb: Option<unsafe extern "C" fn(*const c_char)>, msg: &str) {
    if let Some(f) = cb {
        unsafe { f(c_string(msg).as_ptr()) }
    }
}

/// 借用 `Pixmap` 像素为 `AsImage`（仅在本次调用期间有效）。
pub(crate) fn as_image(p: &Pixmap) -> AsImage {
    AsImage {
        data: p.data().as_ptr(),
        width: p.width(),
        height: p.height(),
        stride: p.width() * 4,
    }
}

/// 第二项为显示器 backingScaleFactor：复制时写入图片分辨率（OUT-1）。
pub(crate) struct MacHost(pub AsHost, pub f32);

impl Host for MacHost {
    fn present(&mut self, _display: &Pixmap, r: SelRect) {
        if let Some(f) = self.0.present {
            unsafe { f(self.0.ctx, r.x, r.y, r.w, r.h) }
        }
    }

    fn set_cursor(&mut self, shape: Shape, _ui: Ui) {
        if let Some(f) = self.0.set_cursor {
            unsafe { f(self.0.ctx, shape as i32) }
        }
    }

    fn close(&mut self) {
        if let Some(f) = self.0.close {
            unsafe { f(self.0.ctx) }
        }
    }

    fn alert(&mut self, msg: &str) {
        if let Some(f) = self.0.alert {
            unsafe { f(self.0.ctx, c_string(msg).as_ptr()) }
        }
    }

    /// PNG 编码在后台完成（大选区可达上百毫秒），结果交给 Swift 写剪贴板（PNG 带 Retina 分辨率）。
    fn copy_image(&mut self, img: Pixmap) {
        let (done, fail, scale) = (self.0.clipboard_png, self.0.notify, self.1);
        std::thread::spawn(move || match airsnip_core::output::encode_png_scaled(&img, scale) {
            Ok(png) => {
                if let Some(f) = done {
                    unsafe { f(png.as_ptr(), png.len(), img.width(), img.height()) }
                }
            }
            Err(e) => notify(fail, &format!("复制失败：{e}")),
        });
    }

    fn pin(&mut self, img: Pixmap, x: i32, y: i32) -> bool {
        match self.0.pin {
            Some(f) => unsafe { f(self.0.ctx, &as_image(&img), x, y) != 0 },
            None => false,
        }
    }

    fn pick_save_file(&mut self, dir: Option<&Path>, name: &str, ext: &str) -> Option<PathBuf> {
        let f = self.0.pick_save_file?;
        let dir = dir.map(|d| c_string(&d.to_string_lossy()));
        let (name, ext) = (c_string(name), c_string(ext));
        let mut out = vec![0 as c_char; PATH_BUF];
        let dir_ptr = dir.as_ref().map_or(std::ptr::null(), |d| d.as_ptr());
        let chosen = unsafe {
            f(
                self.0.ctx,
                dir_ptr,
                name.as_ptr(),
                ext.as_ptr(),
                out.as_mut_ptr(),
                out.len(),
            )
        };
        if chosen == 0 {
            return None;
        }
        // Swift 保证 NUL 结尾；末字节强制置 0 兜底，防止越界读
        out[PATH_BUF - 1] = 0;
        let path = unsafe { CStr::from_ptr(out.as_ptr()) }.to_str().ok()?;
        (!path.is_empty()).then(|| PathBuf::from(path))
    }

    fn save_image(&mut self, img: Pixmap, path: PathBuf) {
        let fail = self.0.notify;
        std::thread::spawn(move || {
            if let Err(e) = airsnip_core::output::save_file(&img, &path) {
                // 保存失败必须提示：静默吞掉会让用户以为截图已保存（OUT-2）
                notify(fail, &format!("保存失败：{e}"));
            }
        });
    }

    fn set_ime(&mut self, enabled: bool) {
        if let Some(f) = self.0.set_ime {
            unsafe { f(self.0.ctx, enabled as i32) }
        }
    }
}
