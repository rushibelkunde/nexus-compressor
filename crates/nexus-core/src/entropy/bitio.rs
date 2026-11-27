//! Little-endian bit packing for raw (non-entropy-coded) fields (Layer 3).
//!
//! The LZ codec stores the "extra bits" of its length and offset codes here.
//! Bits are written LSB-first into a forward byte stream, so the reader can
//! refill a 64-bit accumulator with a single unaligned load.

/// Integer `log2(x)` in 8.8 fixed point (`x >= 1`), used for entropy pricing.
///
/// Pure integer arithmetic so that encoder decisions are identical on every
/// platform (the decoder never calls this).
#[inline]
pub fn log2_fp8(x: u32) -> u32 {
    debug_assert!(x > 0);
    let hb = 31 - x.leading_zeros();
    // Normalise the mantissa to Q30 in [1.0, 2.0) and square repeatedly.
    let mut m = ((x as u64) << 30) >> hb;
    let mut frac = 0u32;
    for bit in (0..8).rev() {
        m = (m * m) >> 30;
        if m >= (2u64 << 30) {
            m >>= 1;
            frac |= 1 << bit;
        }
    }
    (hb << 8) | frac
}

/// Appends a LEB128-style variable-length unsigned integer.
#[inline]
pub fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Reads a LEB128-style variable-length unsigned integer, advancing `pos`.
///
/// Returns `None` on truncated or over-long input.
#[inline]
pub fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let b = *data.get(*pos)?;
        *pos += 1;
        value |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// Forward LSB-first bit writer.
#[derive(Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the writer for reuse, keeping its allocation.
    pub fn clear(&mut self) {
        self.buf.clear();
        self.acc = 0;
        self.nbits = 0;
    }

    /// Writes the low `n` bits of `value` (`n <= 32`).
    #[inline(always)]
    pub fn put(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32);
        debug_assert!(n == 32 || (value >> n) == 0);
        self.acc |= (value as u64) << self.nbits;
        self.nbits += n;
        if self.nbits >= 32 {
            self.buf.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.nbits -= 32;
        }
    }

    /// Flushes pending bits and returns the packed bytes.
    pub fn finish(&mut self) -> &[u8] {
        while self.nbits > 0 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.nbits = self.nbits.saturating_sub(8);
        }
        &self.buf
    }
}

/// Forward LSB-first bit reader over a byte slice.
///
/// Reading past the end yields zero bits and latches [`BitReader::overrun`],
/// so decoders stay memory-safe on corrupt input and can report it afterwards.
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
    overrun: bool,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at the first bit of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            acc: 0,
            nbits: 0,
            overrun: false,
        }
    }

    /// True if any read ran past the end of the input.
    pub fn overrun(&self) -> bool {
        self.overrun
    }

    #[inline(always)]
    fn refill(&mut self) {
        if self.pos + 8 <= self.data.len() {
            let mut w = [0u8; 8];
            w.copy_from_slice(&self.data[self.pos..self.pos + 8]);
            // Bits above `nbits` that get OR-ed in early are the true next
            // stream bits, so re-reading them on the next refill is harmless.
            self.acc |= u64::from_le_bytes(w) << self.nbits;
            self.pos += ((63 - self.nbits) >> 3) as usize;
            self.nbits |= 56;
        } else {
            while self.nbits <= 56 && self.pos < self.data.len() {
                self.acc |= (self.data[self.pos] as u64) << self.nbits;
                self.pos += 1;
                self.nbits += 8;
            }
        }
    }

    /// Reads `n` bits (`n <= 32`).
    #[inline(always)]
    pub fn get(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if self.nbits < n {
            self.refill();
            if self.nbits < n {
                self.overrun = true;
                self.acc = 0;
                self.nbits = 0;
                return 0;
            }
        }
        let v = (self.acc & ((1u64 << n) - 1)) as u32;
        self.acc >>= n;
        self.nbits -= n;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitio_roundtrip_mixed_widths() {
        let mut w = BitWriter::new();
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let mut expected = Vec::new();
        for i in 0..10_000u32 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = (state >> 59) as u32 + (i % 2); // 0..=32
            let n = n.min(32);
            let v = if n == 0 {
                0
            } else {
                ((state >> 20) as u32) & (u32::MAX >> (32 - n))
            };
            w.put(v, n);
            expected.push((v, n));
        }
        let bytes = w.finish().to_vec();
        let mut r = BitReader::new(&bytes);
        for (v, n) in expected {
            assert_eq!(r.get(n), v);
        }
        assert!(!r.overrun());
    }

    #[test]
    fn test_bitio_overrun_is_latched() {
        let mut r = BitReader::new(&[0xFF]);
        assert_eq!(r.get(8), 0xFF);
        assert_eq!(r.get(4), 0);
        assert!(r.overrun());
    }

    #[test]
    fn test_varint_roundtrip() {
        let mut buf = Vec::new();
        let values = [0u64, 1, 127, 128, 300, 65_535, 1 << 32, u64::MAX];
        for &v in &values {
            write_varint(&mut buf, v);
        }
        let mut pos = 0;
        for &v in &values {
            assert_eq!(read_varint(&buf, &mut pos), Some(v));
        }
        assert_eq!(read_varint(&buf, &mut pos), None);
    }

    #[test]
    fn test_log2_fp8_exact_powers_and_monotonic() {
        for k in 0..31 {
            assert_eq!(log2_fp8(1 << k), k << 8);
        }
        let mut prev = 0;
        for x in 1..5000u32 {
            let l = log2_fp8(x);
            assert!(l >= prev);
            prev = l;
        }
    }
}
