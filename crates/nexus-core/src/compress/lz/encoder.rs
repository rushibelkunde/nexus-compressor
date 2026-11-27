//! LZ-ANS v2 encoder: match finders, greedy/lazy parsers and block writer.
//!
//! One [`LzEncoder`] lives for a whole superblock. Its hash tables index the
//! superblock buffer by absolute position, so the dictionary stays warm
//! across sub-blocks without ever being rebuilt (AGENTS.md rule 4), while the
//! sequence store, entropy tables and repeat offsets are reset per sub-block.

use crate::compress::lz::optimal::OptState;
use crate::compress::lz::{value_code, LzParams, Strategy, INITIAL_REPS, MIN_MATCH, NUM_REP_CODES};
use crate::entropy::bitio::{write_varint, BitWriter};
use crate::entropy::ctx_rans::encode_ctx_literals;
use crate::entropy::rans4::encode_stream;

const PRIME32: u32 = 2_654_435_761;
const PRIME64: u64 = 0x9E37_79B1_85EB_CA87;

/// log2 of the 3-byte hash table used by the optimal parser.
pub(super) const HASH3_LOG: u32 = 16;
/// Literal runs up to this length are copied with one fixed-size move.
const WILD: usize = 16;
/// Candidates per row of the row-based match table.
const ROW_ENTRIES: usize = 16;

/// Byte-wise equality mask: bit 7 of every byte of `x` that equals `b`.
#[inline(always)]
fn swar_eq(x: u64, b: u8) -> u64 {
    const LO7: u64 = 0x7F7F_7F7F_7F7F_7F7F;
    let y = x ^ (0x0101_0101_0101_0101u64.wrapping_mul(b as u64));
    // Exact zero-byte detector (no false positives from borrow propagation).
    !(((y & LO7).wrapping_add(LO7)) | y | LO7)
}

/// Reads a little-endian `u32` at `p`.
#[inline(always)]
pub(super) fn read32(buf: &[u8], p: usize) -> u32 {
    debug_assert!(p + 4 <= buf.len());
    // SAFETY: every caller guarantees `p + 4 <= buf.len()` (asserted above in
    // debug builds); search loops stop 8 bytes before the block end.
    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(p) as *const u32).to_le() }
}

/// Reads a little-endian `u64` at `p`.
#[inline(always)]
pub(super) fn read64(buf: &[u8], p: usize) -> u64 {
    debug_assert!(p + 8 <= buf.len());
    // SAFETY: every caller guarantees `p + 8 <= buf.len()`.
    unsafe { std::ptr::read_unaligned(buf.as_ptr().add(p) as *const u64).to_le() }
}

/// Length of the common prefix of `buf[a..]` and `buf[b..limit]` (`a < b`).
#[inline(always)]
pub(super) fn match_len(buf: &[u8], a: usize, b: usize, limit: usize) -> usize {
    debug_assert!(a < b && b <= limit && limit <= buf.len());
    let mut len = 0;
    while b + len + 8 <= limit {
        let x = read64(buf, a + len) ^ read64(buf, b + len);
        if x != 0 {
            return len + (x.trailing_zeros() >> 3) as usize;
        }
        len += 8;
    }
    while b + len < limit && buf[a + len] == buf[b + len] {
        len += 1;
    }
    len
}

#[inline(always)]
pub(super) fn hash4(v: u32, log: u32) -> usize {
    (v.wrapping_mul(PRIME32) >> (32 - log)) as usize
}

#[inline(always)]
pub(super) fn hash3(v: u32, log: u32) -> usize {
    ((v << 8).wrapping_mul(PRIME32) >> (32 - log)) as usize
}

/// Hashes the low `bytes` bytes (4..=8) of `v` to `log` bits.
#[inline(always)]
fn hash_bytes(v: u64, bytes: u32, log: u32) -> usize {
    if bytes <= 4 {
        hash4(v as u32, log)
    } else {
        ((v << (64 - 8 * bytes)).wrapping_mul(PRIME64) >> (64 - log)) as usize
    }
}

