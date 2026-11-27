//! Suffix array construction by induced sorting (SA-IS), Layer 3.
//!
//! Builds the suffix array of a byte string in linear time, independent of
//! how repetitive the input is. This replaces the comparison sort of cyclic
//! rotations used by the v1 BWT, which degrades to quadratic time on runs.
//!
//! Reference: G. Nong, S. Zhang, W. H. Chan, "Two Efficient Algorithms for
//! Linear Time Suffix Array Construction" (2011).

/// Marker for an unfilled suffix-array slot.
const EMPTY: u32 = u32::MAX;

/// Computes bucket boundaries for symbols `0..k`: bucket starts if `ends` is
/// false, bucket ends (exclusive) otherwise.
fn bucket_bounds(counts: &[u32], ends: bool, out: &mut [u32]) {
    let mut sum = 0u32;
    for (c, &n) in counts.iter().enumerate() {
        sum += n;
        out[c] = if ends { sum } else { sum - n };
    }
}

/// Induces the order of all L-type then all S-type suffixes from the LMS
/// suffixes already placed in `sa`.
fn induce(s: &[u32], sa: &mut [u32], is_s: &[bool], counts: &[u32], bounds: &mut [u32]) {
    let n = s.len();
    // L-type suffixes: scan left to right, filling from bucket starts.
    bucket_bounds(counts, false, bounds);
    for i in 0..n {
        let v = sa[i];
        if v != EMPTY && v > 0 {
            let j = (v - 1) as usize;
            if !is_s[j] {
                let c = s[j] as usize;
                sa[bounds[c] as usize] = j as u32;
                bounds[c] += 1;
            }
        }
    }
    // S-type suffixes: scan right to left, filling from bucket ends.
    bucket_bounds(counts, true, bounds);
    for i in (0..n).rev() {
        let v = sa[i];
        if v != EMPTY && v > 0 {
            let j = (v - 1) as usize;
            if is_s[j] {
                let c = s[j] as usize;
                bounds[c] -= 1;
                sa[bounds[c] as usize] = j as u32;
            }
        }
    }
}

/// SA-IS over a string of symbols in `0..k` whose last symbol is a unique,
/// smallest sentinel (0).
fn sais(s: &[u32], k: usize) -> Vec<u32> {
    let n = s.len();
    if n == 1 {
        return vec![0];
    }

    // Classify suffixes: S-type if smaller than the suffix to its right.
    let mut is_s = vec![false; n];
    is_s[n - 1] = true;
    for i in (0..n - 1).rev() {
        is_s[i] = s[i] < s[i + 1] || (s[i] == s[i + 1] && is_s[i + 1]);
    }
    let is_lms = |i: usize| i > 0 && is_s[i] && !is_s[i - 1];

    let mut counts = vec![0u32; k];
    for &c in s {
        counts[c as usize] += 1;
    }
    let mut bounds = vec![0u32; k];
    let mut sa = vec![EMPTY; n];

    // Step 1: sort LMS substrings by placing LMS suffixes at bucket ends in
    // arbitrary order and inducing.
    bucket_bounds(&counts, true, &mut bounds);
    for i in (1..n).rev() {
        if is_lms(i) {
            let c = s[i] as usize;
            bounds[c] -= 1;
            sa[bounds[c] as usize] = i as u32;
        }
    }
    induce(s, &mut sa, &is_s, &counts, &mut bounds);

    // Step 2: name the LMS substrings in their sorted order.
    let lms_sorted: Vec<u32> = sa
        .iter()
        .copied()
        .filter(|&v| v != EMPTY && is_lms(v as usize))
        .collect();
    let m = lms_sorted.len();
    let mut name_of = vec![EMPTY; n];
    let mut names = 0u32;
    let mut prev = usize::MAX;
    for &v in &lms_sorted {
        let cur = v as usize;
        let same = prev != usize::MAX && {
            let mut d = 0usize;
            loop {
                let (a, b) = (prev + d, cur + d);
                if a >= n || b >= n || s[a] != s[b] || is_s[a] != is_s[b] {
                    break false;
                }
                if d > 0 && (is_lms(a) || is_lms(b)) {
                    break is_lms(a) && is_lms(b);
                }
                d += 1;
            }
        };
        if !same {
            names += 1;
        }
        name_of[cur] = names - 1;
        prev = cur;
    }

    // Reduced problem: the names in text order.
    let lms_positions: Vec<u32> = (1..n).filter(|&i| is_lms(i)).map(|i| i as u32).collect();
    let reduced: Vec<u32> = lms_positions.iter().map(|&p| name_of[p as usize]).collect();
    let reduced_sa: Vec<u32> = if (names as usize) < m {
        sais(&reduced, names as usize)
    } else {
        // All names distinct: the order is the inverse permutation.
        let mut inv = vec![0u32; m];
        for (i, &name) in reduced.iter().enumerate() {
            inv[name as usize] = i as u32;
        }
        inv
    };

    // Step 3: place LMS suffixes in their true order and induce the rest.
    sa.fill(EMPTY);
    bucket_bounds(&counts, true, &mut bounds);
    for &r in reduced_sa.iter().rev() {
        let p = lms_positions[r as usize];
        let c = s[p as usize] as usize;
        bounds[c] -= 1;
        sa[bounds[c] as usize] = p;
    }
    induce(s, &mut sa, &is_s, &counts, &mut bounds);
    sa
}

