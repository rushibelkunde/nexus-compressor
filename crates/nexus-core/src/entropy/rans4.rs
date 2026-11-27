//! 8-way interleaved order-0 rANS with 16-bit renormalisation (Layer 3).
//!
//! This is the entropy back-end of the format-v2 codecs. Compared with the
//! original single-state coder in `rans.rs` it:
//!
//! * runs eight independent rANS states over one word stream, so a
//!   superscalar CPU decodes eight symbols per loop iteration without a
//!   serial dependency between them;
//! * renormalises 16 bits at a time (at most one branch per symbol);
//! * encodes with a multiply-by-reciprocal instead of a hardware division;
//! * stores frequency tables compactly (1 byte for most symbols).
//!
//! State invariant: every state lives in `[2^15, 2^31)`. Frequencies are
//! normalised to sum to exactly `2^12`. All arithmetic is integer-only, so the
//! bitstream is identical on every platform.
//!
//! Reference: F. Giesen, "Interleaved entropy coders" (2014).

use crate::entropy::bitio::{log2_fp8, read_varint, write_varint};
use crate::error::{NexusError, Result};

/// Probability precision in bits.
pub const SCALE_BITS: u32 = 12;
/// Total of all normalised frequencies.
pub const SCALE: u32 = 1 << SCALE_BITS;
/// Lower bound of the normalised rANS state interval.
pub(crate) const RANS_L: u32 = 1 << 15;
/// Number of interleaved states.
const WAYS: usize = 8;

/// Stream stored verbatim.
const MODE_RAW: u8 = 0;
/// Stream is one byte value repeated.
const MODE_RLE: u8 = 1;
/// Stream is rANS coded with an order-0 table.
const MODE_RANS: u8 = 2;

/// Streams shorter than this are never worth a frequency table.
const MIN_RANS_LEN: usize = 48;

fn corrupt(msg: &str) -> NexusError {
    NexusError::DataCorruption(msg.into())
}

/// Counts byte occurrences using four independent tables to avoid
/// store-to-load stalls on runs of identical bytes.
pub fn histogram(data: &[u8]) -> [u32; 256] {
    let mut h = [[0u32; 256]; 4];
    let (chunks, remainder) = data.as_chunks::<4>();
    for c in chunks {
        h[0][c[0] as usize] += 1;
        h[1][c[1] as usize] += 1;
        h[2][c[2] as usize] += 1;
        h[3][c[3] as usize] += 1;
    }
    for &b in remainder {
        h[0][b as usize] += 1;
    }
    let mut out = [0u32; 256];
    for i in 0..256 {
        out[i] = h[0][i] + h[1][i] + h[2][i] + h[3][i];
    }
    out
}

/// Normalises raw counts so they sum to exactly [`SCALE`], keeping every
/// present symbol at frequency >= 1.
///
/// `total` must equal the sum of `counts` and be non-zero.
pub fn normalize_freqs(counts: &[u32; 256], total: u32) -> [u16; 256] {
    let mut freqs = [0u16; 256];
    if total == 0 {
        return freqs;
    }
    let mut sum = 0u32;
    let mut largest = 0usize;
    for (sym, &c) in counts.iter().enumerate() {
        if c == 0 {
            continue;
        }
        // Round to nearest, but never below 1.
        let scaled = ((c as u64 * SCALE as u64 + (total as u64 >> 1)) / total as u64) as u32;
        let f = scaled.clamp(1, SCALE);
        freqs[sym] = f as u16;
        sum += f;
        if c > counts[largest] {
            largest = sym;
        }
    }

    if sum < SCALE {
        freqs[largest] += (SCALE - sum) as u16;
    } else {
        // Shave the excess off the most probable symbols, where one unit of
        // frequency costs the fewest bits.
        let mut excess = sum - SCALE;
        while excess > 0 {
            let mut best = usize::MAX;
            for (sym, &f) in freqs.iter().enumerate() {
                if f > 1 && (best == usize::MAX || f > freqs[best]) {
                    best = sym;
                }
            }
            if best == usize::MAX {
                break;
            }
            let take = excess.min((freqs[best] as u32 - 1).div_ceil(2)).max(1);
            freqs[best] -= take as u16;
            excess -= take;
        }
    }
    freqs
}