/// Accumulates the sequences of one sub-block as separate code streams.
pub(super) struct SeqStore {
    pub(super) lits: Vec<u8>,
    /// For each literal, the byte preceding it in the output (only filled
    /// when `lit_classes > 0`).
    ctx: Vec<u8>,
    /// Maximum literal context classes; 0 disables context coding.
    lit_classes: usize,
    pub(super) ll: Vec<u8>,
    pub(super) ml: Vec<u8>,
    pub(super) of: Vec<u8>,
    bits: BitWriter,
    /// Current repeat offsets (most recent first).
    pub(super) reps: [u32; 3],
}

impl SeqStore {
    fn new(lit_classes: usize) -> Self {
        Self {
            lits: Vec::new(),
            ctx: Vec::new(),
            lit_classes,
            ll: Vec::new(),
            ml: Vec::new(),
            of: Vec::new(),
            bits: BitWriter::new(),
            reps: INITIAL_REPS,
        }
    }

    /// Empties the store and guarantees room for a block of `block_len`
    /// bytes, which is what lets [`Self::emit`] skip capacity checks.
    pub(super) fn prepare(&mut self, block_len: usize) {
        self.lits.clear();
        self.ctx.clear();
        self.ll.clear();
        self.ml.clear();
        self.of.clear();
        self.bits.clear();
        self.reps = INITIAL_REPS;
        // Literals never exceed the block; `WILD` covers the over-copy.
        self.lits.reserve(block_len + 2 * WILD);
        // Every sequence consumes at least MIN_MATCH bytes of the block.
        let max_seq = block_len / MIN_MATCH + 1;
        self.ll.reserve(max_seq);
        self.ml.reserve(max_seq);
        self.of.reserve(max_seq);
    }

    /// Appends one sequence: the literals `buf[anchor..mstart]` followed by a
    /// `match_len`-byte copy from `offset` bytes back. Offsets equal to a
    /// repeat offset are emitted as repeat codes automatically.
    ///
    /// Must only be called for sequences inside the block passed to
    /// [`Self::prepare`], in increasing position order.
    #[inline(always)]
    pub(super) fn emit(
        &mut self,
        buf: &[u8],
        anchor: usize,
        mstart: usize,
        match_len: usize,
        offset: usize,
    ) {
        debug_assert!(match_len >= MIN_MATCH && offset >= 1 && anchor <= mstart);
        let ll = mstart - anchor;
        let n = self.lits.len();
        debug_assert!(n + ll + WILD <= self.lits.capacity());
        debug_assert!(self.ll.len() < self.ll.capacity());
        if self.lit_classes > 0 && ll > 0 {
            self.push_contexts(buf, anchor, mstart);
        }
        // SAFETY: `prepare` reserved `block_len + 32` literal bytes and
        // `block_len / 3 + 1` slots per code stream. Sequences cover disjoint
        // parts of the block, so the running totals stay within those bounds
        // (checked by the debug assertions above). The 16-byte over-copy
        // reads within `buf` (explicitly checked) and writes within the
        // reserved slack.
        unsafe {
            let dst = self.lits.as_mut_ptr().add(n);
            if ll <= WILD && anchor + WILD <= buf.len() {
                std::ptr::copy_nonoverlapping(buf.as_ptr().add(anchor), dst, WILD);
            } else {
                std::ptr::copy_nonoverlapping(buf.as_ptr().add(anchor), dst, ll);
            }
            self.lits.set_len(n + ll);

            let s = self.ll.len();
            let (c, nb, ex) = value_code(ll as u32);
            *self.ll.as_mut_ptr().add(s) = c;
            self.ll.set_len(s + 1);
            self.bits.put(ex, nb);

            let (c, nb, ex) = value_code((match_len - MIN_MATCH) as u32);
            *self.ml.as_mut_ptr().add(s) = c;
            self.ml.set_len(s + 1);
            self.bits.put(ex, nb);

            let off = offset as u32;
            let r = self.reps;
            let code = if off == r[0] {
                0
            } else if off == r[1] {
                self.reps = [r[1], r[0], r[2]];
                1
            } else if off == r[2] {
                self.reps = [r[2], r[0], r[1]];
                2
            } else {
                let (c, nb, ex) = value_code(off - 1);
                self.bits.put(ex, nb);
                self.reps = [off, r[0], r[1]];
                NUM_REP_CODES as u8 + c
            };
            *self.of.as_mut_ptr().add(s) = code;
            self.of.set_len(s + 1);
        }
    }

