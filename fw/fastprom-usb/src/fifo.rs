//! A byte ring buffer, for the classes' transmit and receive queues.

/// `N` bytes, first in first out.
#[derive(Debug, Clone)]
pub struct Fifo<const N: usize> {
    buf: [u8; N],
    head: usize,
    len: usize,
}

impl<const N: usize> Default for Fifo<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Fifo<N> {
    /// Empty.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            head: 0,
            len: 0,
        }
    }

    /// Bytes held.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing is held.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Room left.
    pub const fn room(&self) -> usize {
        N - self.len
    }

    /// Drop everything.
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Append as much of `data` as fits; how much did.
    pub fn push(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.room());
        for (k, &b) in data.iter().take(n).enumerate() {
            let at = (self.head + self.len + k) % N.max(1);
            if let Some(slot) = self.buf.get_mut(at) {
                *slot = b;
            }
        }
        self.len += n;
        n
    }

    /// Copy the oldest bytes into `out` without removing them; how many.
    pub fn peek(&self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        for (k, o) in out.iter_mut().take(n).enumerate() {
            let at = (self.head + k) % N.max(1);
            *o = self.buf.get(at).copied().unwrap_or(0);
        }
        n
    }

    /// Remove the oldest `n` bytes (at most what is held).
    pub fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        self.head = (self.head + n) % N.max(1);
        self.len -= n;
    }

    /// Move the oldest bytes into `out`; how many.
    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = self.peek(out);
        self.consume(n);
        n
    }
}
