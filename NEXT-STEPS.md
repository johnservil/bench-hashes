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

## Where things stand (October 8, 2026)

- **The fork**: `servil` = 9d9f3d6; `candidate/no-linger` (28feeb4) adds
  the proofs, the library's code unchanged but the SME2 message kernel's
  branch-free last-block length (285ecf0; Mac jobs 1441-1442, no
  regression). `tools/verify` proves every AArch64 assembly kernel
  (hybrid, 327 cases; SME2, 34) and the library's Rust compression code
  (24, the NEON extended output at every count by induction) equal to the
  compression function of the Lean specification in `c2sp/BLAKE3`. It
  proves the instruction models equal to Arm's Sail specification (136
  of 137 forms, `isla_check.py`), and a safe-Rust version of the
  library's tree walk equal to the specification's tree (`tools/verify/tree`,
  Aeneas and Lean). Its README and the fork's NOTES (October 8) have the
  details.
- **C2SP**: `c2sp/BLAKE3/` is a Lean specification of C2SP's BLAKE3;
  `c2sp/pr/` is the pull request, saved for Zooko's review. Filed:
  C2SP/C2SP#384 (a trace's chunk label), rems-project/isla#107 (the
  snapshot's `dup` index, Sail's fix now known).
- **bench-hashes**: 0.16.1 on `main` with its records.

## Next

1. The SME2 group loop at every group count: `prove_sme2.prove_chunks_every`,
   its harness now checking each group reads its own chunks. It proved
   under the old harness; the run under the new one, and its two loop-step
   mutants (output pointer, counter), are under way. Then it enters the
   suite, and the other SME2 kernels follow.
2. The library on the proved walk (Zooko's decision first): make
   `compress_subtree_wide` call `tools/verify/tree`'s `widecore`. SME2's
   flat path and the hybrids' partial chunk become kernels. `update_rayon`
   still runs the walk through Rayon's `join`; either it moves to the fork's
   pool, or `widecore` takes a `join`. Measured on the Mac first.
3. Promote `candidate/no-linger` once its CI passes. The next runner job
   number is 1443.

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
