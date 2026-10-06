# Notes for the benchmark maintainers

For whoever maintains `bench-hashes` next. This file is about measuring
fairly, accurately, and cheaply. It says nothing about how any contender
is built; the BLAKE3 fork's maintainers keep their own notes in their
own repository, and this benchmark treats every contender as a black
box behind a function call. Keep it that way: a benchmark that knows a
contender's internals starts to accommodate them.

`AGENTS.md` covers style and the VM environment. `README.md` is the
user-facing description. This file is the reasoning behind the design
and the list of ways it can still lie.

## What is settled

**The bytes hashed** (settled several times over, last October 3, 2026,
written down so it stays settled). In memory, any bytes the program has
written: their values do not change any contender's speed. Never memory
read before it is written (uninitialised, or zeroed by the allocator
without a write): the operating system backs a page nobody has written
with one shared page of zeros (Linux maps every such page of a buffer to
the same physical page), so a 64 MiB input would be read from one 4 KiB
page in the L1 cache, faster than any real data; and in Rust, reading
uninitialised memory is undefined behaviour. The benchmark writes counter
words (`make_input`) once, outside the timed work. Files on storage (`bench-hashes
b3sum`) hold BLAKE3's extended output of their names (`contents`): a
filesystem or a drive that compresses would read zeros or counters from
storage almost for free.

