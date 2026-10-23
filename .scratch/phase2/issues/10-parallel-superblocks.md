# 10: Multi-Threaded Parallel Superblocks with Rayon

**What to build:** Parallelize superblock compression using `rayon`.
Each superblock (4MB-16MB) is independent (Layer 0 framing).
Compression of superblocks is distributed across available CPU threads using `into_par_iter()`.
Results are assembled in deterministic superblock order into the final .nxs container.

**Blocked by:** 09 (Pipeline Integration)

**Status:** done

- [x] Add `rayon` dependency to `nexus-core`
- [x] Parallel superblock compression via rayon in `pipeline.rs`
- [x] Maintain deterministic ordering and footer block table indexing
- [x] Integration tests verifying multi-superblock files compress in parallel
- [x] Verify throughput improvement on multi-core CPU
