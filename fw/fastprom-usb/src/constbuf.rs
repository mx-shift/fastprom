//! A byte writer for descriptors built at compile time.
//!
//! Every use is in a `const` or `static` initialiser, so the length check in
//! [`Out::finish`] is a build error, never a run-time panic.

/// `N` bytes, written in order.
pub struct Out<const N: usize> {
    buf: [u8; N],
    at: usize,
}

impl<const N: usize> Out<N> {
    /// Empty.
    pub const fn new() -> Self {
        Self { buf: [0; N], at: 0 }
    }

    /// One byte.
    pub const fn byte(mut self, v: u8) -> Self {
        if self.at < N {
            self.buf[self.at] = v;
        }
        self.at += 1;
        self
    }

    /// A little-endian u16.
    pub const fn u16(self, v: u16) -> Self {
        self.byte(v as u8).byte((v >> 8) as u8)
    }

    /// A little-endian u32.
    pub const fn u32(self, v: u32) -> Self {
        self.u16(v as u16).u16((v >> 16) as u16)
    }

    /// Bytes as they are.
    pub const fn bytes(mut self, b: &[u8]) -> Self {
        let mut i = 0;
        while i < b.len() {
            self = self.byte(b[i]);
            i += 1;
        }
        self
    }

    /// ASCII as UTF-16LE, without a terminator.
    pub const fn utf16(mut self, s: &str) -> Self {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            self = self.u16(b[i] as u16);
            i += 1;
        }
        self
    }

    /// The bytes; refuses (at compile time) any length but `N`.
    pub const fn finish(self) -> [u8; N] {
        assert!(self.at == N, "descriptor length does not match its array");
        self.buf
    }
}
