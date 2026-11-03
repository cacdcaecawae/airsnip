// SPDX-License-Identifier: GPL-3.0-only
//! 设置 C ABI（SYS-3）：Swift 原生设置窗口读写配置文件。键码为 `kVK_*`，修饰为 Carbon 修饰位；
//! 枚举按序号传递（顺序即设置窗口里下拉菜单的顺序）。

use airsnip_core::settings::{Config, NameStyle, PinPosition, ThemeSetting, UiScale};

use crate::guard;

/// 设置窗口可改的配置项。
#[repr(C)]
#[derive(Default)]
pub struct AsSettings {
    pub auto_start: i32,
    /// 0 跟随系统、1 浅色、2 深色。
    pub theme: i32,
    /// 0 自动、1–5 = 100% / 125% / 150% / 175% / 200%。
    pub ui_scale: i32,
    /// 0 日期 + 时间、1 时间戳（毫秒）、2 仅日期。
    pub name_style: i32,
    /// 非 0：截图贴图放屏幕中央。
    pub pin_center: i32,
    pub shot_key: u32,
    pub shot_mods: u32,
    pub pin_key: u32,
    pub pin_mods: u32,
    pub border_key: u32,
    pub border_mods: u32,
    pub longshot_key: u32,
    pub tool_rect: u32,
    pub tool_arrow: u32,
    pub tool_text: u32,
    pub tool_blur: u32,
    pub tool_highlight: u32,
    pub tool_glow: u32,
}

const THEMES: [ThemeSetting; 3] = [ThemeSetting::Follow, ThemeSetting::Light, ThemeSetting::Dark];
const SCALES: [UiScale; 6] = [
    UiScale::Auto,
    UiScale::P100,
    UiScale::P125,
    UiScale::P150,
    UiScale::P175,
    UiScale::P200,
];
const NAMES: [NameStyle; 3] = [NameStyle::DateTime, NameStyle::Timestamp, NameStyle::Date];

fn index<T: PartialEq>(all: &[T], v: &T) -> i32 {
    all.iter().position(|x| x == v).unwrap_or(0) as i32
}

fn pick<T: Copy>(all: &[T], i: i32) -> T {
    all[(i.max(0) as usize).min(all.len() - 1)]
}

impl From<&Config> for AsSettings {
    fn from(c: &Config) -> Self {
        let (h, t) = (&c.hotkey, &c.tool_keys);
        Self {
            auto_start: c.auto_start as i32,
            theme: index(&THEMES, &c.theme),
            ui_scale: index(&SCALES, &c.ui_scale),
            name_style: index(&NAMES, &c.name_style),
            pin_center: (c.pin_position == PinPosition::Center) as i32,
            shot_key: h.screenshot_key,
            shot_mods: h.screenshot_modifiers,
            pin_key: h.pin_key,
            pin_mods: h.pin_modifiers,
            border_key: h.border_key,
            border_mods: h.border_modifiers,
            longshot_key: h.longshot_key,
            tool_rect: t.rect,
            tool_arrow: t.arrow,
            tool_text: t.text,
            tool_blur: t.blur,
            tool_highlight: t.highlight,
            tool_glow: t.glow,
        }
    }
}

impl AsSettings {
    /// 写回配置（其余字段保持原值）。
    fn apply(&self, c: &mut Config) {
        c.auto_start = self.auto_start != 0;
        c.theme = pick(&THEMES, self.theme);
        c.ui_scale = pick(&SCALES, self.ui_scale);
        c.name_style = pick(&NAMES, self.name_style);
        c.pin_position = if self.pin_center != 0 {
            PinPosition::Center
        } else {
            PinPosition::Original
        };
        let h = &mut c.hotkey;
        (h.screenshot_key, h.screenshot_modifiers) = (self.shot_key, self.shot_mods);
        (h.pin_key, h.pin_modifiers) = (self.pin_key, self.pin_mods);
        (h.border_key, h.border_modifiers) = (self.border_key, self.border_mods);
        h.longshot_key = self.longshot_key;
        let t = &mut c.tool_keys;
        (t.rect, t.arrow, t.text) = (self.tool_rect, self.tool_arrow, self.tool_text);
        (t.blur, t.highlight, t.glow) = (self.tool_blur, self.tool_highlight, self.tool_glow);
    }
}

/// 读设置。
///
/// # Safety
/// `out` 为 NULL 或指向可写的 `AsSettings`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_settings_load(out: *mut AsSettings) {
    if let Some(out) = unsafe { out.as_mut() } {
        *out = AsSettings::from(&Config::load());
    }
}

/// 保存设置（立即写入配置文件；下次截图、贴图即生效，全局热键由调用方重新注册）。
///
/// # Safety
/// `s` 指向有效的 `AsSettings`。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn airsnip_settings_save(s: *const AsSettings) -> i32 {
    guard(|| {
        let s = unsafe { s.as_ref() }.ok_or("设置参数为空")?;
        let mut cfg = Config::load();
        s.apply(&mut cfg);
        cfg.save();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 读出 → 写回不改变配置；序号越界时夹到合法范围。
    #[test]
    fn roundtrip_and_clamp() {
        let cfg = Config::default();
        let mut s = AsSettings::from(&cfg);
        let mut back = cfg.clone();
        s.apply(&mut back);
        assert_eq!(format!("{back:?}"), format!("{cfg:?}"));

        (s.theme, s.ui_scale, s.name_style, s.pin_center) = (9, -1, 2, 1);
        s.apply(&mut back);
        assert_eq!(back.theme, ThemeSetting::Dark);
        assert_eq!(back.ui_scale, UiScale::Auto);
        assert_eq!(back.name_style, NameStyle::Date);
        assert_eq!(back.pin_position, PinPosition::Center);
    }
}
