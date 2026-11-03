// SPDX-License-Identifier: GPL-3.0-only
//! 系统主题检测（SYS-6）：读注册表决定跟随系统时的主题，最终主题由 `resolve` 决定。
//! 调色板在 `airsnip_core::theme`（此处再导出，路径不变）。

pub use airsnip_core::theme::*;

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE, PAGE_GUARD, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, VirtualQuery,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

use crate::settings::ThemeSetting;

/// 根据设置解析最终主题：跟随系统则读注册表。
pub fn resolve(setting: ThemeSetting) -> Theme {
    match setting {
        ThemeSetting::Follow => system_theme(),
        ThemeSetting::Light => Theme::Light,
        ThemeSetting::Dark => Theme::Dark,
    }
}

/// 读取 `AppsUseLightTheme`（1 = 浅色，0 = 深色）；读取失败时按深色处理。
fn system_light_theme() -> Option<i32> {
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = w!("AppsUseLightTheme");
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        let err = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
        if err == ERROR_SUCCESS && size == 4 {
            Some(value as i32)
        } else {
            None
        }
    }
}

/// 当前系统主题。
pub fn system_theme() -> Theme {
    match system_light_theme() {
        Some(1) => Theme::Light,
        _ => Theme::Dark,
    }
}

/// 判断 `WM_SETTINGCHANGE` 的 lParam 是否表示主题变更（"ImmersiveColorSet"）。
/// lParam 为 0 表示「非特定设置变更」，系统与第三方程序广播时常见；此时不能当
/// 宽字符串读：`PCWSTR::to_string` 内部走 `wcslen`，对 NULL 会访问冲突（0xC0000005）
/// 直接杀掉常驻进程 —— 登录阶段收到这种广播会让托盘图标随进程一起消失。
/// 非 0 的 lParam 也可能指向不可读内存（个别程序广播垃圾值），故先校验可读性再限长读取。
pub fn is_theme_change(lparam: usize) -> bool {
    const NAME: &str = "ImmersiveColorSet";
    read_setting_name(lparam, NAME.len()).is_some_and(|name| name == NAME)
}

/// 安全读取 lParam 指向的设置名（最多 `max` 个 wchar）。
/// 返回 None：指针为空、内存不可读、或未在限长内遇到终止符（不是关心的设置名）。
/// 不用 `wcslen`：它会一直扫到 0，垃圾指针或紧贴区域末尾的字符串都会读越界。
fn read_setting_name(lparam: usize, max: usize) -> Option<String> {
    if lparam == 0 {
        return None;
    }
    let ptr = lparam as *const u16;
    let mut mbi = MEMORY_BASIC_INFORMATION::default();
    let queried = unsafe {
        VirtualQuery(
            Some(ptr as *const core::ffi::c_void),
            &mut mbi,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried == 0 || mbi.State != MEM_COMMIT || !is_readable(mbi.Protect) {
        return None;
    }
    // 只在本内存区域内读，避免跨页访问触发访问冲突
    let end = (mbi.BaseAddress as usize).checked_add(mbi.RegionSize)?;
    let limit = ((end - lparam) / 2).min(max + 1); // +1 容纳终止符
    let mut buf = Vec::with_capacity(limit);
    for i in 0..limit {
        let c = unsafe { std::ptr::read_volatile(ptr.add(i)) };
        if c == 0 {
            return String::from_utf16(&buf).ok();
        }
        buf.push(c);
    }
    None
}

/// 页面保护属性是否允许读（`PAGE_GUARD` 页访问即异常，同样视为不可读）。
fn is_readable(protect: PAGE_PROTECTION_FLAGS) -> bool {
    protect.0 != 0 && protect.0 & (PAGE_NOACCESS.0 | PAGE_EXECUTE.0 | PAGE_GUARD.0) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// lParam 为 0（无设置名）时必须安全返回 false，不能解引用空指针。
    #[test]
    fn null_lparam_is_not_theme_change() {
        assert!(!is_theme_change(0));
    }

    /// 非 0 但不可读的垃圾指针不能崩溃（个别程序会广播 lParam = 1 之类的值）。
    #[test]
    fn garbage_lparam_is_not_theme_change() {
        assert!(!is_theme_change(1));
        assert!(!is_theme_change(usize::MAX - 1));
    }

    /// 合法字符串仍能识别主题变更。
    #[test]
    fn immersive_color_set_is_theme_change() {
        let name: Vec<u16> = "ImmersiveColorSet\0".encode_utf16().collect();
        assert!(is_theme_change(name.as_ptr() as usize));
    }

    /// 其它设置名不算主题变更。
    #[test]
    fn other_setting_name_is_not_theme_change() {
        let name: Vec<u16> = "Environment\0".encode_utf16().collect();
        assert!(!is_theme_change(name.as_ptr() as usize));
    }

    /// 限长内没有终止符时安全返回 false（不会像 `wcslen` 那样继续往后扫）。
    #[test]
    fn unterminated_name_is_not_theme_change() {
        let name: Vec<u16> = "ImmersiveColorSet!".encode_utf16().collect(); // 无 NUL 且比目标长
        assert!(!is_theme_change(name.as_ptr() as usize));
    }
}
