# Next steps

Read this file first. The work: make the servil fork the fastest BLAKE3 in
every situation a user meets (minimax: judge by the worst plausible case),
natively on the Mac and in the VM (both first-class), measured by this
benchmark. Prefer changes that are simpler and faster together. The
principles are in both repositories' `AGENTS.md`; the fork's hardware
facts, design, and rejected ideas are in its `NOTES-servil.md` (read it
before touching kernels or the pool); this repository's are in `NOTES.md`.

## Resume here (checkpoint, September 26, 2026, evening)

State: fork `servil` c248f2e, pinned here; no candidates open. Records
(VM and Mac `--all`, with SHA3-256) are on 01bc76e (same src). Public
links for Zooko: the WHIR note
https://github.com/johnservil/BLAKE3/blob/servil/docs/whir-merkle-trees.md,
the upstream report https://github.com/BLAKE3-team/BLAKE3/issues/590 (fix
PR #591, with a catalog of every public caller of `blake3::platform`).
README: a short warning in my voice, one speed chart (1 MiB, cores) drawn by
`tools/speed_chart.py` from the Mac record (redraw after each Mac record),
`media/speed-charts.md` behind a link. The Mac runner's installed copy
predates the commonware and sha3-256 keys and the `test` job type: Zooko
restarts it with `setup-mac.sh`.

**The Mac is in use by Zooko (September 26, evening): no benchmark,
perf_regress, or probe results from it, or from the VM on it, count until
he says so.** Work that needs no quiet machine continues: correctness,
tests, docs, numerical tooling (`tools/check-report.py`).

Waiting on a quiet Mac: `candidate/agents-time` (the fork's AGENTS, docs
only: time is discrete), its Mac gate.

Next: the weak cells, when the machine is quiet again.

- **Time kept as measured** (done, September 26): samples are `ns/units`
  (samples v3); statistics run on `Fixed` (Q64.64) and round once, for
  the page; the report shows three significant digits (0.0311). The fork's
  perf_regress, losses.py, and speed_chart.py read it exactly (Fraction).

Found this session: the fork's test suites have never run under macOS
(the runner runs benchmarks, perf_regress, and examples; tests ran only
in the VM, on the M4's SME2 unit). Next: a runner job type `test` (the
suites natively), which needs a runner restart; QUALITY.md says so until
then.

This session (Zooko asleep; his instructions: benchmark commonware's new
BLAKE3, optimise, produce evidence of code quality):

- **BLAKE3 commonware** (commonwarexyz/monorepo PR 4982, 25851f1) is a
  contender here, batches only (2ff362e; NOTES.md "BLAKE3 commonware";
  Zooko: a two-way door). probe/commonware (fork) measured it on the Mac
  P/E (jobs 254-285). Every batch cell it led is now servil's.
