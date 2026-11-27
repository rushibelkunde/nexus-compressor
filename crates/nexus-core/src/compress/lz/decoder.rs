//! LZ-ANS v2 sub-block decoder.
//!
//! Decodes the entropy streams into scratch buffers, then executes the
//! sequences directly into the superblock buffer, so matches into earlier
//! sub-blocks need no separate history copy.

use crate::compress::lz::{
    CODE_BASE, CODE_BITS, INITIAL_REPS, MIN_MATCH, NUM_REP_CODES, NUM_VALUE_CODES,
};
use crate::entropy::bitio::{read_varint, BitReader};
use crate::entropy::ctx_rans::{CtxLitDecoder, MODE_CTX};
use crate::entropy::rans4::decode_stream;
use crate::error::{NexusError, Result};

/// Bytes a wild copy may write past the requested length.
const WILD: usize = 16;

/// Reusable buffers for [`decode_block`], so decoding a superblock performs
/// no per-sub-block allocations after the first.
#[derive(Default)]
pub struct LzDecodeScratch {
    lits: Vec<u8>,
    ll: Vec<u8>,
    ml: Vec<u8>,
    of: Vec<u8>,
}

fn corrupt(msg: &str) -> NexusError {
    NexusError::DataCorruption(format!("LZ block: {msg}"))
}

