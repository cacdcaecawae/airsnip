// SPDX-License-Identifier: GPL-3.0-only
//! 图标资源表：编译期嵌入 `assets/icons/ui/` 下的 SVG。
//! 文件名即变体名的小写形式，新增图标时同时改这里与资源目录（见该目录 README）。

macro_rules! icon_table {
    ($($name:ident => $path:literal),* $(,)?) => {
        /// 界面图标。绘制入口见 `crate::render::icon::draw`。
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Icon {
            $($name),*
        }

        impl Icon {
            /// 全部图标，顺序与资源表一致（测试用）。
            pub const ALL: &'static [Icon] = &[$(Icon::$name),*];

            /// 嵌入的 SVG 源文本。
            pub fn source(self) -> &'static str {
                match self {
                    $(Icon::$name => include_str!(concat!("../../../../../assets/icons/ui/", $path)),)*
                }
            }
        }
    };
}

icon_table! {
    // 覆盖层工具栏（EDT-7 / EDT-1 / EDT-4 / PIN-1 / M4 / OUT-1 / OUT-2）
    Rect => "toolbar/rect.svg",
    Arrow => "toolbar/arrow.svg",
    Text => "toolbar/text.svg",
    Highlight => "toolbar/highlight.svg",
    Blur => "toolbar/blur.svg",
    Glow => "toolbar/glow.svg",
    Fill => "toolbar/fill.svg",
    Undo => "toolbar/undo.svg",
    Redo => "toolbar/redo.svg",
    Pin => "toolbar/pin.svg",
    Longshot => "toolbar/longshot.svg",
    Copy => "toolbar/copy.svg",
    Save => "toolbar/save.svg",
    Bold => "toolbar/bold.svg",
    Italic => "toolbar/italic.svg",
    Minus => "toolbar/minus.svg",
    Plus => "toolbar/plus.svg",
    // 长截图状态条（M4）
    Check => "status/check.svg",
    Cross => "status/cross.svg",
    // 设置面板行内按钮（SYS-2）
    Delete => "panel/delete.svg",
}
