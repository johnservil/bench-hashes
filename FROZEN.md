# What the benchmark asks of BLAKE3-Servil (frozen)

The servil team's contract with itself (Zooko and John Servil): bench-hashes
measures BLAKE3-Servil through exactly the calls and usage patterns below, which
are the API plan (BLAKE3-Servil's `docs/api-design.md`) turned into
measurements. BLAKE3-Servil's work is to be as fast as possible under them.
Changing what the benchmark asks is a decision of Zooko's: edit this file
with the change, the date, and the reason, and the code with it; the test
`frozen_contract_matches_frozen_md` compares the block below with the
code's own tables and fails when they differ.

**The whole benchmark is frozen** (Zooko, October 1, 2026, at bench-hashes
0.10.0, tag v0.10.0+e57316f, measuring BLAKE3-Servil's `servil` at b132f8c), so
that a number, a verdict, and a picture from later mean what they mean
now, and people can compare results by eye: how samples are taken (the
rounds, the sample lengths, the gaps, the contender orders), the
contenders and their versions, the summary rule (each run's mean, pairs
of runs: `clocks::summary`), the load rule (`clocks::load`), the
regression check (`bench-hashes regress`: its points, its pairs, its
margin of 3%), and the
presentation (the map and the report, their layout and
their words). A change to any of these, a bug fix included, is John
Servil's decision (Zooko, November 2, 2026: a momentous, uncertain one
waits for Zooko), recorded here with its date and reason, and it starts a new
release, so results of different benchmarks are never read as alike. The
documents that explain the benchmark (README, METHODOLOGY, CONTRIBUTING,
the notes) may still change, to say it better.

**Changes for the next release**:
- **A contender for BLAKE3-Servil's pipeline** (John Servil, November 7,
  2026; Zooko: measure the new API before deciding its design and whether
  it is promoted). `blake3-servil-pipeline` is `throughput::pipeline`,
  the incremental background pipeline, taking part where bytes arrive
  over time: the receiving cells, beside `blake3-servil-st` (a Hasher,
  update per fragment) and `blake3-servil-mt` (update_multithreaded per
  fragment). One pipeline per thread and limits, kept for the process;
  64 rooms of 16 KiB (1 MiB in flight) and 2 message slots for a message
  received alone. Each fragment is written straight into the pipeline's
  room (the one copy of an arrival, as the Hasher contenders copy into
  their receive buffer), the last fragment's final bytes with
  `Message::end(n)`; the digest is awaited after the last. The pipeline is
  multithreaded, so the shared scenario measures it too. Its other cells
  (many messages at once, continuous messages) follow as the API settles.
- **A run's final load window joins the one before it** (John Servil,
  November 7, 2026). A tail shorter than a window joins the last closed
  window, however long it is; it closed on its own at half a window
  before. macOS posts the machine's ticks in lumps up to a second late
  (#142, Mac probe 1985: 10 CPU-seconds in one 50 ms poll after the
  threads stopped), and a half-second tail holding such a lump read 24
  CPUs busy on 16 CPUs (checks 1978 and 1979, which then gave no
  verdict). Read from the previous window's start, the lump meets the own
  time it belongs to. A run under a second has no window, where before it
  had one at half a second.
- **A probe compares only accepted pairs** (John Servil, November 7,
  2026). When a busy pair runs again, keep both abandoned samples under
  `retried/`, outside the top-level files the probe compares. The raw
  diagnostic record remains available. Mac probe 1969 exposed the bug:
  regress used three quiet pairs, but the probe's report reread four,
  including an abandoned busy pair. The load and comparison rules stay
  as they were.
- **An old-benchmark comparison runs with unchanged library trees**
  (John Servil, November 7, 2026). `check --old-bench REV` compares the
  instruments even when both sides use the same library. The early
  library-tree shortcut applies only when both sides use the current
  benchmark. The later same-code shortcut already had this restriction.
  Mac control 1966 had returned "no regression" without measuring the
  different instruments; that outcome is treated as unmeasured. The
  comparison rule, cells and measurements remain unchanged.
- **Receive and hash one message in fragments** (John Servil, November 6,
  2026, following Zooko's API direction in
  [BLAKE3 #267](https://github.com/servil-software-factory/BLAKE3/issues/267),
  convenience and throughput contracts). `ReceivingMessage` measures a
  16 MiB message at three arrival spacings: saturated, 2 us and 20 us.
  Fragments cycle through 1448 B, 4 KiB, 16 KiB and 64 KiB, cut at the
  message's end. Each scheduled arrival is a fixed offset from the first.
  Consumer work and sleep overshoot leave later deadlines unchanged.
  `clocks::sleep_until` waits only for the remaining time; a late consumer
  finds the next fragment ready, as it would with buffered I/O. Saturated
  input needs no clock reads or waits between fragments.
  Receiving runs in its own phase. The trace description comes from its
  constants and is stored in the samples' `receive trace` header, shown
  in the map's run details. Each fragment is read into a kept 64 KiB
  buffer before its incremental update. Reads, waits, updates and
  finalization are timed together, solo and shared under the existing
  sampling and comparison rules. Current incremental APIs establish the
  baseline for the background pipeline. The map names its column
  "receive + hash, including waits" and its axis is arrival spacing.
  Existing axes retain their calls and schedules.
- **The Hasher is a convenience call; its parallel update a throughput
  function** (John Servil, November 7, 2026, from BLAKE3 #267, Zooko's
  direction of November 6): `convenience::Hasher::new(mode, length_hint)`
  replaces `throughput::Hasher::new(mode)` (the hint is advisory and
  `None` here), `reset(None)` replaces `reset()`, and
  `throughput::update_multithreaded(&mut hasher, piece)` replaces
  `hasher.update_multithreaded(piece)`. The same calls under their new
  names, so every cell's work is as it was; a check of this change runs
  its old side with the bench before it (`--old-bench`).

**Changes for 0.21** (Zooko and John Servil, November 4-6, 2026):
- **The wall clock is the raw counter** (Zooko, November 4, 2026): on
  Linux `CLOCK_MONOTONIC_RAW` in place of `CLOCK_MONOTONIC` (what
  `std::time::Instant` reads there), which a time daemon slews by up to
  500 ppm, a rate that could be in error or an adversary's, and helps no
  measurement here (an i7-12700K had drifted 596 ms from its raw counter
  8 hours after boot). On Darwin `CLOCK_UPTIME_RAW`, raw already. Same
  cost a read (11 ns on the i7), same steps (1 ns; 41.67 ns on an M4
  Max). A run's `sample clock` line names it. Zooko's measure-clocks3
  (December 2025) compares the clocks on both platforms.
- **Two pauses before a call "after idling"** (Zooko and John, November
  4, 2026; bench-hashes #117, Sam Shackleton's protocol): 20 us (the use
  cases `BriefIdleOneMessage`, `BriefIdleManyMessages`,
  `BriefIdleInterleavedTurn`) and 5 ms (`IdleOneMessage`,
  `IdleManyMessages`, `IdleInterleavedTurn`, until 0.20 at 1 ms). After
  20 us a core keeps its clock and its shallowest idle state; after 5 ms
  every machine measured has settled into its deep states, where 1 ms
  was the i7-12700K's transition between the two, bimodal (#117: the
  i7's 16 x 64 B batch x2.0 at 20 us, x2.6 at 1 ms with 24-49%
  detectable, x6.3 at 5 ms with 16%). Each pause is a real program's
  situation, and each is stable. The pauses land when asked: the bench
  sets its threads' timer slack to 1 us (`clocks::tight_timers`; Linux
  gives a thread 50 us by default, which landed the 20 us sleep at 72 us
  and let the i7's core reach C6 in 39% of it; at 23 us it stays in C1). The pauses run in sets of rounds of
  their own, as the other ways of calling do. The run grows by the
  after-pause cells' share. Each samples file records, beside the
  configured idle states, the **effective** ones: the shares of the
  core's hardware C-state residency under each pause, slept at the run's
  start as the cells sleep it (1000 x 20 us, 20 x 5 ms; Linux, Intel's
  `cstate_core` counters; elsewhere "not measured"), since a configured
  state may seldom happen within a pause (the Xeon Gold 6330's C6 within
  1 ms, #117) and the deepest may be one no software counter names (the
  i7's core C7).
- **The controlled block** (John, November 4, 2026; #117, Sam
  Shackleton's protocol): where the bench can write every cpuidle
  state's `disable` file (Linux, run as root), the 5 ms cells run once
  more as `ControlledIdleOneMessage`, `ControlledIdleManyMessages` and
  `ControlledIdleInterleavedTurn`, in rounds of their own with every
  state but POLL and C1 switched off (POLL alone where the machine has no
  plain C1), so the call meets the core awake and the cell measures the
  library apart from the machine's rest; the states are put back when
  those rounds end. The shipped block, what users meet, stays the
  figure users read; the controlled one stands beside it, a fifth
  after-gap column on the map. Each samples file's `idle control` line
  says what the block kept, or why there is none (the i7-12700K in its
  sandbox: no root).
- **A level cell whose instructions moved joins the second stage** (Zooko,
  November 4, 2026, on John's rule; #127): instructions a unit moved
  beyond 0.5% the same way in every pair names a solo cell of a
  single-threaded contender a suspect ahead of the wall-ranked ones,
  the largest move first, with no verdict of its own; wall decides on
  the fresh pairs. A pool's caller spins while it waits, so a
  multithreaded cell's count is no evidence (it moved 1-9% between two
  runs of the same code; data branch, 2026-11-04/judge2-i7). Where the machine counts none, the suspects are wall's
  alone, and the run says so.
- **Every sample keeps its instructions retired, and its last-level-cache
  references and misses, beside its cycles**, per core kind (Zooko,
  November 4, 2026: the most the counters give, kept whether or not a use
  is known yet; the cache pair the same day, at a read cost of tens of
  nanoseconds now that the counters are read in user space). The samples
  format is v10. On the i7-12700K instructions are the same count on a P
  core and an E core and after any pause (28719 for a fixed call, every
  time), where cycles and wall time move with the core's clock (data
  branch, 2026-11-04/clock-survey). The cache events are perf's generic
  pair, whose hardware meaning METHODOLOGY names; the counters are read
  by `rdpmc` where the kernel allows it, since a system call between the
  pause and the call moved the i7's 64 B figure by 13% (data branch,
  2026-11-04/precall-counts-i7).
