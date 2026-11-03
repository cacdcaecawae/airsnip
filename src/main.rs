// SPDX-License-Identifier: GPL-3.0-only
//! 入口：初始化应用壳并进入消息循环。
//!
//! 根包是 Windows 外壳；平台无关部分在 `airsnip-core`，以同名模块再导出，
//! 原有 `crate::editor` / `crate::render` 路径保持不变。macOS 版见 `macos/`。

// release 版为 GUI 子系统（双击不弹黑色控制台窗口）；debug 版保留控制台，
// 便于查看 `perf.rs` 的埋点输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
pub use airsnip_core::{editor, perf, render};

#[cfg(windows)]
pub mod app;
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod longshot;
#[cfg(windows)]
pub mod ocr; // 二期
#[cfg(windows)]
pub mod output;
#[cfg(windows)]
pub mod overlay;
#[cfg(windows)]
pub mod pin;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod theme;
pub mod update;

#[cfg(windows)]
fn main() {
    std::process::exit(app::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("根包 airsnip 是 Windows 版；macOS 版请用 scripts/package-macos.sh 构建 macos/ 工程。");
    std::process::exit(1);
}
