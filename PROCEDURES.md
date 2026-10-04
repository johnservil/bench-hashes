# Procedures and environment

For the servil team: how things are done in this repository. The principles are in `AGENTS.md`; the fork's procedures (the regression check, branches and promotion, the Mac runner, probes) are in its `PROCEDURES.md`.

# Records, runs, and maps

- **Records** measure the pinned fork commit: after a promotion,
  `cargo update -p blake3-servil` here and commit the lock; the VM's with
  `cargo run --release -- --all` from this directory (unpatched; writes
  `benchmark-results/` here); the Mac's as a runner job naming that fork
  commit, flags `["--all"]`, its files copied into
  `benchmark-results/AppleM4Max.darwin25/`. Run the map check on both
  maps before committing.
- **Exploratory runs** go in a scratch directory with the built
  executable (`cd /tmp/qr && /tmp/target/release/bench-hashes --quick
  ...`, or `$(pypy3 /workspace/tools/perf_regress.py build)` for the
  fork's working tree): a run from this directory overwrites the records.
- **Looking at the map**: a screenshot with Playwright and Chromium
  (`tools/graph-check/map.js` opens it the same way), crop with `convert`,
  copy into `/workspace/tmp/`, and read it by its host path
  (`/Users/donaldturnworth/piplayground/blake3-servil/tmp/...`); the
  host sees a new file after a moment.
- **VM setup** after a restart: `sh /workspace/vm/setup.sh` (git and cargo
  for every shell, clang-19, pypy3, rsvg, Node, Chromium, the guest's pre-commit hook). The checks
  run as `NODE_PATH=/workspace/tmp/node_modules node tools/graph-check/map.js MAP.html /usr/bin/chromium`.

# The fork's performance-regression check runs this benchmark

The fork's `tools/perf_regress.py` builds this benchmark twice through the `--config` patch, against the fork's `HEAD` and against its working tree, and runs `bench-hashes regress OLD NEW`: the regression rule, its points, and its margins live here, beside `POINTS`. Both sides use this checkout's source, so a change here never skews that comparison. The fork's `PROCEDURES.md` has the procedure every fork commit follows.

# Before a question goes to Zooko

Before a change or a question goes on Zooko's decision list, ask whether it adds to or rescues a second mechanism for a problem the design already solves (AGENTS.md, "Revisit complexity as you learn"). If it does, settle first which mechanism stays: make the first serve, or remove the second.

# Environment

## Where things are

- This repository (github.com/johnservil/bench-hashes, branch `main`) is checked out at `/workspace/bench-hashes`, nested inside the fork it measures.
- `/workspace` is the fork checkout (github.com/johnservil/BLAKE3, branch `servil`), which a patched build uses as `blake3-servil` (`..`). `build.rs` then embeds that checkout's branch, commit, and clean or dirty fingerprint in the provenance, leaving this directory out of the fork's status; unpatched, it embeds the pinned commit from `Cargo.lock`. `cargo update -p blake3-servil` moves the pin to the fork's `servil` tip, which follows every promotion there.
- `/workspace` is the host checkout mounted through sandboxfs and is the only path that survives a VM restart. `/workspace/vm/` holds the guest-side environment: `vm/home` (the `HOME` for `git` and `cargo`, with `safe.directory = *`, John Servil's identity, and the credential helper), `vm/home/bin/gh-cred.sh` (reads the johnservil classic token from `/workspace/ghtokenclassic.txt`; never print that file), and `vm/setup.sh`, which installs `clang-19`, `pypy3`, and `rsvg-convert`, re-points both repos' credential helpers, and installs the fork's pre-commit hook. Run `sh /workspace/vm/setup.sh` first after a restart. The fork's `AGENTS.md` describes the same layout from its side.

## Building and running

- The VM is Debian 12 on AArch64 with 16 vCPUs (inspect `nproc` after a restart). Its CPU exposes SME2 with 512-bit streaming vectors (`/proc/cpuinfo` lists `sme2`), so the fork's kernels run here. Absolute timings differ from Apple hardware; relative comparisons hold.
- The fork's SME2 kernel is `c/blake3_sme2_aarch64.S`, compiled by the `cc` crate with `-march=armv9-a+sme2`. The system `cc` (GCC 12) and `as` (binutils 2.40) predate SME2, so under them the fork builds without the SME2 kernel and warns (the user's decision, September 25, 2026, for Debian 12 and Raspberry Pi OS users); every VM build takes `CC=clang-19`, which assembles SME2, and `perf_regress` fails stop when a build on an SME2 machine lacks the kernel; `TMPDIR` gives clang a temporary directory that exists in the guest.
- Results land in `benchmark-results/` relative to the current directory, so a run from this directory replaces the records there. Records: pin the fork commit in `Cargo.lock`, then `cd /workspace/bench-hashes && cargo run --release -- --all` (unpatched), and commit the lock with the records. Exploratory runs go in a scratch directory with the built executable: `cargo build --release`, then `cd /tmp/qr && /tmp/target/release/bench-hashes --quick --contenders blake3-official,blake3-servil-st`.
- Release: `python3 tools/gen-ver.py X.Y.Z` from a clean tree makes two version commits and a lightweight tag `vX.Y.Z+<commit>`; push `main`, then the tag by name (`--follow-tags` carries annotated tags only).
- After `vm/setup.sh`, every `git` and `cargo` command runs as it is, with no prefix: it points the guest's system git config at `vm/home/.gitconfig` (its `safe.directory` covers the mount's uid 501 files; the guest runs as uid 0) and sets cargo's target directory and `CC=clang-19` in `$CARGO_HOME/config.toml`. `/tmp/target` is a tmpfs build cache; `CARGO_HOME=/usr/local/cargo`. The toolchain is rustc 1.98.1 without the `rustfmt` component, so there is no formatting check in the guest.
- The contender set is a runtime `Roster` (see `--list`, `--all`, `--contenders`). CommonCrypto SHA-256 reports itself unavailable off Apple; its FFI module compiles only under `target_vendor = "apple"`. `rustup target add aarch64-apple-darwin` lets `cargo check --target aarch64-apple-darwin` type-check that path; the full crate fails to *build* for that target in this VM because the fork's C files need Apple headers.
- Every clock read goes through the fork's `clocks/` crate (a git dependency, like the fork): wall time for samples, the thread's counts per core kind for `--trace-clocks PATH` (one line per sample interval; `tools/analyze-clock-trace.py` reads it: core placement, frequency, windows off the median frequency), and the process's CPU time for the load report. Its documentation says which clocks and why. github.com/johnservil/measure-clocks3 (needs `cargo +nightly`; clone it under `/workspace/tmp` if needed again) has `--pitfall` and `CPU-TIME-CLOCKS-AND-FREQUENCY.md`.
- `rsvg-convert` renders an SVG to PNG to eyeball it: `rsvg-convert -w 1300 file.svg -o out.png`; `tools/graph-check/README.md` drives the map and the guide.
- Commands for the user go on one line, with no `\` continuations.
- Never `sleep` in commands.
- Run long commands (builds, benchmark runs, package installs) without a timeout and let their output stream, so the user can watch progress and interrupt when they choose.
