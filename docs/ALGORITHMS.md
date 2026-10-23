# Algorithms — Project NEXUS

> Mathematical and implementation details for every compression algorithm used in NEXUS.
> AI agents: reference this document when implementing or modifying any algorithm.

---

## 1. Shannon Entropy (Layer 1: Detection)

Used to estimate compressibility and detect data type changes.

### Formula
```
H(X) = -Σ p(x_i) * log2(p(x_i))   for i in 0..255
```

Where `p(x_i)` = frequency of byte value `i` / total bytes in window.

### Interpretation
- `H ≈ 0.0`: All bytes identical (maximally compressible)
- `H ≈ 4.0`: Typical English text
- `H ≈ 5.5`: Source code
- `H ≈ 7.0`: Compressed data or binary
- `H ≈ 8.0`: Random / encrypted (incompressible)

### Implementation Notes
- Use a 4096-byte sliding window
- Maintain a `[u32; 256]` frequency table, updated incrementally
- Use integer approximation: `H ≈ log2(total) - (1/total) * Σ freq[i] * log2(freq[i])`
- Precompute `log2` table for integers 1..4096 to avoid floating-point

---

## 2. LZ77/LZSS (Path A: Core Compressor)

Dictionary-based compression. Replaces repeated byte sequences with (offset, length) references.

### Token Types
```rust
enum Lz77Token {
    Literal(u8),                    // Single unmatched byte
    Match { offset: u32, length: u16 }, // Reference to previous occurrence
}
```

### Match Finding (Hash Chains)

```
1. Build hash table: hash(data[i..i+3]) → position i
2. For each position i:
   a. Hash the 3 bytes at position i
   b. Look up hash table for previous positions with same hash
   c. Follow the hash chain to find the longest match
   d. If match_length >= 3: emit Match(offset, length)
   e. Else: emit Literal(data[i])
3. Update hash table with position i
```

### Hash Function (3-byte)
```rust
fn hash3(data: &[u8], pos: usize) -> u32 {
    let b0 = data[pos] as u32;
    let b1 = data[pos + 1] as u32;
    let b2 = data[pos + 2] as u32;
    (b0 << 16 | b1 << 8 | b2) % HASH_TABLE_SIZE
}
```

### Parameters by Compression Level
| Level | Window Size | Hash Table Size | Min Match | Max Match |
|-------|------------|-----------------|-----------|-----------|
| 1-3 | 32KB | 32K entries | 3 | 258 |
| 4-6 | 128KB | 128K entries | 3 | 258 |
| 7-9 | 1MB | 1M entries | 3 | 65535 |
| 10+ | 4MB | 4M entries | 3 | 65535 |

### Phase 1: Greedy Matching
Take the first match >= min_length found.

### Phase 2: Optimal Parsing (Price-Based DP)
TODO: Implement Storer-Szymanski optimal parsing. For each position, compute
the minimum cost (in bits) to encode all remaining data, working backwards.

---

## 3. Huffman Coding (Phase 1 Entropy Coder — Baseline)

Assigns shorter bit codes to more frequent symbols.

### Algorithm
```
1. Count frequency of each symbol (byte value 0-255)
2. Build min-heap of (frequency, symbol) pairs
3. While heap has > 1 node:
   a. Pop two lowest-frequency nodes
   b. Create parent node with sum of frequencies
   c. Push parent back onto heap
4. Traverse tree to assign bit codes (left=0, right=1)
5. Encode data by replacing each byte with its bit code
```

### Canonical Huffman
Use canonical codes for compact header:
- Sort symbols by code length, then by value
- Store only code lengths (1 byte per symbol)
- Reconstruct codes from lengths at decode time

### Header Format (Phase 1)
```
[1 byte: max_code_length]
[256 bytes: code_length for each symbol 0-255]
[variable: encoded data bitstream]
[padding to byte boundary]
```

---

