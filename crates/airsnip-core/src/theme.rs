// SPDX-License-Identifier: GPL-3.0-only
//! 主题调色板（M3）：浅/深两套配色，覆盖层工具栏与设置面板共用。
//! 跟随系统时的主题检测属于平台层（Windows 读注册表，macOS 读 effectiveAppearance）。

/// 界面主题。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// 主题调色板：界面控件配色（工具栏、设置面板共用）。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 面板/工具栏背景。
    pub bg: [u8; 4],
    /// 外边框。
    pub border: [u8; 4],
    /// 图标与文字主色。
    pub icon: [u8; 4],
    /// 图标与文字次级色（禁用态）。
    pub icon_dim: [u8; 4],
    /// 组分隔线。
    pub divider: [u8; 4],
    /// 强调色（选中/激活）。
    pub accent: [u8; 4],
    /// 强调色软底色（激活按钮背景）。
    pub accent_soft: [u8; 4],
    /// 色块描边。
    pub swatch_border: [u8; 4],
    /// 荧光笔工具按钮的图标色（RGB + 未激活时的 alpha）。
    /// 荧光黄在浅色工具栏上几乎看不见，浅色主题需换成暗金。
    pub highlight_ink: [u8; 4],
    /// 设置面板正文。
    pub text: [u8; 4],
    /// 设置面板次级文字（说明/描述）。
    pub text_dim: [u8; 4],
    /// 设置面板悬停行背景。
    pub row_hover: [u8; 4],
    /// 开关开（滑钮轨道）。
    pub toggle_on: [u8; 4],
    /// 开关关（轨道）。
    pub toggle_off: [u8; 4],
}

/// 深色配色（覆盖层工具栏现行风格）。
pub const DARK: Palette = Palette {
    bg: [32, 32, 32, 240],
    border: [255, 255, 255, 60],
    icon: [255, 255, 255, 235],
    icon_dim: [255, 255, 255, 70],
    divider: [255, 255, 255, 40],
    accent: [26, 115, 232, 255],
    accent_soft: [26, 115, 232, 90],
    swatch_border: [0, 0, 0, 120],
    // 深色工具栏上用荧光黄最贴近高亮效果
    highlight_ink: [255, 230, 0, 160],
    text: [255, 255, 255, 235],
    text_dim: [150, 150, 150, 255],
    row_hover: [255, 255, 255, 18],
    toggle_on: [26, 115, 232, 255],
    toggle_off: [120, 120, 120, 255],
};

/// 浅色配色（白色底、深色字）。
pub const LIGHT: Palette = Palette {
    bg: [248, 248, 248, 246],
    border: [0, 0, 0, 45],
    icon: [40, 40, 40, 235],
    icon_dim: [40, 40, 40, 65],
    divider: [0, 0, 0, 25],
    accent: [26, 115, 232, 255],
    accent_soft: [26, 115, 232, 40],
    swatch_border: [0, 0, 0, 60],
    // 浅底上用暗金：纯荧光黄（#FFE600）在 #F8F8F8 上对比度不足，看不清
    highlight_ink: [166, 124, 0, 220],
    text: [40, 40, 40, 255],
    text_dim: [120, 120, 120, 255],
    row_hover: [0, 0, 0, 10],
    toggle_on: [26, 115, 232, 255],
    toggle_off: [180, 180, 180, 255],
};

/// 取主题对应调色板。
pub fn palette(theme: Theme) -> &'static Palette {
    match theme {
        Theme::Light => &LIGHT,
        Theme::Dark => &DARK,
    }
}