/// Estimated coded size in bits (8.8 fixed point >> 8) of a stream with the
/// given raw counts under the given normalised frequencies.
pub fn estimate_bits(counts: &[u32; 256], freqs: &[u16; 256]) -> u64 {
    let mut bits_fp = 0u64;
    for sym in 0..256 {
        if counts[sym] > 0 && freqs[sym] > 0 {
            let cost = (SCALE_BITS << 8) - log2_fp8(freqs[sym] as u32);
            bits_fp += counts[sym] as u64 * cost as u64;
        }
    }
    bits_fp >> 8
}

/// Serialises a normalised frequency table.
///
/// Layout: `n-1` (u8, `n` = highest present symbol + 1), then for each symbol
/// `0..n`: a zero byte followed by the count of *additional* zeros in the run,
/// or the frequency as one byte (`< 128`) or two bytes (`0x80 | hi`, `lo`).
pub(crate) fn write_table(freqs: &[u16; 256], out: &mut Vec<u8>) {
    let n = freqs.iter().rposition(|&f| f != 0).map_or(1, |p| p + 1);
    out.push((n - 1) as u8);
    let mut sym = 0;
    while sym < n {
        let f = freqs[sym];
        if f == 0 {
            let mut run = 1;
            while sym + run < n && freqs[sym + run] == 0 && run < 256 {
                run += 1;
            }
            out.push(0);
            out.push((run - 1) as u8);
            sym += run;
        } else if f < 128 {
            out.push(f as u8);
            sym += 1;
        } else {
            out.push(0x80 | (f >> 8) as u8);
            out.push(f as u8);
            sym += 1;
        }
    }
}

/// Parses a table written by [`write_table`] and validates that it sums to [`SCALE`].
pub(crate) fn read_table(data: &[u8], pos: &mut usize) -> Result<[u16; 256]> {
    let mut freqs = [0u16; 256];
    let n = *data
        .get(*pos)
        .ok_or_else(|| corrupt("rANS table truncated"))? as usize
        + 1;
    *pos += 1;
    let mut sym = 0;
    let mut sum = 0u32;
    while sym < n {
        let b = *data
            .get(*pos)
            .ok_or_else(|| corrupt("rANS table truncated"))?;
        *pos += 1;
        if b == 0 {
            let run = *data
                .get(*pos)
                .ok_or_else(|| corrupt("rANS table truncated"))? as usize
                + 1;
            *pos += 1;
            sym += run;
        } else if b < 128 {
            freqs[sym] = b as u16;
            sum += b as u32;
            sym += 1;
        } else {
            let lo = *data
                .get(*pos)
                .ok_or_else(|| corrupt("rANS table truncated"))?;
            *pos += 1;
            let f = (((b & 0x7F) as u32) << 8) | lo as u32;
            if f > SCALE {
                return Err(corrupt("rANS frequency out of range"));
            }
            freqs[sym] = f as u16;
            sum += f;
            sym += 1;
        }
    }
    if sym != n || sum != SCALE {
        return Err(corrupt("rANS table does not sum to scale"));
    }
    Ok(freqs)
}

/// Per-symbol encoder constants (reciprocal form of `x / freq`).
#[derive(Clone, Copy, Default)]
pub(crate) struct EncSymbol {
    /// Renormalise while `x >= x_max`.
    pub(crate) x_max: u32,
    /// Fixed-point reciprocal of the frequency.
    pub(crate) rcp_freq: u32,
    /// Additive bias (cumulative start, adjusted for `freq == 1`).
    pub(crate) bias: u32,
    /// `SCALE - freq`.
    pub(crate) cmpl_freq: u16,
    /// Post-multiply shift for the reciprocal.
    pub(crate) rcp_shift: u16,
}

pub(crate) fn build_enc_symbols(freqs: &[u16; 256]) -> [EncSymbol; 256] {
    let mut syms = [EncSymbol::default(); 256];
    let mut start = 0u32;
    for (sym, &f) in freqs.iter().enumerate() {
        let freq = f as u32;
        if freq == 0 {
            continue;
        }
        let mut e = EncSymbol {
            x_max: ((RANS_L >> SCALE_BITS) << 16) * freq,
            cmpl_freq: (SCALE - freq) as u16,
            ..EncSymbol::default()
        };
        if freq < 2 {
            // x / 1 == x: use the all-ones reciprocal (yields x - 1) and fold
            // the missing 1 * SCALE into the bias.
            e.rcp_freq = u32::MAX;
            e.rcp_shift = 0;
            e.bias = start + SCALE - 1;
        } else {
            // Alverson's exact division by multiplication; valid for all
            // x < 2^31, which the state invariant guarantees.
            let mut shift = 0u32;
            while freq > (1u32 << shift) {
                shift += 1;
            }
            e.rcp_freq = (1u64 << (shift + 31)).div_ceil(freq as u64) as u32;
            e.rcp_shift = (shift - 1) as u16;
            e.bias = start;
        }
        syms[sym] = e;
        start += freq;
    }
    syms
}

