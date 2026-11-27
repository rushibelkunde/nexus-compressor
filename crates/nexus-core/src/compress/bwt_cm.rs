//! BWT with adaptive bit-level modelling (Path B2, Layer 3).
//!
//! The Burrows-Wheeler transform groups bytes by the context that follows
//! them, so its output consists of long, slowly drifting runs. Instead of the
//! v1 chain (move-to-front, zero-run coding, static order-0 rANS), which
//! throws most of that structure away, the transformed bytes are coded
//! directly with a small adaptive model:
//!
//! * three bit predictors per byte, addressed by the bits already seen of
//!   the current byte together with nothing, the previous byte, or the byte
//!   before that;
//! * a fixed blend of the three, refined by a secondary estimation stage
//!   keyed on whether the output is currently inside a run;
//! * a 16-bit binary range coder.
//!
//! The suffix array is built with SA-IS, so worst-case inputs (long runs)
//! cost the same as ordinary text.
//!
//! Every probability is an integer in `[1, 65535]` updated with shifts only:
//! no floating point anywhere, per AGENTS.md rule 2.
//!
//! Payload layout: `varint primary_index`, then the range-coded bytes.

use crate::compress::sais::suffix_array;
use crate::entropy::bitio::{read_varint, write_varint};
use crate::entropy::range_coder::{BinDecoder, BinEncoder};
use crate::error::{NexusError, Result};

/// Largest block this path accepts: positions must fit in 24 bits because the
/// inverse transform packs a byte and a link into one `u32`.
pub const MAX_BWT_BLOCK: usize = (1 << 24) - 1;

/// Adaptation rate (shift) of the order-0 predictor: fast, tracks local drift.
const RATE_ORDER0: u32 = 2;
/// Adaptation rate of the context predictors.
const RATE_CONTEXT: u32 = 4;
/// Adaptation rate of the secondary estimation stage.
const RATE_SSE: u32 = 6;
/// Interpolation knots of the secondary estimation stage.
const SSE_KNOTS: usize = 17;

/// Forward BWT over suffix order.
///
/// Returns the transformed bytes and the primary index needed to invert
/// them. `out[0]` is the last input byte (the row of the implicit
/// end-of-string suffix); the row of the whole string is omitted and its
/// rank is the primary index.
fn bwt_forward(data: &[u8]) -> (Vec<u8>, u32) {
    let n = data.len();
    let sa = suffix_array(data);
    let mut out = Vec::with_capacity(n);
    out.push(data[n - 1]);
    let mut primary = 0u32;
    for (i, &p) in sa.iter().enumerate() {
        if p == 0 {
            primary = i as u32 + 1;
        } else {
            out.push(data[p as usize - 1]);
        }
    }
    (out, primary)
}

/// Inverts [`bwt_forward`].
fn bwt_inverse(bwt: &[u8], primary: u32) -> Result<Vec<u8>> {
    let n = bwt.len();
    let primary = primary as usize;
    if n > MAX_BWT_BLOCK || primary == 0 || primary > n {
        return Err(NexusError::DataCorruption(
            "BWT primary index out of range".into(),
        ));
    }

    // First-column start of every byte value.
    let mut starts = [0u32; 256];
    for &b in bwt {
        starts[b as usize] += 1;
    }
    let mut sum = 0u32;
    for s in starts.iter_mut() {
        let count = *s;
        *s = sum;
        sum += count;
    }

    // links[r] = (first-column byte of row r) << 24 | next row + 1. Rows at
    // and after the primary index are shifted by one to account for the
    // omitted row of the whole string.
    let mut links = vec![0u32; n];
    for (i, &b) in bwt.iter().enumerate() {
        let slot = &mut starts[b as usize];
        let row = if i < primary { i } else { i + 1 };
        links[*slot as usize] = ((b as u32) << 24) | row as u32;
        *slot += 1;
    }

    let mut out = vec![0u8; n];
    let mut p = primary;
    for slot in out.iter_mut() {
        // `p` is always in 1..=n for well-formed input; corrupt input is
        // caught by the bounds check rather than trusted.
        let link = *links
            .get(p.wrapping_sub(1))
            .ok_or_else(|| NexusError::DataCorruption("BWT link chain left the block".into()))?;
        *slot = (link >> 24) as u8;
        p = (link & 0x00FF_FFFF) as usize;
    }
    Ok(out)
}