    /// Records the context (preceding output byte) of each literal in
    /// `buf[from..to]`.
    #[inline]
    fn push_contexts(&mut self, buf: &[u8], from: usize, to: usize) {
        if from == to {
            return;
        }
        if from == 0 {
            // The very first byte of a superblock has no predecessor.
            self.ctx.push(0);
            self.ctx.extend_from_slice(&buf[..to - 1]);
        } else {
            self.ctx.extend_from_slice(&buf[from - 1..to - 1]);
        }
    }

    /// Appends the literals that follow the last sequence of the block.
    pub(super) fn push_tail(&mut self, buf: &[u8], anchor: usize) {
        self.lits.extend_from_slice(&buf[anchor..]);
        if self.lit_classes > 0 {
            self.push_contexts(buf, anchor, buf.len());
        }
    }

    /// Serialises the accumulated block (see the module docs for the layout).
    fn write(&mut self, out: &mut Vec<u8>) {
        let nseq = self.ll.len();
        write_varint(out, nseq as u64);
        let ctx_coded = self.lit_classes > 0
            && self.ctx.len() == self.lits.len()
            && encode_ctx_literals(&self.lits, &self.ctx, self.lit_classes, out);
        if !ctx_coded {
            encode_stream(&self.lits, out);
        }
        if nseq > 0 {
            encode_stream(&self.ll, out);
            encode_stream(&self.ml, out);
            encode_stream(&self.of, out);
            let bits = self.bits.finish();
            write_varint(out, bits.len() as u64);
            out.extend_from_slice(bits);
        }
    }
}

/// LZ-ANS v2 encoder with a match-finder that persists across the sub-blocks
/// of one superblock.
pub struct LzEncoder {
    pub(super) params: LzParams,
    /// Effective hash table size (log2), clamped to the buffer size.
    pub(super) hash_log: u32,
    /// Head table: most recent position + 1 per hash (0 = empty).
    pub(super) hash: Vec<u32>,
    /// Lazy strategy: hash chains (previous position + 1 with the same hash,
    /// per position). Optimal strategy: binary-tree child links, two per
    /// position.
    pub(super) chain: Vec<u32>,
    /// 3-byte head table (optimal parser) or 8-byte head table (double-fast).
    pub(super) hash3: Vec<u32>,
    /// log2 size of the long-hash table used by the double-fast strategy.
    long_log: u32,
    /// Row table (`LazyRow`): `ROW_ENTRIES` positions (+1) per row.
    row_pos: Vec<u32>,
    /// Row table: one tag byte (extra hash bits) per stored position.
    row_tag: Vec<u8>,
    /// Row table: index of the most recently written slot of each row.
    row_head: Vec<u8>,
    /// First position not yet inserted into the chain tables.
    pub(super) next_insert: usize,
    pub(super) store: SeqStore,
    pub(super) opt: OptState,
}

