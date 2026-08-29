//! Big-endian buffer helpers.

use byteorder::{BigEndian, ByteOrder};

pub struct BeSlice<'a> {
    inner: &'a [u8],
    pos: usize,
}

impl<'a> BeSlice<'a> {
    pub fn new(inner: &'a [u8]) -> Self {
        Self { inner, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.inner.len().saturating_sub(self.pos)
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn u8(&mut self) -> Option<u8> {
        let b = *self.inner.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    pub fn u16(&mut self) -> Option<u16> {
        if self.remaining() < 2 {
            return None;
        }
        let v = BigEndian::read_u16(&self.inner[self.pos..]);
        self.pos += 2;
        Some(v)
    }

    pub fn u32(&mut self) -> Option<u32> {
        if self.remaining() < 4 {
            return None;
        }
        let v = BigEndian::read_u32(&self.inner[self.pos..]);
        self.pos += 4;
        Some(v)
    }

    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.remaining() < n {
            return None;
        }
        let s = &self.inner[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.inner[self.pos..]
    }
}

#[derive(Default)]
pub struct BeWriter {
    inner: Vec<u8>,
}

impl BeWriter {
    pub fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            inner: Vec::with_capacity(n),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn u8(&mut self, v: u8) {
        self.inner.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        let mut b = [0u8; 2];
        BigEndian::write_u16(&mut b, v);
        self.inner.extend_from_slice(&b);
    }

    pub fn u32(&mut self, v: u32) {
        let mut b = [0u8; 4];
        BigEndian::write_u32(&mut b, v);
        self.inner.extend_from_slice(&b);
    }

    pub fn bytes(&mut self, b: &[u8]) {
        self.inner.extend_from_slice(b);
    }

    pub fn cstr(&mut self, s: &str) {
        self.inner.extend_from_slice(s.as_bytes());
        self.inner.push(0);
    }

    pub fn zeros(&mut self, n: usize) {
        self.inner.resize(self.inner.len() + n, 0);
    }

    pub fn patch_u16(&mut self, at: usize, v: u16) {
        BigEndian::write_u16(&mut self.inner[at..at + 2], v);
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.inner
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.inner
    }
}

/// NUL-terminated ASCII at a packet-relative offset. `0` = absent.
pub fn cstr_at(packet: &[u8], offset: u16) -> Option<&str> {
    if offset == 0 {
        return None;
    }
    let start = offset as usize;
    if start >= packet.len() {
        return None;
    }
    let end = packet[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|i| start + i)?;
    std::str::from_utf8(&packet[start..end]).ok()
}

pub fn write_ascii_padded(dst: &mut [u8], s: &str) {
    dst.fill(0);
    let n = s.len().min(dst.len());
    dst[..n].copy_from_slice(&s.as_bytes()[..n]);
}
