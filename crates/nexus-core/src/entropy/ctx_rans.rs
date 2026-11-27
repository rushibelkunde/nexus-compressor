//! Context-clustered order-1 rANS for LZ literals (Layer 3).
//!
//! A literal is far more predictable given the byte before it ("q" is almost
//! always followed by "u"). A full order-1 model would need 256 frequency
//! tables per block, which costs more in headers than it saves. Instead the
//! 256 possible previous-byte contexts are clustered into a handful of
//! *classes* with similar statistics; each class gets one table and a
//! 256-entry map says which class each context uses.
//!
//! The decoder needs the previous output byte to pick a table, so literals in
//! this mode are decoded one at a time inside the LZ sequence loop rather than
//! up front. That is slower than plain interleaved rANS, which is why only
//! the higher levels use it.
//!
//! Record layout (after the mode byte [`MODE_CTX`]):
//!
//! ```text
//! varint  literal_count
//! u8      class_count (2..=16)
//! stream  context map (256 class ids, see rans4::encode_stream)
//! table   one frequency table per class
//! varint  payload_len
//! bytes   4 x u32 rANS states, then 16-bit renormalisation words
//! ```
//!
//! Everything is integer arithmetic; clustering only affects which valid
//! stream the encoder produces, never how the decoder interprets it.

use crate::entropy::bitio::{log2_fp8, read_varint, write_varint};
use crate::entropy::rans4::{
    build_enc_symbols, decode_stream, encode_stream, normalize_freqs, read_table, write_table,
    EncSymbol, RANS_L, SCALE, SCALE_BITS,
};
use crate::error::{NexusError, Result};

/// Literal-section mode byte for context-coded literals.
pub const MODE_CTX: u8 = 3;
/// Upper bound on the number of context classes.
pub const MAX_CLASSES: usize = 16;
/// Below this many literals the extra tables cannot pay for themselves.
const MIN_LITERALS: usize = 2048;
/// Refinement rounds of the context clustering.
const CLUSTER_ROUNDS: usize = 3;

fn corrupt(msg: &str) -> NexusError {
    NexusError::DataCorruption(format!("context literals: {msg}"))
}

/// Coded size in bits (8.8 fixed point) of a histogram under its own
/// empirical distribution.
fn entropy_bits_fp(hist: &[u32; 256], total: u32) -> u64 {
    if total == 0 {
        return 0;
    }
    let lt = log2_fp8(total) as u64;
    let mut bits = 0u64;
    for &c in hist.iter() {
        if c > 0 {
            bits += c as u64 * (lt - log2_fp8(c) as u64);
        }
    }
    bits
}

/// Approximate serialised size of a frequency table, in 8.8 fixed-point bits.
fn table_bits_fp(hist: &[u32; 256]) -> u64 {
    let nonzero = hist.iter().filter(|&&c| c > 0).count() as u64;
    (nonzero * 10 + 24) << 8
}

/// Result of clustering the previous-byte contexts.
struct Clustering {
    /// Class id for each previous-byte context.
    map: [u8; 256],
    /// Histogram of each class.
    hists: Vec<[u32; 256]>,
}

