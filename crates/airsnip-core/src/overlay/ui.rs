// SPDX-License-Identifier: GPL-3.0-only
//! 覆盖层 UI 缩放（CAP-5）：缩放工具栏、手柄、尺寸提示与光标，
//! 让 4K / 200% 等高密度屏上的控件保持与 1080p 相同的物理尺寸。
//!
//! 覆盖层窗口横跨整个虚拟屏幕，`GetDpiForWindow` 只会返回窗口所在显示器的 DPI，
//! 所以用「光标位置 → 最近显示器 → 该屏有效 DPI + 分辨率」判定用户实际在操作的屏幕。
//!
//! 自动模式取「DPI 缩放」与「分辨率缩放」的较大值：
//! - DPI 项处理「系统缩放已把界面放大」的常规场景（4K@200% → 2.0）；
//! - 分辨率项处理「屏幕密度高但系统没缩放」的场景（4K@100% → 1.52），
//!   此时 DPI 项只有 1.0，靠分辨率项兜底。
//! 两项取大值不会把已经调过系统缩放的 UI 再放大一遍（4K@150% 仍是 1.5）。
//!
//! 任一步失败都回退 1.0，保持改动前的观感。
//!
//! macOS 的覆盖层按显示器分开创建，系统倍率取 `backingScaleFactor`，由平台层调用 [`Ui::auto`]。

#[cfg(windows)]
use std::mem::size_of;

#[cfg(windows)]
use windows::Win32::Foundation::POINT;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

#[cfg(windows)]
use crate::settings::UiScale;

/// 基准 DPI（100% 缩放）。
const BASE_DPI: u32 = 96;
/// 分辨率补偿的基准面积（1920×1080）。
const BASE_AREA: f32 = 1920.0 * 1080.0;
/// 分辨率补偿指数：面积比的 0.3 次方。4K 得 1.52、2560×1440 得 1.19、1080p 得 1.0。
/// 取 0.3 而非 0.5：4K@100% 的用户要的是「别太小」，不是「等比放大一倍」。
const RES_EXPONENT: f32 = 0.3;
/// 缩放下限：100% 及更低不再缩小（本来就小的控件再缩会更难点）。
const MIN_SCALE: f32 = 1.0;
/// 缩放上限：罕见的高缩放下避免工具栏占掉半个屏幕。
const MAX_SCALE: f32 = 3.0;

/// 覆盖层 UI 缩放系数（1.0 = 100% DPI）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ui(f32);

impl Ui {
    /// 按设置解析缩放（覆盖层 / 长截图创建时调用一次）。
    #[cfg(windows)]
    pub fn resolve(mode: UiScale) -> Self {
        match mode.factor() {
            Some(f) => Self::from_scale(f),
            None => Self::auto_at_cursor(),
        }
    }

    /// 由显示器 DPI 换算，夹在 `[MIN_SCALE, MAX_SCALE]`。
    pub fn from_dpi(dpi: u32) -> Self {
        Self::from_scale(dpi.max(1) as f32 / BASE_DPI as f32)
    }

    /// 由倍率直接构造，夹在 `[MIN_SCALE, MAX_SCALE]`。
    pub fn from_scale(scale: f32) -> Self {
        Self(scale.clamp(MIN_SCALE, MAX_SCALE))
    }

    /// 缩放系数。
    pub fn scale(self) -> f32 {
        self.0
    }

    /// 基准像素 → 缩放后整数像素（至少 1，避免被舍成 0）。
    pub fn px(self, base: f32) -> i32 {
        ((base * self.0).round() as i32).max(1)
    }

    /// 基准像素 → 缩放后浮点像素。
    pub fn f(self, base: f32) -> f32 {
        base * self.0
    }

    /// 自动缩放：取「系统缩放倍率」与「分辨率补偿」的较大值。
    /// 系统倍率在 Windows 为 DPI/96，在 macOS 为 `backingScaleFactor`；`w`/`h` 为显示器物理像素。
    pub fn auto(base_scale: f32, w: u32, h: u32) -> Self {
        Self::from_scale(base_scale.max(resolution_scale(w, h)))
    }

    /// 自动：按光标所在显示器计算。
    #[cfg(windows)]
    fn auto_at_cursor() -> Self {
        let Some(m) = monitor_at_cursor() else {
            return Self::from_scale(1.0);
        };
        Self::auto(Self::from_dpi(m.dpi).scale(), m.w, m.h)
    }
}

/// 分辨率补偿：以 1920×1080 为基准，按面积比的 `RES_EXPONENT` 次方温和放大。
fn resolution_scale(w: u32, h: u32) -> f32 {
    (w as f32 * h as f32 / BASE_AREA).powf(RES_EXPONENT)
}

/// 光标所在显示器的 DPI 与物理分辨率。
#[cfg(any(windows, test))]
struct Monitor {
    dpi: u32,
    w: u32,
    h: u32,
}

#[cfg(test)]
impl Monitor {
    fn resolution_scale(&self) -> f32 {
        resolution_scale(self.w, self.h)
    }
}

