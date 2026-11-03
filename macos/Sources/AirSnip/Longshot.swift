// SPDX-License-Identifier: GPL-3.0-only
// 长截图（LNG-1/3/6/7）：视口 = 截图时的选区。用户自己滚动页面，每 90ms 抓一帧视口交核心拼接；
// 视口外一圈 2px 蓝框（不挡鼠标）、视口下方（放不下则上方）状态条、旁边的拼接预览。
// Return 完成（复制 + 保存到默认目录）、⌘S 另存为（复制 + 存到所选位置）、Esc 取消、⌘Z 撤销：
// 会话期间注册为全局热键，焦点一直留在被滚动的应用。定时拍只在会话期间存在。

import AppKit
import CAirsnip
import ScreenCaptureKit

/// 长截图请求：视口所在显示器、视口屏幕矩形（points）与像素尺寸。
struct LongshotRequest {
    let display: DisplayInfo
    let viewport: NSRect
    let width: Int
    let height: Int
}

final class LongshotController {
    private let req: LongshotRequest
    private let ls: OpaquePointer
    private let onEnd: () -> Void
    private var filter: SCContentFilter?
    private let config = SCStreamConfiguration()
    /// 定时拍是否继续（完成时先停拍，再补抓最后一帧）。
    private var ticking = true
    private var inflight: Task<Void, Never>?
    private var ended = false
    private var keys: [HotKey] = []
    private let border: NSPanel
    private let bar: LongshotBar
    private let preview: LongshotPreview?
    private var panels: [NSPanel] { [border, bar] + (preview.map { [$0] } ?? []) }

    init?(_ req: LongshotRequest, onEnd: @escaping () -> Void) {
        guard let ls = airsnip_longshot_new(UInt32(req.width), UInt32(req.height)) else {
            Alerts.show("长截图初始化失败：\(String(cString: airsnip_last_error()))")
            return nil
        }
        self.req = req
        self.ls = ls
        self.onEnd = onEnd
        let vp = req.viewport
        let screen = req.display.frame
        border = LongshotController.borderPanel(around: vp)
        // 状态条与预览放在屏幕可用区域内（避开菜单栏与 Dock：Dock 层级高于浮动窗口）
        let area = NSScreen.screens.first { $0.frame == screen }?.visibleFrame ?? screen
        bar = LongshotBar(viewport: vp, area: area)
        preview = LongshotPreview(viewport: vp, area: area, scale: req.display.scale)
        // 抓取范围：显示器内的 points 坐标（左上原点），输出按视口像素
        config.sourceRect = CGRect(
            x: vp.minX - screen.minX, y: screen.maxY - vp.maxY, width: vp.width, height: vp.height)
        config.width = req.width
        config.height = req.height
        config.showsCursor = false
        config.captureResolution = .best

        bar.onUndo = { [weak self] in self?.undo() }
        bar.onSaveAs = { [weak self] in self?.saveAs() }
        bar.onDone = { [weak self] in self?.finish() }
        bar.onCancel = { [weak self] in self?.end() }
        panels.forEach { $0.orderFrontRegardless() }
        registerKeys()
        Task { @MainActor in await start() }
    }

    /// 会话热键（另存为对话框开着时注销，Return / Esc 留给对话框）。
    private func registerKeys() {
        let hot: [(UInt32, UInt32, () -> Void)] = [
            (0x24, 0, { [weak self] in self?.finish() }),  // Return
            (0x35, 0, { [weak self] in self?.end() }),  // Esc
            (0x06, 0x100, { [weak self] in self?.undo() }),  // ⌘Z
            (0x01, 0x100, { [weak self] in self?.saveAs() }),  // ⌘S
        ]
        for (i, (key, mods, action)) in hot.enumerated() {
            if let k = try? HotKey(keyCode: key, modifiers: mods, id: 10 + UInt32(i), handler: action) { keys.append(k) }
        }
    }

