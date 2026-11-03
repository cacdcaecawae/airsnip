// SPDX-License-Identifier: GPL-3.0-only
// 设置窗口（SYS-3）：通用 / 快捷键 / 保存 / 外观 / 关于，选项与 Windows 设置面板一致。
// 改动即时写入配置文件；下次截图、贴图即生效，全局热键由 AppDelegate 重新注册（失败会提示）。
// 开机自启以系统登录项（SMAppService）的实际状态为准。

import AppKit
import CAirsnip
import ServiceManagement

/// 可改的快捷键：名称、是否全局热键（带修饰键，按「键 + 修饰」比较冲突）及其在设置里的字段。
private enum Shortcut: CaseIterable {
    case shot, pin, border, longshot, rect, arrow, text, blur, highlight, glow

    var name: String {
        ["截图", "贴图", "贴图边框", "长截图", "工具·矩形", "工具·箭头", "工具·文本", "工具·模糊", "工具·荧光笔", "工具·聚光灯"][
            Shortcut.allCases.firstIndex(of: self)!]
    }

    var withMods: Bool { [.shot, .pin, .border].contains(self) }

    var code: WritableKeyPath<AsSettings, UInt32> {
        switch self {
        case .shot: \.shot_key
        case .pin: \.pin_key
        case .border: \.border_key
        case .longshot: \.longshot_key
        case .rect: \.tool_rect
        case .arrow: \.tool_arrow
        case .text: \.tool_text
        case .blur: \.tool_blur
        case .highlight: \.tool_highlight
        case .glow: \.tool_glow
        }
    }

    var mods: WritableKeyPath<AsSettings, UInt32>? {
        switch self {
        case .shot: \.shot_mods
        case .pin: \.pin_mods
        case .border: \.border_mods
        default: nil
        }
    }
}

/// 控件动作的闭包目标（NSControl 的 target 是弱引用，由窗口持有）。
private final class Action: NSObject {
    let run: (NSControl) -> Void
    init(_ run: @escaping (NSControl) -> Void) { self.run = run }
    @objc func fire(_ sender: NSControl) { run(sender) }
}

final class SettingsWindow: NSWindowController, NSWindowDelegate {
    private var s = AsSettings()
    private var actions: [Action] = []
    private var login: NSButton?
    private let onHotKeysChanged: () -> Void
    private let suspendHotKeys: () -> Void

    /// `onHotKeysChanged`：重新注册全局热键；`suspendHotKeys`：录制快捷键期间暂停全局热键。
    init(onHotKeysChanged: @escaping () -> Void, suspendHotKeys: @escaping () -> Void) {
        self.onHotKeysChanged = onHotKeysChanged
        self.suspendHotKeys = suspendHotKeys
        var loaded = AsSettings()
        airsnip_settings_load(&loaded)
        s = loaded
        let tabs = NSTabViewController()
        tabs.tabStyle = .toolbar
        let window = NSWindow(contentViewController: tabs)
        window.title = "AirSnip 设置"
        window.styleMask = [.titled, .closable]
        super.init(window: window)
        for (title, symbol, rows) in [
            ("通用", "gearshape", general()), ("快捷键", "keyboard", shortcuts()), ("保存", "square.and.arrow.down", save()),
            ("外观", "paintpalette", appearance()), ("关于", "info.circle", about()),
        ] {
            tabs.addTabViewItem(page(title, symbol, rows))
        }
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("不从 nib 创建") }

    /// 窗口每次到前台都重读开机自启：用户可能在系统设置里改过。
    func windowDidBecomeKey(_ notification: Notification) {
        login?.state = SMAppService.mainApp.status == .enabled ? .on : .off
    }

    /// 关窗口或切走时结束快捷键录制（经 resignFirstResponder），否则全局热键会一直暂停。
    func windowDidResignKey(_ notification: Notification) {
        window?.makeFirstResponder(nil)
    }

    // MARK: 页面

    private func general() -> [[NSView]] {
        let login = bind(NSButton(checkboxWithTitle: "登录 Mac 时自动启动", target: nil, action: nil)) { [weak self] box in
            self?.setAutoStart((box as! NSButton).state == .on, box as! NSButton)
        }
        self.login = login
        return [
            [label("开机自启"), login],
            [label("语言"), label("简体中文")],
            [label("贴图位置"), popup(["截图原位置", "屏幕中间"], \.pin_center)],
        ]
    }

    private func shortcuts() -> [[NSView]] {
        var rows: [[NSView]] = Shortcut.allCases.map { [label($0.name), field($0)] }
        rows.append([NSGridCell.emptyContentView, note("截图、贴图在任何地方都能用；不带修饰键会抢占其他程序的按键，建议加 ⌃ / ⌥ / ⌘。\n贴图边框在贴图窗口内生效；长截图与工具键在截图时生效。")])
        return rows
    }

    private func save() -> [[NSView]] {
        [
            [label("文件名"), popup(["日期 + 时间（airsnip-20261101-235959）", "时间戳（毫秒）", "仅日期（airsnip-20261101）"], \.name_style)],
            [NSGridCell.emptyContentView, note("另存为对话框预填的文件名。长截图与贴图右键「保存」直接存到「图片 › airsnip」。")],
        ]
    }

    private func appearance() -> [[NSView]] {
        [
            [label("主题"), popup(["跟随系统", "浅色", "深色"], \.theme)],
            [label("界面缩放"), popup(["自动", "100%", "125%", "150%", "175%", "200%"], \.ui_scale)],
            [NSGridCell.emptyContentView, note("作用于截图时的工具栏、手柄与字体列表；标注内容不缩放。")],
        ]
    }