/// Groups the 256 contexts into at most `max_classes` classes with similar
/// symbol statistics: seed with the busiest contexts, refine k-means style,
/// then merge classes whose tables are not worth their header.
fn cluster_contexts(hist: &[u32], ctx_total: &[u32; 256], max_classes: usize) -> Clustering {
    let mut active: Vec<usize> = (0..256).filter(|&c| ctx_total[c] > 0).collect();
    active.sort_by(|&a, &b| ctx_total[b].cmp(&ctx_total[a]).then(a.cmp(&b)));
    let k = max_classes.min(active.len()).max(1);

    let row = |c: usize| -> &[u32] { &hist[c * 256..c * 256 + 256] };
    let mut map = [0u8; 256];
    let mut hists: Vec<[u32; 256]> = vec![[0u32; 256]; k];
    for (j, &c) in active.iter().take(k).enumerate() {
        map[c] = j as u8;
    }
    // Contexts beyond the seeds start in the last (least busy) class.
    for &c in active.iter().skip(k) {
        map[c] = (k - 1) as u8;
    }

    let rebuild = |map: &[u8; 256], hists: &mut Vec<[u32; 256]>| {
        for h in hists.iter_mut() {
            *h = [0u32; 256];
        }
        for &c in &active {
            let h = &mut hists[map[c] as usize];
            for (acc, &v) in h.iter_mut().zip(row(c)) {
                *acc += v;
            }
        }
    };
    rebuild(&map, &mut hists);

    let mut cost = vec![0u32; k * 256];
    for _ in 0..CLUSTER_ROUNDS {
        // Price of each symbol in each class, with add-one smoothing so a
        // symbol the class has never seen is expensive but not impossible.
        for (j, h) in hists.iter().enumerate() {
            let total: u32 = h.iter().sum();
            let lt = log2_fp8(total + 256);
            for s in 0..256 {
                cost[j * 256 + s] = lt - log2_fp8(h[s] + 1);
            }
        }
        let mut changed = false;
        for &c in &active {
            let r = row(c);
            let mut best = (u64::MAX, map[c]);
            for j in 0..k {
                let cj = &cost[j * 256..j * 256 + 256];
                let mut total = 0u64;
                for s in 0..256 {
                    total += r[s] as u64 * cj[s] as u64;
                }
                if total < best.0 {
                    best = (total, j as u8);
                }
            }
            if best.1 != map[c] {
                map[c] = best.1;
                changed = true;
            }
        }
        rebuild(&map, &mut hists);
        if !changed {
            break;
        }
    }

    // Merge classes while the saved table header outweighs the lost precision.
    let mut alive: Vec<bool> = hists.iter().map(|h| h.iter().any(|&c| c > 0)).collect();
    loop {
        let mut best: Option<(i64, usize, usize)> = None;
        for a in 0..k {
            if !alive[a] {
                continue;
            }
            for b in a + 1..k {
                if !alive[b] {
                    continue;
                }
                let mut merged = [0u32; 256];
                for s in 0..256 {
                    merged[s] = hists[a][s] + hists[b][s];
                }
                let ta: u32 = hists[a].iter().sum();
                let tb: u32 = hists[b].iter().sum();
                let before = entropy_bits_fp(&hists[a], ta)
                    + entropy_bits_fp(&hists[b], tb)
                    + table_bits_fp(&hists[a])
                    + table_bits_fp(&hists[b]);
                let after = entropy_bits_fp(&merged, ta + tb) + table_bits_fp(&merged);
                let gain = before as i64 - after as i64;
                if gain > 0 && best.is_none_or(|(g, _, _)| gain > g) {
                    best = Some((gain, a, b));
                }
            }
        }
        let Some((_, a, b)) = best else {
            break;
        };
        let absorbed = hists[b];
        for (acc, &v) in hists[a].iter_mut().zip(&absorbed) {
            *acc += v;
        }
        alive[b] = false;
        for m in map.iter_mut() {
            if *m as usize == b {
                *m = a as u8;
            }
        }
    }

    // Compact class ids.
    let mut remap = [0u8; MAX_CLASSES];
    let mut compact: Vec<[u32; 256]> = Vec::new();
    for j in 0..k {
        if alive[j] {
            remap[j] = compact.len() as u8;
            compact.push(hists[j]);
        }
    }
    for &c in &active {
        map[c] = remap[map[c] as usize];
    }
    for c in 0..256 {
        if ctx_total[c] == 0 {
            map[c] = 0;
        }
    }
    if compact.is_empty() {
        compact.push([0u32; 256]);
    }
    Clustering {
        map,
        hists: compact,
    }
}

