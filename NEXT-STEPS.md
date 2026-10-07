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

## Where things stand (October 7, 2026)

- **The fork**: `servil` = 9d9f3d6; `candidate/no-linger` adds the proofs
  (code unchanged but the SME2 message kernel's branch-free last-block
  length, 285ecf0; Mac jobs 1441-1442, no regression). `tools/verify`
  proves every AArch64 assembly kernel (hybrid, 327 cases; SME2, 34) and
  the library's Rust compression code (23) equal to the compression
  function of the Lean specification in `c2sp/BLAKE3`, taken from Lean
  through a kernel-checked bridge (`tools/verify/lean`).
- **C2SP**: `c2sp/BLAKE3/` is a Lean specification of C2SP's BLAKE3,
  generated and transcribed from it and checked against all of it;
  `c2sp/pr/` is the pull request, saved for Zooko's review
  (`docs/c2sp-lean.md`).
- **bench-hashes**: 0.16.1 on `main` with its records.

## Next

Promote `candidate/no-linger` once its CI passes; the fork's NOTES,
"Future work". The next runner job number is 1443.

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
