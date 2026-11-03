// SPDX-License-Identifier: GPL-3.0-only
// 菜单栏应用壳（SYS-1/SYS-2/SYS-3）：菜单栏图标与菜单（截图 / 设置 / 退出）、全局热键（截图、剪贴板贴图）、
// 截图流程（权限 → 抓屏 → 覆盖层 → 长截图）。待机时没有定时器与轮询，只等热键与菜单事件。
// 再次打开 AirSnip（双击 App）时打开设置窗口，同 Windows 再次启动时唤起已运行的实例。

import AppKit
import CAirsnip

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem?
    private let shotItem = NSMenuItem(title: "截图", action: #selector(startCapture), keyEquivalent: "")
    private var hotKeys: [HotKey] = []
    private var settings: SettingsWindow?
    private var overlay: OverlayController?
    private var longshot: LongshotController?
    private var capturing = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        setupStatusItem()
        registerHotKeys()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        openSettings()
        return true
    }

    /// 按配置注册全局热键（启动时、设置里改键后），失败的一并提示。
    private func registerHotKeys() {
        hotKeys.removeAll()
        var s = AsSettings()
        airsnip_settings_load(&s)
        shotItem.title = "截图（\(hotKeyLabel(keyCode: s.shot_key, modifiers: s.shot_mods))）"
        var failed: [String] = []
        register("截图", s.shot_key, s.shot_mods, id: 1, &failed) { [weak self] in self?.startCapture() }
        // 截图进行中：贴当前选区（同 Windows 截图时按 F3）；否则贴剪贴板里的图片
        register("贴图", s.pin_key, s.pin_mods, id: 2, &failed) { [weak self] in
            // 另存为等模态对话框开着时不处理：此刻核心还停在保存流程里，贴图会提前结束截图
            guard NSApp.modalWindow == nil else { return }
            if let overlay = self?.overlay { overlay.pinSelection() } else { Pins.fromClipboard() }
        }
        if !failed.isEmpty {
            Alerts.show("快捷键注册失败，可能已被其他程序占用：\n\(failed.joined(separator: "\n"))\n可在设置 › 快捷键中更换；截图仍可点菜单栏图标。")
        }
    }

    @objc private func openSettings() {
        if settings == nil {
            settings = SettingsWindow(
                onHotKeysChanged: { [weak self] in self?.registerHotKeys() },
                suspendHotKeys: { [weak self] in self?.hotKeys.removeAll() })
            settings?.window?.center()
        }
        NSApp.activate(ignoringOtherApps: true)
        settings?.showWindow(nil)
    }

    private func register(
        _ name: String, _ key: UInt32, _ mods: UInt32, id: UInt32, _ failed: inout [String],
        _ action: @escaping () -> Void
    ) {
        let label = hotKeyLabel(keyCode: key, modifiers: mods)
        do {
            hotKeys.append(try HotKey(keyCode: key, modifiers: mods, id: id, handler: action))
            NSLog("[airsnip] %@热键 %@ 已注册", name, label)
        } catch {
            failed.append("\(name) \(label)（错误 \((error as? HotKeyError)?.status ?? 0)）")
        }
    }

    private func setupStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        let image = NSImage(systemSymbolName: "scissors", accessibilityDescription: "AirSnip")
        image?.isTemplate = true  // 跟随菜单栏深浅色
        item.button?.image = image
        let menu = NSMenu()
        shotItem.target = self
        menu.addItem(shotItem)
        let prefs = NSMenuItem(title: "设置…", action: #selector(openSettings), keyEquivalent: ",")
        prefs.target = self
        menu.addItem(prefs)
        menu.addItem(.separator())
        let version = String(cString: airsnip_version())
        let about = NSMenuItem(title: "AirSnip \(version)（GPL-3.0）", action: nil, keyEquivalent: "")
        about.isEnabled = false
        menu.addItem(about)
        menu.addItem(NSMenuItem(title: "退出 AirSnip", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"))
        item.menu = menu
        statusItem = item
    }

    /// 截图：权限 → 抓所有显示器 → 显示覆盖层。会话进行中再触发直接忽略。
    @objc func startCapture() {
        NSLog("[airsnip] 收到截图请求")
        guard overlay == nil, longshot == nil, !capturing else { return }
        // 先置位：权限提示框是模态的，开着时再按热键不能叠出第二个
        capturing = true
        guard ScreenPermission.ensure() else {
            capturing = false
            return
        }
        let displays = DisplayInfo.all()
        let pins = Pins.windowIDs
        let windows = ScreenCapture.windowRects(keep: pins)
        let t0 = CFAbsoluteTimeGetCurrent()
        Task { @MainActor in
            defer { self.capturing = false }
            do {
                let shots = try await ScreenCapture.captureAll(displays, keep: pins)
                let t1 = CFAbsoluteTimeGetCurrent()
                let ctrl = OverlayController(shots: shots, windows: windows) { [weak self] req in
                    self?.overlay = nil
                    if let req { self?.longshot = LongshotController(req) { self?.longshot = nil } }
                }
                guard let ctrl else {
                    Alerts.show("截图失败：\(String(cString: airsnip_last_error()))")
                    return
                }
                self.overlay = ctrl
                ctrl.show()
                let t2 = CFAbsoluteTimeGetCurrent()
                NSLog(
                    "[airsnip] 截图：%d 块屏，抓屏 %.0f ms，建覆盖层 + 显示 %.0f ms", shots.count,
                    (t1 - t0) * 1000, (t2 - t1) * 1000)
            } catch {
                Alerts.show("截图失败：\(error.localizedDescription)")
            }
        }
    }
}