/// Encodes `lits` with a context-clustered order-1 model, where `ctxs[i]` is
/// the byte preceding `lits[i]` in the decoded output.
///
/// Appends a complete record (including the [`MODE_CTX`] byte) to `out` and
/// returns `true`, or appends nothing and returns `false` when a plain
/// order-0 stream is estimated to be at least as small. `max_classes` is
/// clamped to [`MAX_CLASSES`].
pub fn encode_ctx_literals(
    lits: &[u8],
    ctxs: &[u8],
    max_classes: usize,
    out: &mut Vec<u8>,
) -> bool {
    let n = lits.len();
    debug_assert_eq!(n, ctxs.len());
    let max_classes = max_classes.min(MAX_CLASSES);
    if n < MIN_LITERALS || max_classes < 2 || n != ctxs.len() {
        return false;
    }

    let mut hist = vec![0u32; 256 * 256];
    let mut ctx_total = [0u32; 256];
    for (&s, &c) in lits.iter().zip(ctxs) {
        hist[(c as usize) << 8 | s as usize] += 1;
        ctx_total[c as usize] += 1;
    }

    let clustering = cluster_contexts(&hist, &ctx_total, max_classes);
    let k = clustering.hists.len();
    if k < 2 {
        return false;
    }

    // Is it actually better than one table for everything?
    let mut order0 = [0u32; 256];
    let mut ctx_bits = 0u64;
    for h in &clustering.hists {
        let total: u32 = h.iter().sum();
        ctx_bits += entropy_bits_fp(h, total) + table_bits_fp(h);
        for s in 0..256 {
            order0[s] += h[s];
        }
    }
    let mut map_record = Vec::new();
    encode_stream(&clustering.map, &mut map_record);
    ctx_bits += (map_record.len() as u64 * 8) << 8;
    let order0_bits = entropy_bits_fp(&order0, n as u32) + table_bits_fp(&order0);
    if ctx_bits + (64 << 8) >= order0_bits {
        return false;
    }

    out.push(MODE_CTX);
    write_varint(out, n as u64);
    out.push(k as u8);
    out.extend_from_slice(&map_record);

    let mut syms: Vec<[EncSymbol; 256]> = Vec::with_capacity(k);
    for h in &clustering.hists {
        let total: u32 = h.iter().sum();
        let freqs = normalize_freqs(h, total);
        write_table(&freqs, out);
        syms.push(build_enc_symbols(&freqs));
    }

    // Same branch-free interleaved coder as rans4, with a per-symbol table.
    let cap = n * 2 + 4 * 4 + 2;
    let mut buf = vec![0u8; cap];
    let mut ptr = cap;
    let mut states = [RANS_L; 4];
    for i in (0..n).rev() {
        let class = clustering.map[ctxs[i] as usize] as usize;
        let e = &syms[class][lits[i] as usize];
        let x = &mut states[i & 3];
        let need = (*x >= e.x_max) as usize;
        buf[ptr - 2..ptr].copy_from_slice(&(*x as u16).to_le_bytes());
        ptr -= 2 * need;
        *x >>= 16 * need;
        let q = (((*x as u64) * (e.rcp_freq as u64)) >> 32) as u32 >> e.rcp_shift;
        *x = *x + e.bias + q * e.cmpl_freq as u32;
    }
    for &s in states.iter().rev() {
        ptr -= 4;
        buf[ptr..ptr + 4].copy_from_slice(&s.to_le_bytes());
    }
    write_varint(out, (cap - ptr) as u64);
    out.extend_from_slice(&buf[ptr..]);
    true
}

/// Streaming decoder for a record written by [`encode_ctx_literals`].
///
/// Malformed or exhausted input never panics: [`Self::next`] returns zeros
/// and latches an error that [`Self::finish`] reports.
pub struct CtxLitDecoder<'a> {
    /// `class_count * SCALE` slot entries: sym | bias << 8 | (freq - 1) << 20.
    table: Vec<u32>,
    map: [u8; 256],
    states: [u32; 4],
    data: &'a [u8],
    ptr: usize,
    index: usize,
    count: usize,
    bad: bool,
}

