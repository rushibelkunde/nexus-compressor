//! Price-based optimal parser for LZ-ANS v2 (levels 10+).
//!
//! Works in two phases per sub-block:
//!
//! 1. **Gather** — walk the block once, recording for every position the set
//!    of matches with strictly increasing length (closest offset first),
//!    found with a binary-tree match finder: every position is a node of a
//!    search tree ordered by the bytes that follow it, so one descent yields
//!    the longest match at every distance scale.
//! 2. **Parse** — a forward dynamic program over block positions where each
//!    node holds the cheapest known way to reach it, priced in 1/256 bit from
//!    the symbol statistics of the previous pass or sub-block.
//!
//! Matches at least `nice_len` long are taken immediately and their interior
//! is skipped in both phases, which keeps long repeats linear-time.
//!
//! All pricing is integer arithmetic; the result is deterministic across
//! platforms.

use crate::compress::lz::encoder::{
    hash3, hash4, match_len, read32, LzEncoder, SeqStore, HASH3_LOG,
};
use crate::compress::lz::{
    value_code, CODE_BITS, INITIAL_REPS, MIN_MATCH, NUM_REP_CODES, NUM_VALUE_CODES,
};
use crate::entropy::bitio::log2_fp8;
use crate::entropy::rans4::histogram;

/// Price unit: 1/256 bit.
const BIT: u32 = 256;
/// 3-byte matches further away than this never pay for their offset.
const MAX_HASH3_OFFSET: usize = 1 << 14;

/// One state of the parse: the cheapest known way to have coded everything
/// before this position.
#[derive(Clone, Copy)]
struct Node {
    /// Total price in 1/256 bit, including the literal-length code of the
    /// literal run that ends here.
    price: u32,
    /// Length of the match that arrives here, or 0 if reached by a literal.
    mlen: u32,
    /// Offset of that match.
    off: u32,
    /// Literals since the last match on this path.
    litlen: u32,
    /// Repeat offsets after reaching this node.
    reps: [u32; 3],
}

const UNREACHED: Node = Node {
    price: u32::MAX,
    mlen: 0,
    off: 0,
    litlen: 0,
    reps: INITIAL_REPS,
};

/// Symbol prices (code price plus extra bits) in 1/256 bit.
struct Prices {
    lit: [u32; 256],
    ll: [u32; NUM_VALUE_CODES],
    ml: [u32; NUM_VALUE_CODES],
    of: [u32; NUM_REP_CODES + NUM_VALUE_CODES],
}

impl Default for Prices {
    fn default() -> Self {
        let mut p = Prices {
            lit: [6 * BIT; 256],
            ll: [0; NUM_VALUE_CODES],
            ml: [0; NUM_VALUE_CODES],
            of: [0; NUM_REP_CODES + NUM_VALUE_CODES],
        };
        p.set_default_ll();
        p.set_default_ml();
        p.set_default_of();
        p
    }
}

/// Price of a symbol seen `count` times out of `total`.
#[inline]
fn stat_price(count: u32, total: u32) -> u32 {
    let base = log2_fp8(total.max(1)).saturating_sub(log2_fp8(count.max(1)));
    // Unseen symbols are possible but should look expensive.
    let p = if count == 0 { base + BIT } else { base };
    p.max(BIT / 8)
}

// The price tables are parallel arrays indexed by symbol; indexing them
// together reads more clearly than zipped iterators.
#[allow(clippy::needless_range_loop)]
impl Prices {
    fn set_default_ll(&mut self) {
        for c in 0..NUM_VALUE_CODES {
            self.ll[c] = 3 * BIT + CODE_BITS[c] as u32 * BIT;
        }
    }

    fn set_default_ml(&mut self) {
        for c in 0..NUM_VALUE_CODES {
            self.ml[c] = 4 * BIT + CODE_BITS[c] as u32 * BIT;
        }
    }

    fn set_default_of(&mut self) {
        self.of[0] = 2 * BIT;
        self.of[1] = 3 * BIT;
        self.of[2] = 3 * BIT + BIT / 2;
        for c in 0..NUM_VALUE_CODES {
            self.of[NUM_REP_CODES + c] = 5 * BIT + CODE_BITS[c] as u32 * BIT;
        }
    }