impl LzEncoder {
    /// Creates an encoder for a superblock of at most `capacity` bytes.
    pub fn new(params: LzParams, capacity: usize) -> Self {
        let cap_log = (usize::BITS - capacity.max(1).leading_zeros()).max(8);
        let hash_log = if params.strategy == Strategy::LazyRow {
            // Rows hold 16 positions each: size the table for ~1 slot/byte.
            params.hash_log.min(cap_log.saturating_sub(4)).max(6)
        } else {
            params.hash_log.min(cap_log).max(8)
        };
        let long_log = (params.hash_log + 1).min(cap_log).max(8);
        Self {
            params,
            hash_log,
            hash: if params.strategy == Strategy::LazyRow {
                Vec::new()
            } else {
                vec![0u32; 1 << hash_log]
            },
            row_pos: if params.strategy == Strategy::LazyRow {
                vec![0u32; ROW_ENTRIES << hash_log]
            } else {
                Vec::new()
            },
            row_tag: if params.strategy == Strategy::LazyRow {
                vec![0u8; ROW_ENTRIES << hash_log]
            } else {
                Vec::new()
            },
            row_head: if params.strategy == Strategy::LazyRow {
                vec![0u8; 1 << hash_log]
            } else {
                Vec::new()
            },
            chain: match params.strategy {
                Strategy::Lazy => vec![0u32; capacity],
                // Binary tree: two child links per position.
                Strategy::Optimal => vec![0u32; capacity * 2],
                _ => Vec::new(),
            },
            hash3: match params.strategy {
                Strategy::Optimal => vec![0u32; 1 << HASH3_LOG],
                Strategy::DoubleFast => vec![0u32; 1 << long_log],
                _ => Vec::new(),
            },
            long_log,
            next_insert: 0,
            store: SeqStore::new(params.lit_classes as usize),
            opt: OptState::default(),
        }
    }

    fn parse(&mut self, buf: &[u8], start: usize, end: usize) {
        debug_assert!(start <= end && end <= buf.len());
        self.store.prepare(end - start);
        let block = &buf[..end];
        match self.params.strategy {
            Strategy::Fast => self.parse_fast(block, start),
            Strategy::DoubleFast => self.parse_double_fast(block, start),
            Strategy::Lazy | Strategy::LazyRow => self.parse_lazy(block, start),
            Strategy::Optimal => self.parse_optimal(buf, start, end),
        }
    }

    /// Compresses `buf[start..end]` as one sub-block, appending the payload
    /// to `out`. `buf[..start]` is the already-encoded window.
    ///
    /// `buf` should be the whole superblock (in the preprocessed domain),
    /// including the sub-blocks after `end`: they are never referenced by
    /// this block's matches, but the same `buf` contents must be presented
    /// on every call so the match finder's index stays consistent.
    ///
    /// The payload may be larger than the input for incompressible data; the
    /// caller is expected to compare sizes and fall back to storing.
    pub fn compress_block(&mut self, buf: &[u8], start: usize, end: usize, out: &mut Vec<u8>) {
        self.parse(buf, start, end);
        self.store.write(out);
    }

    /// Runs only the match search and parse for `buf[start..end]`, without
    /// entropy coding. Exposed for the benchmark profiler.
    #[doc(hidden)]
    pub fn parse_only(&mut self, buf: &[u8], start: usize, end: usize) -> usize {
        self.parse(buf, start, end);
        self.store.ll.len()
    }

    /// Tells the encoder that `buf[..end]` was handled without it (stored or
    /// coded by another path). Those positions are simply never offered as
    /// match candidates.
    pub fn skip_block(&mut self, end: usize) {
        self.next_insert = self.next_insert.max(end);
    }

