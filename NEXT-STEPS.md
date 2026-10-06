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

## Where things stand (October 6, 2026)

- **The fork**: `servil` = 6d9cac3. `candidate/no-linger` (tip 81fa2b2)
  holds, each through the VM's check: the API docs rewritten for callers;
  the startup self-test in `no_std` builds; b3sum hashing several files
  at once; an unused hidden constructor removed; the runner's
  `blake3-commonware`. Mac tests and Mac `perf_regress` passed (jobs
  1421-1422, battery); CI passed on 67a2006 and runs on the tip.
- **bench-hashes**: 0.16.0 on `main` (tag `v0.16.0+9488c3e…`), its lock
  at servil 6d9cac3: the contender BLAKE3 commonware (batches), the map's
  explained headers and provenance door, Escape, `map --beside` (and
  `beside/0.15.2-0.15.3/`). It has no records of its own yet.
- **Shared with commonware**: `shares/commonware-2026-10-06/` (Mac job
  1425, mains), with its link and screenshot.

## Next, in order

1. Promote `candidate/no-linger` to `servil` once CI passes on its tip;
   its perf note; pin bench-hashes to it (`cargo update -p
   blake3-servil`) as 0.16.1.
2. Records for 0.16.x: the Mac (`--all` and b3sum, on mains, the VM
   idle) and the VM; `beside/0.15.3-0.16.x/`; the GitHub Release.
3. The fork's NOTES, "Future work".

The next runner job number is 1426.

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