    /// Re-derives prices from the symbols actually emitted for a block.
    fn update_from(&mut self, store: &SeqStore) {
        if store.lits.len() >= 64 {
            let h = histogram(&store.lits);
            let total = store.lits.len() as u32;
            for s in 0..256 {
                self.lit[s] = stat_price(h[s], total);
            }
        }
        let nseq = store.ll.len() as u32;
        if nseq < 16 {
            return;
        }
        let h = histogram(&store.ll);
        for c in 0..NUM_VALUE_CODES {
            self.ll[c] = stat_price(h[c], nseq) + CODE_BITS[c] as u32 * BIT;
        }
        let h = histogram(&store.ml);
        for c in 0..NUM_VALUE_CODES {
            self.ml[c] = stat_price(h[c], nseq) + CODE_BITS[c] as u32 * BIT;
        }
        let h = histogram(&store.of);
        for c in 0..NUM_REP_CODES {
            self.of[c] = stat_price(h[c], nseq);
        }
        for c in 0..NUM_VALUE_CODES {
            self.of[NUM_REP_CODES + c] =
                stat_price(h[NUM_REP_CODES + c], nseq) + CODE_BITS[c] as u32 * BIT;
        }
    }

    #[inline(always)]
    fn ll_cost(&self, litlen: u32) -> u32 {
        self.ll[value_code(litlen).0 as usize]
    }

    #[inline(always)]
    fn ml_cost(&self, len: usize) -> u32 {
        self.ml[value_code((len - MIN_MATCH) as u32).0 as usize]
    }

    /// Price of coding `off` given the current repeat offsets.
    #[inline(always)]
    fn of_cost(&self, off: u32, reps: &[u32; 3]) -> u32 {
        if off == reps[0] {
            self.of[0]
        } else if off == reps[1] {
            self.of[1]
        } else if off == reps[2] {
            self.of[2]
        } else {
            self.of[NUM_REP_CODES + value_code(off - 1).0 as usize]
        }
    }
}

/// Repeat offsets after coding a match at `off` (mirrors `SeqStore::emit`).
#[inline(always)]
fn apply_offset(r: &[u32; 3], off: u32) -> [u32; 3] {
    if off == r[0] {
        *r
    } else if off == r[1] {
        [r[1], r[0], r[2]]
    } else if off == r[2] {
        [r[2], r[0], r[1]]
    } else {
        [off, r[0], r[1]]
    }
}

/// Scratch state of the optimal parser, reused across sub-blocks.
#[derive(Default)]
pub(super) struct OptState {
    nodes: Vec<Node>,
    /// `matches[match_start[i]..match_start[i + 1]]` are the matches at block
    /// position `i`, as `(length, offset)` with increasing length.
    match_start: Vec<u32>,
    matches: Vec<(u32, u32)>,
    prices: Prices,
    /// True once `prices` reflect real statistics from this superblock.
    have_stats: bool,
    /// Backtracked path: `(block position, length, offset)`, last match first.
    path: Vec<(u32, u32, u32)>,
    /// Matches of at least `nice_len` whose interior was skipped while
    /// gathering: `(block position, length, offset)` in position order.
    longs: Vec<(u32, u32, u32)>,
}

impl LzEncoder {
    /// Parses `buf[start..end]` with the optimal parser, filling the sequence
    /// store. `buf` is the whole superblock: bytes past `end` are never
    /// matched against, but the match finder may read them to order its tree.
    pub(super) fn parse_optimal(&mut self, buf: &[u8], start: usize, end: usize) {
        self.gather_matches(buf, start, end);
        let buf = &buf[..end];
        // The first block of a superblock has no statistics yet: bootstrap
        // with one extra pass over generic prices.
        let passes = if self.opt.have_stats {
            self.params.opt_passes.max(1)
        } else {
            self.params.opt_passes.max(2)
        };
        for _ in 0..passes {
            self.store.prepare(buf.len() - start);
            self.run_parse(buf, start);
            self.opt.prices.update_from(&self.store);
            self.opt.have_stats = true;
        }
    }