    /// 取抓屏范围（排除本应用窗口：状态条、边框、预览、贴图），然后每 90ms 一拍直到完成或取消。
    @MainActor private func start() async {
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            let pid = ProcessInfo.processInfo.processIdentifier
            guard let display = content.displays.first(where: { $0.displayID == req.display.displayID }) else {
                throw NSError(domain: "AirSnip", code: 1, userInfo: [NSLocalizedDescriptionKey: "找不到视口所在的显示器"])
            }
            filter = SCContentFilter(
                display: display, excludingApplications: content.applications.filter { $0.processID == pid },
                exceptingWindows: [])
        } catch {
            fail("抓屏失败：\(error.localizedDescription)")
            return
        }
        guard !ended else { return }  // 准备期间已按 Esc
        await run()
    }

    @MainActor private func run() async {
        while ticking, !ended {
            tick()
            try? await Task.sleep(nanoseconds: 90_000_000)
        }
    }

    /// 定时拍：上一帧还没处理完就跳过。
    private func tick() {
        guard inflight == nil, !ended else { return }
        inflight = Task { @MainActor in
            await captureOnce()
            inflight = nil
        }
    }

    @MainActor private func captureOnce() async {
        guard let filter, !ended else { return }
        let image: CGImage
        do {
            image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
        } catch {
            fail("抓屏失败：\(error.localizedDescription)")
            return
        }
        guard !ended, let rgba = ScreenCapture.rgbaPixels(image, width: req.width, height: req.height) else { return }
        let code = rgba.withUnsafeBufferPointer { buf -> Int32 in
            var img = AsImage(
                data: buf.baseAddress, width: UInt32(req.width), height: UInt32(req.height),
                stride: UInt32(req.width * 4))
            return airsnip_longshot_push(ls, &img)
        }
        if code < 0 {
            fail("长截图失败：\(String(cString: airsnip_last_error()))")
            return
        }
        refresh()
        // 已达高度上限：拼接中直接输出；完成 / 另存为的补抓帧由调用方收尾（另存为仍要弹对话框）
        if code == 1, ticking { complete() }
    }

    private func refresh() {
        bar.update(hint: String(cString: airsnip_longshot_hint(ls)), canUndo: airsnip_longshot_can_undo(ls) != 0)
        preview?.update(ls)
    }

    private func undo() {
        guard !ended else { return }
        airsnip_longshot_undo(ls)
        refresh()
    }

    /// 完成：等在途的一帧，再补抓一帧（滚动后立刻按 Return 时最后一屏还没拍到），然后输出。
    private func finish() {
        guard !ended, filter != nil, ticking else { return }
        ticking = false
        let pending = inflight
        Task { @MainActor in
            await pending?.value
            await captureOnce()
            complete()
        }
    }

    /// 另存为（OUT-2）：同完成先停拍并补抓最后一帧，再弹另存为对话框（格式、命名模板、上次的目录）；
    /// 选好位置就复制并存到那里，取消则恢复热键继续拼接（已拼的内容还在）。
    private func saveAs() {
        guard !ended, filter != nil, ticking else { return }
        ticking = false
        keys.removeAll()
        let pending = inflight
        Task { @MainActor in
            await pending?.value
            await captureOnce()
            guard !ended else { return }  // 补抓时失败或已达高度上限
            var name: UnsafePointer<CChar>?
            var ext: UnsafePointer<CChar>?
            var dir: UnsafePointer<CChar>?
            airsnip_longshot_save_defaults(ls, &name, &ext, &dir)
            let url = SaveDialog.run(
                dir: dir.map { String(cString: $0) }, name: name.map { String(cString: $0) } ?? "airsnip.png",
                ext: ext.map { String(cString: $0) } ?? "png")
            guard !ended else { return }
            NSApp.hide(nil)  // 对话框激活了本应用：把焦点还给被滚动的应用
            if let url {
                complete(path: url.path)
            } else {
                registerKeys()
                ticking = true
                await run()
            }
        }
    }

    /// 输出：复制到剪贴板 + 保存（`path` 为 nil 时 PNG 存默认目录，与 Windows 一致）。
    private func complete(path: String? = nil) {
        guard !ended else { return }
        var png: UnsafePointer<UInt8>?
        var len = 0
        let scale = Float(req.display.scale)
        let code =
            path.map { $0.withCString { airsnip_longshot_finish(ls, scale, $0, &png, &len) } }
            ?? airsnip_longshot_finish(ls, scale, nil, &png, &len)
        if let png { Clipboard.writePNG(Data(bytes: png, count: len)) }
        let err = code != 0 ? String(cString: airsnip_last_error()) : nil
        end()
        if let err { Alerts.show(err) }
    }

    private func fail(_ message: String) {
        guard !ended else { return }
        end()
        Alerts.show(message)
    }

    /// 收尾：停拍、注销热键、关窗口、释放会话。
    private func end() {
        guard !ended else { return }
        ended = true
        ticking = false
        keys.removeAll()
        panels.forEach { $0.orderOut(nil) }
        let ls = self.ls
        // 在途的抓帧回来时看到 ended 直接返回；等它结束再释放会话
        let pending = inflight
        Task { @MainActor in
            await pending?.value
            airsnip_longshot_free(ls)
        }
        onEnd()
    }

    /// 视口外 2px 蓝框（不挡鼠标，滚轮直达目标窗口）。
    private static func borderPanel(around vp: NSRect) -> NSPanel {
        let edge: CGFloat = 2
        let panel = FloatingPanel(frame: vp.insetBy(dx: -edge, dy: -edge))
        panel.ignoresMouseEvents = true
        let view = NSView()
        view.wantsLayer = true
        view.layer?.borderWidth = edge
        view.layer?.borderColor = NSColor(srgbRed: 26 / 255, green: 115 / 255, blue: 232 / 255, alpha: 1).cgColor
        panel.contentView = view
        return panel
    }
}

/// 长截图期间的辅助窗口：透明、浮动、不激活应用；应用隐藏（把焦点还给被滚动的应用）时也保持显示。
class FloatingPanel: NSPanel {
    init(frame: NSRect) {
        super.init(contentRect: frame, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        hidesOnDeactivate = false
        canHide = false
        isReleasedWhenClosed = false
    }
}
