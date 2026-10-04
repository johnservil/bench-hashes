# Next steps

Read this file first. The work: make the servil fork the fastest BLAKE3 in
every situation a user meets, the streaming APIs first (Zooko, September
27, replacing minimax: AGENTS.md, "The streaming APIs first"),
natively on the Mac first, then in the VM (the user's decision,
September 27, 2026: diagnose on the Mac), measured by this
benchmark. Prefer changes that are simpler and faster together. The
principles are in both repositories' `AGENTS.md`; the fork's hardware
facts, design, and rejected ideas are in its `NOTES-servil.md` (read it
before touching kernels or the pool); this repository's are in `NOTES.md`.
Every open item, from every block below, is in one list: the fork's
NOTES "Future work"; the blocks below are history.

## Resume here (October 4, 2026): use cases first, no lingering, Libra

**State.** Fork `servil` = a64d495. Fork `candidate/no-linger` = servil +
no lingering anywhere (0ba5a45; the queue's cells slowed 1.3-2.7x, Mac jobs
1186-1189, and the queue is to be replaced) + docs (api-design.md holds the
use-case catalogue, the APIs kept and considered, and the measurements that
settle them, with results so far). bench-hashes `main` = 0.13.0 (released);
`candidate/no-linger` = 0.13.0 + many messages at once (replaces the pieces
row) + a collection cell (git and Nix sizes) + `bench-hashes b3sum`'s one
process per file + `apps/libra-bench` (712b305). FROZEN "Changes since
0.13.0" lists them; the write and the use of each hash, charged in every
cell (api-design decisions 1 and 5), are decided and not yet built.

**Found.** The stream does not pay in b3sum on the Mac (jobs 1197-1198:
today's reader thread and update_multithreaded beat it; mapping wins for
cached files); it stays until the same is measured on native Linux with
io_uring (fork NOTES, Future work). Several streams at once: the stream
1.31x with one, level at four (job 1199). Many messages at once: SHA-256
1.6x faster than BLAKE3 (job 1201). In Libra, hashing is a small share:
johnservil/libra `faster-add` makes `add` 9.75 -> 7.37 s and `fsck` 5.57 ->
5.20 s by removing copies and batching object-index updates (jobs 1202-1203).

**Posted.** libra-tools/libra#611 (the report and the branch);
libra-tools/git-internal#183 (the refresh bug, header-free BLAKE3 IDs via
derive-key contexts, and a follow-up with the measurements). Watch both
for replies. Branches: johnservil/libra `faster-add`,
johnservil/git-internal `in-place-ids`, fork `probe/hash-each`
(`hash_each_with`, tested, never measured).

**Next.**
1. Measure `hash_each_with` in the collection cell against one call per
   item (Mac), and in Libra (`apps/libra-bench` L4 patches, unbuilt).
2. Build the write and the use of each hash charged in every cell, then
   release 0.14.0.
3. The Bao cells (api-design, measurement 3).
4. The map of small plots for the graph (tmp/map-mockup; Zooko's notes:
   one layout for small and full plots, zoom only, greying by header,
   batch glyph three stacked squares, tooltips and contender toggles back).
5. VM environment: run `sh /workspace/vm/setup.sh` after every restart; set
   `TMPDIR=/var/tmp/libra-bench-tmp` for builds (the inherited TMPDIR is the
   host's macOS path); Libra builds need `-j4` and `/var/tmp` space.

The next runner job number is 1204.

## Resume here (October 2, 2026, evening UTC): one summary, one gate, one tool

**State.** Fork `servil` = eb5e0af (promoted; its perf note has the gate):
`clocks::summary` (each run's mean; pairs of runs; the median of their
ratios and an exact sign test) replaces `clocks::speeds`; Devon Jonte's
busy-tail fix; `tools/b3sum-bench` gone into bench-hashes. Fork
`candidate/api-plan-simple` = servil + docs (PROCEDURES on layout holds,
NOTES "The regression check, calibrated" with the Mac and VM numbers).
bench-hashes `candidate/benchmark-plan` f08807a, its lock at servil
eb5e0af: the mean everywhere (report, graph, guide, checks, compare),
`regress` as eight pairs over the lent cells solo at 3% (no queue cells,
no shared, no control, no confirmation; about 30 s), priming, `compare`
naming unmeasured load, `bench-hashes b3sum`; FROZEN.md "Changes since
0.11.0"; AGENTS (both): the mean rule, and the gate's cells and margins
the team's to set. CI green on 27157be (f08807a adds the VM record).
Devon: #8 merged, #5 and #9 closed as superseded, #4 answered.
Next runner job 1174.

**Released: bench-hashes 0.12.0** (tag v0.12.0+0aca113, GitHub Release
with notes, Pages from main 250f754): records Mac job 1174 and the VM,
both quiet, graph and guide checks pass, CI green on b3b77fa. Mac jobs
1170-1173 had met one to fifteen windows just over 1 CPU of other load.

**The calibration** (the fork's NOTES): the gate as built held a
layout-only change 1 time in 8 on the Mac (servil st lent 64 B at +3.5%,
a real effect of where the code landed), none in the VM; it held +3% 4
of 7 (Mac) and 5 of 8 (VM), +6% every time. PROCEDURES says how a lone
64 B hold near the margin lands.

**Next work:** the queue's cells stable enough to judge (their means move
6-60% between processes of identical code; the streaming APIs come
first); then the rest of the fork's NOTES "Future work".

## Resume here (October 2, 2026, early afternoon UTC): simpler API, b3sum measured

Zooko set the direction this morning, then slept; John Servil worked on.
Run `sh /workspace/vm/setup.sh` first.

**Zooko's decisions (October 2, morning)**, each in its document:
- b3sum hashes on the fork's own pool (the simplicity principle: the pool
  is the first mechanism; candidate/rayon-neon stays unmerged).
- Prefetching after a pause: approved as it is.
- No thread budgets, no time-or-energy choice: threading is in a call's
  name, a call's one option its mode (fork a9ff56b; api-design.md "One
  concept per choice"). b3sum --num-threads warns and is ignored.
- Pin bench-hashes and release once today's API is promoted (decision
  2); the queue's first-use page faults fixed (441ad42, FROZEN.md
  "Changes since 0.10.0"); p4 left to John: kept for warm speed (nonstop
  batches are the streaming case).
- AGENTS: the Mac-VM trade rule is gone; PROCEDURES (both): "Before a
  question goes to Zooko" (a second mechanism?). One list of open items:
  the fork's NOTES "Future work".

**State.** Fork `servil` = `candidate/api-plan-simple` = 44ab752,
promoted through the gate (`git notes --ref=perf show servil`: suites
both machines, Miri at 4 and 16 CPUs, the VM's check by hand across the
API change, the Mac's direct A/B jobs 1139-1142: no evidence of a
change). `candidate/b3sum-pool` (0d99f9a, on 4153dfb): b3sum on the
pool, not promoted (item 4). `probe/b3sum-bench-mac`: the Mac launcher.
bench-hashes **0.11.0 released** (tag v0.11.0+096d6a2, GitHub Release
with notes, Pages building from main f564f67): its lock pins servil
44ab752; records Mac job 1145 and the VM, both quiet, graph and guide
checks pass; CI green on four platforms. gen-ver now rewrites the lock's
own entry (0.10.0's tag had a lock that --locked refused). CI's
perf-regress in the fork checks out bench-hashes' candidate/benchmark-
plan. Next runner job 1146.

**The current to-do list** (Zooko's, October 2; the rest is in Future work):
1. Done: promoted and released (above). Read the fork's CI run of
   44ab752 (its tests workflow was queued) and the Pages site.
2. **Make good benchmarks of b3sum.** Built: tools/b3sum-bench (fork; its
   README). Mac jobs 1136-1137 (two runs agree within 1-3%), VM runs;
   NOTES "b3sum, measured". Next: a runner job type for it (a restart of
   the runner), stdin and pipes as inputs, io_uring as a contender, and
   the read-into-buffers design as one.
3. **Document guarantees about memory usage** (Zooko): what each call
   allocates, ideally nothing or a small fixed bound ("at most X bytes,
   ever"). If it is hard to state simply, simplify the code until it is
   easy. The queue allocates nothing once warm (tests/queue_no_alloc.rs);
   the pool's workers' stacks, the task list's growth, the batch digest
   buffers, and the multithreaded calls' Vecs of pieces are to measure
   and state (the fork's NOTES, "Memory: what each call allocates").
   Zooko (October 2): the docs point to `initialize()` and
   `initialize_multithreaded()` as the way to make every once-per-process
   allocation at start-up, and the design moves every lazy allocation it
   can out of hashing and into them.
   Done (fork 9450ad2; VM check no verdict twice, the control moving;
   Mac perf_regress job 1147: no regression): the crate docs' "Memory"
   section; initialize_multithreaded starts the queue's delivery thread.
   Decided (Zooko, October 2): the multithreaded calls keep their list of
   pieces (48 bytes per 128 KiB of input, at most 200 more per CPU, freed
   at return), documented: a window sliding along the input would bound
   it at the cost of a second mechanism (refilling the window, merging as
   it goes), and simplicity of implementation wins here.
4. Done: b3sum on the pool, promoted (servil a926404; its perf notes have
   the gates): files of 512 KiB or more already in the page cache mapped,
   the rest read while the pool hashes (Mac 1 GiB warm 68.6 -> 37.3 ms,
   cold 367 -> 159, warm mixed tree 34.3 -> 24.4; the fork's NOTES, "b3sum,
   measured"). bench-hashes' candidate lock pins a926404. Next runner job
   1164.

## Resume here (October 2, 2026, morning): a night on the fork, Mac first

Zooko slept; John Servil worked through the night on the fork (Zooko,
October 2: "focus on the macOS/arm64 platform"). Read this block, then
the fork's NOTES sections it names. Run `sh /workspace/vm/setup.sh` first.

**State.** Fork `servil` = aa3d8d8, promoted eleven times tonight through
the gate (`git notes --ref=perf show servil` has each verdict);
`candidate/api-plan-simple` the same. bench-hashes `main` (0.10.0) still
pins the fork's b132f8c; the new pin waits on its own branch (decision 2
below). Next runner job 1136. Every Mac job from 970 on ran on mains
(the Mac was on battery until about 03:55 UTC). CI: GitHub ran three jobs
at a time tonight and later pushes cancelled earlier runs; servil
aa3d8d8's run is the one to read. At 09:50 UTC it had 59 jobs passed, 11
running (Linux/macOS/Windows library tests), 4 failed: the cross targets
(powerpc64, s390x, aarch64, armv7 under qemu) in tests/one_cpu.rs, whose
one-byte affinity mask qemu refuses (it wants whole unsigned longs) and
big-endian reads as CPU 56. Fixed on `candidate/api-plan-simple`
07a0ef6 (test only, not yet promoted): promote it through the gate
(VM suites, Mac test job, perf_regress on both) once that run ends, and
read the next run. Next runner job 1136.

**Landed on servil tonight** (the fork's NOTES have each with its
evidence; CHANGELOG says it for users):
- One-shot calls of 1-64 KiB, and a fresh Hasher's first update,
  prefetch their kernels' code after a pause (NOTES "One-shot calls
  prefetch their code after a pause"). Mac, after other work: 4 KiB
  x0.59-0.61, 8 KiB x0.49-0.68, 16 KiB x0.61-0.66 (16 KiB now ahead of
  SHA-256 ring); the NEON-only path of M1-M3 alike (8-16 KiB x0.65);
  calls of 1 KiB and less and nonstop calls level (four runs a side,
  jobs 1057-1064).
- The flat walk prefetches the next subtree's input (NOTES "The flat
  walk prefetches the next subtree"): hash() of 64-128 MiB x0.89-0.91,
  lent 64 MiB x0.90; servil st flat at 0.155 ns/B from 1 to 128 MiB.
- Extended output on SME2 (blake3_sme2_xof16_512) and NEON
  (src/neon_xof.rs): OutputReader::fill of 1 KiB and more 4.6x as fast
  (0.70 -> 0.155 ns/B), 2.3x on the NEON-only path (0.70 -> 0.30); the
  self-test runs the new kernel (40 cases, all 32 assembly entries).
- update_reader in 1 MiB pieces once a reader passes 64 KiB: files of
  8-64 MiB in the page cache 23-33% less time (b3sum --no-mmap too).
- Safety: Miri found undefined behaviour in the queue's chain (QUALITY.md
  bug 7, fixed in 0c928f9); CI's Miri step now runs the queue and
  update_multithreaded past its lingering threshold. Two bugs of the
  night's own were found and fixed before morning (QUALITY.md bug 6;
  NOTES "The short path's layout"). On the Mac, the debug suites, ASan,
  and TSan are clean on the final code (jobs 996-998, 1031, 1083).
- Reliability: the fork's CI builds and runs again (Rust 1.99, debug
  builds, no_std, wasm32, old assemblers, the cross targets' scripts and
  abort test); a warm Queue allocates nothing on macOS, and
  initialize_multithreaded returns once every pool thread has started.

**For Zooko (decisions):**
1. Decided (Zooko, October 2): b3sum hashes on the fork's own pool
   (0.021 ns/B Mac, 0.023 VM, against update_rayon's 0.054 on the Mac:
   job 1002), smallest form first: b3sum maps the file and calls
   update_multithreaded, and `--num-threads 1` calls update. A public
   thread budget waits until a need shows (fork NOTES, "Future work"). `candidate/rayon-neon`
   (c331f58) stays unmerged: it rescued a second mechanism (Rayon's join
   on NEON) for what the pool already does. update_rayon keeps upstream's
   contract.
2. Decided (Zooko, October 2): release with today's API, once promoted.
   Was: pin bench-hashes to the fork's servil (a new release of the frozen
   benchmark: records, Pages). Prepared on `candidate/pin-servil-aa3d8d8`
   (bench-hashes): the lock at aa3d8d8, the benchmark's tests passing,
   quiet records with that pin from the Mac (job 1134) and the VM, both
   passing the graph check (check.js) and the guide check (guide.js: 48
   routes, 21 endings).