## 4. rANS (Range Asymmetric Numeral Systems) (Phase 2+ Entropy Coder)

Replaces Huffman. Achieves arithmetic-coding-level ratios at Huffman-level speed.

### Core State Transition

**Encoding** (processes symbols in REVERSE order):
```
// Normalize: ensure state is in valid range before encoding
while state >= (freq << 16):
    output_byte(state & 0xFF)
    state >>= 8

// Encode step:
state = (state / freq) * total_freq + (state % freq) + cumulative_freq
```

**Decoding:**
```
// Find symbol from state
slot = state % total_freq
symbol = lookup_symbol(slot)  // via cumulative frequency table
freq = frequency[symbol]
cum = cumulative[symbol]

// Decode step:
state = freq * (state / total_freq) + (state % total_freq) - cum

// Renormalize: refill state from input
while state < (1 << 16):
    state = (state << 8) | read_byte()
```

### Parameters
- `total_freq` (M): 4096 (12-bit scale, power of 2 for fast modulo via bitmask)
- Initial state: `1 << 16` (minimum valid state)
- Renormalization threshold: `1 << 16`

### Frequency Table
- Built from symbol counts in each sub-block
- Normalized to sum to `total_freq` (4096)
- Minimum frequency per present symbol: 1 (never zero for present symbols)
- Stored in mini-header or inline before compressed data

---

## 5. Burrows-Wheeler Transform (Path B)

Permutes a block of data so that symbols with identical preceding contexts cluster together.

### Forward BWT
```
1. Given block of N bytes
2. Generate all N cyclic rotations (conceptually)
3. Sort rotations lexicographically
4. Output = last column of sorted matrix
5. Record primary_index = row where original string appears
```

### Implementation: SA-IS (Suffix Array by Induced Sorting)
- O(n) time, O(n) space
- Build suffix array, then derive BWT from it
- `bwt[i] = data[sa[i] - 1]` (or `data[n-1]` when `sa[i] == 0`)

### Inverse BWT (LF-Mapping)
```
1. Count frequency of each byte in BWT output
2. Build cumulative frequency table C[c] = count of bytes < c
3. Build LF-mapping: for each position i, LF[i] = C[bwt[i]] + rank(bwt[i], i)
4. Starting from primary_index, follow LF-mapping N times to reconstruct original
```

### Block Size
- Default: 900KB (same as bzip2)
- Larger blocks = better compression but more memory (O(5n) for SA-IS)

---

## 6. Move-to-Front Transform (Post-BWT, Path B)

Converts clustered byte sequences into small-integer-dominated streams.

### Algorithm
```rust
fn mtf_encode(input: &[u8]) -> Vec<u8> {
    let mut list: Vec<u8> = (0..=255).collect(); // [0, 1, 2, ..., 255]
    let mut output = Vec::with_capacity(input.len());
    
    for &byte in input {
        let pos = list.iter().position(|&b| b == byte).unwrap();
        output.push(pos as u8);
        // Move to front
        list.remove(pos);
        list.insert(0, byte);
    }
    output
}
```

### Why It Helps
After BWT, repeated characters cluster. MTF turns these clusters into
runs of small numbers (0, 1, 2), which compress extremely well with
zero-run-length encoding + entropy coding.

### Inverse MTF
```rust
fn mtf_decode(input: &[u8]) -> Vec<u8> {
    let mut list: Vec<u8> = (0..=255).collect();
    let mut output = Vec::with_capacity(input.len());
    
    for &idx in input {
        let byte = list[idx as usize];
        output.push(byte);
        list.remove(idx as usize);
        list.insert(0, byte);
    }
    output
}
```

---

## 7. Zero-Run-Length Encoding (Post-MTF, Path B)

After MTF, the output is dominated by zeros. Encode runs of zeros efficiently.

### Encoding Scheme
```
Symbol 0 followed by a count:
  If count fits in 7 bits: [0x00, count as u8 with high bit 0]
  If count > 127: [0x00, 0x80 | (count >> 8), count & 0xFF]
  
Non-zero symbols: passed through unchanged.
```

