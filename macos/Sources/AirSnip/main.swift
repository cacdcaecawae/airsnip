// SPDX-License-Identifier: GPL-3.0-only
// AirSnip macOS 入口：菜单栏常驻应用（无 Dock 图标）。`--selftest` 只做 Swift ↔ Rust 桥接自检（CI 用）。

import AppKit

if CommandLine.arguments.contains("--selftest") {
    exit(SelfTest.run() ? 0 : 1)
}

// 单实例：直接运行二进制时可能已有实例在跑（从访达或 open 打开由系统保证只有一个）。
// 交给已有实例（它收到「再次打开」会弹出设置窗口）后退出。
if let id = Bundle.main.bundleIdentifier,
    let other = NSRunningApplication.runningApplications(withBundleIdentifier: id).first(where: { $0 != .current }),
    let url = other.bundleURL
{
    NSWorkspace.shared.open(url)
    exit(0)
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