/// Returns the suffix array of `data`: the starting positions of all suffixes
/// in lexicographic order (a suffix that is a prefix of another sorts first).
///
/// Runs in O(n) time and uses about 13 bytes of memory per input byte.
pub fn suffix_array(data: &[u8]) -> Vec<u32> {
    if data.is_empty() {
        return Vec::new();
    }
    // Shift symbols up by one so 0 is free to act as the unique sentinel.
    let mut s: Vec<u32> = Vec::with_capacity(data.len() + 1);
    s.extend(data.iter().map(|&b| b as u32 + 1));
    s.push(0);
    let sa = sais(&s, 257);
    // The sentinel suffix always sorts first; drop it.
    sa[1..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(data: &[u8]) -> Vec<u32> {
        let mut sa: Vec<u32> = (0..data.len() as u32).collect();
        sa.sort_by(|&a, &b| data[a as usize..].cmp(&data[b as usize..]));
        sa
    }

    fn lcg(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed;
        move || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            s >> 33
        }
    }

    #[test]
    fn test_sais_small_known_strings() {
        assert_eq!(suffix_array(b""), Vec::<u32>::new());
        assert_eq!(suffix_array(b"a"), vec![0]);
        assert_eq!(suffix_array(b"banana"), vec![5, 3, 1, 0, 4, 2]);
        assert_eq!(suffix_array(b"mississippi"), naive(b"mississippi"));
        assert_eq!(suffix_array(b"aaaaaaaa"), naive(b"aaaaaaaa"));
        assert_eq!(suffix_array(b"abababababab"), naive(b"abababababab"));
    }

    #[test]
    fn test_sais_matches_naive_on_random_inputs() {
        let mut r = lcg(11);
        for round in 0..300 {
            let n = 1 + (r() % 400) as usize;
            // Small alphabets create deep recursion; large ones do not.
            let alphabet = [1, 2, 3, 4, 16, 256][round % 6];
            let data: Vec<u8> = (0..n).map(|_| (r() % alphabet) as u8).collect();
            assert_eq!(suffix_array(&data), naive(&data), "round {round}");
        }
    }

    #[test]
    fn test_sais_matches_naive_on_structured_inputs() {
        let mut r = lcg(12);
        // Repeated phrases with occasional mutations.
        let mut text = Vec::new();
        while text.len() < 20_000 {
            text.extend_from_slice(b"the quick brown fox ");
            if r().is_multiple_of(5) {
                text.push(b'a' + (r() % 26) as u8);
            }
        }
        assert_eq!(suffix_array(&text), naive(&text));
        // 0xFF bytes must not collide with the internal sentinel handling.
        let high: Vec<u8> = (0..5000u32)
            .map(|i| if i.is_multiple_of(7) { 0xFF } else { 0x00 })
            .collect();
        assert_eq!(suffix_array(&high), naive(&high));
    }

    #[test]
    fn test_sais_is_fast_on_long_runs() {
        // A quadratic algorithm would take minutes here.
        let zeros = vec![0u8; 2_000_000];
        let sa = suffix_array(&zeros);
        assert_eq!(sa.len(), zeros.len());
        // Shorter suffixes of a run sort first.
        assert_eq!(sa[0], 1_999_999);
        assert_eq!(sa[1_999_999], 0);
    }
}
