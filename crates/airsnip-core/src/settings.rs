// SPDX-License-Identifier: GPL-3.0-only
//! 配置模型（SYS-5）：Windows `%APPDATA%\airsnip\config.toml`，
//! macOS `~/Library/Application Support/AirSnip/config.toml`；缺失或损坏时回退默认值。
//! 热键与工具键按平台原生键码存储（Windows 为虚拟键码 + `MOD_*` 位），两平台配置文件互不读取。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::editor::Tool;

/// 最近一次截图会话使用的工具（下次截图默认沿用，首次为矩形，M3）。
static LAST_TOOL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// 上次使用的标注工具。
pub fn last_tool() -> Tool {
    let i = LAST_TOOL.load(std::sync::atomic::Ordering::Relaxed) as usize;
    Tool::ALL[i.min(Tool::ALL.len() - 1)]
}

/// 记录本次会话最后使用的工具。
pub fn last_tool_store(t: Tool) {
    LAST_TOOL.store(t as u8, std::sync::atomic::Ordering::Relaxed);
}

/// 覆盖层保存时选定的格式扩展名（OUT-2）：覆盖层线程写，主线程在截图结束时写回配置。
static LAST_SAVE_EXT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 覆盖层保存时选定的目录（OUT-2）：覆盖层线程写，主线程在截图结束时写回配置。
static LAST_SAVE_DIR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 记录本次保存用的格式扩展名（覆盖层线程调用）。
pub fn last_save_ext_store(ext: &str) {
    if let Ok(mut g) = LAST_SAVE_EXT.lock() {
        *g = Some(ext.to_ascii_lowercase());
    }
}

/// 取出并清空记录的扩展名（主线程写回配置用）。
pub fn last_save_ext_take() -> Option<String> {
    LAST_SAVE_EXT.lock().ok().and_then(|mut g| g.take())
}

/// 记录本次保存所在的目录（覆盖层线程调用，OUT-2）。
pub fn last_save_dir_store(dir: &std::path::Path) {
    if let Ok(mut g) = LAST_SAVE_DIR.lock() {
        *g = Some(dir.to_string_lossy().into_owned());
    }
}

/// 取出并清空记录的目录（主线程写回配置用）。
pub fn last_save_dir_take() -> Option<String> {
    LAST_SAVE_DIR.lock().ok().and_then(|mut g| g.take())
}

/// 主题设置：跟随系统 / 浅色 / 深色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeSetting {
    #[default]
    #[serde(rename = "follow")]
    Follow,
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "dark")]
    Dark,
}

/// 未绑定语义：键码为 `UNBOUND` 表示「该项不绑定任何键」（SYS-2）。
/// Windows 为 0：旧配置里不存在 0 值，只在用户显式清空时出现，回退默认逻辑不受影响。
/// macOS 的 0 是 `kVK_ANSI_A`（默认截图热键 ⌃⌘A），改用不会出现的 `u32::MAX`。
#[cfg(not(target_os = "macos"))]
pub const UNBOUND: u32 = 0;
#[cfg(target_os = "macos")]
pub const UNBOUND: u32 = u32::MAX;

/// 热键配置：每键独立虚拟键码 + 修饰键位标志（0 = 无修饰键，SYS-2 需提示风险）。
/// 截图与贴图各自持有修饰键，互不影响（M3 修复历史共享缺陷）。
/// 键码 `UNBOUND` 表示该项被用户清空，注册层跳过、显示为「未设置」。
/// 键码按平台解释：Windows 为虚拟键码 + `MOD_*`；macOS 为 `kVK_*` + Carbon 修饰位
/// （`controlKey` 0x1000、`optionKey` 0x0800、`shiftKey` 0x0200、`cmdKey` 0x0100）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 截图热键虚拟键码（默认 F1 = 0x70）
    pub screenshot_key: u32,
    /// 截图热键修饰键位标志（MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN）
    pub screenshot_modifiers: u32,
    /// 贴图热键虚拟键码（默认 F3 = 0x72）
    pub pin_key: u32,
    /// 贴图热键修饰键位标志
    pub pin_modifiers: u32,
    /// 贴图边框开关热键（默认 Ctrl+F3；`UNBOUND` = 未启用）
    pub border_key: u32,
    /// 贴图边框开关热键修饰键位标志
    pub border_modifiers: u32,
    /// 长截图键虚拟键码（截图覆盖层内按下，默认 S = 0x53；无修饰键，与工具切换键同组；
    /// `UNBOUND` = 未启用）
    pub longshot_key: u32,
    /// 遗留字段（旧版本两键共享修饰键，TOML 键名仍为 `modifiers`）：仅加载迁移用，不再写入。
    #[serde(default, skip_serializing, rename = "modifiers")]
    pub legacy_modifiers: u32,
}