3. Decided (Zooko, October 2): fixed, 441ad42. Was: the benchmark's
   queue cells measure a first-use cost in each cell's
   first sample: the queue's batch buffers come from `vec![0u8; len]`
   untouched, so their pages fault inside the timed sample, where
   take_buffers touches its buffers first (probe in the fork's
   tmp/lentprobe: 330 against 217 us a batch; job 989's traces: each
   cell's first round slow, solo and shared). A measurement fix, so yours.
4. Left to John (Zooko, October 2): kept for warm speed. Was: p4 (a
   batch of 4 after other work costs 1.74 us, 6 messages 0.98, its
   11.3 KB of code cold): your September 27 choice for warm speed.
5. Done: prefetching after a pause, approved as it is (Zooko, October 2).

**Measured and left as they are** (NOTES): hash_many_multithreaded after
other work reads up to 6% apart between servils, explained as its own
spread (12 samples of a continuous 4.8-9.0 ns/msg; bisected, jobs
1089-1121); the batch-tail padding thresholds (right at real gaps, job
1001); the c1 prefetch (too little); a woken worker's prefetch (level);
the task list's ring reset (shared 2x slower, Rejected); cleaning lines
before SME2 reads them (no help); TABLE sizes (level); NEON-only large
inputs (8% from DRAM).

## Resume here (October 1, 2026, late evening): the benchmark is frozen

Read this block, then both AGENTS.md files (new today: "Every piece earns
its place"), both PROCEDURES.md, FROZEN.md. Run `sh /workspace/vm/setup.sh`
first; node, npm, and chromium are apt packages a VM restart loses (`apt-get
install -y nodejs npm chromium`; jsdom and playwright are in
/workspace/tmp/node_modules, NODE_PATH=/workspace/tmp/node_modules).

**State.**
- bench-hashes: `main` = `candidate/benchmark-plan` = b2c028e, released as
  **0.10.0** (tag v0.10.0+e57316f, GitHub Release with notes) and
  **frozen** (FROZEN.md: measurement, contenders, rules, `regress`, and
  presentation; a change is Zooko's decision and a new release). The lock
  pins the fork's `servil` b132f8c; records from it: Mac job 968, VM
  `--all`, both quiet; Pages serve them. CI green on four platforms.
- Fork: `servil` = b132f8c (promoted three times today, each through the
  gate; perf notes on the commits); `candidate/api-plan-simple` = 933396b
  (notes and procedures since). Next runner job 969.
- GitHub: issues enabled on both repositories. Open: bench-hashes #4
  (Devon Jonte's review thread, answered item by item), BLAKE3#1 (his x86
  two-chunk batching, a draft until he trims its 59,000 lines of results;
  judge it by the frozen benchmark). Every other PR and issue answered and
  closed.

**What changed today, in one paragraph each** (the NOTES of each
repository hold the evidence, and the commit messages the numbers):
- **Simpler, by "Every piece earns its place":** the median intervals
  (bands, `~`, two bootstraps), the Python twins (speeds.py, samples.py,
  ab.py, compare-runs.py, check-report.py, losses.py), perf_regress's own
  rule, point tables, shims, slow-speed rule, after-gap cells, and
  narrowing, the long-cell budget, a flaky allocation test, a dead
  `latency` block, the stale host_lab example, all gone. Two speeds are
  drawn each as strong as its share.
- **One implementation of every rule, in Rust:** `bench-hashes compare
  OLD... -- NEW...` and `bench-hashes regress OLD_EXE NEW_EXE`; the fork's
  perf_regress.py (230 lines) only builds the two sides. `regress` judges
  each pair by the fast speeds' ratio, every run over all 14 nonstop
  points, margins 3% solo and 10% shared, no verdict on busy or unobserved
  load. Calibrated (fork NOTES, "perf_regress as it is"): identical code
  never held (VM 8, Mac 7); +9.5% held every time; +3.8% held 3 of 4 on
  the Mac, while in the VM the plant moved SHA-256's code and the control
  refused, which the Mac answers (Zooko's decision).
- **Fixed:** the queue hung on one CPU without SME2 (Devon); harness
  allocation and zeroing inside samples, scheduling balance, guide
  sentences and labels (Devon's commits, cherry-picked); clocks::load's
  short-run message and last-window tail; graphs of full runs threw on
  load (a one-point plot); check.js looped on a graph without 2-4 KiB.
- **The Mac measures quietly only with Activity Monitor closed** (its
  polling keeps half a CPU busy: job 967); expect about one Mac check in
  five to meet a background burst and give no verdict; run it again.

**Next: use the frozen benchmark to make BLAKE3 faster.**
1. Our own optimisations, each through perf_regress on both machines
   (`pypy3 tools/perf_regress.py check`; the Mac via a perf_regress job);
   Devon's BLAKE3#1 when he trims it. Candidates from the records: servil's
   cold-call code size (4 KiB after other work costs 11-15% more per byte
   than 1-2 KiB; NOTES "Cold calls pay for servil's code size"), the batch
   of 4 after other work (more per message than a batch of 2), shared lent
   batches of 64-256 (1.6-1.7x a batch of 16: the SME unit shared).
2. Open, waiting: the run-order effect (back-to-back short runs only);
   servil mt's shared queue 64 B / 64 KiB cells read apart on identical
   code in the VM (each side laid out apart), report-only.

**Pitfalls met today:** `pkill -f` on a pattern your own command contains
kills the shell; chain release steps with `&&` (a `;` pushed a tag past a
failing test once); give long checks a `timeout`; a quick run has no 64
MiB pieces, so check a full run's graph before a release.

## Resume here (October 1, 2026, night): Zooko asleep, work autonomously

Read this block, then the "early morning" block below it, both AGENTS,
and PROCEDURES. Run `sh /workspace/vm/setup.sh` first (if the guest
restarted: node, npm, chromium, gdb are apt-installed and lost on a
restart; `cd /workspace/tmp && npm install jsdom@22 playwright@1.55.1`
for the graph and guide checks, run with NODE_PATH=/workspace/tmp/node_modules).

**Zooko's requests, in order (October 1, night):**
1. **A versioned release of bench-hashes** for other people to download
   and use. Done so far: looked at the Linux engineer's fork,
   github.com/devonjonte/bench-hashes: no commits of its own (every branch
   equals ours), no issues or PRs from him. Open PR #1 on our repository
   ("Results for my computer", user Arqu, September 25): read it; take
   its results if they are clean and from a comparable commit, or say
   why not. Release mechanics: `python3 tools/gen-ver.py X.Y.Z` from a
   clean tree (PROCEDURES; semver, before 1.0 a breaking change bumps the
   minor; the last release is v0.7.0). Releases are tagged on `main`, so
   the release needs `candidate/benchmark-plan` promoted to `main`, and
   bench-hashes' Cargo.toml then names the fork's branch it should
   follow (today `candidate/api-plan-simple`; the fork's `servil` is
   older). Zooko asking for the release is his word to promote the
   benchmark; promoting the fork's candidate to `servil` is a separate
   gate (PROCEDURES: both machines' verdicts). If the fork cannot be
   promoted tonight, release with Cargo.toml following the fork's
   candidate branch and Cargo.lock pinning a commit, and say so in the
   release notes. Before releasing: fresh Mac and VM records in
   `benchmark-results/` (PROCEDURES: records on the pinned fork), README's
   links point at them, CI green, the guide and graph checks pass.
2. **Split from 256 KiB while the workers are awake** (Zooko: "sure, I
   guess"): in the fork's `lanes.rs`, `MIN_SPLIT_LEN` 512 KiB when the
   workers sleep, 256 KiB when they are polling (the pool's `sleepers`
   below its worker count, or `registered`/`lingering`). Evidence: fork
   NOTES "The split below 512 KiB, measured again" (nonstop 256 KiB
   x0.81, batches of 4096 x0.81-0.84; after a gap a lower split is up to
   2.3x slower, so it must stay 512 there). Do it after the release (the
   release ships fork code the Mac has measured), with a Mac A/B
   (old/new/new/old) and perf_regress on both machines.
3. **The held 64 B cell** (job 852): servil st 64 B after other work
   +34% on 7270b21 against fa1ec7b, a path the change never enters.
   Build a layout control (the old code laid out differently, e.g.
   probe/layout-perturb's never-called code) and A/B it on the Mac; until
   then no "no regression" claim for 802b6a5/7270b21.
4. Then the next-session list below (graph's two-speed lines, the fresh
   read of the guide and the report, consistency findings).

**Facts the next session needs:**
- The Mac runner works (Zooko restarted it); next job 853. Keep the VM
  idle while a Mac job runs. Jobs 837-840 ran on battery: check every
  verdict's power lines.
- Never `pkill -f bench-hashes` in a command that contains that string:
  it kills the shell itself. Use `pgrep -x bench-hashes`.
- A NEON-only benchmark build (what x86 and most Arm run):
  `cd tmp/perf-ab/new/src/bench-hashes && CARGO_TARGET_DIR=/tmp/target/nosme2 cargo build --release --features blake3-servil/no_sme2 --config 'patch."https://github.com/johnservil/BLAKE3".blake3-servil.path=".."'`
  (after `pypy3 tools/perf_regress.py build` refreshed that copy).
  Test both builds for any pool or queue change: the SME2 thread hides
  bugs elsewhere (the hang of 7270b21).
- The fork has no CI; its suites ran only in the VM. bench-hashes' CI
  (four platforms, green at 755cad7) prints thread stacks on a Linux hang.
- `tmp/gh-runs.sh` shows the latest CI run; each push starts two runs
  (push and the open PR's pull_request).

## Resume here (October 1, 2026, early morning)

**State:** fork `candidate/api-plan-simple` 3d02b04; bench-hashes
`candidate/benchmark-plan` (Cargo.lock pins the fork at 7270b21). Next
runner job 853. **CI green on all four platforms** (run 36805969393,
755cad7): blocker 1 below is done.

Done since the evening (each commit has its evidence):
- **The queue hung on every machine without SME2** (fork 7270b21; NOTES
  "The queue hung without SME2"): a worker slept on tasks pushed before
  the delivery thread's hold. It hung every Linux CI run since September
  30. Found with gdb in CI and in the VM (no_sme2); fixed by the simpler
  sleep condition; `tests/api_plan.rs` bursts test hangs on the old code.
- hash_multithreaded with no CPU to spare hashes as hash() does (802b6a5).
- CI prints every thread's stack when a Linux quick run hangs.
- The split below 512 KiB measured (no constant wins; decision for Zooko).
- Owned/lent in plain words; CONTRIBUTING's contender checklist; README
  opens with "Is BLAKE3 faster than SHA-256?"; METHODOLOGY corrected
  against the code (eight stale or false claims).

**Open, blocking a "no regression" claim:** the Mac held 7270b21 against
fa1ec7b (job 852) on servil st 64 B after other work, +34% fast / +17%
slow, a path the change never enters (likely layout, job 783's
effect): needs a layout control. The fork has no CI of its own: its
suites (both builds) ran only in the VM.

## Update (October 1, 2026): pieces measured at one long message

Done (NOTES, "A message in pieces: one long message, nonstop"; FROZEN.md):
the pieces sweeps after a gap are gone, nonstop lent pieces keep 64 MiB;
the graph draws a one-point plot, the guide shows `hash`'s cells for
pieces now and then. VM: tests, check-report (668 cells), check.js,
guide.js (48 routes, 21 endings) pass; a full run 64 -> about 40 s.
Mac job 836 (fork 1794a57, bench 2400dfe; quiet, mains): the same
checks pass; measuring 65 -> 41 s; servil mt 64 MiB pieces 0.098 ns/B
(829-830: 0.099-0.102). Next job 837. **Zooko restarts the runner**
before any perf_regress job (its installed copy asks for the removed
"streamed" points). Items 1 and 2 of "Next session" below are done.

Done (item 2, the chips): grouped as the measurements are (Messages /
Batches / Pieces; After idling / After other work; Nonstop, with Owned /
Lent and Solo / Shared joined under it, applying to nonstop plots
alone). A plot shows when every chip that applies to it is pressed; a
chip whose press changes nothing, as the others stand, is dimmed; a
press that would leave no plot is refused. check.js holds all three.

Done (item 3, owned and lent in plain words): nonstop subtitles say
"owned: the program hands each buffer over and fills the next while it
is hashed" or "lent: the program waits for each call to return before
refilling its buffer"; the how-to-read panel says the same, and its two
lines on reads are one. CONTRIBUTING: "Adding a contender" moved up and
rewritten as a checklist that matches the code (it claimed "checking"
and a limit of eight contenders, neither true).

**Zooko, October 1, tested (fork NOTES "The split below 512 KiB,
measured again", jobs 841-851):** no constant beats 512 KiB everywhere:
below it, after a gap, the length at the split runs up to 2.3x slower
than on one thread; nonstop, a 256 KiB split is 19% faster at 256 KiB.
Decision for Zooko: split lower only while workers poll (one branch)?
He also warns that every measurement cut can hide a later
regression: cut only what no plausible change could make informative.

**Open, moved to the fork's probes:** servil mt's lent pieces faster
shared than solo at 256 KiB-4 MiB (VM; no longer measured), and
lingering's ramp at those lengths (perf_regress covers 64 MiB only).

## Resume here (September 30, 2026, night): readiness for announcing

**Goal now (Zooko):** correctness, accuracy, trustworthiness, clarity,
ready to announce the benchmark for everyone to use and build on. Not
simplification for its own sake. Everything below is pushed; the VM
may be idle; run `sh /workspace/vm/setup.sh` first.

**State:** fork `candidate/api-plan-simple` d410d24 (clocks: two gaps,
each timed call after a gap follows the same call, one untimed first);
bench-hashes `candidate/benchmark-plan` 93fa3a2. Next runner job 836.

**Done this session (NOTES has each with evidence):**
- Load detection moved into clocks (every measurement; samples v4 with
  sample starts and load windows; tools/samples.py the one reader;
  perf_regress gives no verdict when busy). AGENTS: contracts change
  everywhere at once; one implementation of every rule; revisit
  complexity as you learn.
- Cause of the cold-cell swings: the hash's instruction lines (NOTES,
  "The cause: where the hash's code is"); then two gaps (after idling,
  after other work), each timed call following the same call, shared
  measured nonstop only (NOTES, "Shared after a gap"); plots after a gap
  without "Solo" and with subtitles saying what the program did.
- Comparisons of the new benchmark with the previous and with v0.7.0 on
  fixed contenders (jobs 799-804): agree where they intend the same.
- servil-only CHECKS removed (Zooko: they belong to the fork's tools);
  `bench-hashes.checks.txt` holds four consistency checks for every
  contender (nonstop no slower than after other work at 64 B-4 KiB;
  shared no faster than solo; core-only hashes no slower shared; no
  slower per unit than a divisor, within 32 KiB, outside idle). A fifth
  (idle agrees with busy at 8 MiB+) was a wrong expectation: macOS clocks
  a mostly idle core lower (3.8-4.0 against 4.5 GHz); now three phases
  (nonstop, after other work, after idling), each among its own kind.
- Portability and memory: CI on four platforms; no target-cpu=native;
  LF line endings; one input buffer (3.3 -> 0.87 GB); all with Mac A/Bs.

**Blocking an announcement, in order:**
1. CI green on all four platforms (check the latest run of
   candidate/benchmark-plan: `sh /workspace/tmp/gh-runs.sh`; the Linux
   quick runs were slow on hosted runners, see if they finish in time).
2. Consistency findings to resolve or state (both machines unless noted):
   servil mt's lent pieces faster shared than solo (256 KiB-4 MiB);
   servil's shared batches of 64 1.4-1.7x slower per message than of 16
   (SME unit shared) and of 256 slower than of 64 (17-24%); servil st's
   batch of 4 after other work 1.5-1.75x slower per message than of 2;
   VM only: ring's nonstop batches 30% slower solo in one process of four
   (per-process state), SHA-256 slowed beside a second copy at 64-256 B.
   Each is a finding about a contender or the machine; decide per item:
   explain in METHODOLOGY, or fix the benchmark if it is ours.
3. Pages: the published records are from the old benchmark; make new
   Mac and VM records (PROCEDURES: records on the pinned fork) once the
   fork is promoted, and point bench-hashes' Cargo.toml at the fork's
   `servil` branch (today `candidate/api-plan-simple`). Promotion waits
   for Zooko (Sept 28: no merge until the APIs and how the benchmark
   calls them are settled).
4. A fresh reading, as newcomer, regular, maintainer, of README (Zooko
   wants "Is BLAKE3 faster than SHA-256?" first for newcomers, the guide
   behind a programmer's door), METHODOLOGY (long; check every claim
   against the current code), the graph (22 plots, long; two-speed lines
   jump), the guide, CONTRIBUTING, the report.
5. Open measurement findings: the first job of a series ran 5-7% fast on
   the cores (confirm with cycles); servil's lent 64-256 KiB cells pay
   about 4x ring's read copy.

**Zooko's decisions (September 30, night):** "after other work" stays.
Remove the "message arriving in 64 KiB pieces" use cases after a gap
(Streaming, IdleStreaming) entirely: FROZEN.md, the guide's pieces shape
(its keeps-up pattern then points at the one-buffer call or the lent
pieces; decide which the guide shows), perf_regress's points, docs.
Evidence reviewed before removal (job 834, solo, after other work): up
to 64 KiB pieces are one buffer plus the timed read and the Hasher's
overhead (+5-20%; 64 B: servil 3.8 -> 15 ns/B, ring 3.8 -> 8.3); above,
servil in 64 KiB updates stays at 0.24 ns/B (one buffer 0.155; mt 0.033
at 8 MiB), SHA-256 within 1-7%: a servil property the nonstop lent-pieces
cells also show. No sign of a bug.

**Next session, in order:**
1. Remove the pieces-after-a-gap use cases (above). Tests, graph check,
   guide test, check-report; a VM run and a Mac run.
2. Graph chips, usable without understanding: group the chips so a row
   that means something only with another is visibly tied to it (the
   what-row's "Messages/Batches/Pieces, owned/lent" are nonstop-only; the
   "One buffer/A batch" chips belong to after idling and after other
   work), show which chips are relevant under the current selection (dim
   the others), and keep every row from being emptied (today the
   Messages/Batches/Pieces row can be emptied in ways others cannot).
3. Plain words for owned and lent: owned, the program hands each buffer
   over for good and fills the next while it is hashed; lent, the
   program waits for each call to return before refilling its buffer.
   Subtitles say that and nothing else; "Messages" / "Batches" name what
   is hashed. Check every page for the same terms.
4. The fresh read-through (item 4 of the blockers): README first screen
   for newcomers, "Is BLAKE3 faster than SHA-256?", the guide behind a
   programmer's door; then METHODOLOGY, graph, guide, report, CONTRIBUTING.
5. CI: https://github.com/johnservil/bench-hashes/actions (Windows passed
   build, tests, and a quick run at a799687; the others passed build and
   tests, their quick runs slow on hosted runners: confirm they finish).

**Still waiting for Zooko:** whether servil's docs keep the promise that
hash_multithreaded "runs no slower than hash" (untested since the
benchmark's servil-only checks left; the fork's project); promotion
(fork candidate -> servil, bench -> main, Pages records), which waits for
his word that the APIs and how the benchmark calls them are settled.

## Resume here (September 30, 2026, late evening)

**State:** fork `candidate/api-plan-simple` d410d24 (clocks: two gaps,
each timed call after a gap follows the same call; hashing source
unchanged since 5cfa2b4); bench-hashes `candidate/benchmark-plan`
68d57a9 (calls after a gap run alone; patterns after idling / after
other work / nonstop; plots after a gap say what the program did). Both
pushed. Latest clean Mac run: job 812 (next job number 814). AGENTS in
both repos: "Revisit complexity as you learn" (a second solution to one
problem means fixing or removing the first).

**Waiting for Zooko:**
1. Whether "after other work" (its other program and 128 MiB walk)
   earns its complexity: he judges from the plots (job 812).
2. Whether the benchmark's CHECKS list ("servil slower than ...") and
   the mt-against-st check move to the fork's tools (improving BLAKE3,
   a separate project from the benchmark), and whether servil's docs
   keep the promise "no slower than hash".

**Next, agreed:** item 5 of the grid discussion, the consistency checks
as code, for every contender alike, in a maintainers' file: patterns
agree at large sizes; nonstop fastest when small; shared never faster
than solo; ordinary-core hashes barely slowed when shared; twice the
data at most twice the time. Then, if the busy pattern stays, a clearer
drawing of two-speed cells (the after-idling lines jump between speeds).

**Open findings:** the first job of a series ran 5-7% fast on the cores
(confirm with cycles); servil's lent 64-256 KiB cells pay about 4x the
read copy ring does (SME2 and freshly written lines?); servil's code size
on calls after a gap (improving BLAKE3). Brave in the background moved
nothing measurable (811 against 812, `tmp/cmp/brave.txt`).

## Update (September 30, 2026, evening): load in clocks

Done at Zooko's request (fork a07a576; this repository's next commit):
`clocks::load` records other programs' load in about-1 s windows during
every measurement, each sample stamped with its start; samples v4 carries
both; `tools/samples.py` (fork) is the one Python reader; perf_regress
gives no verdict when a run was busy. AGENTS (both): "Contracts change
everywhere at once" and "one implementation of every rule".

Consequences for the evidence below: jobs 780 and 782 ran busy in every
run (1.1-1.4 CPUs of other load, Zooko's browser), 783 new-1 and 788 b2
had busy windows. Treat their comparisons as busy-machine evidence.

Next: **Zooko restarts the Mac runner** (`setup-mac.sh` now installs
samples.py; the installed perf_regress must be the new one); then the
caller-relevance experiment (below), on a quiet Mac; measure a load
reading's cost on the Mac. Open: `tools/speeds.py` is still a second
implementation of `clocks::speeds` (held by shared vectors); replace it
with the Rust rule's own output, or a small Rust command Python calls.
**Done (evening): two gaps and the comparisons** (fork b06c074,
bench db459e5; NOTES "The new benchmark against the previous one and
against v0.7.0"). Every call made now and then is measured after idling
and amid other work (clocks::Gap, Zooko's decision, FROZEN.md). The new
benchmark agrees with the previous where both intend the same (nonstop
cells within 1%), and its differences elsewhere follow from what changed.
Open: the first job of a series ran 5-7% fast on the cores (confirm with
cycles); servil's lent 64-256 KiB cells pay 5x the read copy others do
(SME2 and freshly written lines?); servil's cold-path code size.

**Cause found (jobs 792-798, NOTES "The cause: where the hash's code
is"):** the cold cells' excess and per-process swing are servil's
instruction lines: a data sweep leaves them in the core's L1 instruction
cache; the harness's other work, syscalls, sleeps, and moves evict them,
and the call then fetches its code from DRAM (2x at 4-16 KiB; SHA-256's
small code barely moves). Decisions for Zooko: which code state the
cold cells model (reproducible options: code evicted from every cache,
by invalidating the icache before the sweep; code in L2 only, by
invalidating without the sweep; code near, by reading it after the
sweep); and whether servil's cold-path code size is work for the fork.

**Found earlier (jobs 789-791, NOTES "Cold calls: the harness doubles them"):**
the benchmark's cold calls cost up to 2x the same calls in a direct probe
(same instructions and clock, more stall cycles) and vary per process
(4 KiB: 3.1 or 5.2 us). One syscall before the gap doubles 4 KiB through
the 128 MiB sweep. Next: find which state survives the sweep (vary one
factor at a time: a syscall without file I/O, the sweep writing instead of
reading, the thread pinned by QoS, a sweep of the system-level cache's
size), then choose a gap that models real callers reproducibly.

Open question for Zooko: perf_regress's shims for older fork commits
(PROCEDURES, "perf_regress and older commits") are compatibility code
under the new rule.

## Resume here (context reset, September 30, 2026)

**Current task, from Zooko:** establish whether the benchmark's large
small-cell swings describe typical real callers or originate in the
harness. Explain the cause with controlled tests before choosing a fix.
Zooko rejected the proposed multiple-process run as speculation. He wants
reliable, reproducible, legible, accurate measurements, and a clarifying,
beautiful guide with little text. The findings below supersede causal
claims in the earlier checkpoints and in the preceding session's replies.

### State saved

- Fork: `candidate/api-plan-simple`, d005716, pushed. Hashing source in
  `src/`, `c/`, build.rs, Cargo.toml/Cargo.lock is unchanged since 5cfa2b4.
  Changes here are clocks, tools, docs. The latest clock change warms the
  helper's counts/wall reads after the sweep; its claimed benefit needs
  a valid native test (critical builder issue below).
- Benchmark: `candidate/benchmark-plan`, 0e2742c before this handover,
  pushed. Cargo.lock pins d005716. Fourteen Rust tests, nine compiled and
  executed guide examples, Chromium checks (48 table routes, 21 endings)
  passed. Candidate branches remain separate from main-line promotion.
- Latest runner job: 788, complete on AC power. Next number 789.
  Zooko restarted the runner; job 784 verified its normal benchmark path
  works with speeds.py installed. Guest setup/browser tools were
  reinstalled after a VM restart during this session; the user is now
  resetting context only. Run `sh /workspace/vm/setup.sh` at session start.
- `/workspace/tmp/benchmark-alignment/` and `runner/results/` hold the
  evidence; they persist on the mount. Fork worktrees in tmp:
  benchmark-driver (`probe/benchmark-alignment`, 573c7e2), memory-gap
  (`probe/memory-gap`, 953b9b2). Benchmark `probe/layout-perturb` c5ac3a3
  is pushed. All main working-tree code changes are committed.
- Untracked `/workspace/README.html` appeared externally, contains an
  older render of the README. Preserve it; its origin is unknown. A copy
  is in `tmp/context-handover/README.html.preserved`. The tracked README
  has the author sentence changed as Zooko requested.

### Corrections essential to the investigation

1. **User relevance is unestablished.** The observed workload sweeps a
   fixed 128 MiB buffer before each call. Direct probes demonstrate a
   cost after that sweep; neither its typicality nor the large swings'
   cause is established. Cache/TLB/ASLR/memory placement/code layout are
   hypotheses, not findings. The earlier statement that nonstop spread
   is merely ordinary machine noise is also unestablished.
2. **Eight runs were not eight identical executables.** Job 788 had two
   benchmark builds (805324f and deliberately perturbed c5ac3a3), named
   a/ap and b/bp, with two repetitions of each. Hashing code is identical,
   but the maximum/minimum over all eight is confounded as an estimate
   of one executable's reproducibility. Compare genuine same-build
   repetitions separately, speed with speed and share with share.
   The earlier claims that all cells above 512 B stay within 5%, or
   that a quarter of typical-user cells vary by a given amount, exceed
   the evidence. Some same-build repetitions do vary substantially.
3. **Newly verified builder confound: job 788 did not test warming.**
   `tools/perf_regress.py::clocks_patch()` patches clocks to ROOT/clocks,
   irrespective of the requested fork commit. The diagnostic driver
   branch descends from 9cea065, whose clocks helper lacks the warm-up.
   Thus both its 1820efb and d005716 builds used the driver's unwarmed
   clocks. The library provenance names the requested fork, but that
   does not identify the patched clocks. The warm/no-warm conclusion
   is unsupported. Inspect the actual patched source/build commands
   before another experiment. This is a measurement-tool issue, not
   evidence about BLAKE3. The general performance check intentionally
   shares current clocks across sides; changing it needs care.
4. **Archaeology is bounded evidence.** Jobs 785-787 compared v0.1/v0.2/
   v0.3 against 1820efb using bench bc5c657. They found no held solo
   regression in the sampled cells, at a 20% after-gap threshold, with
   older APIs shimmed. This does not prove equality at all sizes or in
   all real workloads. The historical records were back-to-back, unlike
   current cold-cache cells. The latest claim that the regression was
   conclusively refuted was too strong, especially for the pieces APIs.
5. **Rendering omits uncertainty.** The main SVG carries within-run
   confidence bands and two speeds; the new guide currently shows
   median lines and fainter second-speed marks, without confidence
   bands or evidence about repeat-run variability. A narrow within-run
   band does not establish repeatability. Improve the representation
   using verified evidence; do not disguise the unexplained spread.

### Latest guide and user requests

The user was confused by `hash` giving very different results depending
on the load-pattern answer. He requested a fresh intuitive design rather
than more explanation. Current page (`src/guide.html`, generated by
`generate_guide` in src/main.rs): `Use hash`, one sentence, computed
performance sentence, chips reading `Called [now and then / nonstop],
[alone / beside another program]`, throughput chart, doors for the
compiled example and measurement details. Table removed at his request.
Kernel shapes and native hover titles restored; hover shows time per
input, rate, kernel. Full SVG remains separate. New guide data payload
is independent of the SVG, computed from the same Results. Tests in
`tools/graph-check/guide.js`. Latest generated guide is VM data under
`tmp/benchmark-alignment/vm-guide2/benchmark-results/`; screenshots
`guide3-hash.png` and `guide3-hash-nonstop.png`. Job 784's Mac guide is the
previous design (with a table); a new native guide job is still needed.

**Review issues for the next session:**
- A pattern toggle must preserve the actual function. For multithreaded
  pieces the after-gap table calls `update`, while lent nonstop pieces
  call `update_multithreaded`; the current generic toggle can therefore
  falsely imply the same function. Restrict it to truly identical calls
  or model the two functions explicitly. Likewise audit queue kernel
  labels against what the fork reports for queue tasks.
- Rust Mark::DownTriangle emits `downward triangle`, whereas the guide's
  MARK map uses `down-triangle`; it falls back to a circle. Restore it.
- Summary and ratio wording currently use fast-speed medians; assess
  claims against both speeds/shares and uncertainty before stating a
  winner. The temporary guide lost CI bands.
- Read the whole page as newcomer, regular, maintainer. Follow both
  AGENTS style guides. Revisit README/METHODOLOGY descriptions after the
  latest guide changes; some still describe the earlier table design.
- `bench-hashes/README.md` is the future Pages home. Zooko wants a
  newcomer to see “Is BLAKE3 faster than SHA256?” first, with the guide
  behind a programmer's door. Heading renamed “Which function to use”.
  Published Pages/records still reflect older main-branch code; candidate
  README is visible locally or on GitHub's candidate/benchmark-plan.

### Next, in order

1. Read this block, both AGENTS, PROCEDURES, and the relevant NOTES.
   Preserve the user's scope: diagnose user relevance first.
2. Make a small direct-call experiment for representative caller work
   between calls. Hold implementation, inputs, timer, and producer
   fixed; vary one documented factor at a time. Compare with the
   benchmark harness on the same work and the same machine. Collect
   raw samples, wall and per-core-kind cycles through clocks; analyze
   speeds and shares through the shared rule. Native Mac first, VM
   after. Keep the VM idle during native measurements.
3. Treat the 128 MiB sweep as an experimental condition, not a premise
   about normal programs. Investigate tiny hash/hash_many-one and 2-16
   KiB pieces; distinguish genuine API costs from harness costs and
   scheduler/hardware effects. A one-message batch is hash_many with
   one message (same input bytes, batch API contract); its fixed costs
   deserve direct comparison with hash, not hand-waving.
4. Resolve or bound causes, then design the simplest measured remedy
   and an honest visual indication. Zooko explicitly rejected increasing
   processes to hide unexplained behavior. Keep unexplained slowdowns
   open; no “reliable” or “no regression” claim until supported.
5. Fix guide function/pattern fidelity and marks, check fresh renders
   and browser tests, generate a Mac guide, update docs and handover.
   Promotion/Pages publication waits for the contract and validation.

### Evidence map

- 780: 4f29643 versus f3515ab, same fork 9cea065, old/new/new/old;
  historical replay 250a3dc on b9ec183. Mains. Raw traces and reports in
  old-1/new-1/new-2/old-2/historical. Initial memory-gap change.
- 781: fresh-process direct-hash register/8 MiB/128 MiB gap probe;
  gap-samples.csv, gap-report.txt; source probe/memory-gap. Read raw
  wall/cycles alongside distributions; code/context differs from harness.
- 782 (battery), 783 (AC): f3515ab versus 9cf2787, fixed fork 1820efb;
  API preselection and batch-one dispatch fix, old/new/new/old.
  `tmp/benchmark-alignment/preselect-ab*.txt` are comparison outputs.
- 784: native guide bc5c657 on 1820efb, default run ~50.8 s; exact
  report checker 964 cells and browser tests passed. Old guide design.
- 785-787: native perf_regress archaeology; read full logs and shims.
  Against v0.2, shared mt OneMessage 256 KiB slower speed/share changed;
  solo check passed at its margins.
- 788: eight runs, source probe/benchmark-alignment 573c7e2, bench
  805324f / c5ac3a3, a1/ap1/b1/bp1/ap2/a2/bp2/b2. AC throughout.
  Clocks patch confound above means warming was held fixed, contrary
  to the job's intended labels. Check source, not labels alone.

## Earlier checkpoint (September 30, 2026, morning: the table measured, the gap built, the guide)

**Start**: `sh /workspace/vm/setup.sh`. Both repos clean and pushed:
fork `candidate/api-plan-simple` 1820efb (hashing source unchanged
since 5cfa2b4), bench-hashes `candidate/benchmark-plan` 9cf2787 (Cargo.lock
pins the fork at 1820efb). Runner jobs run to 782; the next is 783.
Probes: `probe/benchmark-alignment` (the native A/B driver),
`probe/memory-gap` (fresh-process gap probe). Scratch results:
`/workspace/tmp/benchmark-alignment/`.

**For Zooko, first:**
1. **Restart the Mac runner** (`sh ~/piplayground/blake3-servil/tools/runner/setup-mac.sh`):
   its installed perf_regress.py imports speeds.py, which setup-mac.sh
   left out (jobs 776-779 failed before measuring; setup now copies it).
   Tonight's native runs went through a host_lab driver instead.
2. Done: job 783 repeated 782 on mains and agrees (NOTES, "Job 783");
   battery changed nothing measurable with the busy gap (0 E-core calls
   in both). New finding there: cells under about 1 µs after the gap
   move about 20% with the harness's code layout (a recompile), for ring
   as for servil. Decision wanted: accept and state, or warm the
   harness's code before the call.
3. The guide was redone after your review (bc5c657: one scrolling page,
   a compiled example per call, throughput chart, latency table in
   ns/µs/ms). Job 784 makes one from Mac data through the restarted
   runner: `runner/results/784-guide-mac.*/benchmark-results/AppleM4Max.darwin25/bench-hashes.guide.html`.
   Read it as the three readers; the README now leads newcomers to the
   graph and programmers to the guide (Pages after your reading).

**Done tonight** (each commit message has its evidence):
- The table in the benchmark: three new continuous axes with buffers
  lent (`LentMessages`, `LentPieces`, `LentBatches`: `hash`/
  `hash_multithreaded`, `update`/`update_multithreaded`, `hash_many`/
  `hash_many_multithreaded`, back to back, reads timed), beside the
  queue's owned-buffer axes; after-gap servil mt pieces now call `update`
  (the table). FROZEN.md records it as your decision of September 28,
  evening; perf_regress covers the new axes (8 points).
- The gap as decided: `clocks::measure_after_gaps_prepared` sweeps a
  kept 128 MiB buffer (written pages), integer work for the rest of 1 ms,
  writes the input (timed apart; trace rows `preparation solo and
  shared`), then times the call. VM perf_regress check passed.
- The guide (`bench-hashes.guide.html`, every run): the five questions,
  an "I'm not sure" default on each (one thread, one buffer, keeps up,
  lent, time), the recommended call, and this run's graph focused on
  that call beside SHA-256 ring. Synchronous calls open in latency (ns
  per message or batch), the queue in throughput. Energy says pending.
  Chromium test: 48 table routes, 21 clicked endings, defaults, Back.
- Harness defects found by your historical check and fixed: dispatch
  inside the timed call (API now selected before the gap); a batch of
  one called `hash`, not the frozen `hash_many`; Python's speed split
  disagreed with Rust at Q64 boundaries (13 shared vectors now); decimal
  halfway medians rounded down (the open 51.3875 cell: displayed medians
  now keep the exact measured ratios). The exact report checker passes
  every one of 964 cells (VM and both Mac new runs).
- A sparse `--points` run no longer panics in the graph (found by
  perf_regress narrowing to one queue cell).

**The historical comparison (your method)**, Mac, identical hashing
code (job 780, mains power; `tools/compare-runs.py`):
- Replaying the published record's exact pair (250a3dc on fork b9ec183)
  today, new/old fast speed: from 16 KiB within 1-3% (servil 128 KiB +4%);
  below 16 KiB servil +3-11%, ring +0-8% (solo, one message and pieces).
  The machine matches the record's from 16 KiB; the small cells' drift
  since September 27 is open (both hashes alike, after the gap only).
- New gap against old gap, same code: small calls slower for servil and
  ring alike (64 B servil 61 -> 224 ns, ring 80 -> 202 ns). A
  fresh-process probe (job 781) shows the cold caches' true cost is
  smaller (64 B 52 -> 135 ns, one speed after 128 MiB); the rest was the
  harness (about 101 instructions of dispatch in the interval). Job 782
  (battery, provisional): preselection brings servil 64 B 3.66 -> 2.51
  ns/B fast speed, batches of 16 28.1 -> 20.5 ns/msg (old/old and new/new
  within 4%); ring's 1-message batch +19% beside a same-code spread of
  10-21%: noise until the mains repeat says otherwise.
- From 16 KiB and in the queue's cells: level (within 3-5%, old/old alike).
- Lent against the old after-gap cells: not the same workload (back to
  back with a read, versus after a gap); servil 64 B 0.67 -> 0.78 ns/B,
  ring 0.63 -> 0.76: the read's copy at small sizes, level from 16 KiB.

**Open** (each blocks a "measures correctly" claim until resolved):
1. Small after-gap cells still split in two in the full benchmark (64 B,
   1-4 KiB), where the fresh-process probe shows one speed at 64-512 B:
   something the benchmark's schedule leaves remains. Probe: the same
   cells alone (`--points`), then with neighbours, on mains power.
2. 4 KiB splits even in the fresh probe (1396 ns 88% | 2151 ns 12%, all
   P-cores at 4.5 GHz, cycles 6822 | 10247): servil's, to explain.
3. perf_regress on the Mac with the new benchmark (after the restart).
4. Decisions for you: whether 128 MiB is the gap's size (the sweep's
   duration is unmeasured: it sets a gap of at least 1 ms); whether the guide belongs
   on the Pages home; the queue's model and the trades still wait (below).

## Resume here (September 28, 2026, night: the new adventure; work all night)

**Start**: `sh /workspace/vm/setup.sh`; both repos clean and pushed (fork
`candidate/api-plan-simple`, bench-hashes `candidate/benchmark-plan`,
whose Cargo.toml now follows the fork's `candidate/api-plan-simple` for
both `clocks` and `blake3-servil`). The Mac runner is running (Zooko
restarted it this morning); runner jobs run to 775, the next is 776.
Build the benchmark against the working tree with `pypy3
tools/perf_regress.py build` (a plain `cargo build` in bench-hashes links
the fork from git: the VM "slowdown" of the morning was that).

**The plan for tonight (Zooko, September 28, night)**:
1. Done: the fork's `docs/api-design.md`, "Five questions lead a user to
   one call": the adventure and table rewritten (does the program's
   thread keep up with its data; who controls the buffer the data first
   lands in). Read it first.
2. **Update the benchmark so each cell of that table is measured as its
   users call it**, `FROZEN.md` with it (each cell's call, pattern,
   reason; its test compares). A proposal to build, cell by cell:
   - one thread: `hash`, `Hasher::update` per 64 KiB piece, `hash_many`,
     each call (each message) after the gap: as today's servil st cells;
   - several threads, keeps up: `hash_multithreaded` and
     `hash_many_multithreaded` after the gap (today's); pieces:
     `Hasher::update`, which is servil st's cell (servil mt's streamed
     cell through `update_multithreaded` leaves this column);
   - several threads, does not keep up, buffer yours: the queue's
     continuous cells (today's, from a fixed set of buffers kept across
     samples);
   - several threads, does not keep up, buffer lent: new continuous
     cells of synchronous calls back to back: `hash_multithreaded` per
     message, `update_multithreaded` per 64 KiB piece (the streamed
     message's pieces in swift succession, messages one after another),
     `hash_many_multithreaded` per batch; the other contenders run the
     same producer through their own calls;
   - the gap as decided: walk a fixed buffer larger than the caches
     (evicting code and data), then write the input (untimed, a read),
     then call (api-design.md, "How the benchmark measures each"); build
     it in `clocks::measure_after_gaps` (the buffer and the write belong
     to the caller: a closure for the preparation, timed apart) and
     measure that small cells come out at one speed (NOTES "The busy
     gap": today they split by the previous cell).
   Keep run time near 50 s on the Mac; check the graph (tools/graph-
   check) and the report as the three readers.
3. **Benchmark the code, then optimise under it**: Mac full runs, A/Bs
   old/new/new/old read with `pypy3 tools/ab.py` (speed with speed, beside
   old-vs-old and new-vs-new); never a pooled median (AGENTS, "every cell
   may run at two speeds").

**Decisions of today (Zooko)**, each in its document:
- the energy form does not linger; the energy form itself deferred until
  the benchmark, API, and architecture are settled;
- the split at 512 KiB (taken, c46c57c);
- the continuous cells in a phase of their own (a measurement fix,
  9869b27);
- the queue and its returns kept across samples in a bounded channel
  (no allocation after warm-up; FROZEN.md, c027bfe);
- the gap is busy work (done, ea7621b/bafa9ec), and it will evict the
  caches and write the input before each call (to build, step 2);
- the two-speed rule is shared code (`clocks::speeds`, `tools/speeds.py`,
  `tools/ab.py`, perf_regress; AGENTS in both repos);
- no merge to `servil`/`main` until the APIs and how the benchmark calls
  them are settled.

**Open, for Zooko or for measurement**:
- The concurrency models (fork api-design.md "The queue" has today's; the
  chat of September 28 walked three: today's queue with the program's
  buffers; one process-wide ring of BLAKE3's buffers with fill and hashed
  events; BLAKE3 doing the reads, `hash_range(file, offset, len, tag)`).
  My proposal: keep today's as the base for data in memory, move delivery
  onto the worker that completes the oldest entry (one handover fewer,
  and no delivery thread polling while entries are in flight), model 3
  later for files and sockets. Zooko has not chosen.
- A lone message in the queue waits about 1 us for company, then a wake
  of 15-45 us: hand the first message into an empty queue over at once?
- The queue's shares swing per run (256 B messages, batches of 16, 1 KiB:
  whole runs near 0% or 90% at the slow speed; jobs 766-773): something
  set at process start (thread placement?). Probe where each queue thread
  runs; 15 short runs a side of those cells before any A/B of them.
- The three trades (members-32k, subtrees-32k, linger-4) to measure again
  speed with speed (their earlier readings pooled two speeds).
- The Choose-your-own-adventure is not yet in the crate docs (src/lib.rs
  opens with "For best performance"): write it there once the table
  settles.

## Resume here (September 28, 2026, morning: Zooko's decisions, the two phases)

**Zooko's decisions (3:45 am):** the energy-saving form does not linger
(docs/api-design.md; to build with the time-or-energy argument on the
multithreaded synchronous calls); the split at 512 KiB, taken (fork
c46c57c; the CHANGELOG says so); the trades (lingering on four threads,
messages under 32 KiB gathered, subtree tasks of 32 KiB) still wait for
him.

**The continuous cells were measured at a low clock** (fixed, 9869b27;
NOTES "Threats to validity" 0): sampled in the same rounds as the calls
after the gap, they ran near 3.0-3.3 GHz on the Mac; alone, 4.4 GHz.
Every contender's continuous cells read 30-100% slow, which is why they
matched none of main's back-to-back tables. Now the run measures the
continuous use cases first, in a phase of their own. Mac, the fork at
c46c57c (jobs 740-741, two runs agreeing within 2%): SHA-256's messages
one after another 0.35-0.36 ns/B from 1 KiB (main's back to back 0.34,
plus the read's copy); servil mt 0.75 against 0.56 at 64 B, 0.19-0.20
against 0.37 at 256 B, 0.12 against 0.36 at 1 KiB, 0.056-0.060 from 16
KiB; batches one after another 5.3 -> 2.4 ns/msg against SHA-256's 31.
Every A/B of the continuous cells before 9869b27 ran at the low clock;
runs of those cells alone (the "alone" numbers below) did not.
**The calls after the gap now meet a program that only sleeps and
hashes:** full clock in 11% of solo samples (was 18%), and their medians
split two ways between runs by up to 2x (SHA-256 1 KiB 1.08|2.46 and
1.71|2.59); their CHECKS are mostly noise. What the program does in the
gap (sleep today) is Zooko's open question; a program that works in the
gap keeps its clock up.
**Measured again this morning** (Mac, the fixed benchmark; fork NOTES
"The trades, measured again" and "The 64-byte cell at full clock"):
members-32k, subtrees-32k, and linger-4 each still a trade (numbers
there); the 64-byte continuous cell's spread (0.54-2.3 ns/B a sample,
the fastest matching SHA-256) is stall cycles in the program's thread at
a steady clock and instruction count; `submit` costs about 320
instructions and 115 cycles a message; a slot prefetch was level. The
Mac's test job passed on c46c57c (job 756).
**VM, with the two phases** (full run, `/workspace/tmp/vm-phases/`;
built with `perf_regress.py build --side bench`: a plain `cargo build`
in bench-hashes links the fork from git, `candidate/api-plan`, not the
working tree, and reads 3-5x slow): continuous cells faster for every
contender (SHA-256 0.34-0.36 ns/B against last night's 0.41-0.45); servil
mt 1.13 against SHA-256's 0.52 at 64 B, 0.28 against 0.36 at 256 B, 0.17
at 1 KiB, 0.05 from 256 KiB.
**The runner:** restarted by Zooko (installs `perf_regress.py` with the
fix ed66897: the clocks patch followed the tool's own directory, not
`--root`); its perf_regress jobs work again (open item 4 below is done):
job 764, the split (c46c57c) against 80fed28 on the fixed benchmark, no
regression on the Mac (the VM's pre-commit check agreed).

## Resume here (handover, September 28-29, 2026, overnight session)

**For the next session (John Servil, September 29).** Start with `sh
/workspace/vm/setup.sh`; both repos are clean and pushed (fork
`candidate/api-plan-simple`, bench-hashes `candidate/benchmark-plan`).
The VM was not restarted, so nightly Rust (rust-src, for TSan and ASan)
and `linux-perf` are still installed in the guest (a restart would
remove them). Lessons of the night: trace before guessing
(`--trace-clocks`, and probes that time each stage found the calibration
defect, the task-list lock, and the part-filled-task loop); A/B on the
Mac with old/new/new/old and compare medians from the samples, since
two-speed labels mislead; the VM and the Mac disagreed more than once
(two task lists), so the Mac decides; contended words on lines of their
own paid three times. Scratch probes live in `/workspace/tmp/qlab`
(queue throughput with stage timers, VM) and `/workspace/tmp/idlecheck`
(CPU a process spends while idle).

**At a glance.** The benchmark asks the same of the fork (FROZEN.md
unchanged); five measurement defects in it are fixed (the queue's cells
timed a dozen messages, short streams paid a 64 KiB memset, batches an
allocation, cells freed each other's buffers, gap samples summed 50
gaps), and a full run takes 47 s on the Mac. The fork, measured alone on
the current benchmark (jobs 694-697): messages one after another 2-2.4x
faster, batches one after another 2.6-4x, long messages through
`update_multithreaded` 2.4x (lingering), synchronous calls level. Against SHA-256 on the Mac every continuous cell wins but
64-byte messages (at two speeds, 0.78|1.11 against 0.93 ns/B); the small
synchronous calls after the gap still lose (SHA-256's hardware below 8
KiB). Decisions waiting for Zooko: lingering's bound and energy (4-6x
the energy for 2.4x the speed), the 512 KiB split (Mac +40%, VM -25%),
and the trades listed below. Everything is on `candidate/api-plan-simple`
(fork) and `candidate/benchmark-plan` (bench-hashes), unmerged.

**Where things stand.** The fork's work is on `candidate/api-plan-simple`
(the API plan, `candidate/queue-simple` merged in, and tonight's
changes); bench-hashes' on `candidate/benchmark-plan`. The Mac runner
built every job from those branches (jobs 475-733). Nothing is merged to
`servil` or `main`; the gate (PROCEDURES.md) is still to run. Before
merging: point bench-hashes' Cargo.toml at the fork's `servil` once
`candidate/api-plan-simple` lands there (it names `candidate/api-plan`
for `clocks` today), and restart the Mac runner (`setup-mac.sh`) so its
installed `perf_regress.py` is the new one.

**What changed in the benchmark** (fixes to how it measures; what it asks
of the fork, FROZEN.md, is unchanged):
- The continuous cells were calibrated from one input: a queue's first
  input costs tens of microseconds, so a sample held 12 messages of 64 B,
  never the 1024 in flight FROZEN.md asks for. They now start calibration
  at twice the buffers in flight, and no sample holds fewer (servil mt 64
  B messages 97 -> 3.9 ns/B on the VM before any fork change).
- The streaming use case allocated and zeroed a 64 KiB read buffer per
  message (about 6 us after the gap, every contender alike: 64 B streams
  read 95-105 ns/B); the batch use cases allocated their digest array per
  call (256 KiB with page faults at 8192 messages, BLAKE3 contenders
  only). Both are kept from call to call now.
- The continuous cells' read buffers were one set shared by every cell,
  so each cell freed or grew the previous cell's inside its own sample;
  now a set per count and length.
- A synchronous cell's sample was sized from the call's time back to
  back (a 64-byte call's sample summed 50 calls, each after its own 1 ms
  gap); now from four calls timed after the gap. Run time: VM `--quick`
  46 -> 14 s, a full default run 32 s; Mac full run 96 -> 47 s (job 503),
  builds included.
- The report's opening names the tables it shows and how the program
  calls in each; README, METHODOLOGY, CONTRIBUTING describe the five use
  cases; the graph's door names today's calls.

**What changed in the fork** (fork NOTES, "The queue, as rebuilt" and
"Lingering between multithreaded updates", have the mechanisms and
numbers):
- The queue: submitters and the delivery thread share no lock on their
  common paths (entries chained in submission order), locks polled
  before parking, 64 short messages to a task, small `Queue::fixed`
  batches gathered into tasks. Mac, solo, the start of the night -> its
  end (runs of those cells alone): 64 B messages 2.6 -> 0.65-0.72 ns/B,
  256 B 0.63 -> 0.21, 16 KiB 0.165 -> 0.056; batches of 16 34 -> 5.5
  ns/msg, of 64 21 -> 5, of 256 11 -> 3.3; shared batches of 16 84 ->
  8-9. VM: continuous cells 2.3-2.6x faster (geometric mean),
  synchronous cells level within their noise.
- `Hasher::update_multithreaded` lingers (Zooko's decision in
  docs/api-design.md; the bound, 50 us, is his open question): long
  messages in 64 KiB pieces 2.4x faster (Mac 128 MiB 0.24 -> 0.098 ns/B,
  solo and shared), short ones level.
- The delivery thread hands over a part-filled task of short messages
  only after waiting 16 rounds on it: batches of 16 and 64 lose their
  slow speed (a fifth of the fast one in some samples), 1 KiB messages
  30% faster (Mac jobs 701-704).
- Contended words apart: the task list's lock and counts, and the
  queue's locks, each on lines of their own, and free slots reused
  oldest first: Mac 16 KiB messages 14% faster, 256 B-1 KiB 5-11%, 64 B
  9% (fork NOTES, "Hot words on lines of their own").
- The SME2 thread runs gathered tasks (short messages, small batches) on
  NEON: 64-byte messages and batches of 16 and 64 10-15% faster on the
  Mac; for subtree tasks its SME2 matches NEON in speed (jobs 604-607)
  and stays, for its lower energy per byte.
- Idle workers and the SME2 thread sleep after 50 us with nothing to
  take, even while a queue holds the pool (they polled until the stream
  drained): about 10% less CPU for the queue's short messages, speed
  level on both machines; the VM's continuous 64 B cell 3.9 -> 1.2-1.4
  ns/B.
- `perf_regress` knows the five use cases: after-gap cells at 20%,
  continuous 3% solo and 10% shared, 28 points, about 25-30 s of runs on
  the VM; a first calibration (fork NOTES, "perf_regress"). Advisory
  tonight (Zooko): most commits went in with `--no-verify`.
- `clocks::process_energy_nj` reads the process's energy on macOS (the
  counter probe/energy used), for stage 3; not validated for cells.
- A flaky test fixed: `tests/queue_no_alloc.rs` failed once in 60-100
  runs (half the time under TSan), from before tonight: the pool's task
  list grew with the threads' timing. Each queue now makes room in it for
  what its entries can have waiting; 0 failures in 150 runs and 8 under
  TSan. It also made the Mac's 64 B messages 22% faster (0.95 -> 0.74
  ns/B) and 256 B 13% (jobs 646-649).
- Tests: `tests/api_plan.rs` checks messages in pieces through
  `update_multithreaded` against the reference implementation, and one
  queue shared by several submitting threads; TSan (nightly,
  `-Zsanitizer=thread`) clean on api_plan, the library tests, and a
  million 64-byte messages through the queue; ASan clean on api_plan and
  queue_no_alloc; every suite passes on the VM (86 / 82 / 71 library
  tests, 22 doc, 14 api_plan, 1 queue_no_alloc, 2 vectors, 10 benchmark)
  and the Mac's test job (677; its runner predates the integration
  tests); on the Mac, probe/queue-check (job 678) compared 1.1 million of
  the queue's digests (every shape, many lengths, one queue shared by four
  threads, lingering streams) with the one-shot calls: all equal.
- Tried and left out tonight (fork NOTES has each with its numbers):
  slots on 128-byte lines, a delivery back-off, a short message's digest
  in its slot, grouping a task's short messages into one entry, linking a
  task's members through their entries (candidate/member-links), pollers
  pausing after a failed try_lock, wakes as a tree, the caller's later
  pieces on SME2, 4 KiB pieces, the minimax NEON plans after the gap, no
  SME2 thread, member blocks, the pool's threads at user-interactive QoS.
  Trades left for Zooko: the 512 KiB split, lingering on four threads,
  messages under 32 KiB gathered, subtree tasks of 32 KiB (large messages
  4-14% faster, 64 KiB 5% slower). Probes kept as `probe/*`
  branches, each cited there.

**The fork's night, measured alone** (Mac, the current benchmark on the
starting fork b467ba6 and the final one, full runs old/new/new/old, jobs
694-697, in `/workspace/tmp/overnight/`; geometric mean of new/old
medians by use case): messages one after another 0.49 solo and 0.42
shared, batches one after another 0.39 solo and 0.25 shared, a message
in pieces (servil mt) 0.89 solo and 0.81 shared (long messages 0.4), the
other synchronous cells 0.97-1.07 (their code is unchanged below the
split; single cells swing with the clock states after the gap, cells of
one code path moving opposite ways in st and mt, their 5th percentiles
level). One cell slower beyond that noise in the earlier comparison
(jobs 572-575): shared 256 KiB in pieces, +11% (both copies start
lingering after their second piece).

**VM standing** (full run of the final pair,
`/workspace/tmp/overnight/vm-full/`): the continuous cells win from 256 B
(0.43 against 0.52 ns/B; 1 MiB messages 0.077 against 0.35; batches 3.4-16
against 32-39 ns/msg) and lose at 64 B (1.40 against 0.74).

**Standing, Mac full run of the final pair (job 679,
`/workspace/tmp/overnight/mac-final/`):** the continuous cells all win
against SHA-256 but 64 B messages, which run at two speeds (servil
0.78|1.11 against 0.93 ns/B; shared 1.22 against 0.90; 0.65-0.72 against
0.56 in runs of those cells alone, where the machine stays warm between
samples: a handover's
cache lines cost about what SHA-256 spends on the whole message; the
harness's channel alone takes 25 ns; `Queue::fixed` is the API that
beats it). Streams of 64 KiB pieces win from 16 KiB. Synchronous calls
after the gap win from 16 KiB (one buffer, pieces) and from 12 messages
(batches); below they lose to SHA-256 by 1.3-2x.

**Open, ours to explain or decide:**
1. **Small synchronous calls after the gap** (128 B-8 KiB, batches of
   1-8): probe/after-gap (jobs 478-479) found the core after the 1 ms
   sleep at about 1.3 GHz and a quarter to half of the time on E-cores,
   for SHA-256 alike; servil's cycles per call rise 1.7x there (4 KiB:
   5300 -> 8900) where SHA-256's stay level (6260 -> 6390): the NEON
   hybrids lose more on E-cores than SHA-256's dedicated instructions.
   20 us of integer work before the call halves it (the clock ramp). Back
   to back SHA-256 already leads below about 3 KiB (open problem 1).
   The rejected "minimax" plans, measured again after the gap
   (probe/plans-pairs, jobs 568-571), trade the two clock states at 4 KiB
   and lose at 8 KiB; a first call's cold cost is SHA-256's too
   (probe/first-call). What is left is kernel work for E-cores and low
   clocks, or accepting SHA-256's hardware lead below 8 KiB.
2. **The queue's cells slow the next cell** (VM and Mac, reproduced
   alone): SHA-256's continuous 1 KiB cell runs 2-5% slower beside the
   new fork than beside b467ba6 (Mac jobs 524-527: same clock, 4.42 GHz,
   2-5% more cycles per byte), with no thread left running (a queue
   burst leaves 30 us of CPU). Bisected on the VM: all of it arrives with
   the merge of `candidate/queue-simple` (221ef23: 0.329 -> 0.335 ns/B);
   tonight's queue commits are level. A probe of SHA-256's own loop right
   after a queue burst, an SME2 burst, a NEON burst, or
   `hash_multithreaded` (probe/aftereffect, job 528) shows its cycles per
   byte level (1.538-1.547), so the effect lives in the benchmark's
   state around the cells (its heap, its buffers), not in the core. It
   makes `perf_regress compare` across queue-simple give no verdict on
   the VM (the control moves). Open: what queue-simple leaves in the
   harness's state. With the final fork (idle workers asleep, the task
   list's room fixed) it is about 2% on the VM (0.330 -> 0.337 ns/B).
3. **The lingering bound, and its energy** (Zooko's Q): 50 us, reasoned
   as a wake's cost. Measured (fork NOTES, "Lingering"; jobs 560-567):
   a long message in 64 KiB pieces through `update_multithreaded` is
   1.5-2.6x as fast as before and spends 4-6x the energy (1.7-2.6 nJ/B
   against 0.4-0.48 for `update`): about eight cores poll between
   updates. The time-saving form's trade to decide: keep, shorten the
   bound, cut a lingering job for four threads (probe/linger-4: 18% less
   energy, half the CPU, long streams 7-12% slower solo, 1 MiB 20%
   faster), or give the energy-saving form (stage 2's time or energy
   argument) no lingering. A lingering stream leaves about 1.6 ms of
   worker CPU behind in all, and seems to slow the cells run after it
   5-10% (fork NOTES, "WORKER_IDLE's length"). `clocks::process_energy_nj` (new, macOS)
   reads the counter the probes used; not validated for energy cells.
   It credits energy late: read after the threads have slept 10 ms, a
   64 MiB hash reads 384-412 pJ/B (7% spread); read at once, a third less
   and spread 1.5-3.4x (fork NOTES, "The energy counter's repeatability").
4. The runner's `perf_regress` jobs need the runner restarted.
5. **Shared 16 KiB messages** through the queue: the pair moves little
   more than one program alone. Found: `submit`'s push onto the task list
   (its lock contended by pushers and pollers, 2 KiB tasks copied under
   it; fork NOTES, "A ceiling near one 16 KiB task"). The contended
   words apart made 16 KiB 14% faster solo and 9% shared; two task lists
   (candidate/two-lists) made 64 B 50% slower on the Mac and were left
   out. Next: a lock-free task list; `probe/members-32k` (shared -37%,
   solo +6%) stays a trade.
6. **The multithreaded split at 512 KiB instead of 768** (Zooko's choice
   of September 27, fork NOTES at `MIN_SPLIT_LEN`, is the length where it
   pays on both machines): branch `probe/split-512`. Mac, after the gap
   (jobs 538-541): 512 KiB one message 0.35 -> 0.19-0.24 ns/B, batches of
   8192 22.5 -> 13.3 ns/msg (about 40% faster); VM: 512 KiB 20-30% slower
   than on the caller's thread (0.24-0.34 -> 0.30), 8192 level. A
   decision for Zooko (native first; a VM loss needs his decision).

**Next, in order:** Zooko reviews the two branches (the benchmark fixes
and the fork's changes); run the gate (all suites, `perf_regress` on the
VM and the Mac) and land them; then stage 2's remaining items (the time
or energy argument on the multithreaded synchronous calls) and stage 3
(the energy counter).

## Earlier checkpoint (September 28, 2026, night)


**Where things stand.** The new benchmark runs: bench-hashes branch
`candidate/benchmark-plan` (a41c99a, checked out in the VM), which
depends on the fork's `candidate/api-plan` (a11b990: the plan in
`docs/api-design.md`, `clocks::measure_after_gaps`, public
`Hasher::update_multithreaded`, perf_regress building both sides with the
working tree's `clocks`). `FROZEN.md` is refrozen (Zooko, September 28).
All 10 benchmark tests pass; a `--quick` run with the default contenders
takes 56 s in the VM (was about 10 s). Stage 1's remaining work, in order:

1. **perf_regress is stale; fix it before any fork commit that touches
   code** (the pre-commit hook runs it with this benchmark). It still
   knows the scenario `after-idle` (MARGIN, HOLDING) and the use cases
   by old names. Wanted: solo cells of the synchronous use cases (after
   the gap) at 20% (Zooko's default), solo continuous cells at 3%, shared
   reported at 10% (20% after the gap?); its POINTS and use cases for the
   new axes; and its run time measured (after-gap cells cost a 1 ms gap
   per call: keep the check near its old 16 s, e.g. fewer after-gap
   points).
2. **Run time**: measure a full default run and `--all` (VM, then the
   Mac); the after-gap cells cost `GAP_NS` per call, and `GAP_SAMPLE_NS`
   (2 us of calls per sample) sets how many. The previous sessions cut a
   full run from 5 minutes to 30 s; tune samples and points to get back
   near that, reporting what each cut costs in precision.
3. **Reread and fix the pages** as the three readers: the text report's
   opening lines (the pattern sentence reads awkwardly), the graph (a
   render: chips, subtitles, the "how it was made" door), README,
   METHODOLOGY, CONTRIBUTING, and the fork's PROCEDURES and NOTES where
   they describe the old use cases or the after-idle scenario. The Duo
   comment still says each copy reads the clock as its first act (each
   now sleeps the gap first).
4. **First findings to explain** (quick run, VM): after the gap a 64 B
   call costs about 500 ns (7.8 ns/B) against about 40 ns back to back,
   for every hash alike; the queue loses to SHA-256 for 64 B-4 KiB
   messages (3.3 against 1.3 ns/B at 64 B) and badly for batches of 16
   (630 against 46 ns/msg), and wins from 64 KiB messages and 64-message
   batches.
5. Then the Mac: a runner job on the branch pair (Zooko launches it).
6. Then stage 2 (the fork: `update_multithreaded` for 64 KiB pieces,
   which today stay on the caller's thread; the time or energy argument
   on the multithreaded synchronous calls; lingering between updates)
   and stage 3 (an energy counter, then the energy endings).

Before merging: point Cargo.toml back at the fork's `servil` branch once
`candidate/api-plan` lands there.

The quick run's results (VM, 24 rounds, default contenders) are in
`/workspace/tmp/new-benchmark-quick/` (Zooko looked at them there; host
path `~/piplayground/blake3-servil/tmp/new-benchmark-quick/`). Copy
results into `/workspace/tmp/` for him to see: the VM's `/tmp` is its own.

Open questions of the plan, each marked **Q** in `docs/api-design.md`:
how long lingering between `update` calls may last (after the rest of
the API is settled); the gap's length and what the program does in it
(sleep today); what saving energy means for a multithreaded synchronous
call, and the energy counter; batch lengths beyond 64 B (Remco's 256 B
leaves); the multithreaded `Hasher` form's name. Revisit after building
and measuring: whether "cannot tell? answer intermittent" still holds;
optimising the energy-saving modes for a shared machine.

## Earlier checkpoint (September 28, 2026, late)

**The question of the moment** (Zooko): can the streaming API (`Queue`)
be implemented so that it is *way faster* than any other API? If not,
it is abandoned. Zooko's framing, settled: the streaming API maximises
**throughput** (bytes or messages per second, or per joule) and spends
latency to buy it; wanting the lowest latency per input from the one-shot
forms is valid. Its throughput should be the hashing's, as long as
handovers never slow the hashing threads, provided the program keeps
enough in flight (Little's law: in flight = rate x round trip).

**Next: unfreeze, clarify, refreeze** (Zooko, agreed): the benchmark and
the API both need to say what they are trying to achieve, then be frozen
again in `FROZEN.md`. The new plan (September 28) is in the fork's
`docs/api-design.md`: three choices (the data's shape, intermittent or
continuous load, time or energy) lead to six calls; no thread budget;
the benchmark measures the six intermittent cells separately and the
continuous column in two use cases. Its open questions are marked **Q**.
Implementing it (September 28, overnight session; Zooko approved
the plan and these defaults): stage 1, the benchmark on today's calls (synchronous
calls measured only after the gap of 1 ms, their shared version two
copies after the gap at once; queue use cases "messages" and "batches"
with enough in flight, back to back, solo and shared); stage 2, the fork
(`Hasher::update_multithreaded`, a first simple version; the time or
energy argument on multithreaded synchronous calls; then lingering);
stage 3, an energy counter and the energy endings. perf_regress judges
after-gap cells at 20% to start (tightening toward 3% is its own work).
Timing stays in the `clocks` crate; reuse the current benchmark's
scheduling, made fast (5 min to 30 s) over two days. **Shared scenarios
stay for every use case** (Zooko): a sanity check against designs that
need the machine to themselves, and a pessimistic estimate; not
optimised for directly. To revisit: optimise the energy-saving modes for
a shared machine (background tasks), the time-saving ones for solo.
Earlier points to settle with him (now partly answered there):
- The many-inputs use case keeps four buffers in flight and blocks on
  the fifth: at small sizes that measures the round trip (latency), not
  throughput. Proposal: keep enough in flight to cover it (e.g. about
  1 MiB or about 1024 buffers, whichever is fewer); likewise consider the
  streamed use case (four 64 KiB buffers cap it near 0.1 ns/B).
- A per-message handler is serial by contract (in order, one call at a
  time): the program's own per-message costs (a channel send and receive,
  about 100 ns) bound `Queue::messages` below a `hash()` loop at 64 B. Tiny
  messages belong to `Queue::fixed` (one call per batch) or `hash_many`;
  the benchmark could measure `Queue::fixed` for them.
- Whether "one delivery thread" stays (Zooko's decision): calling a
  handler on the submitting thread would let one short input avoid the
  handover, at the contract's cost.

**State of the code (fork, all on branches; nothing merged since v0.3.0):**
- `candidate/queue-simple` (d95eccc): the fresh design, the one to go on
  with. `submit` cuts a submission into tasks on the caller's thread
  (whole subtrees of at most 64 KiB, `plan_subtrees`; a message is a
  stream of one piece; fixed-length batches as ranges of slots; messages
  under 16 KiB several to a task, 16 per batch, one-block ones side by
  side); one task list; an SME2 thread hashes tasks only on SME2 (under
  the turn), the workers only on NEON (Zooko's suggestion); one delivery
  thread replays and calls handlers, holding the pool while anything is
  in flight. No allocation after warm-up (`tests/queue_no_alloc.rs`, a
  counting global allocator). All suites pass; TSan clean on api_plan
  (before the last two commits: rerun). Needs: Mac gate, NOTES, and the
  VM gap below.
- `candidate/queue-speed` (feed design, earlier today): superseded by
  queue-simple on the Mac everywhere; drop it once queue-simple lands.
- Probe branches from today: `probe/queue-timeline`,
  `probe/queue-process-state`, `probe/queue-sme2-turn`,
  `probe/task-len-32`, `probe/task-len-16`, `probe/queue-throughput`
  (examples/host_lab.rs: many short messages in flight, the throughput
  probe to reuse).
- bench-hashes `main` 183010c: records on fork 61502ef (v0.3.0's code),
  the many-inputs use case, the graph's two-significant-digit labels.
  Runner jobs run to 474; the next number is 475.

**Measured, Mac, servil mt through the queue (queue-simple):**
- Benchmark as frozen (jobs 464-467), ns/B, against the best other API
  (hash / hash_multithreaded; Hasher for streams): many inputs 64 KiB
  0.109 (0.168), 256 KiB 0.065 (0.170), 1 MiB 0.043 (0.068), 4 MiB 0.035
  (0.037); streamed 256 KiB 0.152 (0.236), 32 MiB 0.107 (0.238); losing
  at 64 B-1 KiB inputs and the one-piece 64 KiB stream (latency-bound
  with four buffers).
- Throughput probe (job 474, many in flight): 1 KiB messages 3.7x a
  hash() loop, 16 KiB 1.9x (the VM 2.8x: something serial left on the
  Mac, unfound), 64 B 0.34x (the per-message serial path).
- Tried and lost today (details in the fork's NOTES): the feed with help
  rules; tasks of 16 or 32 KiB (16 KiB: streams 50% slower); a worker
  taking the SME2 turn per task; waking after a 512 KiB burst.
- The VM is behind the feed design on long streams (about 14%) and
  16 KiB inputs; unresolved.

**Open, ours to explain:** some benchmark processes run every servil mt
queue cell 1.1-3.5x slower on the Mac (jobs 440, 446, 448, 454 runs 6-7,
463), servil st level; not reproduced by probes in fresh processes
(jobs 450-451); `--trace-clocks` job 455 met no slow run.

**Lessons (this session):**
- Latency against throughput: a closed-loop program with k in flight
  gets at most k per round trip; decide which one a benchmark measures.
- Measure the stages before guessing: the queue's 1.7 us per message was
  batches of one pushed under a lock, found in one probe after several
  wrong guesses.
- In the VM a contended std Mutex parks the waiter (futex); use try_lock
  on paths that poll.
- Allocation-free claims need a test (a counting global allocator in its
  own test binary); growth must follow the program's in-flight work, not
  thread timing.

### Next, in order

0. **The streaming API: unfreeze, clarify, refreeze** (above), then
   land `candidate/queue-simple` (Mac gate, TSan rerun, the VM gap).
1. **After-idle margin, recalibrated on mains power** (20%, job 338, was
   calibrated when the Mac's power state was unknown); and servil's small
   streams running at two speeds solo (1 KiB streamed: some rounds 4.5x
   slower than SHA-256 where the median is 1.85x; record 405's CHECKS).
2. **The streaming mode**: built (item 0 has what remains); an optional
   io_uring layer and chaining stay in `docs/api-design.md` for later.
3. **servil behind BLAKE3 official**: 4 messages fixed (p4 as two scalars
   beside a pair, Zooko's decision despite E-cores +30%; Mac 20.3 against
   official's 21.6 ns/msg). Left: 12 messages shared (Mac 25.8 against
   21.7; solo 13.4 against 21.4): the copy without the SME2 turn runs
   p9 + p3 (p8 + p4 was level). A p4 kinder to E-cores stays welcome.
4. **SHA-256 at 3-8 KiB** (open problem 1): the only cells within 10% of
   SHA-256 on either machine (servil 2-9% behind at 3 KiB, 3839 B, 4 KiB,
   4470 B; a VM record's warm-up can move them by up to 8%). Elsewhere
   SHA-256 leads twice over below 3 KiB and servil far ahead from 8 KiB.
5. **Which part of the cycles-to-wall-time ratio is ours** (Zooko,
   September 26): reported times stay wall time, never scaled by cycles,
   until we know. Ours: SME2 waits, the power our code draws lowering its
   own clock. The machine's: heat, a host, a scheduler. Measure the
   warm-up to confirm it is the machine's and to size its effect on what
   users read: a traced Mac job of about 10 minutes of load
   (`--trace-clocks`, as job 332), cycles per ns over time by contender.
   Then have every benchmark run record cycles beside wall time (today
   only `--trace-clocks` does; the samples file would carry them).
6. The E-core cells: 2-chunk messages at 4 (p4 two pairs), 1000 B x 4;
   tails of 1-4 multi-block messages past SME2 groups (slow state).
7. **One cell's aftereffects slow the next** (open, ours to explain): on
   the VM, a long run of shimmed batch cells once made the next SHA-256
   64 B cell 3-6% slower; the mechanism is unexplained.
8. The text report's three-reader pass (CHECKS, TWO SPEEDS).
9. A second SME2 thread in the pool (two SME units reachable, job 187).
10. Open, smaller: hash(256 KiB)'s partial slow state; the VM's
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

The fork's `PROCEDURES.md` (the regression check, the gate to `servil`, the Mac runner, probes, the VM) and this repository's `PROCEDURES.md` (records, runs, graphs, its environment).

## Decisions made (don't re-ask)

- **The API plan, September 28** (Zooko; `docs/api-design.md` on the
  fork's `candidate/api-plan`): four questions lead a user to one call,
  the crate docs opening with them: several threads or not; the shape (a
  message in one buffer, a message in pieces, a batch); time or energy
  (several threads only); intermittent or continuous (several threads
  only; "when you finish hashing a message, will there typically be
  another one ready?", and "cannot tell" answers intermittent). Nine
  calls: `hash`, `Hasher::update`, `hash_many` (single-threaded, built
  for intermittent use, always saving time); their `_multithreaded`
  forms; and the queue in three shapes matching the shape question. No
  thread budget. `hash` keeps its name. The queue: event-based through
  handler traits, no polling, no blocking, zero copies, no allocation
  after warm-up. A `Hasher` between updates may linger (bounded; open).
- **The benchmark measures each call only as its contract says users
  call it** (Zooko, September 28): synchronous calls each after the gap
  (1 ms asleep), never back to back; the queue with enough in flight, in
  two use cases (messages, batches). Every cell summing calls shorter
  than a few clock ticks sums single readings (`clocks::measure_after_gaps`;
  the clock ticks at 24 MHz, 41.67 ns, on the M4 Max and the VM).
  perf_regress judges after-gap cells at 20% to start.
- **Shared scenarios stay for every use case** (Zooko, September 28): a
  sanity check against designs that need the machine to themselves, and
  a pessimistic estimate; not optimised for directly.
- **Every clock read for a measurement goes through the fork's `clocks`
  crate** (Zooko, September 28; AGENTS.md "Measuring"): it holds the
  decisions of two earlier sessions on which clocks and how to read them.
- **The VM configures itself with `sh /workspace/vm/setup.sh`**, once per
  session (AGENTS.md "Where to start"): git and cargo then work with no
  prefix (a system gitconfig includes `vm/home/.gitconfig`; cargo's
  config sets `CC=clang-19` and the target directory; the Mac's `HOME`
  and `TMPDIR`, which the shells inherit, are created). Nothing in the
  guest runs it by itself: the disk resets and the shells read no
  startup file.

- **The API plan, being settled one use case at a time** (Zooko,
  September 27; write down, implement only once every use case is
  settled). One-shot, synchronous, one message in memory:
  - `hash()` stays single-threaded (platforms without threads, programs
    whose other cores are busy, the least energy per byte).
  - The docs prominently recommend `hash_multithreaded()` instead: never
    slower, faster for big messages (it leaves the caller's thread from
    768 KiB); the energy advice (single-threaded spends less per byte)
    stays beside it.
  - `initialize()` runs the startup self-test alone; its docs and
    `hash()`'s say calling it early keeps that cost (under 200 µs on an
    Apple M4 Max: 130-165 µs measured) off the first `hash()`.
  - `initialize_multithreaded()` runs the self-test and starts the pool;
    its docs and `hash_multithreaded()`'s say calling it early keeps that
    cost (under 1 ms on an Apple M4 Max: 510-700 µs measured) off the first
    multithreaded call that leaves the caller's thread.
  - A behaviour change of `initialize()`: a minor version bump and a
    changelog entry; measure the pool's memory cost when it is built.
  Benchmarking the one-shot APIs (Zooko agreed): exactly two automated
  scenarios, back to back (perf_regress at 3%: small kernel regressions)
  and after idle (20%: the wake path and cold starts, which back to back
  cannot see, as the 5-8x after-idle pool slowdown showed); no warm-start
  gap scenario (no evidence it would catch anything). The graph plots back
  to back only; the rest stays behind a door. To be written into policy,
  procedure, and code comments once the whole plan is settled.
  The whole plan designs every interface together, consistent with the
  others, and for each says which users and use cases it serves and how
  the benchmark measures it (Zooko, September 27). Its dimensions:
  - the shape of the work: one-shot one message; one-shot batch (the
    padded batch contract, settled); streaming one message (input
    arriving in pieces: `Hasher`, `Stream`); streaming batches (a stream
    of separate inputs: the `Queue` design);
  - threads: single-threaded, multithreaded, and a thread budget;
  - energy efficiency against time efficiency (what each form spends,
    and how a user chooses);
  - the modes: plain, keyed, and key derivation, for every shape;
  - the Rust type signatures, one consistent style across all of them;
  - initialization (settled above), and the "built for" labels (to be
    reworded: the one-shot forms are the simple ones; top speed belongs
    to the streaming forms).
  Settled so far: one-shot one message; the batch contract. Proposed for
  batches (Zooko to confirm): the docs recommend hash_many_multithreaded
  as hash_multithreaded is recommended; output stays 32-byte arrays;
  perf_regress adds the 4- and 12-message points.
- The pool keeps nothing awake between calls (Zooko, September 27):
  its workers poll only while a job is registered and sleep when none
  is; waiting inside a call stays. Every call therefore meets sleeping
  workers, so a call wakes workers only when its input pays for the wake
  and otherwise runs on the caller's thread. The benchmark's back-to-back
  mt cells slow (they measured workers kept awake for a next call, which
  few real programs make); the benchmark gains a sweep of caller gaps
  (real work between calls, timed with them), and the pool is judged by
  the worst gap. Callers with a stream of inputs are told to use batches
  (and later a pipelined API).
- Benchmark time (Zooko, September 26): the caller keeps the machine
  quiet, and the benchmark detects and reports noise and wastes no time
  compensating for it; a small loss of reliability for a large saving
  of time is welcome. Streamed 32-128 MiB stay in `--all` (informative).
  BLAKE3 official mt runs only when named; BLAKE3 official stays in
  `--all` until servil beats it in every cell. The VM's warm-up gets no
  treatment beyond the note. Never change a tracked file and change it
  back (the lock is the side's own). PyPy over CPython wherever it runs.

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
- Name Zooko as "Zooko" alone, everywhere (AGENTS).
- The startup self-test: every assembly entry, at 0.1-0.2 ms once per
  process (option 4), over a smaller budget that leaves kernels out.
- No long random differential runs ("superstitious fuzzing"); no
  emulators, not even for unit tests.
- Time is discrete (every clock ticks): integers, kept as measured,
  lossy steps deferred to one rounding for the reader (AGENTS).
- The benchmark: no 256 B batches, ab-blake3, or commonware; SHA3-256 in
  `--all`. The README shows one speed chart (1 MiB, BLAKE3 every core and
  one core against the fastest SHA-256, SHA3-256, SHA-1), in cores alone,
  labelled by hash, with a "how it was made" door.
- The graph's header sits at the top of the page (the chips replace the
  following header); the page never gets shorter than it loaded.
- Releases follow semver; before 1.0 a breaking change bumps the minor
  version (0.1.0 to 0.2.0: hash_many's one-buffer API, Stream).
- perf_regress holds solo regressions and reports shared ones; a commit
  that slows a shared cell names it and its reason (Zooko, September 26).

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

From `/workspace` in the VM, after `sh /workspace/vm/setup.sh` once per boot:

    cargo test --release --lib [--features no_sme2 | --features pure]
    cargo test --release --doc
    cargo test --release --test api_plan
    cargo test --release --manifest-path test_vectors/Cargo.toml
    cargo test --release --manifest-path clocks/Cargo.toml && pypy3 tools/speeds.py
    pypy3 tools/ab.py OLD NEW NEW OLD   # runner jobs, speed with speed
    cargo test --release --manifest-path bench-hashes/Cargo.toml
    pypy3 tools/perf_regress.py check | compare OLD NEW
    cargo run --release --example host_lab

Expected: 86 / 82 / 71 library tests, 22 doc tests, 15 in `--test api_plan`, 2 vectors, 10 benchmark
tests. Release: `python3 tools/gen-ver.py X.Y.Z` from a clean tree (two
version commits and a lightweight tag; push the branch, `servil` in the
fork or `main` here, then the tag by name).
Never print the credential token (`/workspace/ghtokenclassic.txt`). Only
`/workspace` survives VM restarts. Commands for the user go on one line.