    /// Inserts `pos` into the binary tree and, when `record` is set, appends
    /// every match longer than `best` to the match list. Returns the longest
    /// match length seen (at least `best`).
    ///
    /// The tree lives in `chain`: slots `2p` and `2p + 1` hold the roots of
    /// the subtrees of positions whose following bytes sort below and above
    /// those at `p`. Comparisons are capped at `nice_len`; a candidate that
    /// ties to the cap is replaced by `pos`, which keeps the tree a valid
    /// search tree over the capped keys.
    ///
    /// `buf` must be the whole superblock so that keys are only ever
    /// truncated by its true end: ordering a node on a prefix that later
    /// grows would break the search-tree invariant the length shortcuts rely
    /// on. Reported match lengths are clamped to `block_end`.
    ///
    /// Requires `pos + 4 <= buf.len()` and `pos < block_end`.
    #[inline(always)]
    fn bt_insert(
        &mut self,
        buf: &[u8],
        pos: usize,
        mut depth: u32,
        mut best: usize,
        record: bool,
        block_end: usize,
    ) -> usize {
        let cmp_end = buf.len().min(pos + self.params.nice_len as usize);
        let cap = cmp_end - pos;
        let h = hash4(read32(buf, pos), self.hash_log);
        let mut cur = self.hash[h] as usize;
        self.hash[h] = (pos + 1) as u32;
        let son = &mut self.chain[..];
        let mut ptr0 = 2 * pos + 1;
        let mut ptr1 = 2 * pos;
        let (mut len0, mut len1) = (0usize, 0usize);
        loop {
            if cur == 0 || depth == 0 {
                son[ptr0] = 0;
                son[ptr1] = 0;
                break;
            }
            depth -= 1;
            let c = cur - 1;
            // Both subtree bounds share at least this many bytes with `pos`.
            let mut len = len0.min(len1);
            len += match_len(buf, c + len, pos + len, cmp_end);
            if record {
                let mut usable = len;
                if len == cap && pos + len < block_end {
                    usable += match_len(buf, c + len, pos + len, block_end);
                }
                let usable = usable.min(block_end - pos);
                if usable > best {
                    best = usable;
                    self.opt.matches.push((usable as u32, (pos - c) as u32));
                }
            }
            if len == cap {
                son[ptr1] = son[2 * c];
                son[ptr0] = son[2 * c + 1];
                break;
            }
            if buf[c + len] < buf[pos + len] {
                son[ptr1] = cur as u32;
                ptr1 = 2 * c + 1;
                cur = son[ptr1] as usize;
                len1 = len;
            } else {
                son[ptr0] = cur as u32;
                ptr0 = 2 * c;
                cur = son[ptr0] as usize;
                len0 = len;
            }
        }
        best
    }

    /// Inserts every not-yet-indexed position below `target` into the tree
    /// and the 3-byte table, without collecting matches.
    fn bt_catch_up(&mut self, buf: &[u8], target: usize) {
        let limit = buf.len().saturating_sub(3).min(target);
        // Skipped positions lie inside long matches: a shallow descent is
        // enough to keep the tree useful.
        let depth = (self.params.depth / 2).max(4);
        let mut p = self.next_insert;
        while p < limit {
            self.hash3[hash3(read32(buf, p), HASH3_LOG)] = (p + 1) as u32;
            self.bt_insert(buf, p, depth, 0, false, p + 1);
            p += 1;
        }
        self.next_insert = self.next_insert.max(p);
    }

    /// Phase 1: records all useful matches for every position of the block.
    fn gather_matches(&mut self, buf: &[u8], start: usize, end: usize) {
        let n = end - start;
        let nice = self.params.nice_len as usize;
        let depth = self.params.depth;

        self.opt.matches.clear();
        self.opt.longs.clear();
        self.opt.match_start.clear();
        self.opt.match_start.reserve(n + 1);

        let mut i = 0;
        while i < n {
            let pos = start + i;
            self.opt.match_start.push(self.opt.matches.len() as u32);
            if pos + 4 > end {
                i += 1;
                continue;
            }
            self.bt_catch_up(buf, pos);
            let cur = read32(buf, pos);
            let mut best = MIN_MATCH - 1;

            // Closest 3-byte match.
            let h3 = hash3(cur, HASH3_LOG);
            let c3 = self.hash3[h3] as usize;
            self.hash3[h3] = (pos + 1) as u32;
            if c3 != 0 && c3 - 1 < pos {
                let c = c3 - 1;
                let off = pos - c;
                if off < MAX_HASH3_OFFSET && (read32(buf, c) ^ cur) & 0x00FF_FFFF == 0 {
                    let len = 3 + match_len(buf, c + 3, pos + 3, end);
                    self.opt.matches.push((len as u32, off as u32));
                    best = len;
                }
            }

            if self.next_insert <= pos {
                best = self.bt_insert(buf, pos, depth, best, true, end);
                self.next_insert = pos + 1;
            }

            if best >= nice {
                // Long match: it will be taken whole, so skip its interior.
                let off = self.opt.matches.last().map_or(1, |m| m.1);
                self.opt.longs.push((i as u32, best as u32, off));
                for _ in 1..best {
                    self.opt.match_start.push(self.opt.matches.len() as u32);
                }
                i += best;
            } else {
                i += 1;
            }
        }
        self.opt.match_start.push(self.opt.matches.len() as u32);
        debug_assert_eq!(self.opt.match_start.len(), n + 1);
    }