/// 长截图键默认值：S（Windows VK 0x53 / macOS kVK_ANSI_S 0x01）。
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_LONGSHOT_KEY: u32 = 0x53;
#[cfg(target_os = "macos")]
pub const DEFAULT_LONGSHOT_KEY: u32 = 0x01;

#[cfg(not(target_os = "macos"))]
impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            screenshot_key: 0x70,
            screenshot_modifiers: 0,
            pin_key: 0x72,
            pin_modifiers: 0,
            border_key: 0x72,    // Ctrl+F3
            border_modifiers: 2, // MOD_CONTROL
            longshot_key: DEFAULT_LONGSHOT_KEY,
            legacy_modifiers: 0,
        }
    }
}

/// macOS 默认：⌃⌘A 截图（用户习惯）、⌃⌥P 贴图、⌃F3 贴图边框（避开系统截图 ⌘⇧3/4/5）。
#[cfg(target_os = "macos")]
impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            screenshot_key: 0x00,         // kVK_ANSI_A
            screenshot_modifiers: 0x1100, // controlKey | cmdKey
            pin_key: 0x23,                // kVK_ANSI_P
            pin_modifiers: 0x1800,
            border_key: 0x63,         // kVK_F3
            border_modifiers: 0x1000, // controlKey
            longshot_key: DEFAULT_LONGSHOT_KEY,
            legacy_modifiers: 0,
        }
    }
}

/// 工具切换键（M3）：覆盖层内按数字键切换标注工具，每工具一个虚拟键码；
/// `UNBOUND` = 该项不绑定（SYS-2）。
type ToolKey = u32;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolKeys {
    pub rect: ToolKey,
    pub arrow: ToolKey,
    pub text: ToolKey,
    pub blur: ToolKey,
    pub highlight: ToolKey,
    pub glow: ToolKey,
}

#[cfg(not(target_os = "macos"))]
impl Default for ToolKeys {
    fn default() -> Self {
        Self {
            rect: 0x31,      // 1
            arrow: 0x32,     // 2
            text: 0x33,      // 3
            blur: 0x34,      // 4
            highlight: 0x35, // 5
            glow: 0x36,      // 6
        }
    }
}

/// macOS：数字键 1–6 的 `kVK_ANSI_*` 键码（不连续：5 = 0x17，6 = 0x16）。
#[cfg(target_os = "macos")]
impl Default for ToolKeys {
    fn default() -> Self {
        Self {
            rect: 0x12,
            arrow: 0x13,
            text: 0x14,
            blur: 0x15,
            highlight: 0x17,
            glow: 0x16,
        }
    }
}

/// 贴图初始位置（M2b）：编辑器内贴选区（PIN-1）时显示在截图原位置或屏幕中央；
/// 剪贴板贴图（PIN-2）无「原位置」，始终居中。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PinPosition {
    /// 在截图选区原位置显示（默认）。
    #[default]
    #[serde(rename = "original")]
    Original,
    /// 在虚拟屏中央显示。
    #[serde(rename = "center")]
    Center,
}

/// 保存文件名模板（OUT-2）：保存对话框预填的文件名，可在设置面板选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NameStyle {
    /// `airsnip-20241231-235959`（日期 + 时间，默认）。
    #[default]
    #[serde(rename = "datetime")]
    DateTime,
    /// `1735689599123`（Unix 毫秒时间戳）。
    #[serde(rename = "timestamp")]
    Timestamp,
    /// `airsnip-20241231`（仅日期）。
    #[serde(rename = "date")]
    Date,
}

impl NameStyle {
    pub fn title(self) -> &'static str {
        match self {
            NameStyle::DateTime => "日期 + 时间",
            NameStyle::Timestamp => "时间戳（Unix 毫秒）",
            NameStyle::Date => "仅日期",
        }
    }
}

/// 覆盖层界面缩放（CAP-5）：自动 / 固定百分比。
/// 只影响截图工具栏、手柄、尺寸提示与光标，不影响标注内容（线宽、字号）与导出结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UiScale {
    /// 按显示器 DPI 与物理分辨率自动（默认）。
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "100")]
    P100,
    #[serde(rename = "125")]
    P125,
    #[serde(rename = "150")]
    P150,
    #[serde(rename = "175")]
    P175,
    #[serde(rename = "200")]
    P200,
}

impl UiScale {
    pub const ALL: [UiScale; 6] = [
        UiScale::Auto,
        UiScale::P100,
        UiScale::P125,
        UiScale::P150,
        UiScale::P175,
        UiScale::P200,
    ];

