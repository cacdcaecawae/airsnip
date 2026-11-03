// SPDX-License-Identifier: GPL-3.0-only
// 快捷键录制按钮（SYS-3）：点一下进入录制，按下新按键（Esc 取消）后交给 `onRecord` 校验。
// `withMods` 为 false 时只记键码（覆盖层内的长截图键、工具键不带修饰键）。

import AppKit

final class ShortcutField: NSButton {
    private(set) var code: UInt32
    private(set) var mods: UInt32
    private let withMods: Bool
    /// 校验新键位：返回 false 表示拒绝（如冲突，由回调负责提示）。
    var onRecord: ((UInt32, UInt32) -> Bool)?
    /// 录制开始 / 结束（录制期间暂停全局热键，否则按到现有热键会直接触发截图）。
    var onBegin: (() -> Void)?
    var onEnd: (() -> Void)?
    private var recording = false {
        didSet { refresh() }
    }

    init(code: UInt32, mods: UInt32, withMods: Bool) {
        self.code = code
        self.mods = mods
        self.withMods = withMods
        super.init(frame: .zero)
        bezelStyle = .rounded
        target = self
        action = #selector(begin)
        widthAnchor.constraint(greaterThanOrEqualToConstant: 180).isActive = true
        refresh()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("不从 nib 创建") }

    override var acceptsFirstResponder: Bool { true }

    @objc private func begin() {
        guard !recording else { return }
        recording = true
        window?.makeFirstResponder(self)
        onBegin?()
    }

    override func keyDown(with event: NSEvent) {
        recording ? finish(event) : super.keyDown(with: event)
    }

    /// ⌘ 组合先走按键等价通道，录制中一并截下。
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard recording, event.type == .keyDown else { return super.performKeyEquivalent(with: event) }
        finish(event)
        return true
    }

    override func resignFirstResponder() -> Bool {
        if recording {
            recording = false
            onEnd?()
        }
        return super.resignFirstResponder()
    }

    private func finish(_ event: NSEvent) {
        if event.keyCode != 53 {  // Esc：取消录制
            let (c, m) = (UInt32(event.keyCode), withMods ? carbonMods(event.modifierFlags) : 0)
            if onRecord?(c, m) ?? true { (code, mods) = (c, m) }
        }
        recording = false
        onEnd?()
    }

    private func refresh() {
        title = recording ? "按下新按键…（Esc 取消）" : hotKeyLabel(keyCode: code, modifiers: mods)
    }
}
