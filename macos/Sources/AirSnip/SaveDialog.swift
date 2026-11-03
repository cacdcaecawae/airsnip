// SPDX-License-Identifier: GPL-3.0-only
// 另存为对话框（OUT-2）：格式下拉（PNG / JPEG / WebP 无损，同 Windows）、预填文件名、初始目录，
// 盖在覆盖层之上。选定的格式与目录由核心记录，释放覆盖层时写回配置（airsnip_overlay_free）。

import AppKit
import UniformTypeIdentifiers

/// 格式下拉的目标：切换时改写允许类型，文件名的扩展名随之更新。
private final class SaveFormats: NSObject {
    static let all: [(title: String, type: UTType)] = [
        ("PNG 图片", .png), ("JPEG 图片", .jpeg), ("WebP 图片（无损）", .webP),
    ]
    weak var panel: NSSavePanel?

    @objc func changed(_ sender: NSPopUpButton) {
        panel?.allowedContentTypes = [SaveFormats.all[sender.indexOfSelectedItem].type]
    }

    /// 扩展名 → 下拉序号，未知按 PNG。
    static func index(ofExt ext: String) -> Int {
        switch ext.lowercased() {
        case "jpg", "jpeg": return 1
        case "webp": return 2
        default: return 0
        }
    }
}

enum SaveDialog {
    /// 返回选定路径，取消返回 nil。`name` 含扩展名，`ext` 为默认格式（上次保存的格式，首次 PNG）。
    static func run(dir: String?, name: String, ext: String) -> URL? {
        let panel = NSSavePanel()
        panel.title = "保存截图"
        panel.nameFieldStringValue = name
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        if let dir { panel.directoryURL = URL(fileURLWithPath: dir, isDirectory: true) }

        let formats = SaveFormats()
        formats.panel = panel
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        popup.addItems(withTitles: SaveFormats.all.map(\.title))
        popup.selectItem(at: SaveFormats.index(ofExt: ext))
        popup.target = formats
        popup.action = #selector(SaveFormats.changed(_:))
        formats.changed(popup)
        let row = NSStackView(views: [NSTextField(labelWithString: "格式："), popup])
        row.edgeInsets = NSEdgeInsets(top: 8, left: 16, bottom: 8, right: 16)
        row.frame.size = row.fittingSize
        panel.accessoryView = row

        let ok = Alerts.runAbove(panel) { panel.runModal() } == .OK
        // 下拉的 target 是弱引用，对话框关闭前要保证 formats 还活着
        return withExtendedLifetime(formats) { ok ? panel.url : nil }
    }
}