    /// 设置面板行标签。
    pub fn title(self) -> &'static str {
        match self {
            UiScale::Auto => "界面缩放·自动",
            UiScale::P100 => "界面缩放·100%",
            UiScale::P125 => "界面缩放·125%",
            UiScale::P150 => "界面缩放·150%",
            UiScale::P175 => "界面缩放·175%",
            UiScale::P200 => "界面缩放·200%",
        }
    }

    /// 固定倍率；`Auto` 返回 None（由 `Ui::resolve` 按显示器算）。
    pub fn factor(self) -> Option<f32> {
        match self {
            UiScale::Auto => None,
            UiScale::P100 => Some(1.0),
            UiScale::P125 => Some(1.25),
            UiScale::P150 => Some(1.5),
            UiScale::P175 => Some(1.75),
            UiScale::P200 => Some(2.0),
        }
    }
}

/// 保存相关运行时偏好（OUT-2）：文件名模板 + 上次格式 + 上次目录。
/// 截图开始时从配置快照后传入覆盖层线程（`HWND` 不可跨线程，值可以）。
#[derive(Debug, Clone, Default)]
pub struct SavePrefs {
    /// 保存文件名模板。
    pub name_style: NameStyle,
    /// 上次保存的格式扩展名（下次对话框默认选中该格式）。
    pub last_ext: Option<String>,
    /// 上次保存的目录（下次对话框初始位置；目录已不存在时回退默认目录）。
    pub last_dir: Option<String>,
}

/// 根配置。所有字段带默认值，旧配置文件缺字段时自动补齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 开机自启开关。实际生效状态以注册表 Run 键 + StartupApproved 为准。
    pub auto_start: bool,
    /// 界面语言（一期仅 zh-CN，预留 i18n）。
    pub language: String,
    pub theme: ThemeSetting,
    pub hotkey: HotkeyConfig,
    /// 覆盖层工具切换键（M3）。
    pub tool_keys: ToolKeys,
    /// 是否已提示过无修饰键热键风险（避免每次启动弹窗）。
    pub hotkey_warned: bool,
    /// 贴图初始位置（M2b）。
    pub pin_position: PinPosition,
    /// 贴图 1px 蓝色边框（M2b，默认开）。
    pub pin_border: bool,
    /// 覆盖层界面缩放（CAP-5，默认自动）：4K@100% 这类高密度屏需要手动放大。
    pub ui_scale: UiScale,
    /// 保存文件名模板（OUT-2）。
    pub name_style: NameStyle,
    /// 上次保存用的格式扩展名（OUT-2，运行时记忆）：保存对话框下次默认选中该格式。
    pub last_save_ext: Option<String>,
    /// 上次保存的目录（OUT-2，运行时记忆）：保存对话框下次的初始位置。
    pub last_save_dir: Option<String>,
    /// 长截图拼接高度上限（像素，LNG-7，默认 30000）。
    pub longshot_max_height: u32,
    /// 启动后自动检查更新（SYS-7，默认开）。关闭后只能从关于页手动检查。
    pub auto_check_update: bool,
    /// 用户选择「跳过此版本」的版本号（SYS-7）。
    pub skipped_version: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_start: false,
            language: "zh-CN".to_string(),
            theme: ThemeSetting::Follow,
            hotkey: HotkeyConfig::default(),
            tool_keys: ToolKeys::default(),
            hotkey_warned: false,
            pin_position: PinPosition::Original,
            pin_border: true,
            ui_scale: UiScale::Auto,
            name_style: NameStyle::DateTime,
            last_save_ext: None,
            last_save_dir: None,
            longshot_max_height: 30000,
            auto_check_update: true,
            skipped_version: None,
        }
    }
}

impl Config {
    /// 配置目录：`%APPDATA%\airsnip`。
    #[cfg(not(target_os = "macos"))]
    pub fn config_dir() -> PathBuf {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(appdata).join("airsnip")
    }

