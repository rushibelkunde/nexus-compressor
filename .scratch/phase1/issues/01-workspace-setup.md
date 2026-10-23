# 01: Cargo Workspace + Error Types + Project Skeleton

**What to build:** Initialize the Cargo workspace with three crates (`nexus-core`, `nexus-cli`, `nexus-bench`), define the core `NexusError` enum and `Result` type alias, and create all module files with doc comments (empty bodies). After this ticket, `cargo build` and `cargo test` should pass.

**Blocked by:** None (can start immediately).

**Status:** done

- [x] Workspace `Cargo.toml` with `members = ["crates/*"]`
- [x] `nexus-core/Cargo.toml` with dependencies: `thiserror`, `byteorder`, `xxhash-rust`
- [x] `nexus-cli/Cargo.toml` with dependency on `nexus-core` and `clap`
- [x] `nexus-bench/Cargo.toml` with dependency on `nexus-core`
- [x] `nexus-core/src/lib.rs` with all `pub mod` declarations
- [x] `nexus-core/src/error.rs` with `NexusError` enum and `Result<T>` alias
- [x] All module directories and `mod.rs` files created (block/, preprocess/, compress/, entropy/, format/, neural/)
- [x] `nexus-cli/src/main.rs` with placeholder `fn main()`
- [x] `cargo build` passes
- [x] `cargo test` passes
