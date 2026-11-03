// SPDX-License-Identifier: GPL-3.0-only
//! SVG path 词法：命令字母与数字（支持 `-`/`+` 前缀、小数点、指数、逗号与空白分隔）。

pub(super) struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(src: &'a str) -> Self {
        Self {
            s: src.as_bytes(),
            i: 0,
        }
    }

    pub(super) fn at_end(&mut self) -> bool {
        self.skip_sep();
        self.i >= self.s.len()
    }

    fn skip_sep(&mut self) {
        while matches!(self.s.get(self.i), Some(b' ' | b',' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    /// 下一个字符若是命令字母则消费并返回。
    pub(super) fn take_cmd(&mut self) -> Option<u8> {
        self.skip_sep();
        let c = self.s.get(self.i).copied().filter(u8::is_ascii_alphabetic)?;
        self.i += 1;
        Some(c)
    }

    /// 不消费地窥视下一个字符是否为命令字母。
    pub(super) fn peek_cmd(&mut self) -> bool {
        self.skip_sep();
        self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic)
    }

    /// 读一个数字；不是数字时位置不变并返回 None。
    pub(super) fn num(&mut self) -> Option<f32> {
        self.skip_sep();
        let start = self.i;
        if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
            self.i += 1;
        }
        let mut digits = false;
        while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
            digits = true;
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
                digits = true;
            }
        }
        if !digits {
            self.i = start;
            return None;
        }
        // 指数部分格式不完整时回退，避免把 `1e` 当成合法数字
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'-' | b'+')) {
                self.i += 1;
            }
            let mut exp = false;
            while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                self.i += 1;
                exp = true;
            }
            if !exp {
                self.i = save;
            }
        }
        std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }
}