    /// Greedy parse with a single hash probe per position.
    fn parse_fast(&mut self, buf: &[u8], start: usize) {
        let end = buf.len();
        let log = self.hash_log;
        let hbytes = self.params.hash_bytes;
        let table = &mut self.hash[..];
        let store = &mut self.store;
        debug_assert_eq!(table.len(), 1 << log);

        let mut anchor = start;
        if end - start >= 16 {
            // Positions below `ilimit` allow 8-byte reads at ip + 1.
            let ilimit = end - 9;
            let mut ip = start;
            while ip < ilimit {
                let v = read64(buf, ip);
                let h = hash_bytes(v, hbytes, log);
                // SAFETY: `hash_bytes` returns a value below `1 << log`,
                // which is the table length (asserted above).
                let cand = unsafe {
                    let slot = table.get_unchecked_mut(h);
                    let c = *slot as usize;
                    *slot = (ip + 1) as u32;
                    c
                };

                let rep0 = store.reps[0] as usize;
                let (mut mstart, mut mlen, offset);
                if rep0 <= ip + 1 && read32(buf, ip + 1) == read32(buf, ip + 1 - rep0) {
                    mstart = ip + 1;
                    mlen = 4 + match_len(buf, mstart + 4 - rep0, mstart + 4, end);
                    offset = rep0;
                } else if cand != 0 && read32(buf, cand - 1) == v as u32 {
                    let c = cand - 1;
                    mstart = ip;
                    mlen = 4 + match_len(buf, c + 4, ip + 4, end);
                    offset = ip - c;
                } else {
                    // Accelerate through incompressible regions.
                    ip += 1 + ((ip - anchor) >> 6);
                    continue;
                }
                // Extend backwards over bytes that were emitted as literals.
                while mstart > anchor
                    && mstart > offset
                    && buf[mstart - 1] == buf[mstart - 1 - offset]
                {
                    mstart -= 1;
                    mlen += 1;
                }
                store.emit(buf, anchor, mstart, mlen, offset);
                ip = mstart + mlen;
                anchor = ip;

                if ip < ilimit {
                    // Index a position inside the match so the next occurrence
                    // of this context can be found.
                    table[hash_bytes(read64(buf, ip - 2), hbytes, log)] = (ip - 1) as u32;
                    // Immediate repeat of the second-most-recent offset.
                    while ip < ilimit {
                        let rep1 = store.reps[1] as usize;
                        if rep1 > ip || read32(buf, ip) != read32(buf, ip - rep1) {
                            break;
                        }
                        let l = 4 + match_len(buf, ip + 4 - rep1, ip + 4, end);
                        table[hash_bytes(read64(buf, ip), hbytes, log)] = (ip + 1) as u32;
                        store.emit(buf, ip, ip, l, rep1);
                        ip += l;
                        anchor = ip;
                    }
                }
            }
        }
        store.push_tail(buf, anchor);
    }