/// Decodes one LZ-ANS v2 sub-block payload, appending exactly `out_len` bytes
/// to `buf`.
///
/// `buf` must already contain the preceding sub-blocks of the superblock
/// (the match window). On error `buf` is left with unspecified contents past
/// its original length.
///
/// # Errors
/// Returns `NexusError::DataCorruption` if the payload is malformed, refers
/// to data before the start of the window, or does not produce `out_len` bytes.
pub fn decode_block(
    payload: &[u8],
    buf: &mut Vec<u8>,
    out_len: usize,
    scratch: &mut LzDecodeScratch,
) -> Result<()> {
    let mut pos = 0usize;
    let nseq =
        read_varint(payload, &mut pos).ok_or_else(|| corrupt("missing sequence count"))? as usize;
    // Every sequence produces at least MIN_MATCH bytes.
    if nseq > out_len / MIN_MATCH {
        return Err(corrupt("sequence count exceeds block size"));
    }

    // Literals are either a plain stream decoded up front, or a context-coded
    // record decoded on demand inside the sequence loop.
    let mut ctx_lits = if payload.get(pos) == Some(&MODE_CTX) {
        pos += 1;
        scratch.lits.clear();
        Some(CtxLitDecoder::new(payload, &mut pos, out_len)?)
    } else {
        decode_stream(payload, &mut pos, &mut scratch.lits, out_len)?;
        None
    };
    let mut bit_bytes: &[u8] = &[];
    if nseq > 0 {
        decode_stream(payload, &mut pos, &mut scratch.ll, nseq)?;
        decode_stream(payload, &mut pos, &mut scratch.ml, nseq)?;
        decode_stream(payload, &mut pos, &mut scratch.of, nseq)?;
        if scratch.ll.len() != nseq || scratch.ml.len() != nseq || scratch.of.len() != nseq {
            return Err(corrupt("code stream length mismatch"));
        }
        let blen = read_varint(payload, &mut pos)
            .ok_or_else(|| corrupt("missing extra-bits length"))? as usize;
        let end = pos
            .checked_add(blen)
            .filter(|&e| e <= payload.len())
            .ok_or_else(|| corrupt("extra bits truncated"))?;
        bit_bytes = &payload[pos..end];
    }

    let lits = &scratch.lits[..];
    let start = buf.len();
    let end = start + out_len;
    buf.resize(end, 0);

    let mut bits = BitReader::new(bit_bytes);
    let mut reps = [
        INITIAL_REPS[0] as usize,
        INITIAL_REPS[1] as usize,
        INITIAL_REPS[2] as usize,
    ];
    let mut op = start;
    let mut lp = 0usize;

    for s in 0..nseq {
        let llc = scratch.ll[s] as usize;
        let mlc = scratch.ml[s] as usize;
        let ofc = scratch.of[s] as usize;
        if llc >= NUM_VALUE_CODES || mlc >= NUM_VALUE_CODES {
            return Err(corrupt("length code out of range"));
        }
        let ll = CODE_BASE[llc] as u64 + bits.get(CODE_BITS[llc] as u32) as u64;
        let ml = CODE_BASE[mlc] as u64 + bits.get(CODE_BITS[mlc] as u32) as u64 + MIN_MATCH as u64;

        let offset = match ofc {
            0 => reps[0],
            1 => {
                reps.swap(0, 1);
                reps[0]
            }
            2 => {
                reps = [reps[2], reps[0], reps[1]];
                reps[0]
            }
            _ => {
                let c = ofc - NUM_REP_CODES;
                if c >= NUM_VALUE_CODES {
                    return Err(corrupt("offset code out of range"));
                }
                let off = CODE_BASE[c] as u64 + bits.get(CODE_BITS[c] as u32) as u64 + 1;
                if off > end as u64 {
                    return Err(corrupt("offset beyond window"));
                }
                reps = [off as usize, reps[0], reps[1]];
                off as usize
            }
        };

        let lits_left = match &ctx_lits {
            Some(dec) => dec.remaining(),
            None => lits.len() - lp,
        };
        if ll > lits_left as u64 || ll + ml > (end - op) as u64 {
            return Err(corrupt("sequence overruns block"));
        }
        let ll = ll as usize;
        let ml = ml as usize;

        // Literal run.
        if let Some(dec) = ctx_lits.as_mut() {
            let mut prev = if op > 0 { buf[op - 1] } else { 0 };
            for slot in &mut buf[op..op + ll] {
                prev = dec.next(prev);
                *slot = prev;
            }
        } else if ll <= WILD && lp + WILD <= lits.len() && op + WILD <= end {
            // SAFETY: both 16-byte ranges were just bounds-checked; the
            // source (scratch) and destination (buf) never alias.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    lits.as_ptr().add(lp),
                    buf.as_mut_ptr().add(op),
                    WILD,
                );
            }
        } else {
            buf[op..op + ll].copy_from_slice(&lits[lp..lp + ll]);
        }
        if ctx_lits.is_none() {
            lp += ll;
        }
        op += ll;

        // Match copy.
        if offset > op {
            return Err(corrupt("match reaches before window start"));
        }
        let src = op - offset;
        if offset >= WILD && op + ml + WILD <= end {
            // SAFETY: `src + k + 16 <= op + k` because offset >= 16, and
            // `op + k + 16 <= op + ml + 16 <= end == buf.len()`, so every
            // 16-byte chunk is in bounds and source/destination chunks are
            // disjoint. Bytes written past `op + ml` are overwritten later.
            unsafe {
                let p = buf.as_mut_ptr();
                let mut k = 0;
                while k < ml {
                    std::ptr::copy_nonoverlapping(p.add(src + k), p.add(op + k), WILD);
                    k += WILD;
                }
            }
        } else if offset >= ml {
            buf.copy_within(src..src + ml, op);
        } else if offset == 1 {
            let b = buf[src];
            buf[op..op + ml].fill(b);
        } else {
            // Overlapping copy replicates the `offset`-byte pattern.
            for k in 0..ml {
                buf[op + k] = buf[src + k];
            }
        }
        op += ml;
    }

    let rem = end - op;
    if let Some(dec) = ctx_lits.as_mut() {
        if dec.remaining() != rem {
            return Err(corrupt("trailing literal count mismatch"));
        }
        let mut prev = if op > 0 { buf[op - 1] } else { 0 };
        for slot in &mut buf[op..end] {
            prev = dec.next(prev);
            *slot = prev;
        }
        dec.finish()?;
    } else {
        if lits.len() - lp != rem {
            return Err(corrupt("trailing literal count mismatch"));
        }
        buf[op..end].copy_from_slice(&lits[lp..]);
    }
    if bits.overrun() {
        return Err(corrupt("extra bits exhausted"));
    }
    Ok(())
}
