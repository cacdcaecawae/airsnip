// SPDX-License-Identifier: GPL-3.0-only
//! 图片编码与保存（OUT-2/OUT-5）：PNG / JPEG / WebP，纯 Rust 编码；默认目录与文件命名模板。
//! 剪贴板属于平台层。编码调用方负责放到后台线程执行（OUT-2）。

use std::path::{Path, PathBuf};

use tiny_skia::Pixmap;

use crate::settings::NameStyle;

/// 默认保存目录：Windows `%USERPROFILE%\Pictures\airsnip`，macOS `~/Pictures/airsnip`。
pub fn default_save_dir() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var("HOME");
    PathBuf::from(home.unwrap_or_else(|_| ".".to_string()))
        .join("Pictures")
        .join("airsnip")
}

/// 生成时间戳文件名，如 `airsnip-20240101-120000.png`。
pub fn timestamped_filename(ext: &str) -> String {
    format!("{}.{ext}", base_name(NameStyle::DateTime))
}

/// 按命名模板生成基础文件名（不含扩展名，OUT-2）：
/// 保存对话框用它预填文件名，扩展名由对话框按所选格式补上。
pub fn base_name(style: NameStyle) -> String {
    match style {
        // Unix 毫秒：同一秒内连续保存两次也不重名
        NameStyle::Timestamp => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis().to_string())
            .unwrap_or_default(),
        NameStyle::DateTime | NameStyle::Date => {
            let (y, mo, d, h, mi, s) = local_time();
            if let NameStyle::Date = style {
                format!("airsnip-{y:04}{mo:02}{d:02}")
            } else {
                format!("airsnip-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
            }
        }
    }
}

/// 本地时间（年, 月, 日, 时, 分, 秒）。
#[cfg(windows)]
fn local_time() -> (u32, u32, u32, u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let st = unsafe { GetLocalTime() };
    let f = |v: u16| v as u32;
    (
        f(st.wYear),
        f(st.wMonth),
        f(st.wDay),
        f(st.wHour),
        f(st.wMinute),
        f(st.wSecond),
    )
}

/// 本地时间（年, 月, 日, 时, 分, 秒）：libc `localtime_r`（标准库不提供时区换算）。
#[cfg(unix)]
fn local_time() -> (u32, u32, u32, u32, u32, u32) {
    /// `struct tm`（macOS / glibc 布局一致）。
    #[repr(C)]
    struct Tm {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        wday: i32,
        yday: i32,
        isdst: i32,
        gmtoff: i64,
        zone: *const std::ffi::c_char,
    }
    unsafe extern "C" {
        fn time(t: *mut i64) -> i64;
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    let now = unsafe { time(std::ptr::null_mut()) };
    if unsafe { localtime_r(&now, &mut tm) }.is_null() {
        return (1970, 1, 1, 0, 0, 0);
    }
    let f = |v: i32| v.max(0) as u32;
    (
        f(tm.year + 1900),
        f(tm.mon + 1),
        f(tm.mday),
        f(tm.hour),
        f(tm.min),
        f(tm.sec),
    )
}

/// 保存像素为 PNG/JPEG/WebP（按扩展名决定格式，OUT-2/OUT-5）。
/// 扩展名缺失或不受支持时报错：对话框允许手输文件名，静默按 PNG 写出会得到
/// 内容与扩展名不符的文件。
pub fn save_file(pixmap: &Pixmap, path: &Path) -> Result<(), String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => save_png(pixmap, path),
        Some("jpg") | Some("jpeg") => save_jpeg(pixmap, path),
        Some("webp") => save_webp(pixmap, path),
        other => Err(format!(
            "不支持的图片格式：{}（请用 .png / .jpg / .webp）",
            other.unwrap_or("（无扩展名）")
        )),
    }
}

/// 保存 PNG（快速压缩，OUT-2）。
pub fn save_png(pixmap: &Pixmap, path: &Path) -> Result<(), String> {
    let bytes = encode_png(pixmap)?;
    std::fs::write(path, bytes).map_err(|e| format!("写入 {path:?} 失败：{e}"))
}

/// 保存 JPEG（质量 90，OUT-2）。
pub fn save_jpeg(pixmap: &Pixmap, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("创建 {path:?} 失败：{e}"))?;
    let enc = jpeg_encoder::Encoder::new(file, 90);
    enc.encode(
        pixmap.data(),
        pixmap.width() as u16,
        pixmap.height() as u16,
        jpeg_encoder::ColorType::Rgba,
    )
    .map_err(|e| format!("JPEG 编码失败：{e}"))
}

