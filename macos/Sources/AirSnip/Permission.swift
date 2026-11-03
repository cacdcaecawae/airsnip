// SPDX-License-Identifier: GPL-3.0-only
// 屏幕录制权限（CAP-1）：已授权直接返回；未授权时先触发系统授权框，仍未授权则给出操作指引。
// 不轮询、不反复重试：用户在系统设置里打开权限后需要重新启动 AirSnip 才生效（系统限制）。

import AppKit

enum ScreenPermission {
    /// 「隐私与安全性 › 屏幕录制」设置页。
    private static let settingsURL = URL(
        string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")!

    /// 是否可以截图；未授权时负责提示并返回 false。
    static func ensure() -> Bool {
        if CGPreflightScreenCaptureAccess() { return true }
        // 首次调用会弹出系统授权框；此后再调用只返回 false，不会重复弹框
        if CGRequestScreenCaptureAccess() { return true }
        let alert = NSAlert()
        alert.messageText = "需要「屏幕录制」权限"
        alert.informativeText = """
            AirSnip 需要读取屏幕画面才能截图。
            请在「系统设置 › 隐私与安全性 › 屏幕录制」（macOS 15 起名为「屏幕与系统音频录制」）\
            中打开 AirSnip，然后退出并重新打开 AirSnip。
            """
        alert.addButton(withTitle: "打开系统设置")
        alert.addButton(withTitle: "取消")
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            NSWorkspace.shared.open(settingsURL)
        }
        return false
    }
}