    private func about() -> [[NSView]] {
        let title = NSTextField(labelWithString: "AirSnip 轻截")
        title.font = .boldSystemFont(ofSize: 15)
        var rows: [[NSView]] = [
            [NSGridCell.emptyContentView, title],
            [label("版本"), label("v\(String(cString: airsnip_version())) · GPL-3.0")],
            [label("原作者"), link("冰冻大西瓜（@bd-dxg）", "https://github.com/bd-dxg")],
            [label("项目地址"), link("github.com/cacdcaecawae/airsnip", "https://github.com/cacdcaecawae/airsnip")],
            [label("原版（Windows）"), link("github.com/bd-dxg/airsnip", "https://github.com/bd-dxg/airsnip")],
            [label("第三方许可"), link("查看清单", "https://github.com/cacdcaecawae/airsnip/blob/main/THIRD-PARTY.md")],
        ]
        if let url = Bundle.main.url(forResource: "reward-code", withExtension: "png"), let img = NSImage(contentsOf: url) {
            let view = NSImageView(image: img)
            view.imageScaling = .scaleProportionallyDown
            view.widthAnchor.constraint(equalToConstant: 160).isActive = true
            view.heightAnchor.constraint(equalToConstant: 160).isActive = true
            rows.append([label("支持项目"), view])
        }
        return rows
    }

    // MARK: 控件

    private func page(_ title: String, _ symbol: String, _ rows: [[NSView]]) -> NSTabViewItem {
        let grid = NSGridView(views: rows)
        grid.rowSpacing = 12
        grid.columnSpacing = 12
        grid.column(at: 0).xPlacement = .trailing
        grid.rowAlignment = .firstBaseline
        grid.translatesAutoresizingMaskIntoConstraints = false
        let view = NSView()
        view.addSubview(grid)
        NSLayoutConstraint.activate([
            grid.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            grid.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -20),
            grid.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 24),
            grid.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -24),
            view.widthAnchor.constraint(greaterThanOrEqualToConstant: 480),
        ])
        let vc = NSViewController()
        vc.view = view
        vc.title = title
        // 工具栏式标签页按各页的 preferredContentSize 调整窗口大小
        vc.preferredContentSize = view.fittingSize
        let item = NSTabViewItem(viewController: vc)
        item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: title)
        return item
    }

    private func label(_ text: String) -> NSTextField { NSTextField(labelWithString: text) }

    private func note(_ text: String) -> NSTextField {
        let f = NSTextField(wrappingLabelWithString: text)
        f.textColor = .secondaryLabelColor
        f.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        f.preferredMaxLayoutWidth = 360
        return f
    }

    private func link(_ title: String, _ url: String) -> NSButton {
        let b = bind(NSButton(title: title + " ›", target: nil, action: nil)) { _ in
            if let u = URL(string: url) { NSWorkspace.shared.open(u) }
        }
        b.isBordered = false
        b.contentTintColor = .linkColor
        return b
    }

    /// 下拉菜单：选中序号直接写入设置字段并保存。
    private func popup(_ items: [String], _ key: WritableKeyPath<AsSettings, Int32>) -> NSPopUpButton {
        let p = bind(NSPopUpButton(frame: .zero, pullsDown: false)) { [weak self] control in
            self?.s[keyPath: key] = Int32((control as! NSPopUpButton).indexOfSelectedItem)
            self?.persist()
        }
        p.addItems(withTitles: items)
        p.selectItem(at: Int(s[keyPath: key]))
        return p
    }

    private func field(_ k: Shortcut) -> ShortcutField {
        let f = ShortcutField(code: s[keyPath: k.code], mods: k.mods.map { s[keyPath: $0] } ?? 0, withMods: k.withMods)
        f.onBegin = suspendHotKeys
        f.onEnd = onHotKeysChanged
        f.onRecord = { [weak self] code, mods in
            guard let self else { return false }
            if let other = conflict(k, code, mods) {
                Alerts.show("「\(hotKeyLabel(keyCode: code, modifiers: mods))」已被「\(other)」使用，请换一个。")
                return false
            }
            s[keyPath: k.code] = code
            if let m = k.mods { s[keyPath: m] = mods }
            persist()
            return true
        }
        return f
    }

    @discardableResult
    private func bind<C: NSControl>(_ control: C, _ run: @escaping (NSControl) -> Void) -> C {
        let a = Action(run)
        actions.append(a)
        control.target = a
        control.action = #selector(Action.fire(_:))
        return control
    }

    // MARK: 行为

    /// 冲突检查（同 Windows）：与全局热键比「键 + 修饰」，与覆盖层内的键只比键码。
    private func conflict(_ k: Shortcut, _ code: UInt32, _ mods: UInt32) -> String? {
        Shortcut.allCases.first { o in
            guard o != k else { return false }
            let oc = s[keyPath: o.code]
            let om: UInt32 = o.mods.map { s[keyPath: $0] } ?? 0
            return o.withMods ? (oc == code && om == mods) : oc == code
        }?.name
    }

    private func persist() {
        if airsnip_settings_save(&s) != 0 {
            Alerts.show("保存设置失败：\(String(cString: airsnip_last_error()))")
        }
    }

    private func setAutoStart(_ on: Bool, _ box: NSButton) {
        do {
            try on ? SMAppService.mainApp.register() : SMAppService.mainApp.unregister()
        } catch {
            box.state = on ? .off : .on
            Alerts.show("\(on ? "开启" : "关闭")开机自启失败：\(error.localizedDescription)")
            return
        }
        if SMAppService.mainApp.status == .requiresApproval {
            Alerts.show("请在「系统设置 › 通用 › 登录项」中允许 AirSnip。")
            SMAppService.openSystemSettingsLoginItems()
        }
        s.auto_start = on ? 1 : 0
        persist()
    }
}