    /// Phase 2: forward dynamic program, backtrack, and emit sequences.
    fn run_parse(&mut self, buf: &[u8], start: usize) {
        let end = buf.len();
        let n = end - start;
        let nice = self.params.nice_len as usize;
        let opt = &mut self.opt;
        let prices = &opt.prices;
        let ll0 = prices.ll[0];

        opt.nodes.clear();
        opt.nodes.resize(n + 1, UNREACHED);
        opt.nodes[0].price = ll0;
        let nodes = &mut opt.nodes[..];

        #[inline(always)]
        fn relax(node: &mut Node, price: u32, mlen: usize, off: u32, reps: [u32; 3]) {
            if price < node.price {
                *node = Node {
                    price,
                    mlen: mlen as u32,
                    off,
                    litlen: 0,
                    reps,
                };
            }
        }

        let mut i = 0;
        let mut long_idx = 0usize;
        while i < n {
            let cur = nodes[i];
            let pos = start + i;

            // Extend the literal run by one byte.
            {
                let litlen = cur.litlen + 1;
                let price = (cur.price + prices.lit[buf[pos] as usize] + prices.ll_cost(litlen))
                    .saturating_sub(prices.ll_cost(cur.litlen));
                let next = &mut nodes[i + 1];
                if price < next.price {
                    *next = Node {
                        price,
                        mlen: 0,
                        off: 0,
                        litlen,
                        reps: cur.reps,
                    };
                }
            }

            if end - pos < MIN_MATCH {
                i += 1;
                continue;
            }
            // Every match is followed by a (possibly empty) literal run.
            let base = cur.price + ll0;
            let mut jump = 0usize;

            // Repeat-offset matches.
            for r in 0..NUM_REP_CODES {
                let off = cur.reps[r];
                if off as usize > pos || (r > 0 && cur.reps[..r].contains(&off)) {
                    continue;
                }
                let len = match_len(buf, pos - off as usize, pos, end);
                if len < MIN_MATCH {
                    continue;
                }
                let reps = apply_offset(&cur.reps, off);
                let ofp = base + prices.of[r];
                if len >= nice {
                    relax(
                        &mut nodes[i + len],
                        ofp + prices.ml_cost(len),
                        len,
                        off,
                        reps,
                    );
                    jump = jump.max(len);
                } else {
                    for l in MIN_MATCH..=len {
                        relax(&mut nodes[i + l], ofp + prices.ml_cost(l), l, off, reps);
                    }
                }
            }

            // Matches from the finder, shortest first.
            let ms = &opt.matches[opt.match_start[i] as usize..opt.match_start[i + 1] as usize];
            let mut prev_len = MIN_MATCH - 1;
            for &(len, off) in ms {
                let len = len as usize;
                let reps = apply_offset(&cur.reps, off);
                let ofp = base + prices.of_cost(off, &cur.reps);
                if len >= nice {
                    relax(
                        &mut nodes[i + len],
                        ofp + prices.ml_cost(len),
                        len,
                        off,
                        reps,
                    );
                    jump = jump.max(len);
                } else {
                    for l in (prev_len + 1)..=len {
                        relax(&mut nodes[i + l], ofp + prices.ml_cost(l), l, off, reps);
                    }
                }
                prev_len = len;
            }

            // Landed inside a long match whose interior was not searched:
            // its remaining suffix is still a valid match at the same offset.
            while long_idx < opt.longs.len()
                && (opt.longs[long_idx].0 + opt.longs[long_idx].1) as usize <= i
            {
                long_idx += 1;
            }
            if let Some(&(lpos, llen, loff)) = opt.longs.get(long_idx) {
                let lpos = lpos as usize;
                if lpos < i {
                    let rem = lpos + llen as usize - i;
                    if rem >= MIN_MATCH {
                        let reps = apply_offset(&cur.reps, loff);
                        let ofp = base + prices.of_cost(loff, &cur.reps);
                        if rem >= nice {
                            relax(
                                &mut nodes[i + rem],
                                ofp + prices.ml_cost(rem),
                                rem,
                                loff,
                                reps,
                            );
                            jump = jump.max(rem);
                        } else {
                            for l in MIN_MATCH..=rem {
                                relax(&mut nodes[i + l], ofp + prices.ml_cost(l), l, loff, reps);
                            }
                        }
                    }
                }
            }

            if jump > 0 {
                i += jump;
            } else {
                i += 1;
            }
        }

        // Backtrack from the end of the block.
        opt.path.clear();
        let mut j = n;
        while j > 0 {
            let nd = nodes[j];
            if nd.mlen == 0 {
                j -= 1;
            } else {
                j -= nd.mlen as usize;
                opt.path.push((j as u32, nd.mlen, nd.off));
            }
        }

        let mut anchor = start;
        for &(rel, len, off) in opt.path.iter().rev() {
            let mstart = start + rel as usize;
            self.store
                .emit(buf, anchor, mstart, len as usize, off as usize);
            anchor = mstart + len as usize;
        }
        self.store.push_tail(buf, anchor);
    }
}
