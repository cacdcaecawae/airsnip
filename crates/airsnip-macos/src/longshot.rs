// SPDX-License-Identifier: GPL-3.0-only
//! 长截图 C ABI（LNG-1/3/6/7）：拼接在核心 `Session`，帧由 Swift 抓取后推入（ScreenCaptureKit 是异步的，
//! 核心的帧源是同步拉取，这里用「待取帧」槽位衔接）。所有函数只在主线程调用。

use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::rc::Rc;

use airsnip_core::longshot::session::{Session, Source};
use airsnip_core::output;
use airsnip_core::settings::Config;
use tiny_skia::Pixmap;

use crate::host::as_image;
use crate::{AsImage, guard, pixmap_from};

/// 不透明句柄：`airsnip_longshot_new` 创建、`airsnip_longshot_free` 释放。
pub struct AsLongshot {
    session: Session,
    /// 视口像素尺寸（推入的帧必须一致）。
    size: (u32, u32),
    frame: Rc<RefCell<Option<Vec<u8>>>>,
    /// 借出的缩略图 / 提示 / PNG（下次同类调用或释放前有效）。
    thumb: Option<Pixmap>,
    hint: CString,
    png: Vec<u8>,
}

/// 帧源：取走 Swift 推入的那一帧。
struct Pushed(Rc<RefCell<Option<Vec<u8>>>>);

impl Source for Pushed {
    fn capture(&mut self) -> Result<Vec<u8>, String> {
        self.0.borrow_mut().take().ok_or_else(|| "没有待拼接的帧".into())
    }
}

unsafe fn ls<'a>(p: *mut AsLongshot) -> Option<&'a mut AsLongshot> {
    unsafe { p.as_mut() }
}

/// 开始长截图：视口为 `w × h` 像素，高度上限读自配置。失败返回 NULL（详情见 `airsnip_last_error`）。
#[unsafe(no_mangle)]
pub extern "C" fn airsnip_longshot_new(w: u32, h: u32) -> *mut AsLongshot {
    let mut out = std::ptr::null_mut();
    guard(|| {
        let frame = Rc::new(RefCell::new(None));
        let max = Config::load().longshot_max_height;
        let session = Session::with_source(Box::new(Pushed(frame.clone())), w, h, max)?;
        out = Box::into_raw(Box::new(AsLongshot {
            session,
            size: (w, h),
            frame,
            thumb: None,
            hint: CString::default(),
            png: Vec::new(),
        }));
        Ok(())
    });
    out
}

/// # Safety
/// `p` 为 `airsnip_longshot_new` 返回的句柄或 NULL，释放后不得再用。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_free(p: *mut AsLongshot) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

/// 推入一帧并拼接（RGBA8 预乘，尺寸须等于视口）。返回 0 继续、1 已达高度上限（应结束）、
/// 负数失败（抓帧尺寸不符等，详情见 `airsnip_last_error`）。
///
/// # Safety
/// `p` 为有效句柄或 NULL；`img` 指向有效图像。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_push(p: *mut AsLongshot, img: *const AsImage) -> i32 {
    let Some(l) = (unsafe { ls(p) }) else { return -1 };
    let mut finished = false;
    let code = guard(|| {
        let px = unsafe { pixmap_from(img) }?;
        if (px.width(), px.height()) != l.size {
            return Err("抓帧尺寸与视口不符".into());
        }
        *l.frame.borrow_mut() = Some(px.take());
        l.session.tick();
        if l.session.cancelled {
            return Err(l.session.status.clone());
        }
        finished = l.session.finished;
        Ok(())
    });
    if code != 0 { code } else { finished as i32 }
}

/// 状态条提示（空串 = 无异常，显示默认操作提示）。指针在下次调用前有效。
///
/// # Safety
/// `p` 为有效句柄或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_hint(p: *mut AsLongshot) -> *const c_char {
    let Some(l) = (unsafe { ls(p) }) else {
        return c"".as_ptr();
    };
    l.hint = CString::new(l.session.hint()).unwrap_or_default();
    l.hint.as_ptr()
}

