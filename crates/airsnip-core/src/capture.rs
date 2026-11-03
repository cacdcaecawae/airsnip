// SPDX-License-Identifier: GPL-3.0-only
//! 抓屏结果（CAP-1）：平台层抓取后交给覆盖层的像素数据。
//! Windows 一次抓整个虚拟屏幕（原点可为负）；macOS 每块显示器一份（原点为 0）。

/// 一次抓屏结果。
pub struct CapturedScreen {
    /// 宽度（像素）。
    pub width: u32,
    /// 高度（像素）。
    pub height: u32,
    /// 左上角物理坐标 X（Windows 虚拟屏可为负，多显示器负坐标布局）。
    pub origin_x: i32,
    /// 左上角物理坐标 Y。
    pub origin_y: i32,
    /// RGBA 像素（供 tiny-skia），长度 width * height * 4。
    pub rgba: Vec<u8>,
}