impl<'a> CtxLitDecoder<'a> {
    /// Parses the record header starting at `*pos` (just past the mode byte)
    /// and advances `*pos` past the whole record.
    ///
    /// # Errors
    /// Returns `NexusError::DataCorruption` if the header is malformed or
    /// declares more than `max_len` literals.
    pub fn new(data: &'a [u8], pos: &mut usize, max_len: usize) -> Result<Self> {
        let count = read_varint(data, pos).ok_or_else(|| corrupt("missing count"))? as usize;
        if count > max_len {
            return Err(corrupt("count exceeds block size"));
        }
        let k = *data
            .get(*pos)
            .ok_or_else(|| corrupt("missing class count"))? as usize;
        *pos += 1;
        if !(2..=MAX_CLASSES).contains(&k) {
            return Err(corrupt("class count out of range"));
        }
        let mut map_bytes = Vec::new();
        decode_stream(data, pos, &mut map_bytes, 256)?;
        if map_bytes.len() != 256 || map_bytes.iter().any(|&m| m as usize >= k) {
            return Err(corrupt("invalid context map"));
        }
        let mut map = [0u8; 256];
        map.copy_from_slice(&map_bytes);

        let mut table = vec![0u32; k * SCALE as usize];
        for class in 0..k {
            let freqs = read_table(data, pos)?;
            let base = class * SCALE as usize;
            let mut start = 0usize;
            for (sym, &f) in freqs.iter().enumerate() {
                let f = f as usize;
                for (j, entry) in table[base + start..base + start + f].iter_mut().enumerate() {
                    *entry = sym as u32 | ((j as u32) << 8) | (((f - 1) as u32) << 20);
                }
                start += f;
            }
        }

        let plen =
            read_varint(data, pos).ok_or_else(|| corrupt("missing payload length"))? as usize;
        let end = pos
            .checked_add(plen)
            .filter(|&e| e <= data.len())
            .ok_or_else(|| corrupt("payload truncated"))?;
        let payload = &data[*pos..end];
        *pos = end;
        if payload.len() < 16 {
            return Err(corrupt("payload too short for states"));
        }
        let mut states = [0u32; 4];
        for (j, s) in states.iter_mut().enumerate() {
            let mut w = [0u8; 4];
            w.copy_from_slice(&payload[j * 4..j * 4 + 4]);
            *s = u32::from_le_bytes(w);
        }
        Ok(Self {
            table,
            map,
            states,
            data: payload,
            ptr: 16,
            index: 0,
            count,
            bad: false,
        })
    }

