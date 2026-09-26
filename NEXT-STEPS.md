# Next steps

Read this file first. The work: make the servil fork the fastest BLAKE3 in
every situation a user meets (minimax: judge by the worst plausible case),
natively on the Mac and in the VM (both first-class), measured by this
benchmark. Prefer changes that are simpler and faster together. The
principles are in both repositories' `AGENTS.md`; the fork's hardware
facts, design, and rejected ideas are in its `NOTES-servil.md` (read it
before touching kernels or the pool); this repository's are in `NOTES.md`.

## Resume here (checkpoint, September 26, 2026, 00:30 UTC; the session ran out of context)

State: fork `servil` f70c758 (pinned here); records on it (1fb6bcf), both
machines quiet. GitHub Pages serves them. **The Mac runner had stopped**
(last job 238 at 21:06 UTC); ask Zooko to restart it
(`sh ~/piplayground/blake3-servil/tools/runner/setup-mac.sh`). Job 240 waits
in `runner/jobs/`.

### Work in progress, in order

1. **`candidate/mt-name`** (fork 4db6034, pushed): `kernel_report`'s
   multithreaded split named "split over threads, SME2 on one for large
   inputs" (the tooltip Zooko noticed). Strings only; VM check passed (a
   first pre-commit run cried regression, a second found none). Waits for
   Mac job 240 (`perf_regress` servil vs 4db6034), then promote (note, push,
   delete branch).

2. **`candidate/batch-blocks`** (fork, pushed, two commits; VM checks passed,
   suites 79/75/65 lib, 22 doc; Mac verdict still needed):
   - 220de01: `hash_many` batches equal messages of 2-16 whole blocks. New
     SME2 entry point `blake3_sme2_hash16_messages_512` (the chunk kernel's
     body with a block count in x6 and a counter step in x17; the chunk
     entry sets 15 and 1). The SME2 remainder uses the NEON hybrids only for
     whole chunks and one-block parents, else the C NEON kernel.
   - 8d3a325: `hash_many_equal(input, message_len, out: &mut [[u8; 32]])`
     and `hash_many_equal_multithreaded` (pool `Work::Equal`, `cut_equal`).
   - VM, 2^16 messages, ns each: 256 B servil 39.0 (was 164), mt 8.6; the
     official crate's hidden `Platform::hash_many` 16 per call (Remco's
     method) 90.7. 64 B: servil 10.1, mt 2.2, official method 21.5.

3. **Next on that branch (Zooko, just decided): one batch API, one buffer.**
   Remove every API taking slices of slices: `hash_many(&[&[u8]], ..)`,
   `hash_many_multithreaded`, `hash_many_multithreaded_with_budget`, and
   internally `many::hash_many_on` / `hash_many_until_longer`,
   `lanes::hash_many`, `hash_many_over_pool`, `Work::Messages`,
   `cut_messages`, `Pool::hash_messages`, their tests (convert the 64 B run
   tests to the one-buffer API), the examples `many_probe.rs`,
   `many_split.rs`, and host_lab's use. Add
   `hash_many_equal_multithreaded_with_budget`. Then **rename** the
   one-buffer functions to `hash_many`, `hash_many_multithreaded`,
   `hash_many_multithreaded_with_budget`, in a **second commit**:
   `perf_regress` builds the same benchmark against both commits, so (a)
   step one's benchmark calls `hash_many_equal`, with a perf_regress shim for
   commits lacking it (a copying shim over the slice API: mark it
   `shimmed` so batch cells are not judged), and (b) the rename commit's
   shim is a free forward (`pub fn hash_many(i, l, o) { hash_many_equal(i,
   l, o) }` for commits that have `hash_many_equal` but no one-buffer
   `hash_many`). Confirm the 64 B batch path did not slow with a direct A/B
   (VM and Mac), since the check will not judge step one's batch cells.
   Update docs: "For best performance", kernel_report_many's docs, the
   fork README's preface, NOTES-servil.md.