/// Moves a 16-bit probability toward the observed bit.
#[inline(always)]
fn adapt(p: &mut u16, bit: u32, rate: u32) {
    if bit != 0 {
        *p += (65535 - *p) >> rate;
    } else {
        *p -= *p >> rate;
    }
}

/// Adaptive model for BWT output. Encoder and decoder drive identical
/// instances, so their predictions always agree.
struct BwtModel {
    /// Order-0: indexed by the partial-byte context (1..=255).
    order0: [u16; 256],
    /// Indexed by `previous byte << 8 | partial-byte context`.
    order1: Vec<u16>,
    /// Indexed by `byte before previous << 8 | partial-byte context`.
    order2: Vec<u16>,
    /// Secondary estimation: `(in_run << 8 | partial ctx) * SSE_KNOTS + knot`.
    sse: Vec<u16>,
    c1: usize,
    c2: usize,
    run: u32,
}

/// Per-bit prediction state handed from `predict` to `update`.
struct Prediction {
    /// Final probability of a 1 bit, in `1..=65535`.
    p: u32,
    /// Index of the lower SSE knot used.
    sse_index: usize,
}

impl BwtModel {
    fn new() -> Self {
        let mut sse = vec![0u16; 2 * 256 * SSE_KNOTS];
        for (i, v) in sse.iter_mut().enumerate() {
            // Start as the identity mapping.
            *v = ((i % SSE_KNOTS) * 4096).min(65535) as u16;
        }
        Self {
            order0: [32768; 256],
            order1: vec![32768; 256 * 256],
            order2: vec![32768; 256 * 256],
            sse,
            c1: 0,
            c2: 0,
            run: 0,
        }
    }

    #[inline(always)]
    fn predict(&self, ctx: usize) -> Prediction {
        let p0 = self.order0[ctx] as u32;
        let p1 = self.order1[(self.c1 << 8) | ctx] as u32;
        let p2 = self.order2[(self.c2 << 8) | ctx] as u32;
        let mixed = ((p0 + p1) * 7 + p2 * 2) >> 4;

        // Refine with a piecewise-linear map learned per (run state, context).
        let in_run = (self.run > 2) as usize;
        let sse_index = ((in_run << 8) | ctx) * SSE_KNOTS + (mixed >> 12) as usize;
        let lo = self.sse[sse_index] as i32;
        let hi = self.sse[sse_index + 1] as i32;
        let refined = lo + (((hi - lo) * (mixed & 4095) as i32) >> 12);

        let p = ((mixed as i32 + 3 * refined) >> 2).clamp(1, 65535) as u32;
        Prediction { p, sse_index }
    }

    #[inline(always)]
    fn update(&mut self, ctx: usize, bit: u32, pred: &Prediction) {
        adapt(&mut self.order0[ctx], bit, RATE_ORDER0);
        adapt(&mut self.order1[(self.c1 << 8) | ctx], bit, RATE_CONTEXT);
        adapt(&mut self.order2[(self.c2 << 8) | ctx], bit, RATE_CONTEXT);
        adapt(&mut self.sse[pred.sse_index], bit, RATE_SSE);
        adapt(&mut self.sse[pred.sse_index + 1], bit, RATE_SSE);
    }

    #[inline(always)]
    fn end_byte(&mut self, byte: u8) {
        let b = byte as usize;
        self.run = if b == self.c1 { self.run + 1 } else { 0 };
        self.c2 = self.c1;
        self.c1 = b;
    }
}

/// Compresses `data` with BWT + adaptive modelling.
///
/// Returns `None` if the block is empty or larger than [`MAX_BWT_BLOCK`].
pub fn bwt_cm_compress(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data.len() > MAX_BWT_BLOCK {
        return None;
    }
    let (bwt, primary) = bwt_forward(data);

    let mut header = Vec::with_capacity(data.len() / 3 + 16);
    write_varint(&mut header, primary as u64);
    let mut enc = BinEncoder::new(header);
    let mut model = BwtModel::new();
    for &byte in &bwt {
        let mut ctx = 1usize;
        for shift in (0..8).rev() {
            let bit = ((byte >> shift) & 1) as u32;
            let pred = model.predict(ctx);
            enc.encode(bit, pred.p);
            model.update(ctx, bit, &pred);
            ctx = (ctx << 1) | bit as usize;
        }
        model.end_byte(byte);
    }
    Some(enc.finish())
}