/// 光标所在显示器的 DPI 与分辨率；任一步失败返回 None。
#[cfg(windows)]
fn monitor_at_cursor() -> Option<Monitor> {
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return None;
        }
        let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        if monitor.0.is_null() {
            return None;
        }
        let (mut dx, mut dy) = (BASE_DPI, BASE_DPI);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let w = (info.rcMonitor.right - info.rcMonitor.left).max(1) as u32;
        let h = (info.rcMonitor.bottom - info.rcMonitor.top).max(1) as u32;
        Some(Monitor { dpi: dx.max(1), w, h })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_follows_dpi_and_clamps() {
        assert_eq!(Ui::from_dpi(96).scale(), 1.0);
        assert_eq!(Ui::from_dpi(120).scale(), 1.25);
        assert_eq!(Ui::from_dpi(144).scale(), 1.5);
        assert_eq!(Ui::from_dpi(192).scale(), 2.0);
        // 低于 100% 不下缩，高于 300% 不再放大
        assert_eq!(Ui::from_dpi(72).scale(), 1.0);
        assert_eq!(Ui::from_dpi(480).scale(), 3.0);
        // 非法 DPI 不产生 0 或 NaN
        assert_eq!(Ui::from_dpi(0).scale(), 1.0);
    }

    #[test]
    fn px_rounds_and_never_returns_zero() {
        let ui = Ui::from_dpi(192);
        assert_eq!(ui.px(26.0), 52);
        assert_eq!(ui.px(7.0), 14);
        assert_eq!(ui.f(3.5), 7.0);
        // 极小尺寸在 100% 下也不会变成 0（工具栏/手柄会画不出来）
        assert_eq!(Ui::from_dpi(96).px(0.4), 1);
    }

    /// 分辨率补偿：1080p 不放大，4K 约 1.5 倍，2K 温和放大。
    #[test]
    fn resolution_scale_is_gentle() {
        let at = |w: u32, h: u32| Monitor { dpi: 96, w, h }.resolution_scale();
        assert!((at(1920, 1080) - 1.0).abs() < 0.01, "1080p 不应放大");
        let uhd = at(3840, 2160);
        assert!((1.45..1.60).contains(&uhd), "4K 应约 1.5 倍，实际 {uhd}");
        let qhd = at(2560, 1440);
        assert!((1.10..1.25).contains(&qhd), "2K 应温和放大，实际 {qhd}");
        // 同分辨率下宽屏更宽 → 略大，但差异有限
        assert!(at(3440, 1440) > qhd);
    }

    /// 自动模式取 DPI 与分辨率两者的较大值：已调过系统缩放的屏不会被再放大一遍。
    #[test]
    fn auto_takes_max_of_dpi_and_resolution() {
        // 4K@100%：DPI 项 1.0，分辨率项 ~1.52 → 取 1.52
        let uhd100 = Monitor {
            dpi: 96,
            w: 3840,
            h: 2160,
        };
        assert!(uhd100.resolution_scale() > Ui::from_dpi(uhd100.dpi).scale());
        // 4K@150%：DPI 项 1.5，分辨率项 1.52 → 取 1.52（几乎不变）
        let uhd150 = Monitor {
            dpi: 144,
            w: 3840,
            h: 2160,
        };
        let s = Ui::from_dpi(uhd150.dpi).scale().max(uhd150.resolution_scale());
        assert!((1.45..1.60).contains(&s), "4K@150% 不应被大幅放大，实际 {s}");
        // 4K@200%：DPI 项 2.0 胜出
        let uhd200 = Monitor {
            dpi: 192,
            w: 3840,
            h: 2160,
        };
        assert_eq!(Ui::from_dpi(uhd200.dpi).scale(), 2.0);
        assert!(uhd200.resolution_scale() < 2.0);
        // 1080p@150%：DPI 项 1.5 胜出
        let fhd150 = Monitor {
            dpi: 144,
            w: 1920,
            h: 1080,
        };
        assert_eq!(Ui::from_dpi(fhd150.dpi).scale().max(fhd150.resolution_scale()), 1.5);
    }

    /// macOS Retina（backingScaleFactor 2）：系统倍率胜出；1x 外接 2K 屏取分辨率补偿。
    #[test]
    fn auto_uses_backing_scale() {
        assert_eq!(Ui::auto(2.0, 3024, 1964).scale(), 2.0);
        let qhd = Ui::auto(1.0, 2560, 1440).scale();
        assert!((1.10..1.25).contains(&qhd), "1x 2K 屏应温和放大，实际 {qhd}");
    }

    /// 手动倍率直接生效，并同样夹在合法区间内。
    #[cfg(windows)]
    #[test]
    fn manual_scale_overrides() {
        assert_eq!(Ui::resolve(UiScale::P150).scale(), 1.5);
        assert_eq!(Ui::resolve(UiScale::P200).scale(), 2.0);
        assert_eq!(Ui::resolve(UiScale::P100).scale(), 1.0);
        // 自动模式不会 panic（无显示器时回退 1.0，有显示器时为正数）
        assert!(Ui::resolve(UiScale::Auto).scale() >= 1.0);
    }
}