    /// 配置目录：`~/Library/Application Support/AirSnip`。
    #[cfg(target_os = "macos")]
    pub fn config_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join("Library/Application Support/AirSnip")
    }

    /// 配置文件路径。
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// 加载配置：文件缺失或解析失败时回退默认值（SYS-5）。
    pub fn load() -> Self {
        let mut cfg: Config = match std::fs::read_to_string(Self::config_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Config::default(),
        };
        // 迁移：旧版共享 modifiers（同时作用于两键）→ 拆分到各键
        let h = &mut cfg.hotkey;
        if h.legacy_modifiers != 0 && h.screenshot_modifiers == 0 && h.pin_modifiers == 0 {
            h.screenshot_modifiers = h.legacy_modifiers;
            h.pin_modifiers = h.legacy_modifiers;
        }
        cfg
    }

    /// 保存配置：自动创建目录，写入失败静默忽略。
    pub fn save(&self) {
        let text = toml::to_string_pretty(self).unwrap_or_default();
        let _ = std::fs::create_dir_all(Self::config_dir());
        let _ = std::fs::write(Self::config_path(), text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧配置（共享 modifiers）能正确迁移到各键独立修饰键。
    /// 仅 Windows：旧格式只存在于 Windows 老版本；macOS 缺省修饰键非 0，迁移条件本就不成立。
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn legacy_shared_modifiers_migrate() {
        let old = "[hotkey]\nscreenshot_key = 0x70\npin_key = 0x72\nmodifiers = 1\n";
        let cfg: Config = toml::from_str(old).unwrap();
        let (mut h, _) = (cfg.hotkey.clone(), ());
        assert_eq!(h.legacy_modifiers, 1, "旧 modifiers 应读入遗留字段");
        assert_eq!(h.screenshot_modifiers, 0);
        // 模拟 load() 的迁移逻辑
        if h.legacy_modifiers != 0 && h.screenshot_modifiers == 0 && h.pin_modifiers == 0 {
            h.screenshot_modifiers = h.legacy_modifiers;
            h.pin_modifiers = h.legacy_modifiers;
        }
        assert_eq!(h.screenshot_modifiers, 1);
        assert_eq!(h.pin_modifiers, 1);
    }

    /// 新格式各键独立修饰键，互不干扰。
    #[test]
    fn per_key_modifiers_roundtrip() {
        let cfg = Config {
            hotkey: HotkeyConfig {
                screenshot_key: 0x41,
                screenshot_modifiers: 1, // Alt
                pin_key: 0x72,
                pin_modifiers: 4, // Shift
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.hotkey.screenshot_modifiers, 1);
        assert_eq!(back.hotkey.pin_modifiers, 4);
        assert!(!text.contains("legacy_modifiers"), "遗留字段不应写入配置：{text}");
    }

    /// 命名模板（OUT-2）：旧配置缺字段时回退默认值，新值能往返。
    #[test]
    fn name_style_roundtrip_and_defaults() {
        let old: Config = toml::from_str("[hotkey]\nscreenshot_key = 0x70\n").unwrap();
        assert_eq!(old.name_style, NameStyle::DateTime, "旧配置应回退默认命名模板");
        assert_eq!(old.last_save_ext, None, "旧配置无上次格式记录");

        let cfg = Config {
            name_style: NameStyle::Date,
            ..Default::default()
        };
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.name_style, NameStyle::Date);
        assert_eq!(back.name_style.title(), "仅日期");
    }

    /// 长截图键（M5）：默认 S；旧配置缺字段时回退默认值，新值能往返。
    #[test]
    fn longshot_key_defaults_and_roundtrip() {
        assert_eq!(
            HotkeyConfig::default().longshot_key,
            DEFAULT_LONGSHOT_KEY,
            "默认长截图键应为 S"
        );
        let old: Config = toml::from_str("[hotkey]\nscreenshot_key = 0x70\n").unwrap();
        assert_eq!(
            old.hotkey.longshot_key, DEFAULT_LONGSHOT_KEY,
            "旧配置应回退默认长截图键"
        );

        let cfg = Config {
            hotkey: HotkeyConfig {
                longshot_key: 0x4C, // L
                ..Default::default()
            },
            ..Default::default()
        };
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.hotkey.longshot_key, 0x4C);
    }

    /// 上次保存目录（OUT-2）：旧配置缺字段回退 None，新值能往返。
    #[test]
    fn last_save_dir_roundtrip() {
        let old: Config = toml::from_str("[hotkey]\nscreenshot_key = 0x70\n").unwrap();
        assert_eq!(old.last_save_dir, None, "旧配置应无上次保存目录");

        let cfg = Config {
            last_save_dir: Some(r"D:\shots".to_string()),
            ..Default::default()
        };
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.last_save_dir.as_deref(), Some(r"D:\shots"));
    }

    /// 界面缩放（CAP-5）：旧配置回退自动，固定倍率能往返。
    #[test]
    fn ui_scale_defaults_and_roundtrip() {
        let old: Config = toml::from_str("[hotkey]\nscreenshot_key = 0x70\n").unwrap();
        assert_eq!(old.ui_scale, UiScale::Auto, "旧配置应回退自动缩放");
        assert_eq!(UiScale::Auto.factor(), None, "自动模式不固定倍率");
        assert_eq!(UiScale::P150.factor(), Some(1.5));

        let cfg = Config {
            ui_scale: UiScale::P175,
            ..Default::default()
        };
        let back: Config = toml::from_str(&toml::to_string_pretty(&cfg).unwrap()).unwrap();
        assert_eq!(back.ui_scale, UiScale::P175);
        // 每个选项都有标签与（Auto 之外的）倍率
        for m in UiScale::ALL {
            assert!(!m.title().is_empty());
            if m != UiScale::Auto {
                assert!(m.factor().is_some(), "{m:?} 应有固定倍率");
            }
        }
    }
}