/// rANS-encodes `input` with `freqs`, appending the `WAYS` final states
/// (`u32` each) followed by the 16-bit renormalisation words to `out`.
fn rans_encode(input: &[u8], freqs: &[u16; 256], out: &mut Vec<u8>) {
    let syms = build_enc_symbols(freqs);
    let n = input.len();
    // Every symbol emits at most one 16-bit word; plus the final states,
    // plus 2 bytes of slack so the unconditional word store below never
    // lands before the start of the buffer.
    let cap = n * 2 + WAYS * 4 + 2;
    let mut buf = vec![0u8; cap];
    let mut ptr = cap;
    let mut x = [RANS_L; WAYS];

    // Branch-free step: the renormalisation word is always stored, and the
    // write pointer only moves when the state actually overflowed.
    macro_rules! put {
        ($k:expr, $sym:expr) => {{
            // SAFETY: `$sym` is a u8 so the table index is in range. At most
            // `n` words are emitted, so `ptr >= cap - 2n = WAYS * 4 + 2` here
            // and the 2-byte store at `ptr - 2` is in bounds.
            unsafe {
                let e = syms.get_unchecked($sym as usize);
                let v = x[$k];
                let need = (v >= e.x_max) as usize;
                std::ptr::write_unaligned(
                    buf.as_mut_ptr().add(ptr - 2) as *mut u16,
                    (v as u16).to_le(),
                );
                ptr -= 2 * need;
                let v = v >> (16 * need);
                let q = (((v as u64) * (e.rcp_freq as u64)) >> 32) as u32 >> e.rcp_shift;
                x[$k] = v + e.bias + q * e.cmpl_freq as u32;
            }
        }};
    }

    // Symbols are coded last-to-first; symbol `i` belongs to state `i % WAYS`.
    let mut i = n;
    while i & (WAYS - 1) != 0 {
        i -= 1;
        put!(i & (WAYS - 1), input[i]);
    }
    while i > 0 {
        i -= WAYS;
        put!(7, input[i + 7]);
        put!(6, input[i + 6]);
        put!(5, input[i + 5]);
        put!(4, input[i + 4]);
        put!(3, input[i + 3]);
        put!(2, input[i + 2]);
        put!(1, input[i + 1]);
        put!(0, input[i]);
    }

    for &s in x.iter().rev() {
        ptr -= 4;
        buf[ptr..ptr + 4].copy_from_slice(&s.to_le_bytes());
    }
    out.extend_from_slice(&buf[ptr..]);
}

/// Decodes `out.len()` symbols from a payload produced by [`rans_encode`].
fn rans_decode(data: &[u8], freqs: &[u16; 256], out: &mut [u8]) -> Result<()> {
    if data.len() < WAYS * 4 {
        return Err(corrupt("rANS payload truncated"));
    }

    // Slot table entry: sym | (slot - cum_start) << 8 | (freq - 1) << 20.
    let mut table = vec![0u32; SCALE as usize];
    let mut start = 0usize;
    for (sym, &f) in freqs.iter().enumerate() {
        let f = f as usize;
        for (k, entry) in table[start..start + f].iter_mut().enumerate() {
            *entry = sym as u32 | ((k as u32) << 8) | (((f - 1) as u32) << 20);
        }
        start += f;
    }

    let mut x = [0u32; WAYS];
    for (k, s) in x.iter_mut().enumerate() {
        let mut w = [0u8; 4];
        w.copy_from_slice(&data[k * 4..k * 4 + 4]);
        *s = u32::from_le_bytes(w);
    }
    let mut ptr = WAYS * 4;
    let n = out.len();
    let mask = SCALE - 1;

    #[inline(always)]
    fn step(x: u32, entry: u32) -> u32 {
        ((entry >> 20) + 1)
            .wrapping_mul(x >> SCALE_BITS)
            .wrapping_add((entry >> 8) & 0xFFF)
    }

    let mut i = 0;
    // Fast path: eight symbols per iteration while 16 input bytes remain.
    while i + WAYS <= n && ptr + 2 * WAYS <= data.len() {
        // SAFETY: slot indices are masked to < SCALE == table.len();
        // `i + WAYS <= n` and `ptr + 2 * WAYS <= data.len()` are checked by
        // the loop condition, and `ptr` advances by at most 2 per state.
        unsafe {
            let mut e = [0u32; WAYS];
            for k in 0..WAYS {
                e[k] = *table.get_unchecked((x[k] & mask) as usize);
                *out.get_unchecked_mut(i + k) = e[k] as u8;
                x[k] = step(x[k], e[k]);
            }
            // Branch-free renormalisation: always load the next word, but
            // only shift it in (and advance) when the state underflowed.
            for xk in x.iter_mut() {
                let need = (*xk < RANS_L) as u32;
                let w = u16::from_le(std::ptr::read_unaligned(
                    data.as_ptr().add(ptr) as *const u16
                )) as u32;
                *xk = (*xk << (need * 16)) | (w & need.wrapping_neg());
                ptr += 2 * need as usize;
            }
        }
        i += WAYS;
    }

    // Careful tail: one symbol at a time with checked reads.
    while i < n {
        let k = i & (WAYS - 1);
        let entry = table[(x[k] & mask) as usize];
        out[i] = entry as u8;
        let mut v = step(x[k], entry);
        if v < RANS_L {
            if ptr + 2 > data.len() {
                return Err(corrupt("rANS stream exhausted"));
            }
            v = (v << 16) | u16::from_le_bytes([data[ptr], data[ptr + 1]]) as u32;
            ptr += 2;
        }
        x[k] = v;
        i += 1;
    }
    Ok(())
}