### Simpler Phase 1 variant
```
[0x00, length_byte]: represents `length_byte + 1` zeros
Any other byte: literal
```

---

## 8. Context Mixing (Path C — Fixed-Point Integer Math)

Bit-level compression using multiple predictive models blended by a neural mixer.

### Overview
For each BIT (not byte) in the input:
1. Run N context models, each producing P(bit=1) as integer in [1, 4095]
2. Blend probabilities using logistic mixer (fixed-point weights)
3. Feed blended probability to arithmetic coder
4. Update model weights based on actual bit

### Fixed-Point Probability Scale
- Range: [1, 4095] (12-bit, never 0 or 4096)
- 2048 represents p = 0.5
- 1 represents p ≈ 0.000244 (near certainty of 0)
- 4095 represents p ≈ 0.999756 (near certainty of 1)

### Stretch (Logit) Lookup Table
Maps probability p ∈ [1, 4095] to log-odds in fixed-point:
```
stretch_table[p] = round(log(p / (4096 - p)) * 256)
// Type: i32
// Range: approximately [-2048, +2048]
```

### Squash (Sigmoid) Lookup Table
Inverse of stretch. Maps log-odds back to probability:
```
squash_table[x + 2048] = round(4096 / (1 + exp(-x / 256)))
// Input x: i32, shifted by +2048 to index into array
// Output: u16 in [1, 4095]
```

### Logistic Mixer
```
// inputs: N probabilities p[0..N] in [1, 4095]
// weights: N i16 weights w[0..N]

sum = 0_i32
for i in 0..N:
    sum += w[i] as i32 * stretch_table[p[i] as usize]
    
// Shift to compensate for weight scaling
sum >>= 8  // or appropriate shift based on weight scale

// Clamp and squash
blended_p = squash_table[(sum + 2048).clamp(0, 4095) as usize]
```

### Weight Update (Integer SGD)
After observing actual bit `y` (0 or 1):
```
error = y * 4095 - blended_p  // i32, range [-4095, +4095]
learning_rate_shift = 7       // equivalent to lr ≈ 1/128

for i in 0..N:
    w[i] += ((stretch_table[p[i] as usize] * error) >> (learning_rate_shift + 8)) as i16
```

### Context Models (Phase 4)
Each model maintains a hash table mapping context → probability counter.

| Model | Context Key | What It Captures |
|-------|------------|-----------------|
| Order-1 | previous byte | Byte bigram statistics |
| Order-2 | previous 2 bytes | Trigram statistics |
| Order-4 | previous 4 bytes | Short patterns |
| Order-8 | hash of previous 8 bytes | Longer patterns |
| Word | current word hash | Natural language |
| Sparse | byte at positions -2, -4 | Structural patterns |
| Match | longest match in history | Exact repetitions |
| Record | byte at position - record_length | Fixed-width records |

---

## 9. Arithmetic Coding (Path C & D — Integer Implementation)

Encodes a stream of symbols using fractional bits based on symbol probabilities.

### State Variables
```rust
struct ArithmeticCoder {
    low: u32,   // Lower bound of current interval
    high: u32,  // Upper bound of current interval
    // Invariant: low < high
    // Initial: low = 0, high = 0xFFFF_FFFF
}
```

### Encoding a Bit
Given probability `p1` (P(bit=1)) in [1, 4095] (12-bit scale):
```rust
fn encode_bit(&mut self, bit: u8, p1: u16, output: &mut BitWriter) {
    let range = self.high - self.low;
    // Split point: probability of 0 = (4096 - p1)
    let mid = self.low + (range >> 12) * (4096 - p1 as u32);
    
    if bit == 0 {
        self.high = mid;
    } else {
        self.low = mid + 1;
    }
    
    // Renormalize: shift out matching high bits
    while (self.low ^ self.high) < (1 << 24) {
        output.write_byte((self.low >> 24) as u8);
        self.low <<= 8;
        self.high = (self.high << 8) | 0xFF;
    }
}
```

