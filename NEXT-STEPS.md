# Next steps

Read this file first. The work: make the servil fork the fastest BLAKE3 in
every situation a user meets, the streaming APIs first (AGENTS.md, "The
streaming APIs first"), natively on the Mac first, then in the VM,
measured by this benchmark. Prefer changes that are simpler and faster
together. The principles are in both repositories' `AGENTS.md`; the
fork's hardware facts, design, and rejected ideas are in its
`NOTES-servil.md` (read it before touching kernels or the pool); this
repository's are in `NOTES.md`, its settled questions under "What is
settled". Every open item is in one list: the fork's NOTES, "Future
work". This file says where the work stands; its history is in git.

## Where things stand (October 9, 2026)

- **The fork**: `servil` = 9d9f3d6; `candidate/no-linger` adds the proofs
  (the library's code unchanged but the SME2 message kernel's branch-free
  last-block length, 285ecf0; Mac jobs 1441-1442, no regression).
  `tools/verify` proves every AArch64 assembly kernel (hybrid, 327 cases;
  SME2, 34, and the chunk, parent, extended-output and message kernels at
  every group count by induction) and the library's Rust compression code
  (24) equal to the compression function of the Lean specification in
  `c2sp/BLAKE3`. The instruction models are proved equal to Arm's Sail
  specification: 136 of 137 NEON and integer forms, 19 of 23 streaming
  (`isla_check.py`). `tools/verify/tree` proves the tree walk and the
  Hasher's stack algorithm against the specification's tree, in Lean
  (Aeneas). Its README and the fork's NOTES (October 8) have the details.
- **The probe** `probe/wide-walk` (a1341ee, 4214925): the library's
  single-thread walk is `src/tree_core.rs`, the file `tools/verify/tree`
  proves, at every buffer size the library builds with. Mac A/B (jobs
  1443-1446, 1456-1479, mains): single-threaded within 1% everywhere;
  multithreaded 512 KiB x1.043, median of 14 pairs, 10 slower and 4
  faster (sign test p 0.18), cause unexplained (open); `perf_regress` on
  the Mac (1447) and the VM: no regression.
- **C2SP**: `c2sp/BLAKE3/` is a Lean specification of C2SP's BLAKE3;
  `c2sp/pr/` is the pull request, saved for Zooko's review. Filed:
  C2SP/C2SP#384 (a trace's chunk label), rems-project/isla#107 (the
  snapshot's `dup` index: Sail's fixed unsigned_subrange, not yet in the
  snapshot).
- **bench-hashes**: 0.16.1 on `main` with its records.

## Next

1. Zooko's decision: the library on the proved walk. `update_rayon`
   still runs the walk through Rayon's `join`: it moves to the fork's
   pool, or `widecore` takes a `join` (Aeneas and closures, to try). Then
   the multithreaded 512 KiB cell's cause (possibly 4%) before promotion.
2. The Hasher's stack as code (`HasherProofs.lean` proves the algorithm):
   its `ArrayVec` is outside what Aeneas translates, and a plain array
   zeroes 1.7 KiB per `Hasher::new`; a representation that costs a short
   message nothing, or Aeneas with ArrayVec's operations as stated
   contracts.
3. The SME2 flat walk (`ffi_sme2::flat_walk`) as a kernel with a proved
   contract, the four streaming forms Isla needs over 50 GB for, and the
   ZA loads and stores against Arm's specification.
4. Promote `candidate/no-linger` once its CI passes. The next runner job
   number is 1480.

## Commands

From `/workspace` in the VM, after `sh /workspace/vm/setup.sh` once per boot:

    cargo test --release --lib [--features no_sme2 | --features pure]
    cargo test --release --doc
    cargo test --release --test api_plan
    cargo test --release --manifest-path test_vectors/Cargo.toml
    cargo test --release --manifest-path bench-hashes/Cargo.toml
    pypy3 tools/perf_regress.py check | compare OLD NEW
    RUSTFLAGS="-D warnings" cargo test --no-run [--no-default-features | --features pure]   # as CI builds

Judge each by its exit status. Release: `python3 tools/gen-ver.py X.Y.Z`
from a clean tree (two version commits and a lightweight tag; push the
branch, `servil` in the fork or `main` here, then the tag by name).
Never print the credential token (`/workspace/ghtokenclassic.txt`). Only
`/workspace` survives VM restarts. Commands for the user go on one line.
