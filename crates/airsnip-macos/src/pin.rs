// SPDX-License-Identifier: GPL-3.0-only
//! 贴图 C ABI（PIN-1~6）：贴图窗口在 Swift（NSPanel，缩放、透明度、拖动交给 AppKit），
//! 这里只提供配置读写与「保存到默认目录」的路径。

use std::ffi::c_char;

use airsnip_core::output;
use airsnip_core::settings::{Config, PinPosition};

use crate::guard;

/// 贴图配置（读自配置文件）。键码为 `kVK_*`，修饰为 Carbon 修饰位。
#[repr(C)]
#[derive(Default)]
pub struct AsPinConfig {
    /// 剪贴板贴图全局热键（PIN-2）。
    pub key: u32,
    pub mods: u32,
    /// 贴图窗口内的边框开关键（切换全部贴图）。
    pub border_key: u32,
    pub border_mods: u32,
    /// 非 0：截图贴图放在屏幕中央；0：放在选区原位置（PIN-1）。
    pub center: i32,
    /// 非 0：显示 1px 蓝色边框。
    pub border: i32,
}

/// 读贴图配置。
///
/// # Safety
/// `out` 为 NULL 或指向可写的 `AsPinConfig`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_config_pin(out: *mut AsPinConfig) {
    let Some(out) = (unsafe { out.as_mut() }) else { return };
    let cfg = Config::load();
    *out = AsPinConfig {
        key: cfg.hotkey.pin_key,
        mods: cfg.hotkey.pin_modifiers,
        border_key: cfg.hotkey.border_key,
        border_mods: cfg.hotkey.border_modifiers,
        center: (cfg.pin_position == PinPosition::Center) as i32,
        border: cfg.pin_border as i32,
    };
}

/// 记住边框开关（贴图窗口内切换后写回，之后的新贴图沿用）。
#[unsafe(no_mangle)]
pub extern "C" fn airsnip_config_set_pin_border(on: i32) {
    guard(|| {
        let mut cfg = Config::load();
        cfg.pin_border = on != 0;
        cfg.save();
        Ok(())
    });
}

/// 默认截图目录下的新文件路径（贴图右键「保存」，PNG、时间戳文件名；目录不存在时创建），
/// 写入 `out`（NUL 结尾）。失败返回非 0（详情见 `airsnip_last_error`）。
///
/// # Safety
/// `out` 为 NULL 或指向 `len` 字节的可写缓冲。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_default_save_path(out: *mut c_char, len: usize) -> i32 {
    guard(|| {
        let dir = output::default_save_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
        let path = dir.join(output::timestamped_filename("png"));
        let s = path.to_str().ok_or("保存路径不是合法 UTF-8")?;
        if out.is_null() || s.len() >= len {
            return Err("保存路径过长".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr().cast::<c_char>(), out, s.len());
            *out.add(s.len()) = 0;
        }
        Ok(())
    })
}