4. **Benchmark (bench-hashes) for Remco's case** (after 3):
   - servil st/mt batch cells call the one-buffer API on the contiguous
     batch (both 64 B and 256 B).
   - BLAKE3 official's batch path becomes the crate's hidden
     `blake3::platform::Platform::hash_many::<N>`, 16 messages per call,
     flags 0 / CHUNK_START / CHUNK_END|ROOT, IncrementCounter::No, counter 0
     (exactly Remco's `hash_many_const` in worldfnd/whir
     src/hash/blake3_engine.rs); label it as the crate's hidden batch
     function. Apply to the 64 B batches too (official 47 -> about 21 ns).
   - A new use case "Batches of 256-byte messages" (x axis messages per
     batch, same points as the 64 B batches; a chip for it). SHA-256 and
     the others: a loop of one-shot hashes (no batch API). BLAKE3 official
     mt sits out (listed pale, "not measured here"). Golden vectors for it
     in `tools/gen-test-vectors.py` (MANY_VECTORS equivalent).
   - Re-record both machines; then tell Zooko the numbers for Remco.

### Remco (a potential user; context for 2-4)

Merkle trees for a binary-field SNARK (WHIR), 2^16-2^24 leaves of 256 B,
inner nodes the standalone BLAKE3 hash of two 32-byte children (64 B).
His tree (`src/protocols/merkle_tree.rs`) keeps every node, pads to a power
of two, chooses a hash engine per layer (recorded in its config; nodes may
use truncated permutations), and hashes each layer with its `HashEngine`
trait's `hash_many(size, input, out)`. The plug-in for him is a servil
engine calling the one-buffer `hash_many` (about 20 lines); a servil Merkle
tree would change his commitment format. A generic, opinionated
`servil::merkle` (domain-separated leaves and nodes, openings) is a later
design project for other users: write its trade-offs up for Zooko first.

### Then, from before

1. The text report's three-reader pass (CHECKS, TWO SPEEDS).
2. A second SME2 thread in the pool (two SME units reachable, job 187).
3. Open: hash(256 KiB)'s partial slow state; the VM's per-process two
   speeds; shared streamed 64 B two-speed on the VM.

**The `efficient` module, as measured, for Zooko to decide** (fork NOTES,
"Energy per byte"): SME2 is the cheapest kernel per byte, so an efficient
mode keeps it; its single-threaded calls would equal today's, apart from
the "minimax" NEON plans for 2-15 KiB (E-core cycles -16-24%, P +17%). What
differs is multithreading: the caller on SME2 with the E-cores' NEON
helpers at background QoS hashed 8 MiB 10-27% faster than hash() for a
third less energy, level at 1 MiB, slower below; it needs a second,
sleeping pool. Also to weigh: the pool's idle workers poll through a call,
which doubles the energy of calls with a small thread budget.

## Where things stand (September 25, 2026)

- **Decisions of the day** (fork AGENTS.md): the recommended usage first
  (one thread makes all calls; shared and misuse measured and reported in
  every benchmark, no longer a veto); `perf_regress` holds a change past 3%
  in any solo cell or 10% in any shared cell; accepted trades need every
  slowed cell ahead of every competitor and Zooko's decision; measure wall
  time and cycles, both, always (`examples/support/clocks.rs`).
- **Findings** (fork NOTES-servil.md): integer work runs beside the SME
  unit for free (job 142); streaming mode lowers the P clock to 3.93 GHz
  from 4.51; the SME unit has a slow state (3.2 cycles per ns) after idle
  time; the 18-chunk kernel is faster on P (-10%) and E (cycles -2%).