/// 保存 WebP（无损 VP8L，OUT-5）：纯 Rust 编码（image-webp），无 C 依赖。
pub fn save_webp(pixmap: &Pixmap, path: &Path) -> Result<(), String> {
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out)
        .encode(
            pixmap.data(),
            pixmap.width(),
            pixmap.height(),
            image_webp::ColorType::Rgba8,
        )
        .map_err(|e| format!("WebP 编码失败：{e}"))?;
    std::fs::write(path, out).map_err(|e| format!("写入 {path:?} 失败：{e}"))
}

/// RGBA → BGRA（就地语义复制出新缓冲）。
pub fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    bgra
}

/// 编码 PNG（快速压缩）。
pub fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    encode_png_scaled(pixmap, 1.0)
}

/// 编码 PNG，并按显示器缩放写入分辨率（pHYs，72 × scale DPI；scale 为 1 时不写）：
/// macOS 上按 points 排版的应用（Pages、Keynote）据此按屏幕上看到的大小显示 Retina 截图。
pub fn encode_png_scaled(pixmap: &Pixmap, scale: f32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, pixmap.width(), pixmap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        if scale != 1.0 {
            let ppm = (72.0 * scale / 0.0254).round() as u32;
            enc.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppm,
                yppu: ppm,
                unit: png::Unit::Meter,
            }));
        }
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(pixmap.data()).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WebP 无损编码（OUT-5）：输出应是合法的 RIFF/WEBP 容器。
    #[test]
    fn webp_encode_produces_riff_container() {
        let mut pixmap = Pixmap::new(16, 16).unwrap();
        pixmap.fill(tiny_skia::Color::from_rgba8(200, 30, 30, 255));
        let mut out = Vec::new();
        image_webp::WebPEncoder::new(&mut out)
            .encode(pixmap.data(), 16, 16, image_webp::ColorType::Rgba8)
            .expect("WebP 编码应成功");
        assert!(out.len() > 20, "WebP 输出不应为空：{}", out.len());
        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WEBP");
    }

    /// 命名模板（OUT-2）：三种模板格式正确，时间戳为 13 位 Unix 毫秒，日期为合理的本地日期。
    #[test]
    fn base_name_matches_templates() {
        let dt = base_name(NameStyle::DateTime);
        assert!(dt.starts_with("airsnip-"), "{dt}");
        assert_eq!(dt.len(), "airsnip-20241231-235959".len(), "{dt}");
        let year: u32 = dt[8..12].parse().unwrap();
        assert!((2024..2200).contains(&year), "年份应为当前本地年份：{dt}");
        // Unix 毫秒：纯数字、13 位（当前纪元）、且单调递增
        let ts = base_name(NameStyle::Timestamp);
        assert!(ts.chars().all(|c| c.is_ascii_digit()), "时间戳应为纯数字：{ts}");
        assert_eq!(ts.len(), 13, "时间戳应为 13 位毫秒：{ts}");
        assert!(base_name(NameStyle::Timestamp) >= ts, "时间戳不应倒退");
        let d = base_name(NameStyle::Date);
        assert!(d.starts_with("airsnip-"), "{d}");
        assert_eq!(d.len(), "airsnip-20241231".len(), "{d}");
        assert_eq!(timestamped_filename("png").len(), "airsnip-20241231-235959.png".len());
    }

    /// save_file 只接受三种扩展名：其余报错，避免内容与扩展名不符（OUT-2）。
    #[test]
    fn save_file_dispatches_by_extension() {
        let pixmap = Pixmap::new(4, 4).unwrap();
        let dir = std::env::temp_dir();
        for bad in ["t.bmp", "t", "t.gif"] {
            assert!(save_file(&pixmap, &dir.join(bad)).is_err(), "{bad} 应被拒绝");
        }
        for name in ["out.png", "out.jpg", "out.webp"] {
            let path = dir.join(format!("airsnip-test-{name}"));
            let r = save_file(&pixmap, &path);
            assert!(r.is_ok(), "{name} 应写入成功：{r:?}");
            let _ = std::fs::remove_file(&path);
        }
    }

    /// Retina 复制（OUT-1）：2x 写入 144 DPI，1x 不写分辨率。
    #[test]
    fn png_scaled_writes_dpi() {
        let px = Pixmap::new(4, 4).unwrap();
        let dims = |bytes: Vec<u8>| {
            let r = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            r.info().pixel_dims
        };
        let d = dims(encode_png_scaled(&px, 2.0).unwrap()).expect("2x 应写 pHYs");
        assert_eq!((d.xppu, d.yppu, d.unit), (5669, 5669, png::Unit::Meter));
        assert!(dims(encode_png(&px).unwrap()).is_none(), "1x 不写分辨率");
    }
}