    /// Greedy parse with two hash probes per position: an 8-byte hash that
    /// finds long matches and a short hash that catches the rest.
    fn parse_double_fast(&mut self, buf: &[u8], start: usize) {
        let end = buf.len();
        let slog = self.hash_log;
        let llog = self.long_log;
        let sbytes = self.params.hash_bytes;
        let short = &mut self.hash[..];
        let long = &mut self.hash3[..];
        let store = &mut self.store;

        let mut anchor = start;
        if end - start >= 16 {
            let ilimit = end - 9;
            let mut ip = start;
            while ip < ilimit {
                let v = read64(buf, ip);
                let hs = hash_bytes(v, sbytes, slog);
                let hl = hash_bytes(v, 8, llog);
                let cs = short[hs] as usize;
                let cl = long[hl] as usize;
                short[hs] = (ip + 1) as u32;
                long[hl] = (ip + 1) as u32;

                let rep0 = store.reps[0] as usize;
                let (mut mstart, mut mlen, offset);
                if rep0 <= ip + 1 && read32(buf, ip + 1) == read32(buf, ip + 1 - rep0) {
                    mstart = ip + 1;
                    mlen = 4 + match_len(buf, mstart + 4 - rep0, mstart + 4, end);
                    offset = rep0;
                } else if cl != 0 && read64(buf, cl - 1) == v {
                    let c = cl - 1;
                    mstart = ip;
                    mlen = 8 + match_len(buf, c + 8, ip + 8, end);
                    offset = ip - c;
                } else if cs != 0 && read32(buf, cs - 1) == v as u32 {
                    // A short match is in hand; a long match starting one
                    // byte later is usually the better deal.
                    let v1 = read64(buf, ip + 1);
                    let hl1 = hash_bytes(v1, 8, llog);
                    let cl1 = long[hl1] as usize;
                    long[hl1] = (ip + 2) as u32;
                    if cl1 != 0 && read64(buf, cl1 - 1) == v1 {
                        let c = cl1 - 1;
                        mstart = ip + 1;
                        mlen = 8 + match_len(buf, c + 8, ip + 9, end);
                        offset = mstart - c;
                    } else {
                        let c = cs - 1;
                        mstart = ip;
                        mlen = 4 + match_len(buf, c + 4, ip + 4, end);
                        offset = ip - c;
                    }
                } else {
                    ip += 1 + ((ip - anchor) >> 8);
                    continue;
                }
                while mstart > anchor
                    && mstart > offset
                    && buf[mstart - 1] == buf[mstart - 1 - offset]
                {
                    mstart -= 1;
                    mlen += 1;
                }
                store.emit(buf, anchor, mstart, mlen, offset);
                let mbegin = mstart;
                ip = mstart + mlen;
                anchor = ip;

                if ip < ilimit {
                    // Index two positions inside the match.
                    let p = mbegin + 2;
                    let vp = read64(buf, p);
                    long[hash_bytes(vp, 8, llog)] = (p + 1) as u32;
                    short[hash_bytes(vp, sbytes, slog)] = (p + 1) as u32;
                    let q = ip - 2;
                    let vq = read64(buf, q);
                    long[hash_bytes(vq, 8, llog)] = (q + 1) as u32;
                    short[hash_bytes(vq, sbytes, slog)] = (q + 1) as u32;

                    while ip < ilimit {
                        let rep1 = store.reps[1] as usize;
                        if rep1 > ip || read32(buf, ip) != read32(buf, ip - rep1) {
                            break;
                        }
                        let l = 4 + match_len(buf, ip + 4 - rep1, ip + 4, end);
                        let vi = read64(buf, ip);
                        short[hash_bytes(vi, sbytes, slog)] = (ip + 1) as u32;
                        long[hash_bytes(vi, 8, llog)] = (ip + 1) as u32;
                        store.emit(buf, ip, ip, l, rep1);
                        ip += l;
                        anchor = ip;
                    }
                }
            }
        }
        store.push_tail(buf, anchor);
    }

    /// Row and tag of the `hash_bytes`-byte sequence `v`.
    #[inline(always)]
    fn row_of(&self, v: u64) -> (usize, u8) {
        let bytes = self.params.hash_bytes.clamp(4, 8);
        let h = (v << (64 - 8 * bytes)).wrapping_mul(PRIME64);
        let log = self.hash_log;
        ((h >> (64 - log)) as usize, (h >> (56 - log)) as u8)
    }

    /// Stores position `p` in its row, overwriting the oldest entry.
    #[inline(always)]
    fn row_insert(&mut self, buf: &[u8], p: usize) {
        let (row, tag) = self.row_of(read64(buf, p));
        let head = (self.row_head[row] as usize + ROW_ENTRIES - 1) & (ROW_ENTRIES - 1);
        self.row_head[row] = head as u8;
        let slot = row * ROW_ENTRIES + head;
        self.row_pos[slot] = (p + 1) as u32;
        self.row_tag[slot] = tag;
    }

