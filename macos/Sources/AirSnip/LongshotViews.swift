// SPDX-License-Identifier: GPL-3.0-only
// 长截图状态条与预览（LNG-3/6/7）：都放在屏幕可用区域内。状态条在视口下方（放不下则上方），[提示 …… 撤销 完成 取消]；
// 预览在视口右侧（放不下则左侧）显示拼接结果缩略图，底边与视口底边对齐，背景透明，不挡鼠标。

import AppKit
import CAirsnip

final class LongshotBar: FloatingPanel {
    var onUndo: (() -> Void)?
    var onDone: (() -> Void)?
    var onCancel: (() -> Void)?
    private let label = NSTextField(labelWithString: "")
    private var undo: NSButton!
    private static let defaultHint = "滚动页面 · Return 完成"

    /// `area`：屏幕可用区域（不含菜单栏与 Dock）。下方优先，其次上方；
    /// 上下都放不下（视口几乎占满屏幕）时放进视口底部，状态条不会被拍进去（抓帧排除本应用）。
    init(viewport vp: NSRect, area: NSRect) {
        let (h, margin): (CGFloat, CGFloat) = (36, 6)
        let w = min(max(vp.width, 360), 560)
        let x = min(max(vp.midX - w / 2, area.minX), area.maxX - w)
        let (below, above) = (vp.minY - margin - h, vp.maxY + margin)
        let y = below >= area.minY ? below : above + h <= area.maxY ? above : vp.minY + margin
        super.init(frame: NSRect(x: x, y: y, width: w, height: h))

        let bg = NSVisualEffectView()
        bg.material = .hudWindow
        bg.blendingMode = .behindWindow
        bg.state = .active
        bg.wantsLayer = true
        bg.layer?.cornerRadius = 8
        bg.layer?.masksToBounds = true

        label.stringValue = LongshotBar.defaultHint
        label.font = .systemFont(ofSize: 13)
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let buttons = [
            ("arrow.uturn.backward", "撤销最后一段（⌘Z）"), ("checkmark", "完成（Return）"), ("xmark", "取消（Esc）"),
        ].enumerated().map { i, item -> NSButton in
            let b = NSButton(
                image: NSImage(systemSymbolName: item.0, accessibilityDescription: item.1) ?? NSImage(),
                target: self, action: #selector(tapped(_:)))
            b.tag = i
            b.isBordered = false
            b.toolTip = item.1
            return b
        }
        undo = buttons[0]
        undo.isEnabled = false
        let row = NSStackView(views: [label] + buttons)
        row.spacing = 10
        row.edgeInsets = NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 10)
        row.frame = NSRect(x: 0, y: 0, width: w, height: h)
        row.autoresizingMask = [.width, .height]
        bg.addSubview(row)
        contentView = bg
    }

    func update(hint: String, canUndo: Bool) {
        label.stringValue = hint.isEmpty ? LongshotBar.defaultHint : hint
        undo.isEnabled = canUndo
    }

    @objc private func tapped(_ sender: NSButton) {
        [onUndo, onDone, onCancel][sender.tag]?()
    }
}

final class LongshotPreview: FloatingPanel {
    private let image = NSImageView()
    private let scale: CGFloat

    /// 视口两侧都放不下（剩余宽度 < 120pt）时返回 nil，长截图照常进行。
    init?(viewport vp: NSRect, area screen: NSRect, scale: CGFloat) {
        let margin: CGFloat = 10
        let right = screen.maxX - vp.maxX - margin * 2
        let left = vp.minX - screen.minX - margin * 2
        let w = min(max(right, left), 240)
        guard w >= 120 else { return nil }
        let h = min(min(max(vp.height, 120), 560), screen.height - margin * 2)
        let x = right >= left ? vp.maxX + margin : vp.minX - margin - w
        let y = max(min(vp.minY, screen.maxY - margin - h), screen.minY + margin)
        self.scale = scale
        super.init(frame: NSRect(x: x, y: y, width: w, height: h))
        ignoresMouseEvents = true
        image.imageScaling = .scaleProportionallyDown
        image.imageAlignment = right >= left ? .alignBottomLeft : .alignBottomRight
        contentView = image
    }

    /// 刷新缩略图（按预览窗像素大小向核心要，成本与拼接高度无关）。
    func update(_ ls: OpaquePointer) {
        let size = frame.size
        let thumb = airsnip_longshot_thumbnail(ls, UInt32(size.width * scale), UInt32(size.height * scale))
        guard thumb.width > 0, let cg = CGImage.copyRGBA(thumb) else { return }
        image.image = NSImage(
            cgImage: cg, size: NSSize(width: CGFloat(thumb.width) / scale, height: CGFloat(thumb.height) / scale))
    }
}
