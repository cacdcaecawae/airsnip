// SPDX-License-Identifier: GPL-3.0-only
//! Core Text / Core Graphics / Core Foundation 的最小 C 接口声明（macOS 文本后端用）。
//! 只声明 `coretext.rs` 实际调用的函数；系统框架自带，不引入第三方 crate。
#![allow(non_upper_case_globals, non_snake_case)]

use std::ffi::{c_char, c_void};

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFIndex = isize;
pub type CGContextRef = *mut c_void;
pub type CGColorSpaceRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CGAffineTransform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

/// `CFDictionaryCreate` 的回调表（只取地址，内容不透明）。
#[repr(C)]
pub struct CFDictionaryCallBacks {
    _opaque: [u8; 0],
}

pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kCFNumberDoubleType: CFIndex = 13;
pub const kCTFontItalicTrait: u32 = 1 << 0;
pub const kCTFontBoldTrait: u32 = 1 << 1;
pub const kCGImageAlphaNone: u32 = 0;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub static kCFTypeDictionaryKeyCallBacks: CFDictionaryCallBacks;
    pub static kCFTypeDictionaryValueCallBacks: CFDictionaryCallBacks;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFStringCreateWithBytes(
        alloc: CFTypeRef,
        bytes: *const u8,
        len: CFIndex,
        encoding: u32,
        is_external: u8,
    ) -> CFStringRef;
    pub fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    pub fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    pub fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    pub fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: CFIndex,
        key_cb: *const CFDictionaryCallBacks,
        value_cb: *const CFDictionaryCallBacks,
    ) -> CFTypeRef;
    pub fn CFAttributedStringCreate(alloc: CFTypeRef, s: CFStringRef, attrs: CFTypeRef) -> CFTypeRef;
    pub fn CFNumberCreate(alloc: CFTypeRef, kind: CFIndex, value: *const c_void) -> CFTypeRef;
    pub fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
    pub fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    pub static kCTFontAttributeName: CFStringRef;
    pub static kCTStrokeWidthAttributeName: CFStringRef;
    pub static kCTFontFamilyNameAttribute: CFStringRef;
    pub fn CTFontDescriptorCreateWithAttributes(attrs: CFTypeRef) -> CFTypeRef;
    pub fn CTFontCreateWithFontDescriptor(desc: CFTypeRef, size: f64, matrix: *const CGAffineTransform) -> CFTypeRef;
    pub fn CTFontCreateCopyWithSymbolicTraits(
        font: CFTypeRef,
        size: f64,
        matrix: *const CGAffineTransform,
        value: u32,
        mask: u32,
    ) -> CFTypeRef;
    pub fn CTFontCreateCopyWithAttributes(
        font: CFTypeRef,
        size: f64,
        matrix: *const CGAffineTransform,
        attrs: CFTypeRef,
    ) -> CFTypeRef;
    pub fn CTFontCopyFamilyName(font: CFTypeRef) -> CFStringRef;
    pub fn CTFontGetAscent(font: CFTypeRef) -> f64;
    pub fn CTFontGetDescent(font: CFTypeRef) -> f64;
    pub fn CTFontGetLeading(font: CFTypeRef) -> f64;
    pub fn CTLineCreateWithAttributedString(s: CFTypeRef) -> CFTypeRef;
    pub fn CTLineGetTypographicBounds(line: CFTypeRef, ascent: *mut f64, descent: *mut f64, leading: *mut f64) -> f64;
    pub fn CTLineGetTrailingWhitespaceWidth(line: CFTypeRef) -> f64;
    pub fn CTLineDraw(line: CFTypeRef, ctx: CGContextRef);
    pub fn CTFontManagerCopyAvailableFontFamilyNames() -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub fn CGColorSpaceCreateDeviceGray() -> CGColorSpaceRef;
    pub fn CGColorSpaceRelease(cs: CGColorSpaceRef);
    pub fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        info: u32,
    ) -> CGContextRef;
    pub fn CGContextRelease(ctx: CGContextRef);
    pub fn CGContextSetShouldSmoothFonts(ctx: CGContextRef, b: bool);
    pub fn CGContextSetTextMatrix(ctx: CGContextRef, t: CGAffineTransform);
    pub fn CGContextSetTextPosition(ctx: CGContextRef, x: f64, y: f64);
}