    /// Row-table counterpart of [`Self::search`]: only candidates whose tag
    /// matches are dereferenced, so hash collisions cost no cache miss.
    ///
    /// Requires `ip + 8 <= buf.len()`.
    #[inline(always)]
    fn search_row(&mut self, buf: &[u8], ip: usize) -> (usize, usize) {
        let end = buf.len();
        let v = read64(buf, ip);
        let cur = v as u32;
        let mut best_len = 0usize;
        let mut best_off = 0usize;
        let mut best_gain = 0i32;

        for (r, &rep) in self.store.reps.iter().enumerate() {
            let rep = rep as usize;
            if rep <= ip && (read32(buf, ip - rep) ^ cur) & 0x00FF_FFFF == 0 {
                let len = 3 + match_len(buf, ip - rep + 3, ip + 3, end);
                let gain = len as i32 * 4 - r as i32;
                if gain > best_gain {
                    best_gain = gain;
                    best_len = len;
                    best_off = rep;
                }
            }
        }

        // Catch up on positions skipped over by the previous match.
        let limit = end.saturating_sub(7).min(ip);
        let mut p = self.next_insert;
        while p < limit {
            self.row_insert(buf, p);
            p += 1;
        }
        self.next_insert = self.next_insert.max(p);

        let (row, tag) = self.row_of(v);
        let base = row * ROW_ENTRIES;
        let tags = &self.row_tag[base..base + ROW_ENTRIES];
        let mut lo = [0u8; 8];
        let mut hi = [0u8; 8];
        lo.copy_from_slice(&tags[..8]);
        hi.copy_from_slice(&tags[8..]);
        let mut hits = [
            swar_eq(u64::from_le_bytes(lo), tag),
            swar_eq(u64::from_le_bytes(hi), tag),
        ];

        let nice = self.params.nice_len as usize;
        let mut row_len = 3usize;
        'rows: for (half, m) in hits.iter_mut().enumerate() {
            while *m != 0 {
                let slot = base + half * 8 + (m.trailing_zeros() >> 3) as usize;
                *m &= *m - 1;
                let stored = self.row_pos[slot] as usize;
                if stored == 0 || stored > ip {
                    continue;
                }
                let c = stored - 1;
                if ip + row_len >= end {
                    break 'rows;
                }
                if buf[c + row_len] == buf[ip + row_len] && read32(buf, c) == cur {
                    let len = 4 + match_len(buf, c + 4, ip + 4, end);
                    let off = ip - c;
                    let gain = len as i32 * 4 - (32 - (off as u32 + 3).leading_zeros()) as i32;
                    if gain > best_gain && (len > 4 || off < (1 << 16)) {
                        best_gain = gain;
                        best_len = len;
                        best_off = off;
                    }
                    if len > row_len {
                        row_len = len;
                    }
                    if len >= nice {
                        break 'rows;
                    }
                }
            }
        }