/// Entropy-codes a byte stream, choosing between raw, single-symbol RLE and
/// rANS, and appends a self-describing record to `out`.
///
/// Record layout: `mode: u8`, `len: varint`, then
/// * raw: `len` bytes;
/// * RLE: one byte;
/// * rANS: frequency table, `payload_len: varint`, payload.
pub fn encode_stream(input: &[u8], out: &mut Vec<u8>) {
    let n = input.len();
    if n < MIN_RANS_LEN {
        write_raw(input, out);
        return;
    }
    let counts = histogram(input);
    let distinct = counts.iter().filter(|&&c| c > 0).count();
    if distinct == 1 {
        out.push(MODE_RLE);
        write_varint(out, n as u64);
        out.push(input[0]);
        return;
    }
    let freqs = normalize_freqs(&counts, n as u32);
    // Skip the encode entirely when the table cannot pay for itself.
    let est_bytes = estimate_bits(&counts, &freqs) / 8 + distinct as u64 + 20;
    if est_bytes >= n as u64 {
        write_raw(input, out);
        return;
    }

    let mark = out.len();
    out.push(MODE_RANS);
    write_varint(out, n as u64);
    write_table(&freqs, out);
    let mut payload = Vec::new();
    rans_encode(input, &freqs, &mut payload);
    write_varint(out, payload.len() as u64);
    out.extend_from_slice(&payload);

    if out.len() - mark >= n + 4 {
        out.truncate(mark);
        write_raw(input, out);
    }
}

fn write_raw(input: &[u8], out: &mut Vec<u8>) {
    out.push(MODE_RAW);
    write_varint(out, input.len() as u64);
    out.extend_from_slice(input);
}