- **servil st's calls made one after another are its performance call**
  (Zooko, November 5, 2026, "Cold calls and warm calls, by their API":
  convenience is for a hash now and then, amid other work; performance for
  call after call, the call alone deciding; built at John Servil's request
  on BLAKE3-Servil #253, November 6): the lent messages, the owned messages
  up to 64 KiB (their lent twins) and the pipelined collection's items go
  through `performance::hash`, each message in the whole blocks the bench
  lays out for every contender (no copy). The cells after a gap (after
  other work, after idling) keep `convenience::hash`. On AMD Zen 5 the
  performance call takes the integer-register chunk root for a chunk or
  less (BLAKE3-Servil #252: 19-35% less time back to back).

- **A comparison's pairs each run on one core** (John Servil, November 5,
  2026; bench-hashes #69). `regress`, and so `check`, pins each pair's two
  runs' measuring thread to one CPU around each sample after a gap, the
  pool's workers untouched; pair i takes a core of the parity of i, the
  pairs of a parity turning through the cache domains, one CPU a core,
  performance cores where there are two kinds. The reason: after the
  busy gap, odd cores fill the call's L2 misses from DRAM where even ones
  fill them from the L3 (EPYC 9255, `b069-oddeven`), so the core a run
  happened on was a difference between the sides of a pair: unpinned, a
  small cell after other work swung 1.3-1.8x with its core. Pinned, a
  pair's sides share the core, and the alternation keeps a change that
  helps one parity and hurts the other visible in half the pairs. Each
  run's samples header says where (`measuring thread`). Records stay
  unpinned: a record reports what a caller meets wherever the OS puts
  it. How comparisons are judged is unchanged. Where the machine takes
  no thread affinity (macOS), every run is the OS's, as before.
- **`check` and `regress` ask the machine first** (Devon Jonte, November
  5, 2026, bench-hashes #128): ten quiet seconds before a build or a
  measurement, or no verdict at once. A busy machine had cost whole
  checks that abstained at their end. `regress` tries for up to two
  minutes, since a check's own builds leave load behind for a few
  seconds (John Servil, November 6). `check --when-quiet` waits as long
  as it takes. What a run measures is unchanged.
- **BLAKE3-Servil's throughput calls** (Zooko, November 6, 2026): the
  library's `performance` module is `throughput`, a name for what its calls
  are for, many hashes one after another; a caller hashing now and then
  had read `performance::hash` as the faster call, where
  `convenience::hash` is. The same calls, measured the same way; the
  contract below, the report and the map name them so.
- **`check` measures the cells its author names** (Zooko, November 6,
  2026; Devon Jonte, bench-hashes #128). `check` first prints the diff's
  files, its added and deleted lines, and the items its hunks sit in.
  Then it measures the points its author names: `--points` for the gate,
  `--probe` for a three-pair survey before the change is built, `--cells
  all` for every point. With none named, it stops before a build, and
  each check records how many points it left unmeasured. Until 0.21,
  coverage chose the cells, and a declaration that no coverage region
  holds made a check measure every cell: three checks of November 5-6
  spent 18 minutes on 426 cells, each for one such line.
- **An owned cell measured as its lent twin takes the twin's sample size**
  in a sizes file (Sam Shackleton, bench-hashes #134, a bug fix):
  `--calibrate-only` wrote 1 for those cells, and `regress` gave that
  size to both sides, so a new side that measured such a cell itself
  timed one call a sample (#133: +72% read where the cell was -93%).

**Changes for 0.20** (Zooko, November 2, 2026, bench-hashes #108):
- **The shared scenario measures the multithreaded contenders alone**,
  and records where each copy ran. From September 23 every nonstop cell
  was measured twice, alone and as two copies at once. In six weeks the
  single-threaded contenders' shared cells read x1.00 at the median on
  every machine, apart from where the scheduler placed the copies (an
  SMT pair: x1.5, Sam Shackleton, EPYC 9255); they were 300 of the 381
  shared cells and told nothing a solo cell did not. The multithreaded
  contenders' shared cells told what nothing else could: servil mt ran
  x0.59 beside a neighbour on the EPYC, its pool kept to one L3 domain
  where alone it sprawled (BLAKE3 #196, found so). Two programs on one
  SME cluster or one GPU are the same question, so the scenario stays
  for them (Zooko: it revealed this and may reveal the next). Each
  sample's row now records the CPU its thread ended on (`cpu`, read
  after the timed call), so a reader can tell what two copies shared.
  The samples format is v8.
- **The shared copies are placed** (Zooko, November 2, 2026; bench-hashes
  #116): until 0.20 the scheduler chose where the two copies ran, each
  sample, and the figure followed it (an SMT pair x1.5, two L3 domains
  x0.6 of one; 59 of the Xeon pair's 81 shared cells mixed placements).
  Now each copy's thread is pinned to a chosen CPU for its job: two
  physical cores, never one core's two hardware threads, performance
  cores where the machine has two kinds, under **one last-level cache**
  (what a busy machine most often gives two programs, and the case that
  found #196), and, where the machine has two caches, also under **two**
  (two L3 domains; on Apple silicon two clusters, so two SME units),
  each a layer of its own. Both are measured because the pool's
  placement work (#196) changes what each shows; a one-cache machine
  measures the first alone. Where the two caches are two memory nodes
  (two sockets), each copy moves its input's pages to its CPU's node
  before it runs, as a program's own memory would be (Sam, November 2).
  The header's `shared placements` line names the CPUs, and the nodes
  where it applies. Where the machine offers no pair (macOS, whose
  threads take no affinity; a one-core machine), the copies run wherever
  the operating system puts them, as a layer of their own,
  `shared-os` (0.20.2: 0.20.0 and 0.20.1 measured no shared cell there,
  though this entry said the scheduler places them; John Servil,
  November 3).
- **Every shared sample is contended** (#105): the copy that finishes
  its train first keeps hashing, unsampled, until the other has finished.
  Before, the slower copy's last samples ran alone, about 1% optimistic
  in 0.19 (7% in 0.18, whose copies were uneven). The pool's fairness
  between two callers is BLAKE3-Servil's `tests/pool_fairness.rs` (BLAKE3
  #215) besides.

**Changes for 0.19** (Zooko, October 31, 2026: "the new protocol is worth
it for accuracy under sustained load, which the GPU work needs, even if it
worsens the VM results a bit"; and, the same day, "don't wait for me on
anything", John Servil releasing it):

- **Continuous means continuous.** A nonstop cell ("one input after
  another, as fast as the program can") took each 1 ms sample in a visit
  of its own, between other contenders' samples. Whatever settles more
  slowly than a millisecond met every sample cold: a core's clock, thread
  placement, a GPU's clock (30-40 ms of load on an M4 Max, BLAKE3-Servil's
  tmp/metal-night probes). Now a nonstop visit is a train: the cell's own
  load, untimed, for 100 ms, then 6 samples back to back, solo, then the
  same for the two shared copies; 2 visits a cell (`NONSTOP_TRAIN`,
  `Visits::nonstop`). The calls after a gap are unchanged.
  - Why 100 ms: settling curves (samples back to back from cold, the
    study's `tools/settle.py`, in the tree until 0.21) put every solo cell within 3% by 100 ms on the M4
    Max, the Linux VM and the EPYC 9255, and the GPU path's engagement and
    ramp at about 50 ms; 50 ms reads about as well on the CPUs (the Mac's
    worst solo cell 1.106 against 1.080) and leaves the GPU no margin.
  - Why 2 visits of 6: the spread of a run's mean splits into the samples
    within a visit, the visits within a run, and the runs
    (the study's `tools/variance.py`, in the tree until 0.21): on the Mac and the EPYC the runs' part, which
    no visit count shrinks, and the samples' part lead, so 4 visits of 3
    would cost twice the nonstop time for a p90 of 2.8% against 3.4%
    (Mac), 2.2% against 2.6% (EPYC). The widest cells are servil mt's own
    two speeds (#196 in BLAKE3-Servil), which no setting fixes.
  - What changed in the results, old (0.18.3) against new, three pairs of
    full runs: the Mac (M4 Max) nonstop cells repeat better, median
    spread 1.5% to 1.1% (solo) and 1.2% to 0.8% (shared), p90 4.0% to
    4.6% and 3.5% to 3.8%; the EPYC 9255 (Sam Shackleton, bench #96)
    median 0.7% to 0.4%, p90 5.2% to 7.6%; the VM read 2-4% slower and
    slightly less repeatable. Official mt streams read 21-27% faster on
    the Mac, servil mt's 10-12%. The longer nonstop phase leaves the Mac's
    cores about 1% slower in the phases after it (P cores 4.02-4.06 GHz
    against 4.07-4.11; SHA-256's SHA instructions, 4%), so a cell after a
    gap reads that much slower than under 0.18. A control (0.19's code
    measuring 0.18's way) read level with 0.18 on every machine (all 1590
    cells on the EPYC).
  - Cost: a full run on the Mac takes 352 s (0.18: 230), the owned cells
    measured once and calibration cached (below) paying part of it.
- **Owned and lent cells that run the same calls are measured once**
  (`owned_runs_lent`): every contender but servil mt's stream, messages up
  to a piece, measures the lent cell, and its samples stand for the owned
  row too, as the samples file's header says. Two measurements of
  identical calls in one run differed by up to 40% (0.18's runs on the
  M4 Max and the EPYC 9255; median 0.3-1.3%, the worst in batch cells).
- **Calibration is cached** (`sizes_context`): a run keeps its sample
  sizes in the user's cache directory, a file for each context, and a
  later run in that context reuses them (about 50 s on the Mac). The
  context is the bench's version, commit and clean status, the machine,
  the power source and mode, and the contenders, points and threads. The code under test is no
  part of the context: a nonstop cell whose median sample falls under a
  quarter of its target deletes the file. The samples file's `sample
  sizes` line names the sizes' source.
- **Load is other programs' CPU time, measured on macOS again** (the
  fork's clocks, where its probes are; as of 0.19.3 below).
- **Comparisons judge each measurement once** (`judge_once`; Devon Jonte,
  bench #98): an owned cell measured as its lent twin is one measurement,
  counted once in the false discovery rate's family and among the
  suspects, its verdict printed on both rows.
- **0.19.1: `check`'s commit of the working tree has an identity of its
  own** (as it has a date of its own): git otherwise guessed one from the
  host's name, and on the Mac's runner on another network refused the
  guess ("benchrunner@Mac.(none)", job 1830). `check` alone changes; a
  run measures as 0.19.0's.
- **0.19.2: `check --old-bench` makes the old side's worktree afresh**
  each check: one left by a check from before the separate checkouts
  pointed at a repository gone (job 1831). `check` alone changes.
- **0.19.3: on Linux, other programs' load is their own CPU time,
  summed** (BLAKE3-Servil's clocks at ac9e6b7d): every other process's user
  and system time from `/proc`, where it lists the whole machine; the
  machine's busy time less ours, in one subtraction, in a PID namespace
  and on macOS. On 112 CPUs the machine's user time less ours read 1-6
  CPUs where no other process took 0.5 (bench #98, Sam Shackleton). The
  load reading alone changes; a run measures as 0.19.0's.
- **0.19.4: `check`'s first stage is 4 pairs, a link a side each**
  (Devon Jonte's lean variant of his check; Zooko, November 1: "I love
  saving 12 minutes without losing sensitivity"): its 4 pairs choose the
  suspects as well as 8 did on #119, ranked by evidence, and the 64-pair
  second stage stays. About 12 minutes less a check.
- **0.19.5: the record shown whole** (Devon Jonte, bench #103, #104,
  #107). A dirty tree's fingerprint is its BLAKE3 hash written as a sigil.
  The map's "About this run" lists every header line, each chart draws
  its record into the image (machine, date, the bench's and BLAKE3-Servil's
  commits, the compiler), and the overview links each machine's record.
  A phone keeps the grid's row names in view; a map's layer shows its own
  rows and columns; a cell's call name fits. What a run measures is as
  0.19.0's.
- **0.19.6: each samples file records the machine's idle states** (Devon
  Jonte, bench #106, #109): the cpuidle driver, governor and CPU 0's states
  with their exit latencies and target residencies, `none` without a
  driver, `not reported by this OS` elsewhere. The cells after a pause
  measure the state the core slept in (the i7: 13.5, 25.8 and 58.2 ns a
  message after C1E, C6 and C8). What a run measures is as 0.19.0's.
- **0.19.7: two of the map's fixes.** Every layer keeps every column at
  its place, a column it doesn't measure a blank track (Sam's review of
  #107); a BLAKE3-Servil source with no repository (a local clone's) is a
  row without a link, where the map panicked after measuring (Sam, #111).
  What a run measures is as 0.19.0's.
- **Studies read samples in Python** (`tools/samples.py`): `means` and
  `settle` left the program for `tools/points.py` and `tools/settle.py`,
  beside `tools/variance.py`; comparisons and verdicts stay in it. (The
  studies done, the Python reader and the three tools left the tree for
  0.21, Zooko, November 5: one reader of the format, the program's.)

**Changes for 0.18** (Zooko, October 24-28, 2026; a patch release's
entries name it):
- **The batch calls take a `Batch`** (Zooko, October 28; BLAKE3 #205):
  `hash_many`, `hash_many_multithreaded` and `Stream::submit_batch` take
  `&Batch`, whose bytes past each message the program cannot write, so a
  nonzero one, which the AArch64 kernels would hash into a wrong digest,
  cannot be written. The bench's batches fill their blocks (64 B and
  4 KiB messages, 256 B leaves, 64 B nodes) and are borrowed in place
  with `Batch::from_blocks`: the same blocks, read the same way. (0.18.3)
- **A stream's fill returns the bytes it wrote** (Zooko, October 28;
  BLAKE3 #204). `submit_with` and `submit_batch_with` now call `fill`
  as `Read::read_exact` calls `read`, until each piece is full, so a
  fill that writes less is never hashed with stale bytes. The bench's
  fills write all of each room and return its length: the same calls,
  writing the same bytes. (0.18.2)
- **BLAKE3 official, multi-threaded, in every `--all` run** (Zooko,
  October 27). Since September 26 it ran only when named, on the ground
  that servil's multi-threaded form beat it at every point. The one run
  since that named it (EPYC 9255, October 23) read 128 MiB level, and
  without it every "times official" on an every-core cell compared
  servil on every core with official on one. Each library is measured
  in both its forms, so every comparison is like for like. (0.18.1)
- **The overview: a column a kind of machine** (Zooko, October 27). Nine
  machines have runs, and a column each did not fit. The grid shows
  three columns, Arm, AMD and Intel, each the machine most like its
  family's machines shipping through 2027 (`overview::CHOSEN`: Apple M4
  Max, AMD Ryzen 7 9700X, Intel Core i7-12700K). Every other machine
  waits behind a door, a card each in plain words, and a card puts its
  machine in its family's column. Every cell holds each form of each
  library its run measured, and the page opens on BLAKE3-Servil,
  multi-threaded, beside SHA-256; the legend shows the rest. So the rows
  "1 MiB, one thread" and "128 MiB, every core" become "1 MiB" and
  "128 MiB": the legend, not the row, says which forms are drawn. (0.18.1)
- **Official b3sum 1.8.7** (October 27, for 0.18.0): the b3sum run's
  official contender is b3sum 1.8.7 (was 1.8.2), the release BLAKE3-Servil
  comes from and the one the hash cells' official crate already is
  (Cargo.lock). Zooko, October 25: official BLAKE3 1.8.7 is the one
  reference point; a b3sum chart against 1.8.2 compared BLAKE3-Servil with an
  older official than every other chart.
- **A batch of 4 KiB messages** (Zooko, October 24, for 0.18.0): a use
  case of its own, ManyLongMessages, batches of 4, 16, 64 and 256
  messages of 4 KiB each, after other work: servil st's
  `performance::hash_many` and servil mt's `hash_many_multithreaded`
  with 4096 for the length; BLAKE3 official and the rest message by
  message (official's hidden batch function hashes each message as one
  chunk, so a program with longer messages calls `blake3::hash` on each).
  It is the one cell on the path `hash_many` takes for messages of 2 to
  15 chunks, across the messages at every step of their tree
  (`hash_across`), which no cell reached before (the data branch,
  `2026-10-24/shortcuts`). On the map it is a row of its own, a batch of
  longer messages, in the after-other-work column. A batch's message
  length is its point's (bytes over messages), one rule for every batch
  cell.
- **Streams and lent calls of 128 B and 512 B messages** (Zooko, October
  26): the ContinuousMessages and LentMessages axes gain 128 B and 512 B
  (64 B, 128 B, 256 B, 512 B, 1 KiB, then factors of four to 64 MiB),
  the two axes keeping one set of sizes. Why: BLAKE3-Servil's stream
  gathers messages of one length into a task and hashes them side by
  side with a kernel for each length from 64 B to 1 KiB
  (`Task::run_members`); no cell streamed 128 B or 512 B messages, so
  those two kernels' speed went unmeasured (BLAKE3 #183).
- **BLAKE3-Servil's names made consistent** (Zooko, October 26): the plain
  hash's mode is `Mode::Plain` (was `Mode::Hash`), and a `Hasher` is made
  by `Hasher::new(mode)` alone. The calls and their work are unchanged:
  the same calls under their 0.8 names.
- **Official's b3sum on one thread too** (Zooko, October 25, answering
  official BLAKE3's maintainer): `official-1-thread`, official's `b3sum`
  with `--num-threads 1`, beside BLAKE3-Servil's `b3sum-1-thread`, so every
  b3sum cell compares the two on one thread as well as on every core.
  `tools/b3sum-contenders.py` adds it.
- **BLAKE3-Servil's two kinds of call** (Zooko, October 24): BLAKE3-Servil
  splits its API into `convenience` (one call per hash, any slice) and
  `performance` (every way to hash fast, inputs as `Block`s; BLAKE3 #146).
  The benchmark's calls are the same calls under their new names, listed
  in the frozen block: servil st's one-message cells call
  `convenience::hash`, every other servil call is in `performance`, and a
  message given to a performance call comes in its whole blocks
  ("Inputs in blocks" below).
- **The outboard and verify use cases go** (Zooko, October 24, deleting
  BLAKE3-Servil's outboard and `Verifier` API): only BLAKE3 official (through
  bao-tree) and servil took part, so without servil's cells they compare
  nothing of ours. bao-tree leaves the dependencies, and its line leaves
  the map.
- **Inputs in blocks** (Zooko, October 24, deciding on BLAKE3 #141 and
  #146): BLAKE3-Servil's performance calls take their inputs as `Block`s, 64
  bytes on a 64-byte boundary, so the bench aligns the inputs of every cell
  that has a performance-call contender, for every contender alike
  (`UseCase::in_blocks`): every batch, and the one-message, lent,
  continuous and collection cells, each message starting a 64-byte
  boundary with whole blocks behind it, zeros after its end (`BLOCK_SPARE`,
  `block_message`, `block_copy`). A collection's items each start their own
  blocks; a cell's bytes, what its speed divides by, stay its messages'.
  The pieces and interleaved cells feed servil's `Hasher`, which takes any
  slice, and keep the allocator's placement (glibc places a large buffer
  16 bytes into a line), as a real program's does. The check
  `assert_blocks_aligned` stops a run whose input in blocks is placed
  otherwise, and `every_batch_reaches_the_contenders_on_a_64_byte_boundary`
  holds each kind of copying cell to it, with a control. 0.17's cells
  measured inputs at the allocator's placement: compare them with 0.18's
  only with that difference stated.
- **Produced leaves use the shared sampler** (Zooko, October 22, approval
  to proceed with sampler consolidation): whole jobs, 5 ms sizing and
  three samples per cell per round stay unchanged. `clocks::measure`
  owns calibration, wall time, process CPU time and caller thread counts.
  Shared rows keep process CPU time absent; solo rows retain it. The
  existing v6 core-count field now records available caller counts, with
  energy absent. The wall report uses the same shared mean selection as
  compare and the map; it had its own plain sum. Hash calls, generated
  input, checksum work and stored digests stay unchanged.

**Changes for 0.17** (Zooko, October 16-23, 2026):
- **The overview** (Zooko, October 22): one page above the maps, built
  from every machine's published run; its representative cells and rules
  are the section "The overview" below, and the frozen block's
  `overview cell` lines.
- **One mean, every sample counted, for 0.17.0** (Zooko, October 23,
  deciding on bench-hashes #60): a cell's figure is its mean, total time
  over total work, with no sample set aside. It replaces the
  slowest-`ceil(n / 100)` rule of October 19 (bench #41), which the code
  had declared and `Mean::of` had bypassed (#59: it summed every sample
  before the shared selection saw them). Why the rule goes rather than
  the bypass: at the budgets the benchmark runs on, 6 samples nonstop and
  24 after a gap, setting one sample aside erased a slow state that came
  once in ten samples (the data branch, `2026-10-22/mean-trim-controls`:
  ten deterministic phases, each read as level, 10% of the raw cost
  gone), where the plain mean shows it, and a stall that the trim would
  have softened is what the pairs of runs and the load readings catch.
  The rule lives in `clocks::summary::mean` (BLAKE3-Servil's line, 7f9f1f7),
  with the fixture as its test; every reader here passes raw samples to
  it through one `mean_totals`, so the report, the comparison, the
  regression check and the maps draw one figure. Reports of 0.16 and
  before were plain means too, read with their own tools; the
  October 22 corrected-rule reanalysis (`2026-10-22/x86-pair-trim-reanalysis`)
  is the trimmed reading of the pair kernels' data, kept as what it is.
  **Samples v7** (Zooko, October 23): the samples file's first line says
  `v7` from this release on, its columns v6's, so that the release and
  rule a file was collected under show in the file itself; a v6 file is
  read with the tools of the commit that wrote it, as before.
  Reliance stays as METHODOLOGY states it: fresh controls at these
  budgets, with independent layouts and intermittent slow modes, govern
  it, and the general-reliance NO-GO stays in force until they pass.
- **One message of 1536 B** (Zooko, October 23, for 0.17.0; Sam
  Shackleton's finding on bench #63): a point between 1 KiB and 2 KiB in
  OneMessage and IdleOneMessage, one whole chunk and half of another, the
  shape on which a generated x86 kernel ran 34.6% slower (1,152 to 1,984
  bytes) while no single-message cell could show it: the points went from
  1 KiB to 2 KiB. Every run now carries it.
- **BLAKE3 commonware retires** (Zooko, October 20): its kernels were
  commonware's pull request #4982, which closed unmerged; commonware's
  shipped BLAKE3 is the official `blake3` crate (a thread-pool split of
  messages from 128 KiB beside it), which the benchmark measures already as
  BLAKE3 official and BLAKE3 official mt. A comparison with code its
  authors chose not to ship serves no reader. The contender crate, its
  cells and its colour go; the batch charts open with three lines.
- **The guide goes** (Zooko, October 20): `bench-hashes.guide.html`, its
  compiled examples and its two Playwright checks. The servil crate's
  documentation ("Which call to use") answers the same question where
  programmers already are, and the map's rows show each call's measured
  speed; two mechanisms for one decision were one too many. The README
  points to the table.
- **Many messages at once, a turn after a gap** (Zooko, October 19): two
  new use cases, InterleavedTurn and IdleInterleavedTurn, the map's two
  empty cells of the many-messages row. The 256 open messages of
  Interleaved, one turn of their pieces a call (up to 64 pieces, 256 KiB),
  each after other work or after idling, as a server handles a turn of
  uploads between its other tasks or after waiting on the network; solo,
  as every call after a gap.
- **A Merkle tree** (Zooko, October 19, for v0.8): a new use case,
  MerkleTree, the commitment a zero-knowledge prover makes to its
  Reed-Solomon codeword (WHIR, BLAKE3-Servil's docs/whir-merkle-trees.md). The
  program writes its leaves (256 bytes each) into its buffer, then hashes
  them and each layer of parents (two children's digests) to the root,
  every layer kept, at 1024 to 262144 leaves, solo and shared. Leaves and
  parents are hashed apart (Zooko, October 19: a tree that hashes them
  alike lets two child digests pass for a leaf, the standard Merkle tree
  vulnerability): the BLAKE3 contenders as keyed hashes under a leaf key
  and a parent key derived from their contexts, every other hash with a
  prefix byte, 0x00 to a leaf and 0x01 to a parent (RFC 6962). Each
  contender hashes a layer with its batch call (BLAKE3 official: the hidden
  `Platform::hash_many` with the key, as WHIR's engine calls it; servil:
  `hash_many_with`, mt `hash_many_multithreaded_with`), and one call a node
  where it has none. BLAKE3
  official mt takes no part. The map
  draws it in its own row, the nonstop column.
- **A collection, pipelined** (Devon Jonte's fork, October 18; Zooko's
  question): a new use case, ContinuousCollection, at the two collections
  (git objects, Nix files), solo and shared: the items one after another,
  each read into the program's buffer, then hashed, as a program reads a
  store's files and hashes them. BLAKE3-Servil mt streams them
  (`submit_with`), every other hash hashes each on the program's thread.
  The map draws it in the collection's row, pipelined column.
- **BLAKE3-Servil st takes part in the pipelined (continuous) cells**
  (Devon Jonte's fork, October 18; Zooko's question): as every hash
  without a stream does, on the program's thread, one input after another
  read into the program's buffer. A program that keeps to one thread
  hashes its inputs so; leaving servil st out broke the column's own rule
  and hid the one-thread line beside official's.
- **Samples v6: energy, and where each sample ran** (Devon Jonte's fork,
  October 18; bench-hashes #38, energy back by Zooko's decision): an
  `energy nJ` column beside `cpu ns`, each solo nonstop sample's CPU-package
  energy and each b3sum run's; a `core counts` column, each sample's calling-thread time and
  cycles on P and E cores; one `contender` key per b3sum build (`NAME: build ...`; it was
  `contender NAME`, a name inside the key). The map draws energy dashed
  (GB per joule, or nJ per byte). v5 files are read with the tools of the commit that wrote them.
- **Energy beside wall time and CPU time** (Zooko, October 18, 2026,
  reversing October 17's lab-only rule; bench-hashes #38): the samples
  record energy where the platform gives a counter, and a cell without
  one says so. The map gains a toggle panel of wall-clock time, CPU time,
  and energy, wall-clock time the default. Speed verdicts and the
  regression check stay on wall time.
- **The map shows what its lines measure, by a panel** (Zooko's spec of
  October 18, #38; Devon Jonte): chips at the top of the page named for what a line's height is: per
  wall-clock second (the default, solid lines), per CPU second (dotted: the processor time a unit took, all the program's threads,
  where the run recorded it), and energy (shown, not yet counted); any or
  all at once, on one axis. A chart with no CPU time recorded says so.
  The panel sticks at the top of the page, each section's names under it.
  Every chart on the page shares one log scale (over the measures shown),
  so one way of calling's level compares with another's; small charts
  label each power of ten and bracket, at their last point, the span from
  the worst line shown to the best ("x6.2"; Zooko, October 19); an opened
  chart keeps the bracket, at every case where it holds separate cases. In
  an opened chart the contenders' names above the plot are the toggles,
  and the x axis names what its points are, with their unit (Zooko,
  October 19). The code paths are reported nowhere: a stream's or a
  pool's cannot be reported as a one-shot call's are, so none are (Zooko,
  October 19, reversing October 18).
  Beside them, an upright switch: knob up, rate (GB/s, "higher is better", the default); knob down, cost (ns per
  byte, lower is faster; Zooko, October 18, the graph's toggle before the
  map, brought back): with CPU time, the processor time a byte costs
  across every thread, so a multithreaded call's speed and its total work
  read side by side.
- **The consistency checks retire** (Zooko, October 18; bench-hashes #4,
  T3, and #37): over six runs on an i7-12700K they found nothing real and
  raised about eight false alarms a run, while `compare` already judges
  every cell with the false-discovery rate controlled.
- **The b3sum suite's contenders built as each project recommends**
  (Devon Jonte, #32; Zooko, October 17): BLAKE3-Servil's b3sum statically
  linked on Linux, as its README recommends, official BLAKE3's from
  crates.io; METHODOLOGY says what linking alone is worth (a 64-byte file
  in 0.49 against 0.74 ms on an i7-12700K), so part of BLAKE3-Servil's lead in
  the process-per-file cells is its linking.
- **Fresh link layouts for the b3sum suite** (Devon Jonte, #33; Zooko,
  October 17): a contender may be several builds of one program, one a
  round (`tools/b3sum-contenders.py` with `LAYOUTS=N`), as the library's
  comparisons relink each side per pair.
- **The b3sum suite's contenders run in a Williams design** (Devon Jonte,
  #35; Zooko, October 17), the main run's: with the rotation, each kept one
  predecessor, whose after-effects moved single-file cells by up to 10%
  (the build after `sha256sum` about 10% slower, whichever it was;
  October 18). b3sum results before this change may be off by that much.
- **Every comparison prints the ratio it judges** (decided on Devon
  Jonte's fork, October 18; bench-hashes #47, BLAKE3 #82): the geometric
  mean of the pairs' ratios, the figure `clocks::summary::discoveries`
  tests. `compare` prints it with each side's geometric mean of its runs'
  means (whose ratio it is), `regress` beside each pair's ratio, and
  b3sum's `xN` is it, each round a pair (it was the ratio of the
  contenders' means over all rounds, beside a mark judged on this one).
  The median of the rounds' ratios, the rule before October 17 (#28),
  stays out: three identical b3sum builds read x0.62, x0.69 and x0.98 by
  it, a cell that runs at two speeds landing on one mode or the other; a
  geometric mean blends the modes as a mean does.
- **`regress` with too few pairs for an interval says so** in place of
  stopping (Devon Jonte, #31; Zooko, October 17).
- **The comparison rule judges every cell at once** (Devon Jonte, #11,
  #13; BLAKE3-Servil #44, #45): `compare` and `regress` judge a comparison's cells
  together, their false discovery rate controlled at 5% with the 3%
  margin, by `clocks::summary::discoveries`, in place of a sign test per
  cell; and each pair of runs links each side afresh, its functions in a
  random order, since a binary's layout alone moved cells by several
  percent. On unchanged code the per-cell sign test called 15 to 27 of 700
  cells faster or slower; this rule reads all 700 level, and finds the
  cells a real change moves. `regress --full` judges every cell of a
  default run. METHODOLOGY keeps the rule's choices and their evidence.
- **A cell's samples are sized after three untimed batches, from the
  fastest of three probes** (Devon Jonte, #12): one untimed batch and one
  probe left a cell still warming in some builds, which halved its samples
  and read it 4-10% slower with no slower hashing.
- **`regress` checks the load around its runs** (Zooko's proposal, October
  17, replacing #10's least run length of 0.7 s): each run is its child,
  whose CPU time counts as `regress`'s own, so its load windows stay a
  second long however short or long a run is; a run is no evidence when a
  window it overlaps is busy. Runs need no least length, so a check waits
  less and fits more pairs. **`regress --pairs N --rounds K --keep DIR`**
  set a check's pairs and samples per cell and keep every run's samples
  file (defaults unchanged); and
  **one calibration a side a check** (Devon Jonte; Zooko, October 17): `regress` calibrates
  each side once before its pairs (`--calibrate-only PATH`; a cell both
  sides measure takes the old side's size) and every run takes those
  sample sizes (`--sizes PATH`, one untimed sample warming each cell in
  calibration's place); on an i7-12700K, 18 pairs of 4-round full runs in
  17 minutes against 26, every group of cells judged more narrowly; and
  **`regress --contenders A,B [--points P,Q]`** (#19) judges the cells a
  run with them measures, every one, for a check of one design's cells.
- **CPU time beside wall time** (Devon Jonte; issue #16): each solo
  sample of the continuous and lent cells records the process's CPU time
  across it, all its threads (`clocks::process_cpu_ns`, read outside the
  timed interval), in a new `cpu ns` column (samples v5; a b3sum row
  holds each run's CPU time, from `clocks::child`); `compare` prints each
  cell's CPU-time ratio beside its wall-time verdict. Samples after gaps
  have none (the gap's other work is no part of the call's), nor do
  shared ones (both copies share the process). What a call costs the
  machine, beside what its caller waits for: a pool that spins, or wakes
  more threads, shows here. The verdicts, the map and the guide are
  unchanged.
- **`bench-hashes b3sum` adds many large files** (Devon Jonte, #17): 16
  files of 64 MiB, as many bytes as the 1 GiB file, in one run as
  `b3sum *.iso` hashes them (`--quick`: 4 of 16 MiB). Many large files
  at once are common, and no other input covered them.
- **`bench-hashes leaves` measures produced Merkle leaves** (Devon Jonte,
  #14; a new use case, Zooko, October 18): 16384 and 1048576
  64-byte leaves from a counter, handed to BLAKE3-Servil four ways (lent to
  `hash_many_multithreaded`, lent to a stream, copied into its ring, or
  produced straight into it by `submit_batch_with`), each job timed whole
  from the first leaf produced to the last digest read, solo and two
  programs at once, CPU time beside wall time solo. A subcommand like
  `b3sum`, outside the default run and the map. A prover's encoder makes
  its leaves as it goes; this measures what each way of handing them over
  costs that program.
- **Every digest of a batch is stored in its slot** (Devon Jonte, #15; a
  bug fix): the harness stored only a batch's first digest, so the batch
  cells did less than this contract asks; they read a little slower now.
- **A batch's digests stored in at most two moves** (Devon Jonte, #36;
  Zooko, October 18): the
  same use, each digest in its slot, copied as one run up to the array's
  end and one on from its start; #15's loop copied each digest with its
  own `memcpy` call, which servil mt's stream caller paid for every
  message in series with the workers. The batch cells read faster on the
  same code (servil mt ContinuousBatches 13-18%, the lent batches and
  official's 5-13%; SHA-256 level), most of what #15 had added: the old
  map's pipelined-batch lead over today's came from this copy.
- **b3sum's suite adds the system's `sha256sum`, `sha1sum` and `md5sum`**
  (whichever implementation the platform ships; Devon Jonte, #9; Zooko,
  October 20: the default programs, since those are what a user chooses
  between, where it took the fastest SHA-256 it could find): the sum
  programs people type, beside the b3sums.
- **b3sum's "from storage" cells start from storage** (Devon Jonte, #18;
  a bug fix): each eviction is confirmed with `mincore`, retried until no
  page of the file stays cached (a b3sum that leaves its mappings to exit
  can leave pages that cannot be dropped for 20-50 ms), so those cells no
  longer run partly from the page cache.
- **BLAKE3-Servil's queue retired; its stream measures the continuous cells.**
  BLAKE3-Servil mt's ContinuousMessages and ContinuousBatches call
  `stream()`: each message or batch read straight into the stream's ring,
  its digests into places the program keeps and cycles through the
  stream's handler (the queue's buffers and digest space were cycled
  through its handler and a channel). The stream covers the queue's three
  shapes with one API, its cells measured faster than the queue's on an
  i7-12700K (streaming batches x0.62, messages x0.82 against servil
  beb9abe's queue, three pairs of default runs), and retiring the queue
  leaves BLAKE3-Servil one scheduler (BLAKE3-Servil #59). The guide's examples and the
  map's calls for these cells name the stream's.

**Changes in 0.16.0** (Zooko, October 6, 2026):
- **A new contender, BLAKE3 commonware** (`blake3-commonware`):
  commonware-cryptography's BLAKE3 batch kernels from its pull request
  #4982 (commit 3aa183f0), which hash one message per SIMD lane (NEON 4
  on AArch64; AVX2 8 and AVX-512 16 on x86-64), so that their author can
  compare them with BLAKE3-Servil's on x86-64 machines we lack. It takes
  part in the batch use cases alone (its other calls are BLAKE3
  official's), through `Blake3::hash_many` over the batch's 64-byte
  messages, its digests in a new `Vec` each call; on one thread. Its
  kernels are copied unchanged into `contenders/commonware-blake3`, which
  keeps the benchmark's build free of the rest of commonware's crate.
  `--all` includes it.
- **A run's line says what every reader needs; its provenance sits
  behind a door** (Zooko: the line was too much information, and the
  provenance missing). The line: the machine, its OS, the day, and how
  many seconds other programs kept it busy. "About this run" opens on the
  time, the machine's cores, the power, the load in full, bench-hashes's
  release and commit, BLAKE3-Servil's commit, the crates' versions, the
  compiler and target, and the samples file, linked to the exact code.
- **Escape puts an open chart back** (Zooko).
- **Two runs side by side** (Zooko): `bench-hashes map NEW --beside OLD`
  draws the older run's lines dashed in each chart; the releases'
  comparisons are in `beside/`.
- **A row's or column's name says what it means**, while the pointer
  rests on it or after a click or tap; a click no longer greys its charts
  (Zooko: he could find no explanation of the columns, and the greying
  served nobody).

**Changes in 0.15.3** (Zooko, October 6, 2026), the map's touch alone;
the measurements are 0.15.0's:
- **On a touch screen a tap on an open chart shows the values at its
  point**, as a mouse's pointer does, and the chart's button puts it back
  (a tap anywhere put it back before: Zooko, "tapping on the plot closes
  it instead of opening a tooltip"). A mouse works as before.

**Changes in 0.15.2** (Zooko, October 6, 2026), the map's words alone;
the measurements are 0.15.0's:
- **The first column reads "now and then, after other work"**, as the
  second reads "now and then, after a pause": both calls come now and
  then, and the column says what came before (Zooko: "why does one use
  between and the other after").

**Changes in 0.15.1** (Zooko, October 5, 2026), the regression check
alone; the benchmark's measurements and the map are 0.15.0's:
- **A pair with a run that is no evidence runs again**, up to three
  times, in the same order: on the Mac a process launch now and then
  costs other programs about 1.7 CPU-seconds inside a run's half-second
  window (fork runner jobs 1304-1305), so nearly every check had one busy
  run and gave no verdict. Cells, margins, pairs, and the sign test are
  unchanged; a pair still busy after its repeats leaves the check without
  a verdict.

**Changes in 0.15.0** (Zooko, October 5, 2026), the benchmark for the
optimisation pass that follows it: its maps before and after BLAKE3-Servil's
changes are drawn by one benchmark.
- **A message received, verified as it arrives** (Zooko, "14. Yes"): a
  receiver of iroh-blobs' wire format. Messages of 1 MiB and 64 MiB one
  after another, each arriving as its encoding (its 16 KiB groups and the
  parent nodes above them in pre-order, as bao-tree's `encode_ranges`
  writes a whole blob; made once per input, the sender's work, outside
  the timed calls), in pieces of up to 64 KiB read into a kept buffer,
  verified against the message's hash as they arrive, each verified group
  copied into the message's kept buffer. BLAKE3 decodes with bao-tree
  0.16.1 (`decode_ranges`, no outboard kept), drawn as bao-tree
  (iroh-blobs); both BLAKE3-Servil contenders with a `Verifier`, on the
  calling thread (it has no multithreaded form). Each copies
  every received byte in and every verified byte out once.
- **Many messages at once arrive in turns** (Zooko, October 5: "add the
  lanes across messages before freezing the API"): the pieces come 64 at
  a time, as one wait of a server's event loop delivers them, each read
  into its own kept buffer. BLAKE3-Servil takes each turn in one call,
  `Hasher::update_each` (one thread) or `Hasher::update_each_multithreaded`,
  then `Hasher::finalize_each` for the messages that ended in it; every
  other contender updates per piece and finishes per message, as before.
  The Hasher gathers each message's pieces into whole 16 KiB itself
  (Zooko, October 5), so no contender gathers. Every contender's numbers
  move from 0.14's: the pieces sit in 64 buffers instead of one.
- **A collection, multithreaded, in one call:** BLAKE3-Servil mt hashes a
  collection with `hash_each_multithreaded_with`, the multithreaded twin
  of servil st's `hash_each_with` (Zooko, October 5: each servil
  contender makes the fastest call under its choice of threads, one
  thread or many).

**Changes in 0.14.1** (Zooko, October 5, 2026), the map only; the measurements are 0.14.0's:
- **The map's charts share one rate axis per section, in GB/s** (Zooko,
  "so I could tell at a glance how the columns affect the rows"): a
  section's every chart spans the same range, from all its cells and
  contenders, with faint lines at each power of ten; a batch's rate is
  its messages' bytes per second (its time per batch stays in the
  tooltip).
- **The outboards' BLAKE3 line is named "bao-tree (iroh-blobs)"**, its
  own contender in its own colour: the BLAKE3 contender builds outboards
  with bao-tree, Rüdiger Klaehn's and n0's crate, which iroh-blobs uses.

**Changes in 0.14.0** (Zooko, October 3-5, 2026):
- **The map replaces the graph, the guide's chart, and b3sum's bar chart**
  (Zooko, October 5, 2026): `bench-hashes.map.html`, every cell a small
  chart placed by what the program hashes and how it calls, opening in
  place with its values under the pointer, each call's name a link to its
  documentation (servil-software-factory.github.io/BLAKE3), b3sum's results a section
  in the same form. Every run, and every b3sum run, draws it from the
  samples files in its folder; `bench-hashes map FOLDER` draws it again.
  The guide's answers link to their charts on the map. The README's 1 MiB
  bar chart stays.
- **Every cell charges the program's write and the use of each hash**
  (docs/api-design.md, decisions 1 and 5): after a gap, the write of the
  input is part of the sample, as it is in every nonstop cell; and every
  cell uses each hash the same way, storing it in the program's slot for
  its message (an array of 1024 slots, kept), wherever the design delivers
  it. A design that overlaps writing with hashing, or that adds latency or
  handovers before a hash reaches the program, shows it. Cells after a gap
  read slower than in 0.13.0 by their write (a copy of the input).
- **Messages with their outboards, for verified streaming** (Zooko,
  October 4, 2026): messages of 1 MiB and 64 MiB one after another, each
  written into a kept buffer, then hashed with its outboard: the tree's
  parent nodes above 16 KiB groups, in pre-order, as iroh-blobs stores
  them and as a server makes them when it adds a file. BLAKE3 builds it
  with bao-tree 0.16.1 (`PreOrderMemOutboard::create`), BLAKE3-Servil with
  `outboard_with` on one thread and `outboard_multithreaded_with` on
  several; all give the same bytes. Verifying a
  range as it arrives costs about what building costs (BLAKE3-Servil's
  `verify_range_with`), so one cell serves both.
- **A collection of messages of different lengths, each hashed once**: a
  program naming every item of a collection by its hash, as git, Bazel,
  Nix, and the content-addressed stores do, all in memory, one item after
  another, through each contender's one-message call. Two collections,
  each 2048 items in the shares of real ones by octave of size
  (`COLLECTIONS`): git/git's objects (422,404 at c46c1e3) and the files
  under 1 MiB of 1000 nixos-25.05 store paths (192,213 files); within an
  octave every item has one length, one and a half times its lower bound.
  BLAKE3-Servil on one thread hashes the whole collection in one call of
  `hash_each_with` (BLAKE3-Servil's call for this use case, short items side by
  side in the SIMD lanes); every other contender calls its one-message
  call once per item.
  No cell hashed many small items of different lengths, the main work of
  these programs (docs/api-design.md, "The measurements that settle
  them", 2).
- `bench-hashes b3sum` adds the tree of 1000 files of 16 KiB hashed with a
  b3sum process each, one after another, as a shell loop runs them: what
  a process's start-up (the self-test, the pool) costs beside its hashing
  (docs/api-design.md, "The measurements that settle them", 1).
- **Many messages at once replaces a message in pieces.** A server
  receiving many messages from its connections at once, each in pieces,
  interleaved, on a fixed schedule (`INTERLEAVED`): 256 messages open,
  piece k going to open message k mod 256; piece lengths cycling through
  1448 B (a TCP segment's payload), 1448, 4 KiB (a page-sized read),
  1448, and 16 KiB (a TLS record); message lengths cycling through 64 B,
  1000 B, 4470 B, 16 KiB, 100,000 B, 1 MiB, and 16 MiB, a new message
  opening as each ends. Each contender keeps one incremental hasher per
  open message, each piece read into a kept buffer, then fed to its
  message. No other cell had messages open at once, and a program serving
  many connections meets it. One long message in 64 KiB pieces goes: with
  no thread lingering between updates (BLAKE3-Servil's candidate/no-linger),
  pieces lent to an incremental call hash as one message of their length
  does, which the one-message cells show; the guide shows those for
  `update_multithreaded`. The regression check drops its "lent pieces
  64 MiB" point.

**Changes in 0.13.0** (Zooko, October 2, 2026):
- Each run draws a chart from its samples file: `bench-hashes.chart.svg`
  (one 1 MiB message after other work, as bars) and, for b3sum,
  `b3sum.chart.svg`; `bench-hashes chart SAMPLES.tsv` draws one again
  from a stored file. The READMEs show the published records' charts.

**Changes in 0.12.0** (Zooko, October 2, 2026),
from the regression check's calibration (BLAKE3-Servil's NOTES, "The
regression check, calibrated"):
- Every summary is a run's mean, the total time of its samples over the
  total work they did (`clocks::summary`); the two-speed rule, its
  cut-offs, the graph's second line and its footnote, and the guide's
  fainter dots go. Results of 0.11.0 and earlier were summarised by
  medians, speed by speed: compare them only release with release.
- `regress` judges eight pairs of runs over the lent cells (`lent 64 B`,
  `lent 64 KiB`, `lent 1 MiB`, `lent pieces 64 MiB`, `lent batch 16`,
  `lent batch 4096`), solo: a cell is slower when the median of its
  pairs' ratios exceeds 3% and an exact sign test agrees. The queue's
  cells, the shared scenario, the SHA-256 control, and the confirmation
  stage go.
- A cell's samples are sized after one untimed call of its batch (Devon
  Jonte's finding: the queue's first calls sized its 64 B samples short),
  in place of the single-call retiming.
- `compare` says so beside any run whose load was unmeasured, as well as
  busy (Devon Jonte's finding).
- `bench-hashes b3sum` measures builds of b3sum (it was BLAKE3-Servil's
  tools/b3sum-bench), in this samples format.

**Changes in 0.11.0** (Zooko, October 2, 2026):
- BLAKE3-Servil's queue takes a mode and a handler (`Queue::messages(mode,
  handler)`): BLAKE3-Servil removed its time-or-energy choice, whose
  complexity outweighed its likely use, and the benchmark called
  `Efficiency::Time`, the queue's only way of hashing now. The calls and
  their patterns are otherwise the same.
- The programs' kept buffers are written when made (a bug fix): the
  queue's batch buffers and digest space, its message buffers, and the
  lent pieces' read buffer came from `vec![0; len]`, zeroed pages the
  system maps only at their first write, so they faulted inside each
  cell's first sample, a first-use cost no kept buffer of a real program
  pays (330 against 217 us a queue batch in BLAKE3-Servil's tmp/lentprobe).

Why each piece is here (Zooko, September 28, 2026, replacing the contract
of September 27, whose queue cells measured a round trip rather than
throughput and whose synchronous cells measured calls back to back, which
their users seldom make):

- **Four questions lead a user to one call** (Zooko, September 28,
  evening, `docs/api-design.md`): several threads; data shape; whether
  the receiving thread keeps up; who controls the buffer. The benchmark
  measures the owned-buffer and lent-buffer columns separately.
- **Calls after a gap**: `hash`, `hash_multithreaded`, `hash_many`, and
  `hash_many_multithreaded`, each message or batch after a gap of one of two kinds, each measured
  and compared (Zooko, September 30, 2026; `clocks::Gap`). *After
  other work*, as a program hashes between other tasks, or on a machine
  busy with other programs: a fixed other program (about 1 MiB of
  distinct code), a walk of a kept 128 MiB buffer at 64-byte intervals,
  then integer arithmetic for any remaining millisecond; the whole
  program runs even when it lasts longer. *After idling*, as a server
  waiting for its next request: the thread sleeps 1 ms. Each timed call
  follows the same call (one untimed call, gap and all, before a
  sample's first: Zooko, September 30, 2026), since a gap leaves some of
  what ran before it in the caches, and the benchmark's own schedule
  would otherwise decide what that was (bench-hashes NOTES at `aadeca9d`, "Shared
  after a gap"). The program writes the input after the gap, before the
  call, and the write is part of the sample (Zooko, October 3, 2026,
  replacing September 28's exclusion).
- **Continuous load, buffers owned**: the queue, one message or batch
  after another, with about 1 MiB or 1024 buffers in flight, whichever
  is fewer. Messages up to 64 KiB arrive in one buffer, longer messages
  in 64 KiB pieces. The other contenders use the same producer and
  synchronous calls, serving as the comparison for pipelining.
- **Continuous load, buffers lent** (Zooko, September 28, evening):
  whole messages, a long message in 64 KiB pieces, and batches, read and
  hashed back to back through synchronous calls. The producer's buffer is
  lent until each call returns; reads and hashing take turns. The servil
  single-threaded calls measure the one-thread column; its multithreaded
  calls measure the several-threads, lent-buffer column. Message lengths
  and batch counts match the owned-buffer continuous axes, so the
  comparisons use the same work quantities. Whole messages arrive in one
  buffer even beyond 64 KiB. Pieces use `update` on one thread and
  `update_multithreaded` on several threads.
- **A message in pieces: one long message, nonstop** (Zooko, October 1,
  2026, replacing a sweep of the one-message sizes after each gap and
  nonstop): a message in 64 KiB pieces is measured at 64 MiB, nonstop
  alone. A message of up to 64 KiB is one piece, the one-message call's
  work, so its cell repeated the one-message cell (within 0.5% at 64 KiB
  on the Mac, jobs 829-830). On one thread the cost of a long message's
  pieces follows from the 64 KiB one-message cell (3-8% above it). A
  multithreaded incremental call can spread a long message's pieces over
  threads, a rate no one-message size predicts (servil mt 0.10 ns/B,
  against 0.23 for one 64 KiB message and 0.036 for one 64 MiB buffer):
  one long message shows it, and the sweep added nothing more (0.10-0.12
  ns/B from 256 KiB to 64 MiB). The sweep after a gap took a third of
  each gap phase's time.
- **The program's side of the queue allocates nothing after warm-up**
  (Zooko, September 28, 2026, morning): the program makes its queue and
  a bounded channel for the returns once and keeps both, as a program
  makes one queue for its life, and the channel is a ring allocated when
  it is made. Until then each sample made a new queue (its slots
  allocated inside the sample) and returned buffers through an unbounded
  channel, which allocates a block every few dozen messages; the queue's
  own contract (no allocation after warm-up) was never reached.
- **Batches under the padded batch contract** (Zooko, September 26): the
  caller lays out and zero-pads the messages.
- **Shared scenarios for the nonstop use cases** (Zooko, September 28;
  after the gaps removed September 30): two copies at once, one input
  after another; a sanity check against designs that need the machine
  to themselves, and a pessimistic estimate of what users see; measured
  and reported, not optimised for directly. After a gap a copy met the
  code its twin had just run in a cache the gap left warm, and read
  faster than one program alone (bench-hashes NOTES at `aadeca9d`, "Shared after a gap"). The map
  draws solo and shared.
- **Planned, to add under this contract**: keyed and derive-key spot
  checks in perf_regress.

## The overview

(Zooko, October 22, 2026; its design as of October 24.) One page above
the maps, built from every machine's published run, that answers "how
does BLAKE3-Servil do on my kind of machine?" and "how does it do for my
use, across machines?".

- **The first view: three bars a machine** (the machines in the grid),
  on one logarithmic scale for every machine: BLAKE3-Servil's
  multi-threaded form, BLAKE3 official's (off until asked for) and the
  fastest SHA-256 the run measured (sha2, ring, or the OS's). A bar's full
  length is hashing nonstop on every core (ContinuousMessages 64 KiB:
  servil's stream, official's multi-threaded form where measured); the
  part inside it, one message of 1 KiB after other work, where SHA-256
  with the CPU's instructions leads. So the view spans where servil
  trails to where it leads most.
- **The grid: a column a kind of machine, a row a representative cell.**
  The columns are Arm, AMD and Intel (and "Other" for a run on another
  CPU), each the family's machine in `overview::CHOSEN` where the
  results hold it, else the family's first. The page's door, "All N
  machines measured", shows every machine as a card in plain words (its
  kind, design and year, `overview::KNOWN`, with the headline contest at
  a glance); a card puts its machine in its family's column, and each
  column's button opens the door at its family. In each
  cell a bar a hash, each cell on its own linear scale (its fastest bar
  fills the width, that speed written under its end). The rows, chosen
  once, here; the overview reads these cells and no others:

  Each cell holds every form of each library its run measured (servil's
  two, official's one or two) and the fastest SHA-256; the column below
  names the form the row's text report and the first view compare.

  | row | scenario, use case, point | servil's form for the text | the situation |
  |---|---|---|---|
  | 64 B; 1 KiB | solo OneMessage 64 B; solo OneMessage 1 KiB | blake3-servil-st | a key, an identifier, a small record, hashed between a program's other work |
  | 1 MiB | solo OneMessage 1 MiB | blake3-servil-st | a buffer the size of a file |
  | 128 MiB | solo OneMessage 128 MiB | blake3-servil-mt | a download or a disk image |
  | 64 KiB messages, one after another | solo ContinuousMessages 64 KiB | blake3-servil-mt (the stream) | messages arriving one after another, as fast as the program can feed them |
  | 1024 messages of 64 B, batched | solo ManyMessages 1024 | blake3-servil-st | a Merkle layer, a content-addressed store, a database's keys |
  | 1000 files, b3sum | b3sum, warm, mixed tree, 1000 files, 74 MiB | BLAKE3-Servil's b3sum (against official b3sum and the system's sha256sum, sha1sum and md5sum: still what many people run on a download, whatever their security) | the command people actually run, process start to exit |

  Seven cells. **The budget is eight.** Adding one names the one it
  displaces, or argues the rare exception, here. Each cell is a card that
  opens its chart on that machine's map.
- **One colour a library.** A library's single- and multi-threaded forms
  share its colour, the multi-threaded striped (on the map, a double
  line); SHA-1 and MD5 are drab greys, broken as cryptographic hashes and
  shown as the checksums people still run.
- **A bracket: servil against its strongest rival.** In every cell, and
  on each first-view bar's two measures, the bracket joins BLAKE3
  servil's bar and the fastest other bar shown and gives the factor
  between them in the winner's colour; with servil hidden there is none.
  The map's brackets follow the same rule.
- **The legend is the hashes drawn,** a button each, a library's two
  forms one above the other, under "Show:", with a line saying a click
  shows or hides. A button hides or shows its hash's bars (in the first
  view, its form's part of a bar); an off button keeps its name readable
  and its swatch hollow. The page opens on BLAKE3-Servil, multi-threaded,
  and SHA-256 (Zooko, October 27).
- **Machines** are each one published run, the newest of the current
  benchmark release on that machine: its CPU name, core count and OS from
  the run's own header, its date, and the servil commit it measured. A
  machine whose last run is another release's keeps a column that names
  it and links its map, with no cells, until a run under this release
  replaces it: one page shows one release's cells and never averages
  two. A virtual machine is kept (it is data): a card behind the door,
  never a family's first choice.
- **Nonstop compares the calling thread's view.** In ContinuousMessages
  BLAKE3 official hashes on the one calling thread, while servil's stream
  uses the machine: what one calling thread gets from each library, the
  stream's promise. A pool of official callers would be the machine's
  baseline; Zooko decided against that contender (October 23): the stream
  earns its place on the machines measured.
- **One pipeline, no copies.** The overview reads the same samples files
  the maps are drawn from, through the same reader and the same summary
  rule (one mean, every sample counted); a run redraws it, and a test
  fails while it is stale. No number on the page is typed by hand.
- **What the page cannot say.** A figure from one run on one machine is
  that run's; it carries the run's load reading on its map, and no
  verdict (slower and faster are the regression check's words, from
  pairs of runs).

```frozen
use case OneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 1536 B, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case ManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case ManyLongMessages: 4, 16, 64, 256
use case IdleOneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 1536 B, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case IdleManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case BriefIdleOneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 1536 B, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case BriefIdleManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case ControlledIdleOneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 1536 B, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case ControlledIdleManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case ContinuousMessages: 64 B, 128 B, 256 B, 512 B, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB
use case ContinuousBatches: 16, 64, 256, 1024, 4096, 16384, 65536
use case LentMessages: 64 B, 128 B, 256 B, 512 B, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB
use case Interleaved: 256
use case Collection: git objects, Nix files
use case LentBatches: 16, 64, 256, 1024, 4096, 16384, 65536
use case ContinuousCollection: git objects, Nix files
use case MerkleTree: 1024, 4096, 16384, 65536, 262144
use case InterleavedTurn: 256
use case IdleInterleavedTurn: 256
use case BriefIdleInterleavedTurn: 256
use case ControlledIdleInterleavedTurn: 256
use case ReceivingMessage: saturated, 2 us, 20 us
receive trace: 16777216 B per message; fragment cycle 1448, 4096, 16384, 65536 B; arrival spacing saturated: 0 ns, 2 us: 2000 ns, 20 us: 20000 ns; deadlines fixed from the first arrival, independent of consumer work; receiving through finalization timed together
scenarios: solo, shared-one-cache, shared-two-caches, shared-os
shared measures: ContinuousMessages, ContinuousBatches, LentMessages, Interleaved, Collection, LentBatches, ContinuousCollection, MerkleTree, ReceivingMessage
shared placements: two cores under one last-level cache; two cores under two (where the machine has two); where it offers no pair (macOS), wherever the OS runs them
shared contenders: blake3-official-mt, blake3-servil-mt, blake3-servil-pipeline
blake3-servil-pipeline ReceivingMessage: throughput::pipeline: one Stream kept for the process (64 rooms of 16 KiB, 1 MiB in flight; 2 message slots), Stream::message per message with its length as the hint, each received fragment written into the rooms Message::input gives and committed (the one copy of an arrival), the last fragment's final bytes ended with Message::end(n), then collect and ready until the digest is delivered; 1448 B, 4 KiB, 16 KiB and 64 KiB arrivals in a repeating cycle, cut at message end; fixed arrival deadlines from the first fragment (saturated, 2 us, 20 us); reads, waits, writes and the wait for the digest timed together
blake3-servil-st ReceivingMessage: convenience::Hasher::new(Mode::Plain, None), update per received fragment, finalize after the last; 1448 B, 4 KiB, 16 KiB and 64 KiB arrivals in a repeating cycle, cut at message end; a kept 64 KiB receive buffer; fixed arrival deadlines from the first fragment (saturated, 2 us, 20 us); reads, waits, updates and finalization timed together
blake3-servil-mt ReceivingMessage: convenience::Hasher::new(Mode::Plain, None), update_multithreaded per received fragment, finalize after the last; 1448 B, 4 KiB, 16 KiB and 64 KiB arrivals in a repeating cycle, cut at message end; a kept 64 KiB receive buffer; fixed arrival deadlines from the first fragment (saturated, 2 us, 20 us); reads, waits, updates and finalization timed together
blake3-servil-st InterleavedTurn: the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after other work: throughput::update_each, then throughput::finalize_each for the messages that ended in it
blake3-servil-st IdleInterleavedTurn: the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after idling: throughput::update_each, then throughput::finalize_each for the messages that ended in it
blake3-servil-mt InterleavedTurn: the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after other work: throughput::update_each_multithreaded, then throughput::finalize_each for the messages that ended in it
blake3-servil-mt IdleInterleavedTurn: the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after idling: throughput::update_each_multithreaded, then throughput::finalize_each for the messages that ended in it
blake3-servil-st MerkleTree: throughput::hash_many(Mode::Keyed(key), &Batch::from_blocks(layer, len), out) for each layer, a tree a sample: the leaves (256 B) written into a kept buffer and hashed under the leaf key, then each layer of parents (64 B, two children's digests) under the parent key into a kept buffer of its own, to the root
blake3-servil-mt MerkleTree: throughput::hash_many_multithreaded(Mode::Keyed(key), &Batch::from_blocks(layer, len), out) for each layer, a tree a sample: the leaves (256 B) written into a kept buffer and hashed under the leaf key, then each layer of parents (64 B, two children's digests) under the parent key into a kept buffer of its own, to the root
blake3-servil-st OneMessage: convenience::hash(input), each call after other work
blake3-servil-st ManyMessages: throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after other work
blake3-servil-mt OneMessage: throughput::hash_multithreaded(Mode::Plain, blocks, len), each call after other work
blake3-servil-mt ManyMessages: throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after other work
blake3-servil-st ManyLongMessages: throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 4096), out), the batch borrowed in place, each call after other work
blake3-servil-mt ManyLongMessages: throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 4096), out), the batch borrowed in place, each call after other work
blake3-servil-mt ContinuousMessages: throughput::stream(Mode::Plain, handler, body), one a sample: one message after another, each read straight into the stream's ring (submit_with), its digest into one of 1024 kept places, used as the place comes back and the place used again
blake3-servil-mt ContinuousBatches: throughput::stream(Mode::Plain, handler, body), one a sample: one batch after another, each read straight into the stream's ring (submit_batch_with), its digests into a kept place of the batch's size (enough that the ring fills first, up to 1024), the place used whole as it comes back and used again
blake3-servil-st ContinuousMessages: throughput::hash(Mode::Plain, blocks, len), one message after another, each read into a kept buffer of the program's in whole blocks and hashed on its thread (a Hasher fed 64 KiB pieces above 64 KiB)
blake3-servil-st ContinuousBatches: throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, one batch after another, each read into a kept buffer of the program's and hashed on its thread
blake3-servil-mt ContinuousCollection: throughput::stream(Mode::Plain, handler, body), one a sample: the collection's messages one after another, each read straight into the stream's ring (submit_with), its digest into one of 1024 kept places, used as the place comes back and the place used again
blake3-servil-st ContinuousCollection: throughput::hash(Mode::Plain, blocks, len), the collection's messages one after another, each read into a kept buffer of the program's in whole blocks and hashed on its thread
blake3-servil-st LentMessages: throughput::hash(Mode::Plain, blocks, len), one message after another, each read into a kept buffer in whole blocks and lent until the call returns
blake3-servil-st Interleaved: a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; throughput::update_each per turn, then throughput::finalize_each for the messages that ended in it, each buffer lent until the calls return
blake3-servil-st Collection: throughput::hash_each(Mode::Plain, items, out), every item of the collection in one call, in memory
blake3-servil-st LentBatches: throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, batches one after another, each read into a kept buffer and lent with kept digests until the call returns
blake3-servil-mt LentMessages: throughput::hash_multithreaded(Mode::Plain, blocks, len), one message after another, each read into a kept buffer and lent until the call returns
blake3-servil-mt Interleaved: a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; throughput::update_each_multithreaded per turn, then throughput::finalize_each for the messages that ended in it, each buffer lent until the calls return
blake3-servil-mt Collection: throughput::hash_each_multithreaded(Mode::Plain, items, out), every item of the collection in one call, in memory
blake3-servil-mt LentBatches: throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, batches one after another, each read into a kept buffer and lent with kept digests until the call returns
blake3-servil-st IdleOneMessage: convenience::hash(input), each call after idling
blake3-servil-st IdleManyMessages: throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after idling
blake3-servil-mt IdleOneMessage: throughput::hash_multithreaded(Mode::Plain, blocks, len), each call after idling
blake3-servil-mt IdleManyMessages: throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after idling
overview cell: solo OneMessage 64 B, blake3-servil-st (64 B)
overview cell: solo OneMessage 1 KiB, blake3-servil-st (1 KiB)
overview cell: solo OneMessage 1 MiB, blake3-servil-st (1 MiB)
overview cell: solo OneMessage 128 MiB, blake3-servil-mt (128 MiB)
overview cell: solo ContinuousMessages 64 KiB, blake3-servil-mt (64 KiB messages, one after another)
overview cell: solo ManyMessages 1024, blake3-servil-st (1024 messages of 64 B, batched)
overview cell: b3sum warm mixed tree, 1000 files, 74 MiB (1000 files, b3sum)
```
