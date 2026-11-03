// SPDX-License-Identifier: GPL-3.0-only
// Swift ↔ Rust 核心桥接自检（无 GUI）：版本号、图像编码写文件、错误码与错误说明。

import CAirsnip
import Foundation

enum SelfTest {
    static func run() -> Bool {
        print("AirSnip core \(String(cString: airsnip_version()))")
        // 2×1 RGBA8 预乘：不透明红 + 半透明绿
        let px: [UInt8] = [255, 0, 0, 255, 0, 128, 0, 128]
        let dir = FileManager.default.temporaryDirectory
        func save(_ name: String) -> Int32 {
            px.withUnsafeBufferPointer { buf in
                var img = AsImage(data: buf.baseAddress, width: 2, height: 1, stride: 8)
                return airsnip_save_image(&img, dir.appendingPathComponent(name).path)
            }
        }
        func lastError() -> String { String(cString: airsnip_last_error()) }
        guard save("airsnip-selftest.png") == AIRSNIP_OK else {
            print("FAIL 保存 PNG：\(lastError())")
            return false
        }
        let png = try? Data(contentsOf: dir.appendingPathComponent("airsnip-selftest.png"))
        guard let png, png.count > 24, png[1...3].elementsEqual("PNG".utf8) else {
            print("FAIL PNG 文件内容不对")
            return false
        }
        guard save("airsnip-selftest.bmp") == AIRSNIP_ERR_FAILED, lastError().contains("不支持") else {
            print("FAIL 不支持的扩展名应返回错误码与说明：\(lastError())")
            return false
        }
        print("selftest ok（\(png.count) 字节 PNG；错误路径：\(lastError())）")
        return true
    }
}