        if self.next_insert == ip {
            self.row_insert(buf, ip);
            self.next_insert = ip + 1;
        }
        (best_len, best_off)
    }

    /// Inserts every not-yet-indexed position below `target` into the hash
    /// chains (positions need 8 readable bytes).
    #[inline(always)]
    pub(super) fn insert_upto(&mut self, buf: &[u8], target: usize) {
        let limit = buf.len().saturating_sub(7).min(target);
        let log = self.hash_log;
        let hbytes = self.params.hash_bytes;
        let mut p = self.next_insert;
        while p < limit {
            let h = hash_bytes(read64(buf, p), hbytes, log);
            self.chain[p] = self.hash[h];
            self.hash[h] = (p + 1) as u32;
            p += 1;
        }
        self.next_insert = self.next_insert.max(p);
    }

    /// Finds the best match at `ip` among the repeat offsets and the hash
    /// chain. Returns `(length, offset)`, with length 0 if nothing useful.
    ///
    /// Requires `ip + 8 <= buf.len()`.
    #[inline(always)]
    fn search(&mut self, buf: &[u8], ip: usize) -> (usize, usize) {
        if self.params.strategy == Strategy::LazyRow {
            return self.search_row(buf, ip);
        }
        let end = buf.len();
        let v = read64(buf, ip);
        let cur = v as u32;
        let mut best_len = 0usize;
        let mut best_off = 0usize;
        let mut best_gain = 0i32;

        // Repeat offsets: nearly free to code, so a 3-byte match already pays.
        for (r, &rep) in self.store.reps.iter().enumerate() {
            let rep = rep as usize;
            if rep <= ip && (read32(buf, ip - rep) ^ cur) & 0x00FF_FFFF == 0 {
                let len = 3 + match_len(buf, ip - rep + 3, ip + 3, end);
                let gain = len as i32 * 4 - r as i32;
                if gain > best_gain {
                    best_gain = gain;
                    best_len = len;
                    best_off = rep;
                }
            }
        }

        self.insert_upto(buf, ip);
        let h = hash_bytes(v, self.params.hash_bytes, self.hash_log);
        let mut cand = self.hash[h];
        if self.next_insert == ip {
            self.chain[ip] = cand;
            self.hash[h] = (ip + 1) as u32;
            self.next_insert = ip + 1;
        }

        let nice = self.params.nice_len as usize;
        let mut chain_len = 3usize; // chain matches must beat this length
        let mut depth = self.params.depth;
        while cand != 0 && depth > 0 {
            depth -= 1;
            let c = cand as usize - 1;
            if c >= ip || ip + chain_len >= end {
                break;
            }
            // Cheap reject: the byte that would extend the best match must agree.
            if buf[c + chain_len] == buf[ip + chain_len] && read32(buf, c) == cur {
                let len = 4 + match_len(buf, c + 4, ip + 4, end);
                if len > chain_len {
                    chain_len = len;
                    let off = ip - c;
                    let gain = len as i32 * 4 - (32 - (off as u32 + 3).leading_zeros()) as i32;
                    // A 4-byte match far away costs more than its literals.
                    if gain > best_gain && (len > 4 || off < (1 << 16)) {
                        best_gain = gain;
                        best_len = len;
                        best_off = off;
                    }
                    if len >= nice {
                        break;
                    }
                    // A good match is in hand: stop digging so deep.
                    if len >= 32 {
                        depth >>= 2;
                    }
                }
            }
            cand = self.chain[c];
        }
        (best_len, best_off)
    }

    /// Relative benefit of a match, in the units used by [`Self::search`].
    #[inline(always)]
    fn gain(&self, len: usize, off: usize) -> i32 {
        let r = self.store.reps;
        let off32 = off as u32;
        let cost = if off32 == r[0] {
            0
        } else if off32 == r[1] {
            1
        } else if off32 == r[2] {
            2
        } else {
            (32 - (off32 + 3).leading_zeros()) as i32
        };
        len as i32 * 4 - cost
    }

    /// Hash-chain search with lazy match evaluation.
    fn parse_lazy(&mut self, buf: &[u8], start: usize) {
        let end = buf.len();
        let nice = self.params.nice_len as usize;
        let lazy = self.params.lazy;
        let mut anchor = start;
        let mut ip = start;

        while ip + 8 <= end {
            let (mut len, mut off) = self.search(buf, ip);
            if len == 0 {
                ip += 1 + ((ip - anchor) >> 8);
                continue;
            }
            let mut mstart = ip;

            // Lazy evaluation: prefer a better match starting 1-2 bytes later.
            if lazy > 0 {
                while len < nice && mstart + 9 <= end {
                    let (len2, off2) = self.search(buf, mstart + 1);
                    if len2 > 0 && self.gain(len2, off2) > self.gain(len, off) + 4 {
                        mstart += 1;
                        len = len2;
                        off = off2;
                        continue;
                    }
                    if lazy >= 2 && mstart + 10 <= end {
                        let (len3, off3) = self.search(buf, mstart + 2);
                        if len3 > 0 && self.gain(len3, off3) > self.gain(len, off) + 7 {
                            mstart += 2;
                            len = len3;
                            off = off3;
                            continue;
                        }
                    }
                    break;
                }
            }

            while mstart > anchor && mstart > off && buf[mstart - 1] == buf[mstart - 1 - off] {
                mstart -= 1;
                len += 1;
            }
            self.store.emit(buf, anchor, mstart, len, off);
            ip = mstart + len;
            anchor = ip;
        }
        self.store.push_tail(buf, anchor);
    }
}