/// Decodes one record written by [`encode_stream`] starting at `*pos`,
/// replacing the contents of `out` and advancing `*pos` past the record.
///
/// `max_len` bounds the decoded size so corrupt headers cannot trigger huge
/// allocations.
pub fn decode_stream(
    data: &[u8],
    pos: &mut usize,
    out: &mut Vec<u8>,
    max_len: usize,
) -> Result<()> {
    let mode = *data
        .get(*pos)
        .ok_or_else(|| corrupt("entropy stream truncated"))?;
    *pos += 1;
    let n =
        read_varint(data, pos).ok_or_else(|| corrupt("entropy stream length truncated"))? as usize;
    if n > max_len {
        return Err(corrupt("entropy stream length exceeds block bound"));
    }
    out.clear();
    match mode {
        MODE_RAW => {
            let end = pos
                .checked_add(n)
                .filter(|&e| e <= data.len())
                .ok_or_else(|| corrupt("raw stream truncated"))?;
            out.extend_from_slice(&data[*pos..end]);
            *pos = end;
        }
        MODE_RLE => {
            let b = *data
                .get(*pos)
                .ok_or_else(|| corrupt("RLE stream truncated"))?;
            *pos += 1;
            out.resize(n, b);
        }
        MODE_RANS => {
            let freqs = read_table(data, pos)?;
            let plen = read_varint(data, pos)
                .ok_or_else(|| corrupt("rANS payload length truncated"))?
                as usize;
            let end = pos
                .checked_add(plen)
                .filter(|&e| e <= data.len())
                .ok_or_else(|| corrupt("rANS payload truncated"))?;
            out.resize(n, 0);
            rans_decode(&data[*pos..end], &freqs, out)?;
            *pos = end;
        }
        _ => return Err(corrupt("unknown entropy stream mode")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(input: &[u8]) -> usize {
        let mut enc = Vec::new();
        encode_stream(input, &mut enc);
        let mut pos = 0;
        let mut dec = Vec::new();
        decode_stream(&enc, &mut pos, &mut dec, input.len()).unwrap();
        assert_eq!(pos, enc.len());
        assert_eq!(dec, input);
        enc.len()
    }

    fn lcg_bytes(n: usize, seed: u64, shape: impl Fn(u64) -> u8) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                shape(s >> 33)
            })
            .collect()
    }

    #[test]
    fn test_rans4_empty_and_tiny() {
        roundtrip(b"");
        roundtrip(b"x");
        roundtrip(b"hello");
    }

    #[test]
    fn test_rans4_single_symbol_run() {
        let size = roundtrip(&vec![7u8; 100_000]);
        assert!(size < 8);
    }

    #[test]
    fn test_rans4_skewed_distribution_compresses() {
        // Geometric-ish distribution over a handful of symbols.
        let input = lcg_bytes(200_000, 1, |r| (r & (r >> 8) & (r >> 16) & 0x0F) as u8);
        let size = roundtrip(&input);
        assert!(size < input.len() / 2);
    }

    #[test]
    fn test_rans4_uniform_random_falls_back_to_raw() {
        let input = lcg_bytes(50_000, 2, |r| r as u8);
        let size = roundtrip(&input);
        assert!(size <= input.len() + 8);
    }

    #[test]
    fn test_rans4_all_lengths_around_interleave_boundaries() {
        for n in 40..140 {
            let input = lcg_bytes(n, n as u64, |r| (r % 5) as u8);
            roundtrip(&input);
        }
    }

    #[test]
    fn test_rans4_rare_symbols_freq_one() {
        // 255 symbols seen once each among a dominant symbol: exercises the
        // freq == 1 encoder path and the normalisation excess removal.
        let mut input = vec![0u8; 300_000];
        for s in 1..=255usize {
            input[s * 1000] = s as u8;
        }
        roundtrip(&input);
    }

    #[test]
    fn test_rans4_reciprocal_matches_division() {
        // The encoder's reciprocal must equal true division for every
        // frequency over the whole state range it can see.
        let mut freqs = [0u16; 256];
        for f in 1..=255u16 {
            freqs[f as usize] = f;
        }
        let syms = build_enc_symbols(&freqs);
        let mut s = 99u64;
        for f in 1..=255u32 {
            let e = &syms[f as usize];
            for _ in 0..2000 {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                let x = ((s >> 33) as u32) & 0x7FFF_FFFF;
                let x = x.max(1);
                let q = (((x as u64) * (e.rcp_freq as u64)) >> 32) as u32 >> e.rcp_shift;
                if f >= 2 {
                    assert_eq!(q, x / f, "x={x} f={f}");
                } else {
                    assert_eq!(q, x - 1);
                }
            }
        }
    }

    #[test]
    fn test_rans4_normalize_sums_to_scale() {
        for seed in 0..50u64 {
            let input = lcg_bytes(1000 + seed as usize * 37, seed, |r| {
                (((r & 0xFF) * ((r >> 8) & 0xFF)) >> 8) as u8
            });
            let counts = histogram(&input);
            let freqs = normalize_freqs(&counts, input.len() as u32);
            assert_eq!(freqs.iter().map(|&f| f as u32).sum::<u32>(), SCALE);
            for s in 0..256 {
                assert_eq!(counts[s] > 0, freqs[s] > 0);
            }
        }
    }

    #[test]
    fn test_rans4_corrupt_input_is_rejected_not_panicking() {
        let input = lcg_bytes(5000, 3, |r| (r % 7) as u8);
        let mut enc = Vec::new();
        encode_stream(&input, &mut enc);
        for cut in [0, 1, 2, 5, enc.len() / 2, enc.len() - 1] {
            let mut pos = 0;
            let mut dec = Vec::new();
            let _ = decode_stream(&enc[..cut], &mut pos, &mut dec, input.len());
        }
        let mut pos = 0;
        let mut dec = Vec::new();
        assert!(decode_stream(&enc, &mut pos, &mut dec, 10).is_err());
    }
}
