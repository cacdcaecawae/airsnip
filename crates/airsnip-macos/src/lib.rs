// SPDX-License-Identifier: GPL-3.0-only
//! macOS 静态库：AirSnip 共享核心的 C ABI，声明见 `include/airsnip.h`。
//!
//! 约定（docs/macos-port.md §3.2）：
//! - 返回 `int32_t` 状态码：0 成功，`AIRSNIP_ERR_FAILED` 失败，`AIRSNIP_ERR_PANIC` 内部 panic；
//!   详情由 `airsnip_last_error()` 取（线程局部，下次失败前有效）。
//! - panic 一律在边界内 `catch_unwind` 转成错误码，不跨 FFI 传播（构建用 `release-macos` 配置，panic=unwind）。
//! - 图像为 RGBA8 预乘（tiny-skia 原生格式），Swift 传入的缓冲由 Rust 复制，调用返回后即可释放。
//! - 字符串入参为 NUL 结尾 UTF-8；出参为静态或线程局部存储，调用方不释放。

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};

use tiny_skia::{IntSize, Pixmap};

mod host;
mod keys;
mod longshot;
mod overlay;
mod pin;
mod settings;
mod text;

pub const AIRSNIP_OK: i32 = 0;
pub const AIRSNIP_ERR_FAILED: i32 = -1;
pub const AIRSNIP_ERR_PANIC: i32 = -2;

/// 图像缓冲（借用）：`data` 至少 `stride * height` 字节，每行前 `width * 4` 字节有效。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AsImage {
    pub data: *const u8,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: &str) {
    let c = CString::new(msg.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = c);
}

/// FFI 边界守卫：错误转状态码并记录详情，panic 不外泄。
pub(crate) fn guard(f: impl FnOnce() -> Result<(), String>) -> i32 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => AIRSNIP_OK,
        Ok(Err(e)) => {
            set_error(&e);
            AIRSNIP_ERR_FAILED
        }
        Err(_) => {
            set_error("内部错误（Rust panic）");
            AIRSNIP_ERR_PANIC
        }
    }
}

/// 核心版本号（静态字符串）。
#[unsafe(no_mangle)]
pub extern "C" fn airsnip_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// 本线程最近一次失败的说明（UTF-8；无错误时为空串）。指针在本线程下次失败前有效。
#[unsafe(no_mangle)]
pub extern "C" fn airsnip_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// 按扩展名（png / jpg / jpeg / webp）编码并写入文件（OUT-2/OUT-5）。
///
/// # Safety
/// `img` 指向有效的 `AsImage`，其 `data` 至少 `stride * height` 字节；`path` 为 NUL 结尾字符串。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_save_image(img: *const AsImage, path: *const c_char) -> i32 {
    guard(|| {
        let pixmap = unsafe { pixmap_from(img) }?;
        let path = unsafe { utf8_arg(path) }?;
        airsnip_core::output::save_file(&pixmap, std::path::Path::new(path))
    })
}

/// 复制借用的图像缓冲为 `Pixmap`（按 stride 逐行拷贝）。
pub(crate) unsafe fn pixmap_from(img: *const AsImage) -> Result<Pixmap, String> {
    let img = unsafe { img.as_ref() }.ok_or("图像参数为空")?;
    if img.data.is_null() || img.stride < img.width.saturating_mul(4) {
        return Err("图像缓冲非法".into());
    }
    let size = IntSize::from_wh(img.width, img.height).ok_or("图像尺寸非法")?;
    let row = img.width as usize * 4;
    let mut buf = Vec::with_capacity(row * img.height as usize);
    for y in 0..img.height as usize {
        let src = unsafe { std::slice::from_raw_parts(img.data.add(y * img.stride as usize), row) };
        buf.extend_from_slice(src);
    }
    Pixmap::from_vec(buf, size).ok_or_else(|| "图像缓冲非法".into())
}

unsafe fn utf8_arg<'a>(p: *const c_char) -> Result<&'a str, String> {
    if p.is_null() {
        return Err("字符串参数为空".into());
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_err(|_| "字符串不是合法 UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_error() -> String {
        unsafe { CStr::from_ptr(airsnip_last_error()) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn version_matches_package() {
        let v = unsafe { CStr::from_ptr(airsnip_version()) }.to_str().unwrap();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }

    /// 带行填充（stride > width*4）的缓冲按行拷贝，写出的 PNG 尺寸正确。
    #[test]
    fn save_image_honours_stride_and_writes_png() {
        let (w, h, stride) = (3u32, 2u32, 16u32);
        let mut data = vec![0u8; (stride * h) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                data[y * stride as usize + x * 4..][..4].copy_from_slice(&[10, 20, 30, 255]);
            }
        }
        let img = AsImage {
            data: data.as_ptr(),
            width: w,
            height: h,
            stride,
        };
        let path = std::env::temp_dir().join("airsnip-ffi-test.png");
        let c = CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { airsnip_save_image(&img, c.as_ptr()) },
            AIRSNIP_OK,
            "{}",
            last_error()
        );
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&bytes[1..4], b"PNG");
        // IHDR：宽高为大端 u32，位于偏移 16 / 20
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), w);
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), h);
    }

    #[test]
    fn invalid_arguments_fail_with_message() {
        let c = CString::new("x.png").unwrap();
        assert_eq!(
            unsafe { airsnip_save_image(std::ptr::null(), c.as_ptr()) },
            AIRSNIP_ERR_FAILED
        );
        assert!(last_error().contains("为空"), "{}", last_error());
        let data = [0u8; 4];
        let img = AsImage {
            data: data.as_ptr(),
            width: 1,
            height: 1,
            stride: 4,
        };
        let bad = CString::new("x.bmp").unwrap();
        assert_eq!(unsafe { airsnip_save_image(&img, bad.as_ptr()) }, AIRSNIP_ERR_FAILED);
        assert!(last_error().contains("不支持"), "{}", last_error());
    }

    #[test]
    fn panic_becomes_error_code() {
        assert_eq!(guard(|| panic!("boom")), AIRSNIP_ERR_PANIC);
        assert!(last_error().contains("panic"));
    }
}