/// 是否有可撤销的段。
///
/// # Safety
/// `p` 为有效句柄或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_can_undo(p: *mut AsLongshot) -> i32 {
    unsafe { ls(p) }.is_some_and(|l| l.session.can_undo()) as i32
}

/// 撤销最后拼上的一段（LNG-7）。
///
/// # Safety
/// `p` 为有效句柄或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_undo(p: *mut AsLongshot) {
    if let Some(l) = unsafe { ls(p) } {
        l.session.undo();
    }
}

/// 拼接结果缩略图（LNG-3，不超过 `max_w × max_h`）；还没有内容时宽高为 0。缓冲在下次调用前有效。
///
/// # Safety
/// `p` 为有效句柄或 NULL。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_thumbnail(p: *mut AsLongshot, max_w: u32, max_h: u32) -> AsImage {
    let empty = AsImage {
        data: std::ptr::null(),
        width: 0,
        height: 0,
        stride: 0,
    };
    let Some(l) = (unsafe { ls(p) }) else { return empty };
    l.thumb = l.session.thumbnail(max_w, max_h);
    l.thumb.as_ref().map_or(empty, as_image)
}

/// 完成（LNG-6）：拼接结果编码为 PNG（带 Retina 分辨率），保存到默认截图目录，
/// 并借出 PNG 字节供写剪贴板（`out_png` / `out_len`，释放句柄前有效）。
/// 编码成功但保存失败时仍借出 PNG，返回非 0（详情见 `airsnip_last_error`）。
///
/// # Safety
/// `p` 为有效句柄或 NULL；`out_png`、`out_len` 指向可写位置。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_longshot_finish(
    p: *mut AsLongshot,
    scale: f32,
    out_png: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let Some(l) = (unsafe { ls(p) }) else { return -1 };
    guard(|| {
        l.png = output::encode_png_scaled(&l.session.result()?, scale)?;
        unsafe {
            *out_png = l.png.as_ptr();
            *out_len = l.png.len();
        }
        let dir = output::default_save_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("保存失败：无法创建目录 {}：{e}", dir.display()))?;
        let path = dir.join(output::timestamped_filename("png"));
        std::fs::write(&path, &l.png).map_err(|e| format!("保存失败：{e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每行灰度各不相同的「页面」，从第 `top` 行起截 `h` 行作为一帧。
    fn frame(top: u32, w: u32, h: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in top..top + h {
            let v = (y.wrapping_mul(2_654_435_761) >> 24) as u8;
            for x in 0..w {
                let c = v.wrapping_add((x % 5) as u8);
                px.extend_from_slice(&[c, c, c, 255]);
            }
        }
        px
    }

    fn push(l: *mut AsLongshot, data: &[u8], w: u32, h: u32) -> i32 {
        let img = AsImage {
            data: data.as_ptr(),
            width: w,
            height: h,
            stride: w * 4,
        };
        unsafe { airsnip_longshot_push(l, &img) }
    }

    /// 首帧 + 向下滚动一帧 → 结果比视口高；撤销后回到首帧高度；尺寸不符的帧报错。
    #[test]
    fn push_scrolled_frames_then_undo() {
        let (w, h) = (64, 120);
        let l = airsnip_longshot_new(w, h);
        assert!(!l.is_null());
        assert_eq!(push(l, &frame(0, w, h), w, h), 0);
        assert_eq!(push(l, &frame(40, w, h), w, h), 0);
        let grown = unsafe { (*l).session.height() };
        assert!(grown > h, "滚动后应拼接出更高的结果：{grown}");
        assert_eq!(unsafe { airsnip_longshot_can_undo(l) }, 1);
        unsafe { airsnip_longshot_undo(l) };
        assert_eq!(unsafe { (*l).session.height() }, h);
        assert!(push(l, &frame(0, w, 10), w, 10) < 0, "尺寸不符的帧应报错");
        let thumb = unsafe { airsnip_longshot_thumbnail(l, 32, 32) };
        assert!(thumb.width > 0 && thumb.width <= 32 && thumb.height <= 32);
        unsafe { airsnip_longshot_free(l) };
    }
}