/// Decompresses a payload produced by [`bwt_cm_compress`] into `n` bytes.
///
/// # Errors
/// Returns `NexusError::DataCorruption` if the payload is malformed. Wrong
/// (but well-formed) bytes are caught by the sub-block checksum one layer up.
pub fn bwt_cm_decompress(payload: &[u8], n: usize) -> Result<Vec<u8>> {
    if n == 0 || n > MAX_BWT_BLOCK {
        return Err(NexusError::DataCorruption(
            "BWT block size out of range".into(),
        ));
    }
    let mut pos = 0usize;
    let primary = read_varint(payload, &mut pos)
        .filter(|&p| p >= 1 && p <= n as u64)
        .ok_or_else(|| NexusError::DataCorruption("BWT primary index invalid".into()))?
        as u32;

    let mut dec = BinDecoder::new(&payload[pos..]);
    let mut model = BwtModel::new();
    let mut bwt = vec![0u8; n];
    for slot in bwt.iter_mut() {
        let mut ctx = 1usize;
        for _ in 0..8 {
            let pred = model.predict(ctx);
            let bit = dec.decode(pred.p);
            model.update(ctx, bit, &pred);
            ctx = (ctx << 1) | bit as usize;
        }
        let byte = ctx as u8;
        *slot = byte;
        model.end_byte(byte);
    }
    bwt_inverse(&bwt, primary)
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

    fn roundtrip(data: &[u8]) -> usize {
        let payload = bwt_cm_compress(data).expect("non-empty block");
        let restored = bwt_cm_decompress(&payload, data.len()).unwrap();
        assert_eq!(restored, data);
        payload.len()
    }

    #[test]
    fn test_bwt_transform_roundtrip_small_cases() {
        for data in [
            &b"a"[..],
            b"ab",
            b"aa",
            b"banana",
            b"mississippi",
            b"abracadabra abracadabra",
            &[0u8, 0, 0, 0],
            &[255u8, 0, 255, 0, 255],
        ] {
            let (bwt, primary) = bwt_forward(data);
            assert_eq!(bwt.len(), data.len());
            assert_eq!(bwt_inverse(&bwt, primary).unwrap(), data);
        }
    }

    #[test]
    fn test_bwt_transform_roundtrip_random() {
        let mut r = lcg(21);
        for round in 0..200 {
            let n = 1 + (r() % 600) as usize;
            let alphabet = [2, 3, 5, 26, 256][round % 5];
            let data: Vec<u8> = (0..n).map(|_| (r() % alphabet) as u8).collect();
            let (bwt, primary) = bwt_forward(&data);
            assert_eq!(bwt_inverse(&bwt, primary).unwrap(), data, "round {round}");
        }
    }

    #[test]
    fn test_bwt_cm_roundtrip_edge_cases() {
        assert!(bwt_cm_compress(b"").is_none());
        roundtrip(b"x");
        roundtrip(b"xy");
        assert!(roundtrip(&vec![0u8; 200_000]) < 200);
        let mut r = lcg(22);
        let random: Vec<u8> = (0..50_000).map(|_| r() as u8).collect();
        // Incompressible data may expand slightly but must survive.
        assert!(roundtrip(&random) < 52_000);
    }

    #[test]
    fn test_bwt_cm_compresses_text_well() {
        let words: [&[u8]; 10] = [
            b"the ",
            b"quick ",
            b"brown ",
            b"fox ",
            b"jumps ",
            b"over ",
            b"lazy ",
            b"dog ",
            b"compression ",
            b"window\n",
        ];
        let mut r = lcg(23);
        let mut text = Vec::new();
        while text.len() < 300_000 {
            text.extend_from_slice(words[(r() % 10) as usize]);
        }
        let size = roundtrip(&text);
        // Ten equiprobable words carry log2(10) = 3.32 bits per ~6-byte word.
        // A good model should land close to that bound (about 20.5 KB).
        assert!(size < 24_000, "got {size}");
    }

    #[test]
    fn test_bwt_cm_corrupt_payload_never_panics() {
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(200);
        let payload = bwt_cm_compress(&data).unwrap();
        let mut r = lcg(24);
        for _ in 0..300 {
            let mut bad = payload.clone();
            let i = (r() as usize) % bad.len();
            bad[i] ^= 1 << (r() % 8);
            let _ = bwt_cm_decompress(&bad, data.len());
        }
        for cut in [0, 1, 2, payload.len() / 2] {
            let _ = bwt_cm_decompress(&payload[..cut], data.len());
        }
        assert!(bwt_cm_decompress(&payload, 0).is_err());
    }
}
