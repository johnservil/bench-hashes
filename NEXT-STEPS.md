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

1. Done: the library's walk is the proved one (0156c58, Zooko's go-ahead),
   after `update_rayon`'s removal (0e156c8). `perf_regress` passes on the
   VM and the Mac (jobs 1497, 1498; battery, busy pairs rerun).
2. The Hasher's stack as code: `src/stack_core.rs` on probe/stack-array
   (an array made at the first push, in place of ArrayVec; no cost on the
   VM); its push and merge proved against `HasherProofs.lean` (e8a75e3).
   Mac jobs 1490-1496 (battery, every run busy): `perf_regress` passes,
   and the earlier slow cells read level (64 B x1.000, 2 messages
   x1.000). Merged (40663e0, Zooko's go-ahead). The VM's check held once
   at LentBatches|16 (+8.3%), a path that never reaches the stack, then
   passed four times; the Mac's check (job 1499) gave no verdict, the Mac
   busy. Rerun it with the Mac idle before promotion.
   Then consider removing every use of `arrayvec` (the SIMD pointer
   tables, hazmat, the lanes, `to_hex`'s `ArrayString`, which is public
   API) for simplicity: one dependency fewer, and less unsafe code under
   the proofs; only where the result is simpler, not merely different.
3. Timing (the fork's QUALITY.md, "Timing and secrets", says where each
   condition stands; change it with every step here): whole `hash` calls
   are proved to run one path per length (the fork's
   `tools/verify/prove_timing.py`, 294 lengths, to 64 KiB on NEON and 4
   KiB on SME2). Next: `keyed_hash`, `derive_key`, the Hasher; SME2 past 4
   KiB (its pointer table in vector lanes); the new instruction forms
   against Isla; then measure DIT's cost (Zooko
   expects it too costly to be on by default, and an option's complexity
   too high for its benefit).
4. A description of what is proved about correctness, for two audiences:
   people deciding whether to use the crate, and experts (maintainers,
   developers changing the code or adapting the techniques, security
   reviewers). Both must be able to (1) read the English statement of
   what is proved (the output is the correct BLAKE3 hash of the input);
   (2) find the minimal, self-contained, self-documenting Lean statement
   it rests on, and map it to that sentence from the Lean source alone
   (for example, whether it proves agreement with the C2SP standard or
   with the BLAKE3 C code); (3) run the Lean checker themselves and read
   its verdict. Keep it current with every change to the proofs.
5. The SME2 flat walk (`ffi_sme2::flat_walk`) as a kernel with a proved
   contract, the four streaming forms Isla needs over 50 GB for, and the
   ZA loads and stores against Arm's specification.
6. Promote `candidate/no-linger` once its CI passes. GitHub Actions is
   disabled for the fork (a dispatch answers "Actions has been disabled
   for this repository"; no run since dc56276, whose last job was
   cancelled at 02:00 UTC, October 9); Zooko to look at the fork's
   Actions settings or GitHub's mail; the settings show nothing, and the
   account's other repository runs, so only GitHub Support can lift it.
   Proposed first: the long SME2 proofs out of every push into a workflow
   started by hand before promotion (done: `sme2-every.yml`). The
   candidate changes the message kernel, so promotion needs those proofs:
   GitHub cannot run them, so run its six groups on the VM (stopped
   October 10 to save the Mac's battery; restart them on the tip).
   dc56276's two failures are fixed in
   63a6e26. The next runner job number is 1500.
7. SHA-256 as competitive as it can be on the benchmark (Zooko, October
   10): give the SHA-256 contenders (sha2, ring) their fastest builds and
   calls on each platform (the ARMv8 SHA-256 instructions, features and
   flags, batch or multi-buffer APIs where a crate has them), so every
   comparison is against SHA-256 at its best. A change to what the
   benchmark measures is Zooko's decision, recorded in FROZEN.md.

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
