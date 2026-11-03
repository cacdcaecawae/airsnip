// SPDX-License-Identifier: GPL-3.0-only
//! AirSnip 共享核心：与平台无关的标注对象模型、tiny-skia 渲染、截图覆盖层交互、长截图拼接、
//! 图片编码、调色板与配置模型。Windows 外壳（根包 `airsnip`）与 macOS 外壳（`crates/airsnip-macos` +
//! `macos/`）共用；平台能力经 `overlay::Host` 注入，少量平台分支（文字后端、界面缩放的显示器查询、
//! 配置路径、本地时间）以 `cfg` 隔离。

pub mod capture;
pub mod editor;
pub mod longshot;
pub mod output;
pub mod overlay;
pub mod perf;
pub mod render;
pub mod settings;
pub mod theme;