- Fork `servil`: release **blake3-servil 0.1.0** (tag `v0.1.0+7fe31c3...`,
  `tools/gen-ver.py`, Zooko's technique), then `Hasher::update_multithreaded`
  (c6d61a6) and docs. bench-hashes: 0.7.0 released, then the **streamed use
  case** (64 KiB pieces, solo and shared; six plots); records on fork
  c6d61a6, both quiet.
- Kept probes: `probe/sme-scalar`, `probe/sme2-hybrid`, `probe/neon-cold`,
  `probe/mixed-parents`, `probe/overlap-old`/`-new`.

## How to work

- **VM setup** after a restart: `sh /workspace/vm/setup.sh` (clang-19,
  pypy3, rsvg, the guest's pre-commit hook). Node and npm for the graph
  check: `apt-get install -y nodejs npm`.
- **Every `git` and `cargo` command** in the VM takes
  `HOME=/workspace/vm/home CARGO_TARGET_DIR=/tmp/target CC=clang-19 TMPDIR=/tmp`,
  `git commit` included (the hook builds; without `CC` it aborts the
  commit and leaves the branch where it was).
- **The gate to `servil`** (fork AGENTS.md "Branches"): work on
  `candidate/<topic>`; every suite; `perf_regress` on the VM (the hook, or
  `compare servil candidate/<topic>`) and on the Mac (a runner job); a
  fast-forward; the verdicts as a git note. Changes that trade one cell for
  another go to the user with their numbers.
- **The Mac** (fork `tools/runner/README.md`): the user starts the runner
  with `sh ~/piplayground/blake3-servil/tools/runner/setup-mac.sh`. Write
  `runner/jobs/NNN-name.json` naming pushed commits; wait with
  `pypy3 tools/runner/wait_for.py NNN-name`; results in
  `runner/results/`. Keep the VM idle while a Mac job runs. Mac-only
  measurements (cycles by core kind, QoS) go in a `probe/<topic>` branch
  that replaces `examples/host_lab.rs` (fork NOTES, "Probes on the Mac").
- **Records** measure the pinned fork commit: after a promotion,
  `cargo update -p blake3-servil` here and commit the lock; the VM's with
  `cargo run --release -- --all` from this directory (unpatched; writes
  `benchmark-results/` here); the Mac's as a runner job naming that fork
  commit, flags `["--all"]`, its files copied into
  `benchmark-results/AppleM4Max.darwin25/`. Before committing, run the
  graph check on both graphs and the list script on both samples files.
- **The graph's script**: `tools/graph-check/README.md` (jsdom harness,
  snapshots to render with `rsvg-convert` and look at).
- **Golden vectors** come from `tools/gen-test-vectors.py` (reference
  implementation and hashlib); a new benchmark size needs its vector
  there, and a regeneration that changes existing lines is a review item.

## Decisions made (don't re-ask)

- Contenders: at most two settings each (single-threaded, multithreaded
  uncapped); two scenarios (solo; shared = two copies of itself); wall
  time for everyone; tables per scenario; the text report keeps KERNELS;
  user views omit maintainer detail.
- One SME2 call at a time per process (fork 30c599b): taken, costs in
  shared cells accepted.
- k8 as two scalars beside a quad and a pair (P -16%, E +7% at 8 KiB):
  taken. k4 as two pairs and the "minimax" plans: rejected.
- Branch naming `candidate/<topic>`; no promotion without the Mac verdict.
- The Mac runner is launched manually by the user; code from GitHub only.

## Decided September 25 (Zooko)

- **The recommended usage first** (fork AGENTS.md): optimize for one
  thread making all calls; misuse and shared machines measured and cared
  for, no longer a veto. To ask: should `perf_regress` report shared-cell
  regressions without stopping? Also to write: a "for best performance"
  section in the API docs and the fork's README.

- The fork builds without SME2 (a warning) when the compiler cannot
  assemble it: `candidate/sme2-optional-build`, gated like any code change.
- README invites results as pull requests (a folder per machine).
- Every run reports other programs' load in its provenance (NOTES.md).

## Idea: a truly streaming (pipelined) hasher (Zooko, September 25)

`Hasher::update` is synchronous: the caller waits while we hash, and our
resources idle while the caller produces the next piece, a pipeline
bubble at every call. A pipelined API buffers between the two: the caller
hands over pieces and returns at once while our threads (the SME2
streamer, NEON workers) hash behind it; `finalize` drains. BLAKE3 suits
this as SHA-256 cannot: every piece's place in the tree is known from its
offset, so pieces hash in parallel and out of order, and only the CV-stack
merge runs in order. Design points:
- Back-pressure: bounded buffers; when full, the producer blocks (simplest,
  the standard bounded-channel answer), or an async form returns Pending.
- Copying: `update(&[u8])` borrows, so hashing after return means copying,
  which on M4 costs about what hashing costs at mt speeds. Zero-copy
  forms: the caller fills our buffers (`buffer() -> &mut [u8]`, then
  `submit(n)`; blocking on `buffer()` is the back-pressure), or hands us
  owned buffers. Buffers of one power-of-two size make every piece a whole
  subtree.
- Contract: multithreaded by nature (another thread hashes); fits the
  "one caller thread, we spread under the hood" recommended usage.
- Benchmark: a use case where the producer does work per piece (a copy
  from a source buffer, as a read would), timed end to end, so the overlap
  shows; synchronous contenders run the same producer.

## Open problems

Each stays open until controlled, explained to users with how to control
it, or at least predicted (AGENTS.md, "we own every slowdown").

1. **2-4 KiB and 2304-4470 B against SHA-256** (the list's winnable part):
   2 KiB is one NEON pair's chain, 3 KiB a pair beside a free scalar
   chunk, 4 KiB two scalars beside a pair (integer-bound); ideas estimated,
   not built: parents and root inside k4 (about 3.6%), a direct small-tree
   path (1-2%); a faster pair chain would move 2-3 KiB.
2. **Benchmarks on hardware they cannot see or steer** (the VM): runs
   report other programs' load from OS counters, but this hypervisor
   reports no steal time, so host load stays invisible in the guest (a
   reference loop timed beside the samples would show it; NOTES.md, "Load
   from other programs"). The host
   places vCPUs on P- or E-cores at will; cells come out two-speed with
   run-to-run splits. Round-by-round pairing and two-speed reporting exist;
   to weigh: inferring each sample's core kind from a reference loop timed
   beside it, extending runs until each speed's share is known.
3. **P/E classification of every sample on the Mac** (the counters exist
   in `--trace-clocks`): tables from P-core samples, E shares in the
   maintainer report, `perf_regress` P against P.
4. **Judging two-speed changes**: `perf_regress` reports each cell's 90th
   percentile but judges the 5th; the turn got no verdict because it moves
   the control. A rule for such changes is open.
5. **Shared cells are coin tosses** under the turn (which copy holds it):
   records of identical code differ by up to 60% in shared small batches
   on the VM. Predict or control.
6. **NEON goes cold** after stretches without vector work (1000 one-block
   messages cost 23% more per message than 1024 in a tight loop). Probed
   September 25 (fork NOTES, "SME2 remainders"): the remainder's order is
   not the cause. The SME unit has a slow state (cycles per ns 3.2
   against 3.93) entered after idle time of about a quarter microsecond;
   what else enters it is open (fork NOTES, "SME2 remainders"). The
   overlap group (candidate/overlap-group) trades and waits unpromoted.
   Next: measure the state machine directly (SME2 work, then X ns of
   other work, then SME2 work: speed against X and against the first
   stretch's length, and against the number of streaming sessions), then
   an overlap group inside one streaming session (a kernel entry).
7. **SME2 batch rates with work between calls** (about 12 ns/msg, not the
   benchmark's 10): whether batches should use SME2 from 16 messages.
8. **The E-core trigger's mechanism** (controlled by the turn; unexplained).
9. Later: `tools/promote.py` (check the gate, write the note, fast-forward;
   a pre-push hook refusing a `servil` tip without both verdicts); the
   Mac's serial 128 MiB rise; `many::TABLE` natively; release 0.7.0 of
   bench-hashes and a first fork tag; a GPU kernel.

## Commands

From `/workspace` in the VM, each with the prefix above:

    cargo test --release --lib [--features no_sme2 | --features pure]
    cargo test --release --doc
    cargo test --release --manifest-path test_vectors/Cargo.toml
    cargo test --release --manifest-path bench-hashes/Cargo.toml
    pypy3 tools/perf_regress.py check | compare OLD NEW
    cargo run --release --example host_lab

Expected: 71 / 67 / 57 library tests, 20 doc tests, 2 vectors, 7 benchmark
tests. Release: `python3 tools/gen-ver.py X.Y.Z` from a clean tree (two
version commits and a lightweight tag; push `main`, then the tag by name).
Never print the credential token (`/workspace/ghtokenclassic.txt`). Only
`/workspace` survives VM restarts. Commands for the user go on one line.
