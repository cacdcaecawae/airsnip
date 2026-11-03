// swift-tools-version:5.9
// SPDX-License-Identifier: GPL-3.0-only
// AirSnip macOS 外壳（AppKit）。Rust 核心静态库需先构建：
//   cargo build -p airsnip-macos --profile release-macos
// 再在本目录 `swift build`。Xcode 可直接打开本 Package.swift。

import Foundation
import PackageDescription

/// Rust 静态库所在目录（release-macos 配置的产物）。
let rustLibDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("../target/release-macos")
    .standardized.path

let package = Package(
    name: "AirSnip",
    platforms: [.macOS(.v14)],
    targets: [
        // C 接口：crates/airsnip-macos/include/airsnip.h
        .systemLibrary(name: "CAirsnip", path: "Sources/CAirsnip"),
        .executableTarget(
            name: "AirSnip",
            dependencies: ["CAirsnip"],
            linkerSettings: [
                .unsafeFlags(["-L", rustLibDir]),
                // 静态库不携带链接指令：Rust 核心用到的系统框架需在此显式链接
                .linkedFramework("CoreFoundation"),
                .linkedFramework("CoreText"),
                .linkedFramework("CoreGraphics"),
            ]
        ),
    ]
)