    /// Literals not yet decoded.
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.count - self.index
    }

    /// Decodes the next literal given the byte that precedes it in the output.
    #[inline(always)]
    pub fn next(&mut self, prev: u8) -> u8 {
        if self.index >= self.count {
            self.bad = true;
            return 0;
        }
        let j = self.index & 3;
        let x = self.states[j];
        let class = self.map[prev as usize] as usize;
        // `class < class_count` was validated in `new`, and the slot is
        // masked below SCALE, so this index is always in range.
        let entry = self.table[(class << SCALE_BITS) | (x & (SCALE - 1)) as usize];
        let mut x = ((entry >> 20) + 1)
            .wrapping_mul(x >> SCALE_BITS)
            .wrapping_add((entry >> 8) & 0xFFF);
        if x < RANS_L {
            if self.ptr + 2 <= self.data.len() {
                x = (x << 16)
                    | u16::from_le_bytes([self.data[self.ptr], self.data[self.ptr + 1]]) as u32;
                self.ptr += 2;
            } else {
                self.bad = true;
            }
        }
        self.states[j] = x;
        self.index += 1;
        entry as u8
    }

    /// Reports whether decoding stayed within the record.
    pub fn finish(&self) -> Result<()> {
        if self.bad {
            Err(corrupt("stream exhausted"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed;
        move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            s >> 33
        }
    }

    /// A first-order Markov source: the next byte depends strongly on the
    /// previous one, so order-1 coding must clearly beat order-0.
    fn markov_text(n: usize, seed: u64) -> Vec<u8> {
        let mut r = lcg(seed);
        let mut out = Vec::with_capacity(n);
        let mut prev = b'a';
        for _ in 0..n {
            let choices = [prev.wrapping_add(1), prev.wrapping_add(7), b' ', b'e'];
            let pick = match r() % 16 {
                0..=9 => choices[0],
                10..=12 => choices[1],
                13..=14 => choices[2],
                _ => choices[3],
            };
            let b = b'a' + (pick % 26);
            out.push(b);
            prev = b;
        }
        out
    }

    fn contexts_of(data: &[u8]) -> Vec<u8> {
        let mut c = Vec::with_capacity(data.len());
        c.push(0);
        c.extend_from_slice(&data[..data.len().saturating_sub(1)]);
        c
    }

    fn roundtrip(lits: &[u8], ctxs: &[u8], classes: usize) -> Option<usize> {
        let mut enc = Vec::new();
        if !encode_ctx_literals(lits, ctxs, classes, &mut enc) {
            assert!(enc.is_empty());
            return None;
        }
        assert_eq!(enc[0], MODE_CTX);
        let mut pos = 1;
        let mut dec = CtxLitDecoder::new(&enc, &mut pos, lits.len()).unwrap();
        assert_eq!(pos, enc.len());
        assert_eq!(dec.remaining(), lits.len());
        for (i, (&s, &c)) in lits.iter().zip(ctxs).enumerate() {
            assert_eq!(dec.next(c), s, "literal {i}");
        }
        dec.finish().unwrap();
        assert_eq!(dec.remaining(), 0);
        Some(enc.len())
    }

    #[test]
    fn test_ctx_rans_markov_beats_order0() {
        let data = markov_text(200_000, 1);
        let ctxs = contexts_of(&data);
        let size = roundtrip(&data, &ctxs, 16).expect("context coding should win");
        let mut plain = Vec::new();
        encode_stream(&data, &mut plain);
        assert!(
            size * 100 < plain.len() * 80,
            "ctx {size} vs order-0 {}",
            plain.len()
        );
    }

    #[test]
    fn test_ctx_rans_roundtrip_various_class_limits() {
        let data = markov_text(60_000, 2);
        let ctxs = contexts_of(&data);
        for classes in [2, 3, 5, 8, 12, 16, 99] {
            roundtrip(&data, &ctxs, classes);
        }
    }

    #[test]
    fn test_ctx_rans_declines_when_context_is_useless() {
        // Uniform random bytes: no context helps, order-0 must be preferred.
        let mut r = lcg(3);
        let data: Vec<u8> = (0..50_000).map(|_| r() as u8).collect();
        let ctxs = contexts_of(&data);
        assert!(roundtrip(&data, &ctxs, 16).is_none());
        // Too few literals.
        assert!(roundtrip(&data[..500], &ctxs[..500], 16).is_none());
    }

    #[test]
    fn test_ctx_rans_arbitrary_contexts() {
        // Contexts need not be the previous literal (after a match they are
        // the last matched byte), so feed unrelated context bytes.
        let data = markov_text(80_000, 4);
        let mut r = lcg(5);
        let ctxs: Vec<u8> = data.iter().map(|&b| b ^ ((r() % 3) as u8)).collect();
        roundtrip(&data, &ctxs, 12);
    }

    #[test]
    fn test_ctx_rans_corrupt_input_never_panics() {
        let data = markov_text(30_000, 6);
        let ctxs = contexts_of(&data);
        let mut enc = Vec::new();
        assert!(encode_ctx_literals(&data, &ctxs, 16, &mut enc));
        let mut r = lcg(7);
        for _ in 0..500 {
            let mut bad = enc.clone();
            let i = 1 + (r() as usize) % (bad.len() - 1);
            bad[i] ^= 1 << (r() % 8);
            let mut pos = 1;
            if let Ok(mut dec) = CtxLitDecoder::new(&bad, &mut pos, data.len()) {
                let mut prev = 0u8;
                for _ in 0..dec.remaining() + 5 {
                    prev = dec.next(prev);
                }
                let _ = dec.finish();
            }
        }
        for cut in [1, 2, 3, 10, enc.len() / 2] {
            let mut pos = 1;
            assert!(CtxLitDecoder::new(&enc[..cut], &mut pos, data.len()).is_err());
        }
    }
}