- **Batch speedups, all promoted with both gates** (fork NOTES "What runs
  where", `hash_many`): padded SME2 groups for 2-16 blocks (9fd0ac9);
  NEON parent plans for 2-16 blocks below ten (666e550; 256 B x 2-9
  -40 to -58%); 2-15 chunks side by side on SME2 (0bed4e7; 16 x 4 KiB
  1310 -> 661 ns/msg on the Mac, commonware 1757); one-block padded groups
  (ffcef50); the padded batch contract, any length (7cd10ec; odd lengths
  had trailed commonware 1.7x); two-chunk messages side by side on NEON
  (0869c79).
- **Code quality** (fork NOTES "Testing", "Checks beyond the suites"):
  coverage 93% of lines and the tests it prompted; ASan, TSan (31 runs),
  Miri (pure; 41 min) clean, each with a positive control; guard-page
  tests for every kernel; a differential run against the reference (3.8
  million steps, seed 2, 20 min, clean); four bugs fixed (d395f9e).
  Raw logs in the fork's `tmp/quality/`. Nightly with miri, rust-src,
  llvm-tools and the x86-64 std are installed in the guest until restart.

### Zooko's answers (September 26, morning) and tonight's tasks

- **One-block tails from 5: landed** (servil 9eb2613, Zooko, after the
  correction): the old path ran 10.6 ns/msg at 64 B x 24 in the SME
  unit's fast state and 25.5-31.5 in its slow one; the padded group runs
  13.3 always (official 21.0-21.7). His reasons: the slow state improves
  far more than the fast state slows, and the fast state stays ahead of
  the nearest competitor. (My first summary had said we trailed at 24;
  the trailing cells at 24 are 256 B messages.)
- **`perf_regress` and shared cells: report, don't hold**, on the
  presumption that a commit which slows a shared cell has a reason worth
  more (a larger gain elsewhere, simpler code); the commit message names
  the cells, their numbers, and that reason.
- **The note for Remco: post it on GitHub** where Zooko can link it;
  say that Zooko showed us Remco's comments and asked for something
  useful to him.
- **The `efficient` module and the Merkle API: worth building, later.**
  Both are in "Ideas" below.
- **New: a quality-assurance document** linked from the fork README: every
  step taken for correctness and safety, how a user verifies it, the four
  bugs with enough detail to find each bug and fix, and the formal
  verification tools considered (why not yet, or what happened).
- Then keep improving code, docs, and speed until further notice.

### Next, in order

0. Records on dddb5d3 (VM and Mac, `--all`), graph checks, commit.
   Then weak cells: 2-chunk messages at 4 on E-cores (p4 two pairs),
   1000 B x 4 on E-cores; tails of 1-4 multi-block messages past SME2
   groups (slow state); a benchmark cell of odd-length messages (2000 B)
   would show the padded contract.

1. ~~**The padded batch contract**~~ done (7cd10ec); left: a probe of
   129-byte messages at stride 192 against 256 (the rounding rule). Kernels take
   the last block's length (SME2 `z14`, NEON packed word, hybrids); any
   message length batches: chunk k of 16 messages in 16 lanes at counter
   k, then each parent level across messages as one parent-kernel batch.
   Tests from the reference implementation at 1-3000 B and around every
   block and chunk boundary; a benchmark cell of long messages (2000 B)
   against a loop of hash(); a probe of 129-byte messages at stride 192
   against 256 settles the rounding rule.
2. **Weak cells** (minimax; both records): 256 B at 4-12 messages servil
   level with official or 1% behind (Mac 4: 99.2 against 98.0; both NEON
   four-wide); 256 B at 17-31 messages about 400 ns per call beyond the
   kernels (24: 74 ns/msg against 38 at 16; open problem 6); 64 B at 4
   messages servil 24.0 against official 22.1 (Mac; the hybrids against
   the C four-lane kernel); SHA-256 faster than BLAKE3 below 16 messages
   of 256 B (open problem 1's sizes).
3. **One cell's aftereffects slow the next** (open, ours to explain): on
   the VM, servil f70c758's shimmed 256-byte batch cells (the slice API
   behind a copying wrapper, one message at a time) made the next SHA-256
   64 B cell 3-6% slower, reproducibly, in `perf_regress`'s full point
   list and in no shorter one. 3d83912 stops running unjudged cells,
   which avoids it; the mechanism (allocator, caches, clock or SME state
   after long integer runs) is unexplained, and a user's program could
   meet it.
4. The text report's three-reader pass (CHECKS, TWO SPEEDS).
5. A second SME2 thread in the pool (two SME units reachable, job 187).
6. Open, smaller: hash(256 KiB)'s partial slow state; the VM's
   per-process two speeds; shared streamed 64 B two-speed on the VM.

### Remco (a potential user)

Merkle trees for a binary-field SNARK (WHIR), 2^16-2^24 leaves of 256 B,
inner nodes the standalone BLAKE3 hash of two 32-byte children (64 B).
His tree (`src/protocols/merkle_tree.rs` in worldfnd/whir) keeps every
node, pads to a power of two, chooses a hash engine per layer (recorded
in its config; nodes may use truncated permutations), and hashes each
layer through its `HashEngine` trait's `hash_many(size, input, out)`; his
BLAKE3 engine calls the official crate's hidden `Platform::hash_many`
sixteen messages at a time. A servil Merkle tree would change his
commitment format (see "Idea: a full-fledged Merkle tree API").

## How to work

- **VM setup** after a restart: `sh /workspace/vm/setup.sh` (clang-19,
  pypy3, rsvg, the guest's pre-commit hook). The graph check needs Node
  and jsdom: `apt-get install -y nodejs npm`, then `npm install jsdom@22`
  in `/tmp/gc` and `NODE_PATH=/tmp/gc/node_modules node
  tools/graph-check/check.js GRAPH.svg`.
- **Every `git` and `cargo` command** in the VM takes
  `HOME=/workspace/vm/home CARGO_TARGET_DIR=/tmp/target CC=clang-19 TMPDIR=/tmp`,
  `git commit` included (the hook builds; without `CC` it aborts the
  commit and leaves the branch where it was).
- **The gate to `servil`** (fork AGENTS.md "Branches"), for every change,
  a README's included: work on `candidate/<topic>`; every suite;
  `perf_regress compare servil candidate/<topic>` on the VM and as a Mac
  runner job; a fast-forward; the verdicts as a note in `refs/notes/perf`
  (`git notes --ref=perf add`, pushed with `servil`); delete the branch;
  pin here. Changes that trade one cell for another go to Zooko with
  their numbers.
- **`perf_regress` and older commits**: the benchmark calls the current
  fork API; `tools/perf_regress.py` shims older commits (renaming their
  old functions, forwarding or wrapping the new names). A comparison with
  a wrapped side measures and judges the one-message cells alone. A
  benchmark change that calls a new fork API needs a shim there.
- **The Mac** (fork `tools/runner/README.md`): Zooko starts the runner
  with `sh ~/piplayground/blake3-servil/tools/runner/setup-mac.sh`. Write
  `runner/jobs/NNN-name.json` naming pushed commits; wait with
  `pypy3 tools/runner/wait_for.py NNN-name`; results in
  `runner/results/`. A job runs once per file name: a rewritten job keeps
  its old result, so a changed job takes a new number. Keep the VM idle
  while a Mac job runs. A direct A/B is four `benchmark` jobs, old new
  new old, run back to back (spread apart, the control moves). Mac-only
  measurements (cycles by core kind, QoS) go in a `probe/<topic>` branch
  that replaces `examples/host_lab.rs` (fork NOTES, "Probes on the Mac");
  the `probe/*` branches on origin are those probes, each cited in the
  fork's NOTES where its finding is.
- **Records** measure the pinned fork commit: after a promotion,
  `cargo update -p blake3-servil` here and commit the lock; the VM's with
  `cargo run --release -- --all` from this directory (unpatched; writes
  `benchmark-results/` here); the Mac's as a runner job naming that fork
  commit, flags `["--all"]`, its files copied into
  `benchmark-results/AppleM4Max.darwin25/`. Run the graph check on both
  graphs before committing.
- **Exploratory runs** go in a scratch directory with the built
  executable (`cd /tmp/qr && /tmp/target/release/bench-hashes --quick
  ...`): a run from this directory overwrites the records.
- **Looking at a graph**: `rsvg-convert -w 1300 GRAPH.svg -o
  /tmp/g.png`, crop with `convert`, copy into `/workspace/tmp/`, and read
  it by its host path
  (`/Users/donaldturnworth/piplayground/blake3-servil/tmp/...`); the
  host sees a new file after a moment.
- **Golden vectors** come from `tools/gen-test-vectors.py` (reference
  implementation and hashlib); a new benchmark size needs its vector
  there, and a regeneration that changes existing lines is a review item.

## Decisions made (don't re-ask)

- Contenders: at most two settings each (single-threaded, multithreaded
  uncapped); two scenarios (solo; shared = two copies of itself); wall
  time for everyone; tables per scenario; the text report keeps KERNELS;
  user views omit maintainer detail. BLAKE3 official's batches go through
  its hidden `Platform::hash_many`, sixteen per call, as programs that
  want its batch speed call it; the graph says so beside its name.
- The recommended usage first (fork AGENTS.md): one thread makes all
  calls; misuse and shared machines measured, reported, and cared for,
  no longer a veto.
- One SME2 call at a time per process (fork 30c599b): taken, costs in
  shared cells accepted. The overlap group for 13-15 one-block leftovers
  (fork 4d0751f): taken, its slowed cells accepted (Zooko, September 25).
- k8 as two scalars beside a quad and a pair (P -16%, E +7% at 8 KiB):
  taken. k4 as two pairs and the "minimax" plans: rejected.
- One batch API, one buffer (Zooko, September 26): `hash_many(input,
  message_len, out)` and its multithreaded forms.
- **The padded batch contract** (Zooko, September 26): message i starts
  at byte i x s, s = message_len rounded up to a multiple of 64 (64 for
  an empty message); the caller zeroes the bytes between one message's
  end and the next one's start (the caller's obligation, so the kernels
  never mask); any message length; `assert` on the lengths, `debug_assert`
  on the zero padding (hot path); no base alignment unless a measurement
  shows it pays. Public docs state it without a new term ("slot").
- The fork builds without SME2 (a warning) when the compiler cannot
  assemble it (Debian 12, Raspberry Pi OS).
- The README's warning (new, AI-written, unscrutinized, unused) stands in
  one place, the fork README's top; no copies elsewhere (Zooko).
- Branch naming `candidate/<topic>`; no promotion without the Mac verdict.
- The Mac runner is launched manually by Zooko; code from GitHub only.

## Idea: the `efficient` module (worth building, later; Zooko, September 26)

SME2 is the cheapest kernel per byte (fork NOTES, "Energy per byte"), so
an energy-efficient mode keeps it; single-threaded calls equal today's,
apart from the "minimax" NEON plans for 2-15 KiB (E-core cycles -16-24%,
P +17%). Multithreaded: the caller on SME2 with the E-cores' NEON helpers
at background QoS hashed 8 MiB 10-27% faster than hash() for a third less
energy, level at 1 MiB, slower below; it needs a second, sleeping pool.
The pool's idle workers poll through a call, which doubles the energy of
calls with a small thread budget.

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

## Idea: a full-fledged Merkle tree API (worth building, later; Zooko, September 26)

A `servil::merkle` module that builds, opens, and verifies Merkle trees,
so a user like Remco calls one function per tree instead of looping
`hash_many` over layers. Write its trade-offs up for Zooko before
building. Design points, as we know them now:
- It rides on the batch API: leaves through `hash_many(leaves, leaf_len,
  ..)`, each node layer through `hash_many(previous_layer, 64, ..)`. A
  layer's digests lie back to back, so each pair of children is already
  one 64-byte message in place: zero copying from leaves to root, and
  the multithreaded forms split a layer over threads.
- Fused layers: hash the leaves and the lowest node layers together
  while the digests are still in cache (or in the SME2 unit's registers)
  instead of writing every layer to memory and reading it back.
- Domain separation between leaves and nodes (against second-preimage
  tricks): BLAKE3's keyed mode or `derive_key` contexts give it at no
  cost; a prefix byte would break the 64-byte alignment. An opinionated
  default and, perhaps, a mode that reproduces a plain-hash format such
  as Remco's (his commitments use plain BLAKE3 of the children), since
  changing a proof system's commitment format is its authors' call.
- What the caller gets back: the root alone, or every layer (openings
  need them); openings (authentication paths) and their verification.
- Leaf counts that are no power of two: pad to one, or carry an odd node
  up; the padded batch contract (Decisions) sets the leaf layout.

## Open problems

Each stays open until controlled, explained to users with how to control
it, or at least predicted (AGENTS.md, "we own every slowdown").

1. **2-4 KiB and 2304-4470 B against SHA-256** (the report's CHECKS we can win):
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
   overlap group for 13-15 one-block leftovers is in (4d0751f, a trade
   Zooko accepted). Next: measure the state machine directly (SME2 work,
   then X ns of other work, then SME2 work: speed against X, against the
   first stretch's length, and against the number of streaming sessions),
   then an overlap group inside one streaming session (a kernel entry).
7. **SME2 batch rates with work between calls** (about 12 ns/msg, not the
   benchmark's 10): whether batches should use SME2 from 16 messages.
8. **The E-core trigger's mechanism** (controlled by the turn; unexplained).
9. Later: `tools/promote.py` (check the gate, write the note, fast-forward;
   a pre-push hook refusing a `servil` tip without both verdicts); the
   Mac's serial 128 MiB rise; `many::TABLE` natively; a GPU kernel.

## Commands

From `/workspace` in the VM, each with the prefix above:

    cargo test --release --lib [--features no_sme2 | --features pure]
    cargo test --release --doc
    cargo test --release --manifest-path test_vectors/Cargo.toml
    cargo test --release --manifest-path bench-hashes/Cargo.toml
    pypy3 tools/perf_regress.py check | compare OLD NEW
    cargo run --release --example host_lab

Expected: 75 / 71 / 61 library tests, 21 doc tests, 2 vectors, 7 benchmark
tests. Release: `python3 tools/gen-ver.py X.Y.Z` from a clean tree (two
version commits and a lightweight tag; push the branch, `servil` in the
fork or `main` here, then the tag by name).
Never print the credential token (`/workspace/ghtokenclassic.txt`). Only
`/workspace` survives VM restarts. Commands for the user go on one line.