### Decoding a Bit
```rust
fn decode_bit(&mut self, p1: u16, input: &mut BitReader) -> u8 {
    let range = self.high - self.low;
    let mid = self.low + (range >> 12) * (4096 - p1 as u32);
    
    let bit = if self.code <= mid { 0 } else { 1 };
    
    if bit == 0 {
        self.high = mid;
    } else {
        self.low = mid + 1;
    }
    
    // Renormalize
    while (self.low ^ self.high) < (1 << 24) {
        self.low <<= 8;
        self.high = (self.high << 8) | 0xFF;
        self.code = (self.code << 8) | input.read_byte() as u32;
    }
    
    bit
}
```

### CRITICAL: All math is u32/u64 integer. NO floats. NO rounding ambiguity.

---

## 10. Delta Encoding (Preprocessor)

### Forward Transform
```rust
fn delta_encode(input: &[u8]) -> Vec<u8> {
    if input.is_empty() { return vec![]; }
    let mut output = Vec::with_capacity(input.len());
    output.push(input[0]); // First byte stored as-is
    for i in 1..input.len() {
        output.push(input[i].wrapping_sub(input[i - 1]));
    }
    output
}
```

### Inverse Transform
```rust
fn delta_decode(input: &[u8]) -> Vec<u8> {
    if input.is_empty() { return vec![]; }
    let mut output = Vec::with_capacity(input.len());
    output.push(input[0]);
    for i in 1..input.len() {
        output.push(input[i].wrapping_add(output[i - 1]));
    }
    output
}
```

### Using `wrapping_sub/add` ensures bijectivity for all byte values (no overflow panic).

---

## 11. x86 Filter / E8E9 Transform (Preprocessor)

Converts relative CALL/JMP addresses in x86 code to absolute addresses.

### Forward Transform
```
For each position i in input:
    if input[i] == 0xE8 (CALL) or input[i] == 0xE9 (JMP):
        addr = read_le_i32(input, i + 1)
        absolute_addr = addr + i
        write_le_i32(output, i + 1, absolute_addr)
```

### Inverse Transform
```
For each position i in input:
    if input[i] == 0xE8 or input[i] == 0xE9:
        absolute_addr = read_le_i32(input, i + 1)
        relative_addr = absolute_addr - i
        write_le_i32(output, i + 1, relative_addr)
```

### Why It Helps
x86 binaries contain many CALL instructions to the same function. Each CALL encodes
a different relative offset. Converting to absolute addresses makes all calls to the
same function produce identical 4-byte sequences, dramatically improving LZ77 matching.

---

## 12. Float Byte-Plane Splitting (Preprocessor)

Separates IEEE 754 float arrays by byte position for better entropy coding.

### Forward Transform (32-bit floats)
```
Input:  [f0_b0, f0_b1, f0_b2, f0_b3, f1_b0, f1_b1, f1_b2, f1_b3, ...]

Split into 4 planes:
Plane 0: [f0_b0, f1_b0, f2_b0, ...]  ← Sign+exponent, very low entropy
Plane 1: [f0_b1, f1_b1, f2_b1, ...]  ← Upper mantissa, moderate entropy
Plane 2: [f0_b2, f1_b2, f2_b2, ...]  ← Lower mantissa, higher entropy
Plane 3: [f0_b3, f1_b3, f2_b3, ...]  ← LSB mantissa, highest entropy

Output: [Plane 0] [Plane 1] [Plane 2] [Plane 3]
```

### Inverse: Interleave the 4 planes back together.

### Why It Helps
In arrays of similar-magnitude floats, Plane 0 (exponent bytes) will be nearly
identical across all values, compressing to almost nothing. This is much better
than compressing the interleaved float bytes where each byte seems "random".