**The benchmark checks no contender's outputs, nor do its tests**
(Zooko, September 26 and October 3, 2026): correctness is each
contender's own tests' business (the fork's `QUALITY.md`).

**Sizes.** Twenty-seven: 64 B to 128 MiB by powers of two but 16 MiB,
plus 3 KiB and 3 MiB, plus four sizes of real data between 2 and 8 KiB,
chosen by use rather than by any implementation's structure: 2304 B (an
802.11 frame body at its maximum), 3839 and 7935 B (the 802.11n A-MSDU
maxima), 4470 B (the Packet over SONET/SDH MTU). None is a multiple of a
chunk, as real inputs seldom are; they showed a trailing partial chunk
costing the fork 7-65% until it ran beside the whole ones. Sixteen through 1 MiB came first; 2, 4, and 8 MiB were added
to show the plateau, then 16 to 128 MiB when the fork's multithreaded
rate was still climbing at 8 MiB (it levels from 4 MiB with the ranked
pool; Rayon's still falls at 128 MiB on the VM). 16 MiB was dropped
after both machines' records showed every contender's median there
within run-to-run noise of the value 8 and 32 MiB predict (log-log);
32, 64, and 128 MiB each carry something the others do not (Rayon's bend
on the VM, serial servil's rise at 128 MiB on the Mac). 3 KiB and 3 MiB are
non-power-of-two trees, one at the SIMD ramp and one at the plateau; a
contender whose work splitting assumes powers of two shows it there (and
one did).

**BLAKE3 commonware** (added September 26, 2026; a two-way door, Zooko:
remove it whenever it stops paying for its build time). Commonware's
cryptography crate, at the commit of its pull request 4982
(github.com/commonwarexyz/monorepo, 25851f1, open when added), gives
BLAKE3 its own batch kernels: `Blake3::hash_many` over any
`AsRef<[u8]>` messages, runs of equal length a vector's width at a time
(NEON four lanes, AVX2 eight, AVX-512 sixteen, spare lanes repeating the
first message), a group of one through the official crate. Its one
message, its stream, and its multithreading (`hash_with` over a
`Strategy`, the official crate's hazmat subtrees on Rayon) are the
official crate's, so it takes part in the batch use cases alone. The
bencher hands it the messages as `[u8; N]` arrays in place (no table to
build) and counts the returned `Vec` as part of the call, as its API has
it. Its default features stay on: run-time AVX detection on x86-64 needs
"std", which also builds BLS signatures (155 crates in all, about 20 s of
build). Before it joined, probe/commonware (fork job 254, then 256-271)
measured it beside servil on the Mac, P- and E-cores: servil led every
batch cell but 3 x 256 B (144 against 183 ns/msg, P-core) and 2-3 x 256
B on E-cores; fork 9fd0ac9 and 666e550 closed those (3 x 256 B: 79).

**Batch sizes.** Twenty-four on each of two axes: 1 to 262144 messages
of 64 B (a Merkle tree's inner nodes) and of 256 B (its leaves, as in
WHIR), with 3, 6, 12, 24, 48 beside the powers of two to leave SIMD
groups partly filled. Samples on that axis divide by messages, so the
statistics pipeline is unchanged and only the unit names and the rate
scale (1 GB/s per ns/B; 1000 Mmsg/s per ns/msg) differ per plot.

**Round counts** are a plain 96 (24 with `--quick`). They used to
be the least multiple of the point count and the order count at or above
a target, so every order and starting point recurred equally often; that
made removing one point cost several times the run (47 points and 8
orders: 376 rounds). The imbalance a plain count leaves is a fraction of
a sample per cell.

**Inputs** are little-endian 64-bit counter words `seed << 48 | index`:
every block differs, and hash speed does not depend on the bytes.

**Time budget for long cells.** 78% of a full run went to 58 cells
whose single hash takes over 2 ms (SHA-1DC at 128 MiB, 170 ms a sample).
Sampling them in every fourth round alone moved noisy cells' medians by
up to 16% (Rayon at 16 MiB), so the budget adapts: from 4 ms a hash, every
fourth round, and every round while the median's 95% interval is wider
than 1%. Simulated on the recorded samples: every such median within
0.6% of the full one. Measured: 150 s -> 101 s a run; budgeted cells
agree with a full run to a median 0.68%, against 1.37% run-to-run for
cells sampled every round. Then a 2% target and at most every second
round for unsure long cells, and a 250 us calibration probe: 80 s a run,
medians at x0.9955 and x1.0072 of two full-sample runs, which differ from
each other by x0.9886. Rejected: all cells in every second round (a
bimodal cell's median moved 35%), adaptive sampling for short cells
(1 ms samples rarely reach a 1% interval, so little is saved), and 0.5 ms
samples (62 s, every median 1.6% slow).

**Interleaving.** Williams orders over the contenders, size order
rotated per round. Every contender takes every position and follows
every other equally often. The solo and shared samples of a batch are
taken back to back within one interval, so they share whatever the
machine was doing at that moment.

**Time basis.** Wall time, always. Cycle normalisation (cycles per byte
at the run's sustained clock) was used for solo samples on Apple until
September 2026 and removed: the core's cycle counter does not see time
spent waiting on the SME unit (a 25% slower SME2 batch cell read the
same cycles per message at a lower apparent clock), so normalising
hid real SME2 slowdowns, and would hide a GPU's the same way; and a
contender's own power draw throttling the clock is a cost its user pays.

**Scenarios.** Every run measures solo (one copy on one thread) and
shared (two independent copies on two persistent threads, each over its
own input of the size, with different contents so they share no cache
lines, released together). Each shared copy's own time is a sample, two
per interval; the later finish, used until September 2026, measured how
unevenly two copies are served rather than what each user gets.

**Two speeds.** A cell whose samples split at a gap of 4% of the median
or more, a tenth or more on each side, with the sides' medians 1.25× or
more apart, has two speeds; every report shows both (text `a|b`). The graph
(September 25, 2026, Zooko: readers were mystified by lines splitting
and merging) draws each point's common speed as the line and the rare one
as dots and segments dimmed by its share (rare over common samples, floor
0.15); the hover says "Two speeds observed. See footnote [*]." and the
footnote under the plots names performance and efficiency cores as one
cause.
The 1.25× floor: on the VM the machine's own noise puts a tenth to a
third of many cells' samples 10–14% slow for every contender alike, which
drew a second line nearly everywhere at 4% alone; SME2 unit sharing splits
1.7–2.0×. Open: `perf_regress` judges a cell by its 5th percentile, the
faster speed alone; a regression confined to the slower speed passes it.

**After idle** (September 26, 2026, Zooko: so the benchmarks catch a
slowdown that only calls made now and then meet; in the text and the
checks, not the graph). Each sample interval ends with one copy sleeping
IDLE_NS (1 ms, past the fork's 200 us of polling) and then calling: one
call, or as many as fill IDLE_BURST_NS (10 us), since a single call
shorter than a few microseconds cannot be timed on a 24 MHz counter. It
found servil mt 5-6x slower than servil st at 64-512 KiB and 3-4x in
batches of 1024-4096 (the pool's workers asleep, woken per call), in
every run. On the VM every contender's calls after idle come at two
speeds about 3.5x apart (the vCPU woken cold or warm, independently per
call), so CHECKS compares the two cells' fast speeds there: paired by
round, the worse-speed rule turned the lottery into x6-9 findings
against SHA-256, and even the median ratio misfired at 12 samples
(servil mt against st at 3839 B, the same code). The samples are a
phase of their own after the rounds, which run as before: solo cells'
5th percentiles vary as much between runs with it as without (VM,
alternated: 1.40% against 1.53%). A first comparison against an older
sequence had read 1.3% against 0.48% and blamed the sleeps: the VM had
been quieter when the older sequence ran, the mistake A B B A exists to
prevent. perf_regress judges after-idle cells at a 20% margin and holds a
change on them. A default run: 15 s -> about 20 s.

**Full by default, `--quick` on request** (September 2026, for people
who run it once and publish what they get). A full run: every point, 96
rounds, the long-cell budget, SHA-1DC in `--all`: 60 s on the VM for the
default roster, about 140 s for `--all`. `--quick` stops below 1 MiB
and 10,000 messages, 24 rounds, SHA-1DC only when named: about 12 s, and
it may misread a cell.

**The default roster is fixed** (September 2026): BLAKE3 servil, servil
mt, sha2, and ring. It replaced a run that measured every contender and
kept the Pareto-best per family, which cost `--all`'s time and printed
whatever the measurement chose. SHA-256 keeps two crates because
neither dominates on either machine: sha2 leads at 64 B (0.50 against
0.65 ns/B) and 128 B and in every batch of 64-byte messages (30 against
40 ns/msg), ring from 256 B (0.30 against 0.35 ns/B from 1 KiB). Whether
ring leads on x86 is unmeasured (it has SHA-NI, AVX, and SSSE3 paths,
sha2 SHA-NI and portable code). The graph opens showing servil mt, SHA-256
ring, and crates.io BLAKE3 (`SHOWN_AT_FIRST`; Zooko, September 25, 2026:
fewer lines for the viewer, sha2 one click away); a run without any of
them shows everything.

**The streamed use case** (September 25, 2026, Zooko): the one-message
sizes fed through each contender's incremental API in 64 KiB pieces, solo
and shared; one piece size, to keep the run time down (a full default run
grows by about half). It exposes what one-shot calls hide: first quick
run (VM), BLAKE3 servil through `Hasher` at 3 KiB 0.53 ns/B against 0.33
one-shot, 3839 B 0.58 against 0.34, 7935 B 0.44 against 0.27 (the hasher
cannot plan the whole input, and holds the last chunk back until
finalize); BLAKE3 official mt's `update_rayon` per 64 KiB piece 2.7-7 ns/B; the
fork's `update_multithreaded` 0.09-0.14 from 64 KiB. `hash_batch` takes
the `Point` (use case and message count) so a stream and a message of one
size are told apart; `--points` names a streamed point `streamed LABEL`.

**Load from other programs** (September 25, 2026, after a Mac record
read 8-10% slow across every contender with dips in the batch plot, most
likely from the user's own work on the Mac): at round boundaries the run
reads the machine's busy CPU time (`/proc/stat`, macOS
`host_statistics(HOST_CPU_LOAD_INFO)`) and its own process CPU time; the
difference over 5 s windows (10 ms ticks make shorter ones noisy) is other
programs' load, in milli-CPUs. Busy: the busiest window at one CPU or
more (first 0.5; the Mac's steady desktop-plus-VM background of 0.40-0.56
CPUs, job 114, flagged a run whose medians matched e16e836's to 1%). Provenance, text report, and samples (`# load:`, `# other load by
5 s window`, `# steal by 5 s window`) carry it. VM quiet 0.02 CPUs
average, 0.07 worst; two `yes` loops read 2.04. **Open**: this VM's
hypervisor reports no steal (0 since boot through Mac jobs), so host load
stays invisible in the guest; a reference loop timed beside the samples
would see it.

**Load moved into clocks** (September 30, 2026, after Zooko noticed he had
run Brave through the day's Mac jobs): the benchmark's own detector had
flagged jobs 780 and 782 busy in every run (1.1-1.4 CPUs of other load on
average) and single runs of 783 and 788, and nobody read it. It also
summed macOS's four 32-bit tick counters before a wrapping difference, so
one counter wrapping would have read as enormous load. Now `clocks::load`
(the fork) is the one implementation, and every measurement records load
without code of its own: windows of about a second, read between samples
(VM: a reading 8.6 us, a tick between readings 18 ns), each `Batch`
stamped with its start, a busy window reported on stderr as it closes.
Samples v4 carries every window (`# load windows`) and each sample's start
(`start ms`); `tools/samples.py` (the fork) is the one Python reader, and
perf_regress gives no verdict when any of its runs was busy. VM: quiet
0.00 average, 0.02 worst window; two `yes` loops 1.98-2.01 in each of
their windows, 2859 of 5904 samples in them.

**Graph labels** (September 2026, after overlaps in the published
graph): right-hand names stack 34 px apart, so eight fit inside a plot;
x labels place the powers of two first, then the sizes between, on two
rows, a second-row tick never crossing a first-row label; value labels
go on columns at least 110 px apart with 32 px free on each side (none
in the crowded 2-8 KiB stretch until the zoom spreads it), and step
clear of every shown dot in their column. The static render uses the
contenders shown at first for its axes and labels, as the script does.

**Checks compare round by round.** Judging each cell by its slower speed
(30f6776) compared unlike moments: on the Mac a tenth of the solo samples
from 256 B to 8 KiB ran 1.65-3.3x slow for every contender (every fourth
round, when the full run's long all-core cells are sampled; likely an
efficiency core), and a cell that happened to split was compared at its
slow speed with a neighbour that had not: servil "x4.70 slower than SHA-256
ring" at 256 B. Every comparison now pairs the samples of one round (each
cell records its rounds), judges the ratio's worse speed where the ratios
split, and needs it 5% above 1 with its 95% interval above 1. A moment
that slows both sides cancels; a slowdown of one side counts.

**Checks.** The report's CHECKS section lists what a regression hunter
looks for, for the servil contenders: slower than another contender at a
point (servil against single-threaded contenders, servil mt against all,
servil mt against servil), and slower per unit at a point N than at a
smaller point M dividing N (M's work N / M times would have been
faster); 5% or more, intervals apart; identical claims merge across the
two contenders and scenarios; worst first. "Divides" matters: 3 messages
slower per message than 2 is no defect, 128 slower than 64 is.

**Correctness** is each crate's tests' business (Zooko, September 26,
2026): the benchmark checks no digests. It used to check every contender
against golden digests at every timed input, both seeds, one-shot and
streamed, before calibrating: 15 s of an --all run, 10 of them BLAKE3
official mt's streamed update_rayon, and a vector file with its generator
to keep. The timed loop still hands every digest to `black_box`, and a
test checks that the dispatch observes every iteration's digest.

**Provenance.** The build script embeds the git state of this
repository and of the fork checkout (branch, commit, clean or a hash of
the diff). A report that says `dirty-…` measured uncommitted code.
Before publishing a result, commit first.

## Threats to validity, and what was done about each

These are the ways the benchmark has been wrong so far. Each was found
by a result that looked too neat.

00. **The busy gap, and what it leaves** (September 28, 2026, Mac jobs
   774-775). With 1 ms of integer work in place of the sleep, the calls
   after the gap run at full clock on P-cores (4.4-4.6 GHz, no E-core
   samples in a traced run), and two runs agree closely; from 8 KiB the
   cells read about as back to back. Small cells still split in two
   (servil st 512 B 0.66-0.70 or 1.74-1.93 ns/B, SHA-256 0.46-0.53 or
   0.98-1.03, all on P at full clock; the slow ones take about twice the
   cycles). A probe of the same calls after the same gap in a fresh
   process (probe/busy-gap, job 775) found one speed, with or without the
   counts' system call around each call and with vector work in the gap.
   In the traced run the slow samples mostly follow a large cell (a 32-128
   MiB stream) or open their point: caches the neighbour left cold. So the
   calls after the gap meet warm caches or the neighbour's aftermath, by
   the schedule; which a program's gap should leave (warm, or evicted by
   work that touches memory) is Zooko's to decide. perf_regress shows
   these cells' slow speed and does not judge it until then.

0. **The program's sleeps lower the clock of its later work** (September
   28, 2026, Mac jobs 735-741). Continuous cells sampled in the same
   rounds as the synchronous cells (each call after 1 ms asleep) ran at
   2.1-3.5 GHz, median 3.3, even right after another continuous cell;
   the same cells alone ran at 4.3-4.5 GHz (the Mac's performance
   controller follows the process's recent history, not one sample's).
   Every contender's continuous cells read 30-100% slow (SHA-256's 64 B
   0.95 against 0.56 ns/B), unlike main's back-to-back tables, which
   took the after-idle samples after all the rounds. Now each phase
   calibrates and samples on its own, continuous first (9869b27):
   SHA-256's continuous cells 0.35-0.36 ns/B from 1 KiB, its old
   back-to-back 0.34 plus the read's copy; two runs agree within 2%.
   The synchronous cells now meet the clock of a program that only
   sleeps and hashes (full clock after the gap in 11% of samples, was
   18%); what the program does in the gap is Zooko's open question.

1. **Waking a thread is not free.** The duo release was a
   `std::sync::Barrier`. On a 2-CPU VM, one copy woke 300 µs after the
   other because the caller's own thread had just used that CPU. The
   copies now poll an atomic generation counter and are already in the
   instruction stream when it flips; each reads the sample clock as its
   first act. Any future "release together" mechanism must not depend
   on the OS scheduler.

2. **A busy spin steals a scheduler quantum.** The first polling
   release used `spin_loop`. Two such spinners on a 2-CPU machine held
   both CPUs for ~2 ms each, and any contender with worker threads saw
   them start 2 ms late: a 29 µs hash measured 2 ms. Polls in the
   harness now `yield_now()` between checks. Rule: the harness must
   never hold a CPU it isn't measuring on.

3. **The caller must not be a third contender.** In duo, the main
   thread posts the job then *sleeps* on a condvar until both copies
   finish. If it spun, it would be a third thread competing for CPUs on
   a 2-CPU machine. Solo samples run on the main thread with the copy
   threads asleep.

4. **Calibration must match the measured shape.** Iterations per batch
   are calibrated solo (~1 ms). A duo batch of the same iterations takes
   at least as long, so it lands at or above the target; that is
   acceptable. Calibrating under duo would tie the batch size to the
   contender's contention behaviour.

5. **Two copies must hash different bytes.** `make_input_seeded(size,
   1)` for copy 1. Sharing one buffer lets two copies share L2 lines
   and understates memory cost.

6. **Stack frames count.** A 6 KiB frame inlined into `hash()` cost
   every 64-byte call a page probe. The harness is not immune: keep the
   timed region a straight line from `now()` to `since_ns()` around
   `run_batch`, with nothing allocated inside.

7. **Two-mode cells are real.** `find_modes` splits a cell whose
   samples cluster ≥4% apart with ≥10% on each side; the hover shows
   both. Don't "fix" this by taking more samples — it usually means the
   contender behaves two ways depending on what ran before it, which is
   information.

## Things a contender could do that this benchmark would reward unfairly

Watch for these when reading a result. None is currently detected
automatically.

- **Caching across calls.** Every batch hashes the same buffer
  `iterations` times. A contender that memoised on pointer+length would
  score infinitely well. Inputs are `black_box`ed but the bytes don't
  change. Defence if needed: rotate among several equal-size buffers
  within a batch, or perturb one byte per iteration outside the timed
  region. Not done yet because no contender does this and it costs
  cache locality that every contender would then pay.

- **Knowing the harness's thread count.** A multithreaded contender
  could detect "exactly two callers" and behave specially. The duo count
  is fixed at two; a `--trio` or `--n N` mode would make gaming it
  harder and is a natural next step (see below).

- **Persistent worker threads that stay hot.** A contender whose workers
  keep polling between calls looks better in a tight benchmark loop than
  in a program that hashes once a second. Spins that yield are fair to
  other threads; spins that don't are threat #2 from the contender's
  side. Consider a `--gap MS` option that sleeps between batches so
  workers must actually wake.

- **Reading the environment.** The fork once honoured a `BLAKE3_LANES`
  override; that is gone, and `hash_multithreaded_with_budget` is the
  way to cap threads. The fork reads no `BLAKE3_*` variables now, so the
  report has nothing to record there.

## Open questions and next steps

- **More than two copies.** Duo catches whole-machine pools but a
  contender sized to half the machine looks perfect under duo and bad
  under trio. `--copies N` generalising `Duo` is the obvious extension;
  `Duo` was written with two hardcoded, and `finished: [Option<u64>; 2]`
  is the main thing to generalise.

- **Cross-process contention.** Duo copies are threads in one process.
  A contender can coordinate across threads (shared counters) in ways it
  can't across processes. A `--duo-process` mode spawning a second
  `bench-hashes` would test the honest case. Measured by hand once: two
  processes of the fork's mt hash each ran at single-threaded speed,
  which is the right answer, but nothing automated checks it.

- **Idle between calls.** See "persistent worker threads" above.

- **Noise floor.** `~` marks cells whose 95% median interval is at least
  5% of its median. Keep VM and native results separate: both are target
  deployments. Narrow within-run bands still allow between-run drift;
  alternate baseline and candidate builds when assessing small gains.

## Running it

    cargo run --release -- --all
    cargo run --release -- --quick --all
    cargo run --release -- --contenders blake3-official,blake3-servil-mt

Results are `benchmark-results/{CPU}.{OS}/bench-hashes.result.txt`,
`.graph.svg`, and `.samples.tsv`. In the VM prefix commands with
`HOME=/workspace/vm/home CC=clang-19 TMPDIR=/tmp CARGO_TARGET_DIR=/tmp/target`.
On macOS these environment overrides are unnecessary.

The fork is a git dependency at the commit `Cargo.lock` pins; with
`--config 'patch."https://github.com/johnservil/BLAKE3".blake3-servil.path=".."'`
it is the enclosing checkout (the fork's `tools/perf_regress.py build`
does that in a copy of this repository with a lock of its own, since the
patch changes the lock), and its provenance records that
checkout's commit and working-tree fingerprint. Keep that provenance
with each measurement.

**The sampling schedule, thinned (September 25, 2026).** Every cell samples
in a share of the rounds at its own offset: a steady cell aims at 24
samples of the 96 rounds, one whose median is unsure (a 95% order-statistic
interval wider than 2%, or fewer than 6 samples) at 48; a long cell (one
hash of 4 ms or more) at 8, 16 while unsure. VM, default roster, runs old /
new / new / old: 99 s, 48 s, 48 s, 107 s. Cell medians, |log ratio|, solo:
old against old median 4.3% (90th percentile 7.1%), new against new 1.1%
(3.4%), new against old 1.5% (4.4-4.9%); shared alike (2.6%, 0.9%, 1.0-1.2%);
new medians 0.3-0.6% slower on average, inside the noise; two-speed cells
23-25 against 19-21.

**Thinned again (September 26, 2026):** steady cells 12 samples (24
while unsure), long cells 4 (8); then no "unsure" doubling at all
(steady 12, long 6), below. The between-run variance of a cell's
median barely falls with more samples (0.53% at 12, 0.37% at 24, median
cell; the worst tenth, servil's SME2 cells, 2.7% against 2.8%: set by how
long each run spends in each state), so the extra samples bought little.
VM, default roster, full runs old / new / new / old / old / new: 47.8,
25.2, 25.0, 47.4, 45.5, 24.3 s; cell medians, |log ratio|, solo: old
against old median 3.45% (90th percentile 6.5%), new against new 1.63%
(6.1%), new against old 2.23% (6.5%); shared 3.40%, 2.20%, 2.43%; new
0.6% faster on the median (shorter runs sit less of the VM's warm-up,
NOTES-servil.md); two-speed cells level. A second change to how many
processes a run uses was weighed and left: since the fork's scratch
alignment (servil f38786d) the per-process part is small, and what is
left changes within a process over seconds, which rounds spread over the
run already sample.

Then the doubling went (Zooko asked for a looser 4% threshold, which
saved 1.5 s of 25: on the VM the order-statistic interval of 12 samples
spans the tenth to a third of samples that run 10-14% slow, so most cells
stayed unsure at 4% too). Without it, full default runs old / new / new
/ old / old / new: 25.2, 18.7, 18.7, 25.0, 24.9, 18.9 s; cell medians,
|log ratio|, solo: old against old 1.94% (90th percentile 5.7%), new
against new 1.36% (7.1%), new against old 1.44% (6.2%); shared 2.22%,
1.73%, 1.72%; two-speed cells level; cells marked `~` 108-109 against
124-159.

**Shorter runs sit less of the VM's warm-up** (fork NOTES, "perf_regress"):
the first 47 s `--all` record (servil ad24649) read SHA-256, ring, and
BLAKE3 official 7-8% faster than the 140 s record before it (ee956b7),
servil level (-1 to +0.3% solo). Near-ties on the VM within about 8%
depend on the run's length; Zooko chose to leave it (September 26).
The Mac warms too, less: over 4 minutes of sustained load SHA-256 drifts
about +3-4%, BLAKE3 st about +1% (fork job 329).
Summary, and set aside (Zooko, September 26, 2026): some contenders fare
better than others as the machine warms under sustained load. On the Mac
the P-core clock falls with load, by how much power the running code
draws (SHA-256 4.40 -> 4.35 GHz, servil's integer + NEON code 4.24 ->
4.11 over 2.5 minutes, the SME2 path flat; job 332); cycles per byte stay
constant. Where the clock levels off is unmeasured; no warm-up, no
second record; the rounds give every contender the same exposure.

**Open: a median exactly halfway between two display values.** The Mac
record of servil ad24649 (job 330) has one cell (CommonCrypto, shared, 8
messages) whose exact median is 51.3875 ns/msg: check-report.py, exact,
rounds it half up to 51.388; the benchmark shows 51.387, since each
sample's ns/units becomes Q64.64 rounded down before the median, a
rounding before the page's. Fix: the median of two middle samples as an
exact midpoint of the measured values (Measured), rounded once. Most VM cells stay unsure at 2% and take 48.

## Aligning the five-question table (September 30, 2026)

The benchmark adds `LentMessages`, `LentPieces`, and `LentBatches` at the
owned-buffer continuous axes' sizes, using kept producer buffers and
synchronous calls back to back. Whole messages remain whole beyond
64 KiB; pieces use incremental calls. Both servil contenders take part,
measuring the one-thread and several-threads columns separately.
After-gap servil mt pieces use `update`; `update_multithreaded` belongs
to continuous lent pieces. FROZEN carries the September 28 evening
decision and dispatch contract.

The gap now walks a kept 128 MiB buffer at 64-byte intervals, spends any
remaining part of 1 ms on integer work, then writes the input. Written
counter bytes give the working set physical pages. The complete sweep is
required even when it outlasts 1 ms; preparation follows it and is timed
apart. Hashing and preparation have their own wall time and counts in
`--trace-clocks` (the latter's rows: `preparation solo and shared`). The
trace analyzer keeps preparation apart. The native effect, especially
the small cells' speed split, still requires evidence; a VM alone gives
no per-thread cycles and makes no verdict about the native effect.

The offline HTML guide follows the table, with explicit defaults at each
question and honest energy-pending endings. Its embedded SVG uses the
same run data. Synchronous endings show per-input latency, scaled from
Q64.64 before rounding; queue endings show throughput. Queue::messages
shows sizes up to 64 KiB and Queue::pieces beyond. A missing measurement
has an explicit unmeasured ending. Chromium checks 48 decision-table
combinations, 21 clicked endings, defaults, Back and restart, the actual
visible plots and contenders, and latency/throughput units. Desktop and
mobile renders were read as newcomer, regular, and maintainer. Twelve
Rust tests and the full graph's jsdom check pass. VM default full run:
42.4 s of measurements, under tmp/benchmark-alignment/vm-final.

The regression tool's narrowing exposed an existing graph failure: a
run selecting only a queue cell still selected servil st (which takes
no part), and graph provenance expected a plot for it. Graph eligibility
now requires a measured cell for each selected contender and two points
per shown axis. Sparse runs retain text and samples; dedicated tests
hold these cases. JSON encoding now escapes control characters, including
newlines, so a whole SVG can travel inside the guide's script string;
'<' is escaped there to keep embedded text inside its string.

Zooko asked for a historical sanity check: compare similar tasks for
servil and SHA-256 ring against git's recorded results, distinguish
implementation changes, defects, fixed old measurements, and correctly
different workloads. Native job evidence and the detailed comparison
will follow here and in NEXT-STEPS (its git history, September 30,
2026); each unexplained difference stays
open. The first four native jobs (776-779) failed before measurement,
from installed perf_regress.py's missing speeds.py dependency. The fork's
setup now copies it; native diagnostics use the checkout's tool tonight.

### Historical checks found dispatch and display defects

Mac job 780 ran old/new/new/old on the identical hashing implementation
(fork 9cea065; benchmark 4f29643 versus f3515ab), then replayed a published
record's exact old pair (250a3dc, fork b9ec183). Mains power and High Power
mode throughout; compiler f428d123a matches the historical record. Full
default runs took 49.3-50.0 s of measurements, meeting the time target.

The memory-working gap changes the workload, and its cold costs are
real. At 64 B, servil st's old fast speed was about 61 ns and the new
fast speed about 224 ns; ring moved from about 80 to 202 ns. New small
calls ran around 4.1 GHz versus the old 4.4-4.5, and instructions were
steady (about 11 extra instructions from the prepared-call path); cycles
increased. That establishes a workload effect, and leaves room for a
harness effect. Small cells still split after the sweep: the promised
one-speed outcome remains open, rather than a success claim.

Fresh-process probe/memory-gap (953b9b2, job 781) made the distinction:
48 samples per cell, four separately timed calls per sample after their
own gaps, lengths 64 B/512 B/1 KiB/4 KiB/64 KiB, register work versus
8 MiB and 128 MiB sweeps. 64 B after 128 MiB: one speed at 135 ns, versus
52 ns (85%) / 291 ns (15%) after register work, near 4.47 GHz. 512 B:
417 ns versus 344; 4 KiB: 1396 ns (88%) / 2151 ns (12%) even in this
fresh process, all on P-cores. The full benchmark's 224/406 ns at 64 B
and 2490/3708 ns at 4 KiB therefore contain additional effects. Its
64 B interval also executed about 101 instructions beyond the direct
probe. Whole-message API selection now occurs before the gap and clocks,
so the interval enters one preselected call rather than the dispatch
and assertions for every use case. The native A/B of this fix follows.

The frozen batch call also exposed a dispatch defect: a batch of one
used `hash`, where FROZEN promised `hash_many`. All batch points now use
the batch API, including one message. Servil after-gap batches select
their call and take their kept digest space before the gap. A test
observes the actual selected hash versus hash_many calls, supplementing
the textual contract test.

The independent report check found two numerical defects. Python's
speed splitter used exact Fractions where Rust uses nearest Q64.64 and a
half-up Q64 midpoint; the fork's twin now matches that representation,
held to 13 shared vectors. Separately, an exact decimal halfway could
round down after Q64 approximation (the earlier open 51.3875 cell, and
5.3375 in this session). Each displayed median now retains the original
two middle measured ratios. Integer cross-products form their exact
midpoint, scale it for per-call latency where needed, and round once for
the reader. Comparisons and bootstrap intervals stay in fixed point.
Independent rational anchors hold both halfway cases. The exact checker
now verifies all 964 cells of a default full VM run, including every
new axis. Fourteen Rust tests pass. These changes correct representation
and dispatch, while every stored clock reading remains as measured.

`tools/compare-runs.py` accepts two files for a historical comparison or
four old/new/new/old, prints each speed and share, and puts same-code
repetitions beside a four-run comparison. An explicit --map names
use cases whose workloads deserve comparison. Two-file comparisons make
no claim about repetition. It uses the fork's shared Python rule.

### Job 783 (mains) follows 782 (battery): observed differences, cause unresolved

Job 783 repeated 782 on mains power: same-code repeats within 6%;
preselection made servil's cold 64 B call 18% faster (fast speed 3.91
-> 3.18 ns/B; 782: 3.66 -> 2.51) and batches of 16 11% (28.6 -> 25.5
ns/msg; 782: 27%). Battery power changed nothing measurable with the busy
gap: in jobs 780 (mains) and 782 (battery) 0 of about 3600 after-gap
calls ran on E-cores, and the clock distribution was the same (median
4.04-4.07 GHz, p10 3.32-3.33). The battery finding of September 2026
(233 of 400 calls on E-cores) belonged to the sleeping gap.

Both jobs also show the 1-message batch 17-27% slower on the new side
for servil and ring alike, with same-code repeats within 3-8%. Ring's
code path in that cell is unchanged; the servil cell moved from `hash`
to the frozen `hash_many`. A call of about 200 ns after a 128 MiB sweep
runs from cold instruction caches, so it measures the harness's code
layout beside the hash: any recompile can move such a cell by about
20%. Open: whether to accept that spread as the cold cells' nature and
say so in the report, or warm the harness's own code (never the hash's)
before the call. Until decided, differences under about 25% in cells
below 1 µs after the gap are not evidence about the hash.

### Cold-cell variability experiment (job 788): interpretation superseded

The experiment used bench 805324f and the perturbed c5ac3a3, two
repetitions of each at two requested fork commits. Some repetitions of
a given build varied substantially in small cells. The earlier causal
interpretation (“per-process, not per-binary”) exceeded the evidence;
job 788 establishes variability, with its cause and user relevance open.

At the context-reset audit we verified an additional confound:
perf_regress.py patches clocks to ROOT/clocks. The diagnostic driver
branch is based on 9cea065 and has no warm-up. Consequently the nominal
1820efb/d005716 sides both used that same unwarmed helper. Job 788 gives
no warm-versus-unwarmed measurement. Combining all eight configurations
also overstates evidence about one executable's repeatability. The
NEXT-STEPS block of that day (its git history) recorded the corrected
scope and next experiment; it supersedes prior causal statements here.

**Archaeology (jobs 785-787):** v0.1.0, v0.2.0, and v0.3.0 each against
today's fork under today's benchmark: no slower solo cell for one
message, pieces, or batches (one shared two-speed cell, mt 256 KiB,
against v0.2.0). The published Mac records since September 25 (calls
back to back) show servil st tying ring at 4 KiB (0.29-0.33 against
0.29-0.31 ns/B), losing 2x at 1 KiB, winning from 16 KiB; today's
nonstop cells reproduce that. The "now and then" crossing at 32 KiB is
the cold-cache pattern's, for code that has not changed in effect:
servil's cold cost between 4 and 32 KiB exceeds ring's, an open
finding about servil rather than a regression. perf_regress judges
those cells at a 20% margin, so a loss under 20% would pass it.

**A 1-message batch** is `hash_many` called with one message: the batch
function's fixed cost at the start of its axis (a Merkle layer of one
node). It stays as the axis's first point and is the cell most exposed
to the cold spread above.


## Context-reset audit: scope of reliability and regression claims

Zooko's current question: do the benchmark's large swings describe users'
typical operation? This is unestablished. A direct-call probe demonstrates
a cost under the 128 MiB sweep; that workload's typicality and the causes
of the large swings remain questions. Cache, TLB, code placement, memory
placement, and scheduler state are hypotheses to isolate. Claims about
“ordinary noise”, “the cause is process layout”, and a universal 5% bound
were premature. The guide currently omits confidence bands and any
between-run uncertainty evidence. Its precise-looking lines therefore
need review alongside diagnosis.

Zooko rejected the multi-process aggregation proposal. Next work holds
the implementation fixed and compares representative direct callers with
the harness while varying documented work between calls, one factor at
a time. It establishes user relevance and cause before remedy. The
archaeology jobs' passed checks rule out only held regressions in their
sampled, shim-compatible cells at the stated margins (20% after the gap);
they do not prove all historical APIs equivalent. Historical back-to-back
records and current cold-cache cells differ in workload.

The fork's latest timing-helper warm-up (d005716) is committed and pinned,
but its native effect needs an experiment that actually varies clocks.
General regression comparisons intentionally share current clocks across
sides: keep that purpose separate from clocks A/B experiments. NEXT-STEPS's
git history (September 30, 2026) holds that day's branches, raw evidence,
and decisions.

### Cold calls: the harness doubles them (jobs 789-791, September 30, evening)

A direct-call probe (fork `probe/caller-relevance`, host_lab) times the
benchmark's own calls with the same producer and the same clocks function
(`measure_after_gaps_prepared`, 128 MiB), after seven kinds of caller
work, in fresh processes, each beside a benchmark run of the same cells
(servil st; ring beside it). Mac, mains, every run quiet by clocks::load
(0.12-0.23 CPUs). Median ns/call, cycles/call in brackets:

| cell | probe, busy 1 ms | probe, 128 MiB | benchmark | benchmark without shared copies |
|---|---|---|---|---|
| 64 B | 42-62 | 104-136 [990-1110] | 203-229 [1450-1540] | 155-161; 229-274 with HB_ADDRS |
| 4 KiB | 1300-1370 | 1450-1640 [7200-7800] | 3094-3230, and 5188 in one process [14400-23300] | 3052-3219; 5208-5791 with HB_ADDRS |
| 16 KiB | 3690-3770 | 3920-4150 [13500-14200] | 8580-8770 [29200-29500] | 7500-7520; 8812-9125 with HB_ADDRS |
| 64 KiB | 13600 | 13800-13930 | 14850-14960 | 14540-14560 |

Findings. Instructions per call are identical in the probe and the
benchmark (3080 at 64 B, 47,606-47,626 at 4 KiB), and so is the clock
(about 4.5 GHz; 3.3 on the SME2 path), so the benchmark's extra time is
stall cycles in the same instructions. Four processes of the probe agree
within 5% per cell. The benchmark's per-process state moves 4 KiB by 1.65x
(3146 against 5188 ns, same build, one speed each). Removing the shared
copies (probe/harness-bisect `HB_NO_DUO`) takes away about two thirds of
the excess over the probe at 64 B-1 KiB, a quarter at 16 KiB, and none at
4 KiB. One call a sample
(each after another cell's call) and a branchy sort after the sweep cost
the probe at most about 20%. Appending one line to a file before each
sample's gap (`HB_ADDRS`: open, write, close, then the 1 ms gap and the
sweep) moved 4 KiB from 3.1 to 5.2-5.8 us in all eight processes. So what
the program does before the gap reaches through the 128 MiB sweep, and
a sweep does not make the cold call reproducible. The mechanism is open:
which state survives a 128 MiB read sweep (the system-level cache, the
predictors, the kernel's work after a syscall, thread placement)?
Buffer addresses show no pattern (produced page-aligned in every process).

User relevance. Real programs do system calls and other work between
hashes, so the benchmark's slow state may well be what users meet, and the
probe's fast one what a tight probe meets; neither is established as
typical. The cold cells' spread is the harness's state and not
measurement noise: until the mechanism is known, differences under about
2x in cold cells from 4 to 16 KiB say nothing about the hash.
VM (no cycle counts): the same pattern (probe 4 KiB 2359 ns, benchmark
5729; without shared copies 3146-3250).

### The cause: where the hash's code is (jobs 792-798, September 30, evening)

Changing one thing at a time (fork `probe/caller-relevance`, benchmark
`probe/harness-bisect`; Mac, mains, quiet by clocks::load; three probe
processes agree within about 5%, and user-interactive QoS changes
nothing) located the benchmark's cold-call excess in the hash's
instruction lines. Probe, servil `hash`, median ns/call:

| before the call | 64 B | 4 KiB | 16 KiB |
|---|---|---|---|
| 128 MiB sweep (base) | 104-167 | 1520-1800 | 3910-4090 |
| open and close /dev/null, then the sweep | 230-410 | 2610-2960 | 5740-6180 |
| the text's icache lines invalidated, then the sweep | 375-583 | 3570-3820 | 7100-7480 |
| 10 ms of register work, then the sweep | 136-172 | 1980-2330 | 4700-5160 |
| sleep 1 ms, then the sweep | 333-438 | 3390-3490 | 6730-6850 |
| the sweep, then the text read as data, line by line | 52-94 | 1375-1510 | 3700-3920 |
| the sweep, then one call on another buffer | 42-63 | 1234-1300 | 3740-3750 |
| icache invalidated, no sweep | 146-156 | 1690-1710 | 4250-4280 |

getpid, a write to an open /dev/null, fstat, dup and close, a yield, a
small String, a fresh 1 MiB heap block, and reading the text's pages one
byte each (warm translations, cold lines) change little. Instructions
per call and the clock stay the same throughout; only stall cycles move.
The thread stays on its CPU in 90-99% of calls; a moved call is slow,
and the stayed calls after open-close are slow too.

The mechanism: a 128 MiB data sweep empties L2 and the system-level
cache but leaves the core's L1 instruction cache, so in the probe the
hash's code survives next to the core. Whatever else runs on the core
between calls (kernel code for open(), interrupts over longer gaps, a
core idling in sleep, the harness's own code, a move to another core)
takes those lines, and the call then fetches its code from DRAM.

The benchmark confirms it: with its text read as data after each sweep
(`HB_CODE_LINES`), servil's cold cells fall to 64 B 65-68 ns, 4 KiB
1.59-1.72 us, 16 KiB 4.2-4.3 us in all four processes, each at one speed,
with or without the shared copies (as it is: 190-302, 3083-3094,
8667-8688; job 798). SHA-256 ring moves little (4 KiB 1448-1542 ->
1333-1437, 16 KiB 5083-5542 -> 4875-5417): its code is small. So in the
benchmark's cold cells ring beats servil at 4 and 16 KiB because
servil's code comes from DRAM; with its code near, servil ties at 4 KiB
and wins at 16 KiB. The VM agrees (servil 4 KiB 5916 -> 2083 ns, 16 KiB
9958 -> 4583). The per-process swing and the shared copies' share are
the same mechanism: how much of servil's code the harness's other work
leaves in the instruction cache.

Left open: the benchmark with its code warmed stays 10-20% above the
probe at 4 KiB (1.6-1.7 against 1.38-1.44 us).

### The new benchmark against the previous one and against v0.7.0 (jobs 799-804, September 30)

Fixed contenders: servil at hashing source 5cfa2b4 (fork b06c074, and
d005716 for the previous benchmark's clocks), crates.io blake3 1.8.7,
ring 0.17.14, sha1-checked 0.10.0, sha3 0.11 (not in v0.7.0). Mac, mains,
every run quiet by its load detector; order previous, new, tagged,
tagged, new, previous; full default runs of six contenders (tagged: five).
The tag (bench-hashes v0.7.0, 86b5c5b) ran as `probe/v0.7.0-on-current-fork`
(8a3006a): its batch calls in today's form with its slice vector still
built, today's key for servil st, dot shapes repeating: nothing else.
Its samples (v2) and the previous (v3) were read with the fork's d005716
loader, the new (v4) with tools/samples.py, all through tools/speeds.py.
Scripts and outputs: `/workspace/tmp/cmp/` (part1-part5).

Same intention, previous and new (nonstop owned and lent cells): runs
new 1, new 2, and previous 2 agree within 1% (median over each
contender's 29-47 solo cells: -3 to +6 permille against new 1). The
first run of the series (799, previous 1) read 5-7% faster for the
contenders on the cores (SHA-1DC, ring, SHA3, crates.io BLAKE3) and level
for servil (SME2): machine state at the start of a series after the Mac
had idled, most likely the P-cores' clock. Open: confirm with cycles (a
traced first job after an idle Mac); until then, the first job of a
series is not evidence against the rest.

Different intention, previous and new (calls amid other work): from 1 MiB
every contender agrees within 1-3%; below, all are slower, by their code
size (64 B: servil st 2.5 -> 10-11.5 ns/B, crates.io BLAKE3 3.5-3.9 ->
10.4-12, ring 2.9-3.4 -> 7.3-7.5, SHA3 4.4-4.6 -> 7.2, SHA-1DC 10.5-11.3
-> 39-40), as the other program intends. After idling: two speeds in
most small cells (a fast one near the busy gap's, a slow one 2-4x),
15-25% slower at 1 MiB (the clock after a sleep), level from 8 MiB.

The tag measured back to back without a read (one message; batches).
Against the lent cells (back to back, each input read first): large
inputs pay the read's copy, about 0.012-0.019 ns/B for every contender
(64 MiB); small ones about 4-9 ns a message (64 B). servil at 64-256 KiB
pays much more for the copy (64 KiB 0.167 -> 0.227 ns/B; ring 0.293 ->
0.308): a hypothesis to test, that SME2 reading freshly written lines
costs more than the cores do. crates.io BLAKE3's batches halve (46.6 ->
23.8 ns/msg): the tag called it once per message, today's benchmark
through Platform::hash_many. Against the calls amid other work, the
single-threaded contenders agree within about 3% from 8 MiB; servil mt
is slower after a gap at 1-8 MiB (1 MiB 0.068 -> 0.092-0.125 ns/B) where
back to back kept its workers awake, level by 128 MiB.

### Shared after a gap (jobs 808-809, September 30)

After either gap, a shared copy of a small call read 10-50% faster than
the solo call, for every contender (ring, SHA-1DC too). Not two copies
of one code warming each other at once: run one after the other
(`HB_DUO_SERIAL`, probe/shared-after-gap), the copies stayed faster
(servil 4 KiB after other work: solo 5.17 us, copies 4.1-4.3; ring 1 KiB
652 against 486-500 ns), with the same instructions and clock, only
more stall cycles solo. Reversing each interval's order (shared first,
`HB_SHARED_FIRST`) reversed the advantage: servil 4 KiB after idling,
solo 2.0-2.1 us (fast speed) against the copies' 5.5-7.0. So whichever
sample ran second found the code the first had just run, through the
sleep (in the cores' caches) and even through the other program and the
128 MiB walk (most likely the system-level cache). A timed call's cost
depended on the benchmark's schedule: small cells' later calls followed
the same call, large cells' single call another cell, a copy its own
solo sample.

The fix (Zooko, September 30): each timed call follows the same call,
one untimed call before a sample's first (clocks::measure_after_gaps_prepared);
the shared scenario measures the nonstop use cases alone. The walk and
the other program stay as the busy program's other work, no longer as a
way to erase what ran before. Each gap cell now takes the full 12
samples; a full VM run takes 64 s (was 45).

### Consistency checks replace CHECKS (September 30, late)

Zooko: servil-only checks do not belong in bench-hashes (improving the
benchmark and improving BLAKE3 are separate projects). Removed: the
report's CHECKS and TWO SPEEDS sections (servil slower than others, mt
against st, two-speed servil cells; the fork's tools/losses.py keeps the
to-do list), the clock-state split they used, the per-sample rounds and
clocks kept for them, the samples file's per-call clock lines
(--trace-clocks has the counts), AFTER_GAP_DIVISOR (1), Scenario::PLOTTED
(Scenario::ALL). main.rs 7927 -> about 7530 lines.

Added: `bench-hashes.checks.txt`, relations every contender keeps
(METHODOLOGY, "Consistency checks"), with a test that each fires. First
VM findings: servil mt's lent pieces faster shared than solo at 256 KiB
and 1 MiB (two copies keep the pool's lingering workers awake?);
servil's batches of 256 slower per message than of 64 (17-24%); SHA-256
after idling 11-21% slower than after other work at 8-32 MiB (VM); in
one process of four, ring's nonstop batches 30% slower solo than shared
(54 against 42 ns/msg; the same executable read 42 in the others): a
per-process state, open. Check 5 at first compared work past the caches
(each level of the memory hierarchy costs more per byte, for every
contender), and fast speeds of idle cells (different clock states):
narrowed to 32 KiB, outside the idle use cases.

Evidence of no change in what is measured (VM, full default runs,
`tmp/simplify/`): interleaved before, after, after, before, new/old
median over cells: after other work -0.1%, after idling +0.3%, nonstop
+0.05%, each inside its same-code spread (medians 3.1-3.6%, 3.0-3.6%,
1.0-1.1%). Two sequential runs of each first read the gap cells 1-2%
slower: drift, gone when interleaved.

### Each way of calling in its own phase (jobs 818-823, September 30, late)

The first consistency check ("after idling agrees with after other work
from 8 MiB") fired on both machines for SHA-256: after idling 11-22%
slower at 8-32 MiB. Traced (job 819): the same calls' core clock, 3.76-4.05
GHz after idling against 4.48 after other work, all on P-cores. Measured
alone with busy neighbours (job 818) they agreed (ring 32 MiB 0.291
against 0.289). So the idle cells' clock followed their neighbours: in a
full run a large idle cell samples every eighth round among hundreds of
sleep-dominated calls, and macOS sets a core's clock from its recent use.

The one mechanism that already existed for this (the nonstop cells in a
phase of their own) now covers every pattern: three phases, nonstop,
after other work, after idling. Interleaved Mac A/B (820-823, before
8a0459d, after 4b1c1c9): the idle large calls read slower, as a mostly
idle program meets them (ring 8 MiB 0.31-0.32 -> 0.43-0.45 ns/B; servil
st 8 MiB 0.155 -> 0.19), repeatably (821/822). The check was a wrong
expectation, not a bug: removed; METHODOLOGY states the effect.

### Release readiness, first pass (September 30, late night; jobs 824-835)

- CI (`.github/workflows/ci.yml`): build, test, quick run on Linux x86-64
  and arm64, macOS, Windows. It found two defects: `target-cpu=native`
  (in `.cargo/config.toml`) broke ring's build on GitHub's macOS VM (its
  native CPU lacks features every Apple arm64 has); Windows checkouts
  turned FROZEN.md to CRLF and failed its test (`.gitattributes` now
  keeps LF). The Linux jobs' quick runs had not finished at the handover.
- No native: Mac interleaved (828-831) every contender about 0.5% slower
  nonstop and 0.2-1.2% after other work, alike (the harness's code);
  comparisons unchanged. Builds are now for the target's generic CPU.
- Memory: a buffer per point held 3.3 GB; one per size 1.0 GB (Mac A/B
  824-827: nonstop +0.1%, after other work +0.4%, after idling -0.15%);
  one buffer for every point, each a prefix, 0.87 GB (832-835: +0.06%,
  +0.7%, -1.6%, each inside its same-code spread; a test pins the
  prefix property).

### A message in pieces: one long message, nonstop (October 1)

Zooko's decision (FROZEN.md): the pieces sweeps after other work and
after idling go, and nonstop lent pieces keep one point, 64 MiB.
Evidence (Mac jobs 829-830, nonstop, solo, ns/B): a message of up to 64
KiB is one piece and reads as the one-message cell (256 B-64 KiB within
+-3%, 64 KiB within 0.5%; 64 B: the Hasher's fixed cost, servil +16-17%,
SHA-256 -6 to -12%). A long message on one thread costs 3-8% above the
64 KiB one-message cell (servil st 0.235-0.245 against 0.228; ring
0.309-0.319 against 0.302). servil mt's `update_multithreaded` sustains
0.099-0.121 from 256 KiB to 64 MiB, a rate no one-message cell predicts
(0.228 at 64 KiB, 0.036 for one 64 MiB buffer): the 64 MiB point keeps
it. After a gap the sweep took a third of each gap phase; a full VM run
now takes about 40 s (was 64 s).

Analysis for other machines: the pieces are 64 KiB, inside every
current core's own cache, so on one thread their cost follows from the
64 KiB one-message cell anywhere; the only machine-independent unknown
is whether an incremental API spreads a long message over threads, which
one long message shows.

What the cut loses, stated: the VM finding "servil mt's lent pieces
faster shared than solo at 256 KiB-4 MiB" is no longer measured (at 64
MiB the VM reads shared 17% slower, as expected; the Mac 3% apart), and
perf_regress no longer covers lingering's ramp (pieces of a 256 KiB-4
MiB message). Both are the fork's to probe.

The graph draws a plot of one point in its middle, whatever the zoom
(`x_fraction`, `windowFor`; check.js holds it); the guide shows that
cell for nonstop pieces, and for pieces now and then shows `hash`'s
cells, labelled by piece length ("each piece costs about what hash costs
on one buffer of the piece's length"). Its chart's range now reaches a
gridline at each end, so a chart of close values has numbers on its axis.

Kept after review (Zooko, October 1): the 27 one-message sizes and 24
batch counts (engineers come for the number at their size and for where
contenders cross); both pause kinds; shared; servil mt below its split
(where it starts using threads depends on the machine, and the full
sweep is what would show a change there).

### Devon Jonte's audit (October 1, 2026)

Devon Jonte (github.com/devonjonte, optimising BLAKE3 for x86-64)
reviewed 80cd052 on an i7-12700K under Linux and sent his findings as a
branch of his fork and as PR #2 (`candidate/devon-harness-fixes`, with an
AUDIT.md). His three commits are in our history as he wrote them
(e793c78, e80a4e3, ddce746); the follow-up commit adjusts them and folds
AUDIT.md into this section. What he found, and what became of it:

Defects, fixed:
- **Allocation inside continuous samples.** The fork's queue of batches
  paired its kept buffers and digest spaces anew in every sample (a
  zip, a collect, an unzip: three vectors inside the timed interval).
  The pairs are now kept together (`BATCH_PAIRS`). His test of it (a
  counting allocator around a warmed producer) failed about one run in
  ten on correct code: the fork's queue adds a block of slots whenever
  more submissions wait undelivered than ever before, which the delivery
  thread's timing decides, so a sample can meet the queue's growth (at
  most a block per 16 of the program's buffers over the queue's life,
  microseconds in a millisecond sample). No warm-up controls it, so the
  test is gone (AGENTS.md, "Every piece earns its place").
- **Zeroing inside samples.** A smaller batch truncated the kept digest
  space and the next larger one zeroed its tail inside its sample; the
  space now keeps its length and each call takes a prefix.
- **Stale graph and guide.** A sparse run replaced the samples and
  report and left an older run's SVG and HTML beside them; it now
  removes both.
- **The guide's sentence.** Faster at middle sizes and slower at the
  last read "Slower at every size"; ties read as losses. The sentence now
  says "Slower at every size" only when it is, "The two trade places"
  for a mix, and "As fast as ... or slower" when it never leads.
- **The guide's medians** rounded the Q64.64 approximation where the
  report rounds the exact midpoint (2135/400: 5.337 against 5.338); both
  use the exact one now.
- **The report check** passed a report with its whole shared section
  removed; it now requires every sampled cell once (`check-report.py`,
  with tests in `test-check-report.py`), and takes `--rules` for a
  standalone checkout.
- **Contender order.** Long cells (one hash of 4 ms or more) sampled at
  every second visit and met two of the four orders; and a whole-roster
  design filtered to a use case's contenders lost its balance. Each
  point's design is now built over the contenders that take part, every
  one samples at every visit, and a point takes whole cycles of orders
  (12 samples or more). The long-cell budget (`LONG_HASH_NS`,
  `LONG_SAMPLES`) is gone. Cost: a full default VM run 39 -> 48 s.
  Tests check the realized orders for every roster prefix, use case,
  round count, and offset.
- **Shared samples' load windows**: each copy records its own start.
- **The guide's chips** could show `update_multithreaded` cells under an
  `update` recommendation and the converse; the downward-triangle mark
  had another name in Rust than in the guide's script.
- **Trace counts** where the platform gives none (Linux) were written as
  zeros; they are empty now, and the trace reader says so.
- And a test of the official crate's batch wrapper against separate
  `blake3::hash` calls (flags, slicing, order).

In the fork: **the queue hung on one CPU** without SME2 (`taskset -c
0`): fixed (fork NOTES, "The queue on one CPU").

Open, as he left them, with our reading:
- **Shared copies in the bootstrap.** A shared sample's two copies run
  at once and may be correlated, while the bootstrap resamples them as
  independent: shared cells' intervals (the report's `~`, the graph's
  bands) may read narrower than they are. The remedy is the shared rule's
  (clocks::speeds and tools/speeds.py, with their vectors): resample
  rounds, both copies together.
- **The provenance fingerprint** includes untracked files, which the
  build script does not watch outside `src/`: a stale dirty fingerprint
  is possible, with the build itself unchanged. Small.
- **Kernel labels** of the queue and the multithreaded incremental API
  name the one-shot paths; they may omit the helper threads. To check.
- **Consistency checks**: per-byte speed need not be monotonic across
  block, SIMD, tree, or wake boundaries, so a finding needs explaining
  before it is called a contender's bug (METHODOLOGY already says so).
- **SHA-256 under target-cpu=native** on his x86 ran 100x slower (VEX
  instructions interleaved with SHA-NI, an AVX/SSE transition): the
  generic build we use is right; never publish native-build SHA-256.
