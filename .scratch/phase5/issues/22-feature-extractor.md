# 22: Multi-Context Neural Feature Extraction

**What to build:** Implement byte and bit context feature extraction in `crates/nexus-core/src/compress/neural/features.rs`.
- Extract sparse context features from the data stream:
  - Byte unigram ($b_{-1}$) and bigram ($b_{-2}, b_{-1}$)
  - Bit position in byte (0..7) and bit history prefix within current byte
  - Sparse distance tokens ($b_{-4}, b_{-8}$)
  - Text word status (alphanumeric vs punctuation)
- Efficient sparse feature embedding table converting context hash keys to compact fixed-point feature vectors.
- Speed: feature extraction inner loop < 10 CPU cycles per bit.

**Blocked by:** Phase 4 complete

**Status:** done

- [x] `compress/neural/features.rs`: `FeatureExtractor` extracting context representations
- [x] Sparse hash indexing into embedding tables
- [x] Unit tests verifying distinct feature activations for different data contexts
