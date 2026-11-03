// SPDX-License-Identifier: GPL-3.0-only
// 覆盖层文字输入（EDT-4）：实现 NSTextInputClient 接系统输入法（含中文拼音）。
// 组合串经 setMarkedText、上屏文字经 insertText 送核心（核心自绘文字与组合串下划线）；
// 输入法不处理的按键（回车、退格、方向、Esc）经 doCommand 交回核心的按键分发（见 OverlayView）。

import AppKit
import CAirsnip

extension OverlayView: NSTextInputClient {
    private static let noRange = NSRange(location: NSNotFound, length: 0)

    func insertText(_ string: Any, replacementRange: NSRange) {
        guard imeEnabled, let ov = overlay else { return }
        marked = ""
        airsnip_overlay_text(ov, OverlayView.plain(string), 1)
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard imeEnabled, let ov = overlay else { return }
        marked = OverlayView.plain(string)
        markedSelection = selectedRange
        airsnip_overlay_text(ov, marked, 0)
    }

    /// 输入法要求按当前样子确认组合串。
    func unmarkText() {
        guard !marked.isEmpty else { return }
        insertText(marked, replacementRange: OverlayView.noRange)
    }

    func hasMarkedText() -> Bool { !marked.isEmpty }

    func markedRange() -> NSRange {
        marked.isEmpty ? OverlayView.noRange : NSRange(location: 0, length: marked.utf16.count)
    }

    func selectedRange() -> NSRange { marked.isEmpty ? NSRange(location: 0, length: 0) : markedSelection }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        nil
    }

    func characterIndex(for point: NSPoint) -> Int { NSNotFound }

    /// 候选框位置：光标竖线（屏幕坐标）。
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let ov = overlay, let window else { return .zero }
        var caret = [Int32](repeating: 0, count: 3)
        guard airsnip_overlay_caret(ov, &caret) != 0 else { return .zero }
        let r = viewRect(x: Int(caret[0]), y: Int(caret[1]), w: 1, h: Int(caret[2]))
        return window.convertToScreen(convert(r, to: nil))
    }

    /// 覆盖层在屏蔽窗口层级：告诉输入法，候选框要放在它之上。
    func windowLevel() -> Int { window?.level.rawValue ?? 0 }

    private static func plain(_ string: Any) -> String {
        (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
    }
}
