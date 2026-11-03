// SPDX-License-Identifier: GPL-3.0-only
// 宿主回调表（AsHost）：Rust 覆盖层需要的平台能力 → Swift。
// C 函数指针不能捕获上下文，覆盖层视图经 ctx（未持有引用）传回；视图寿命长于覆盖层。

import AppKit
import CAirsnip

enum HostBridge {
    static func make(for view: OverlayView) -> AsHost {
        AsHost(
            ctx: Unmanaged.passUnretained(view).toOpaque(),
            present: { ctx, x, y, w, h in HostBridge.view(ctx)?.present(x: x, y: y, w: w, h: h) },
            set_cursor: { ctx, shape in HostBridge.view(ctx)?.setShape(shape) },
            close: { ctx in HostBridge.view(ctx)?.requestClose() },
            alert: { _, msg in
                if let msg { Alerts.show(String(cString: msg)) }
            },
            pin: { ctx, img, x, y in
                guard let view = HostBridge.view(ctx), let img, let image = CGImage.copyRGBA(img.pointee) else {
                    return 0
                }
                return view.pin(image, x: x, y: y) ? 1 : 0
            },
            pick_save_file: { _, dir, name, ext, out, len in
                HostBridge.pickSaveFile(dir: dir, name: name, ext: ext, out: out, len: len)
            },
            set_ime: { ctx, on in HostBridge.view(ctx)?.imeEnabled = on != 0 },
            clipboard_png: { png, len, _, _ in
                guard let png else { return }
                Clipboard.writePNG(Data(bytes: png, count: len))
            },
            notify: { msg in
                guard let msg else { return }
                let text = String(cString: msg)
                DispatchQueue.main.async { Alerts.show(text) }
            })
    }

    private static func view(_ ctx: UnsafeMutableRawPointer?) -> OverlayView? {
        ctx.map { Unmanaged<OverlayView>.fromOpaque($0).takeUnretainedValue() }
    }
}

extension HostBridge {
    /// 另存为（OUT-2，见 SaveDialog）：选定路径写入 `out`（NUL 结尾）。
    static func pickSaveFile(
        dir: UnsafePointer<CChar>?, name: UnsafePointer<CChar>?, ext: UnsafePointer<CChar>?,
        out: UnsafeMutablePointer<CChar>?, len: Int
    ) -> Int32 {
        guard let out, let name, len > 0,
            let url = SaveDialog.run(
                dir: dir.map { String(cString: $0) }, name: String(cString: name),
                ext: ext.map { String(cString: $0) } ?? "png")
        else { return 0 }
        let bytes = Array(url.path.utf8)
        guard bytes.count < len else {
            Alerts.show("保存路径过长：\(url.path)")
            return 0
        }
        bytes.withUnsafeBufferPointer { src in
            out.withMemoryRebound(to: UInt8.self, capacity: len) { dst in
                dst.update(from: src.baseAddress!, count: bytes.count)
                dst[bytes.count] = 0
            }
        }
        return 1
    }
}

extension OverlayView {
    /// 截图贴图（PIN-1）：与屏幕上一样大；按设置放在选区原位置或屏幕中央。
    func pin(_ image: CGImage, x: Int32, y: Int32) -> Bool {
        let size = NSSize(width: CGFloat(image.width) / scale, height: CGFloat(image.height) / scale)
        var origin: NSPoint?
        if Pins.config.center == 0, let window {
            origin = window.convertToScreen(viewRect(x: Int(x), y: Int(y), w: image.width, h: image.height)).origin
        }
        return Pins.show(image, size: size, at: origin)
    }
}

enum Clipboard {
    /// 写剪贴板（OUT-1）：PNG + TIFF 两种表示（部分应用只认 TIFF）。数据已复制，可从任意线程调用。
    static func writePNG(_ png: Data) {
        DispatchQueue.main.async {
            let item = NSPasteboardItem()
            item.setData(png, forType: .png)
            if let tiff = NSBitmapImageRep(data: png)?.tiffRepresentation {
                item.setData(tiff, forType: .tiff)
            }
            publish(item)
        }
    }

    /// 贴图复制（PIN-4）：PNG + TIFF，带显示大小（Retina 截图的分辨率信息）。
    static func write(_ image: CGImage, size: NSSize) {
        let rep = NSBitmapImageRep(cgImage: image)
        rep.size = size
        let item = NSPasteboardItem()
        if let png = rep.representation(using: .png, properties: [:]) { item.setData(png, forType: .png) }
        if let tiff = rep.tiffRepresentation { item.setData(tiff, forType: .tiff) }
        publish(item)
    }

    private static func publish(_ item: NSPasteboardItem) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.writeObjects([item])
    }
}

enum Alerts {
    /// 高于覆盖层（屏蔽窗口层级）一级，提示框与对话框不会被覆盖层挡住。
    static let level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()) + 1)

    static func show(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "AirSnip"
        alert.informativeText = message
        _ = runAbove(alert.window) { alert.runModal() }
    }

    /// 以模态方式显示 `window`，并保证它在覆盖层之上。
    /// runModal 会把窗口层级重置为模态面板级（8），事先设的 level 无效：窗口被压在覆盖层下，
    /// 看不见也点不到，模态循环又挡住覆盖层的事件，表现为卡死。所以进入模态循环后再抬层级。
    static func runAbove<T>(_ window: NSWindow, _ run: () -> T) -> T {
        RunLoop.main.perform(inModes: [.modalPanel]) { window.level = level }
        NSApp.activate(ignoringOtherApps: true)
        return run()
    }
}
