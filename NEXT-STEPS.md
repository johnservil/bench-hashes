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

## Where things stand (October 6, 2026, evening)

- **The fork**: `servil` = 9d9f3d6 (= `candidate/no-linger`): the API
  docs rewritten for callers; the startup self-test in every build;
  b3sum hashing several files at once (trees 1.6-3.4x faster on the
  Mac). A rolled NEON path for calls after a pause was built, measured
  slower, and reverted (NOTES, "How much of a cold one-shot call is its
  code"): the cold cost of 2-8 KiB calls is the unrolled kernels' code,
  and only rolled hybrid kernels beside them could recover it, for about
  10%.
- **bench-hashes**: 0.16.1 on `main` with its records (Mac jobs
  1433-1434, mains, quiet; the VM), its GitHub Release, and
  `beside/0.15.3-0.16.1/`. The commonware share is in
  `shares/commonware-2026-10-06/`.

## Next

The fork's NOTES, "Future work". The next runner job number is 1435.

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
