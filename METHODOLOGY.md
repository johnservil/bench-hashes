# How bench-hashes measures

This file explains what a run measures, how it controls measurement
noise, and what each contender runs. [README.md](README.md) says how to
run it.

Every run measures each contender in twelve use cases. Four are calls
made now and then, each after a gap of one of two kinds. **After other
work** runs after the program has executed a fixed other program and read
128 MiB of data, as a program hashes between its other tasks. **After
idling** runs after the program has slept 1 ms, as a server waits for its
next request. Each kind measures **a message in one buffer** at
twenty-eight sizes from 64 B to 128 MiB (reported per byte) and **a
batch** of 64-byte messages at twenty-four counts from 1 to 262144
(reported per message).

Eight hash one input after another, as fast as the program can, each
input first read or written into a buffer (timed with the hashing):

- With buffers the program **owns** and hands over, filling the next
  while one is hashed: **messages**, at eleven sizes from 64 B to
  64 MiB, and **batches**, of 16 to 65536 messages.
- With buffers it **lends** to a call until the call returns, so reading
  and hashing take turns: messages and batches at the same sizes, and
  **64 MiB messages in 64 KiB pieces**, through each contender's
  incremental API.
- **Many messages at once, arriving in pieces** (a server's uploads):
  256 messages open at once, their pieces arriving in turns of 64, each
  in its own buffer.
- **A collection** of 2048 items of every size, each hashed once, all in
  memory, as git, Nix, and content-addressed stores name their objects:
  two collections in the shares of real ones (git's objects, and the
  files of Nix store paths).
- **A message with its outboard**, for verified streaming: messages of
  1 MiB and 64 MiB hashed with the tree's parent nodes above 16 KiB
  groups, as iroh-blobs stores them.
- **A message received in that form**, verified as it arrives.

FROZEN.md lists each contender's calls in each use case.

Each nonstop use case runs in two scenarios: **solo**, one copy of the
contender, and **shared**, two copies at once. The calls after a gap run
solo. The report shows each use case once per scenario, solo first; the
map shows both.

## Contenders

A default run measures BLAKE3 servil, multithreaded (servil mt) and
single-threaded (servil st), and SHA-256 from two crates, sha2 and ring,
since each is the faster SHA-256 at some sizes. `--all` adds every other
contender the machine can run: the crates.io BLAKE3 crate (BLAKE3
official), on one thread and on its Rayon pool (`blake3-official-mt`),
SHA3-256 (the `sha3` crate, with the CPU's SHA-3 instructions
where it has them), SHA-1DC (`sha1dc`, SHA-1 with the collision detection
git uses, left out of `--quick --all`), and CommonCrypto's SHA-256 on
Apple platforms (`sha256-cc`). `--contenders` names any set. `--list`
shows every key.

## Input sizes

The one-message axis tests every power-of-two input size from 64 B to
128 MiB except 16 MiB, plus 1536 B, 3 KiB, and 3 MiB. It also tests four sizes of
real data between 2 and 8 KiB: 2304 B (an 802.11 frame body), 3839 and
7935 B (802.11n A-MSDU maxima), and 4470 B (Packet over SONET/SDH MTU). Real inputs are
seldom multiples of BLAKE3's 1 KiB chunk, and those four sizes test
non-chunk lengths.

Below 1 KiB a BLAKE3 input is one chunk. From 2 KiB to 16 KiB its SIMD
paths fill (4-way NEON at 4 KiB, a sixteen-lane SME2 group at 16 KiB),
and above 16 KiB the single-thread bulk rate settles. 3 KiB is where
`blake3-servil`'s integer + NEON hybrid kernels first overtake hardware
SHA-256. The sizes past 1 MiB show the plateau: a contender whose 32, 64,
and 128 MiB means agree has levelled out. Multithreaded contenders reach that plateau later because thread
hand-offs and subtree merges amortise over larger inputs. From 8 MiB up
an input exceeds most machines' last-level cache, so memory bandwidth
governs the rate. 3 MiB tests a
tree that is not a power of two (a 2 MiB left subtree beside a 1 MiB
right one).

The report gives each cell's mean time per unit in nanoseconds (the
total time of its samples over the total work they did, what a caller
pays on average), to
three decimals or three significant digits, whichever shows more; lower
is better. Each sample is kept as measured, the clock's nanoseconds over
the units they covered, and the statistics work on it in fixed point with
64 fractional bits, rounding only for the page. Every time is wall
time on the platform's hardware counter (`CLOCK_UPTIME_RAW` on Darwin,
`CLOCK_MONOTONIC` on Linux, via `std::time::Instant`), so a throttled
clock, a busy SME unit, or a GPU's latency counts as the user would
feel it.

## Many messages at once

A server receives many messages from its connections at once, each in
pieces, and hashes each as its pieces arrive. This use case measures
that on a fixed schedule (`INTERLEAVED`), the same in every run: 256
messages open at once, the pieces dealt to them in turn. Most pieces are
one TCP segment's payload (1448 B), with a page-sized read (4 KiB) and a
TLS record (16 KiB) among them, in a fixed cycle; each is cut at its
message's end. Message lengths cycle through 64 B, 1000 B, 4470 B, 16
KiB, 100,000 B, 1 MiB, and 16 MiB: many short, a few long. When a
message ends, its hash is used and the next opens in its place, and the
schedule carries over from sample to sample, so samples see the steady
state. Each piece is read, timed, as a memory copy into a buffer of the
program's, then handed to its message's incremental hasher
(`Hasher::update` in crates.io BLAKE3 and in BLAKE3 servil,
`update_rayon` for BLAKE3 official mt, `update_multithreaded` for
BLAKE3 servil mt, `Digest::update` in sha2, sha1-checked, and sha3,
ring's `Context::update`, CommonCrypto's `CC_SHA256_Update`). The time
is per byte of pieces.

A single long message arriving in pieces is not measured: each piece
lent to an incremental call costs about what one message of the piece's
length costs, which the one-message cells show.

## One input after another

A program that hashes many files, records, or network objects, or the
layers of a Merkle tree as they arrive, hands a hash one input after
another. The owned-buffer continuous use cases measure that: each input is read,
timed, as a memory copy into a buffer of the program's, then hashed, and
a sample covers many inputs (at least twice the buffers in flight, below),
timed from the first read to the last digest. Every contender but BLAKE3
servil mt hashes each input after reading it, so reading and hashing take
turns: a message of up to 64 KiB through its one-shot call, a longer one
through its incremental API per 64 KiB piece, a batch as the batch use
case hashes it. BLAKE3 servil mt takes them through the fork's stream,
built for throughput: each message read straight into the stream's ring
(`submit_with`), each batch likewise (`submit_batch_with`), their digests
into places the program keeps: 1024 for messages, and for batches enough
places of a batch's size that the ring fills first. The stream hands each
place back, its digests in it, to its handler on the program's thread,
which uses the digests and frees the place for another input, so reading
and hashing overlap. The places are made once and kept, and the stream's
storage is the thread's from its first stream on, so after warm-up neither
the program nor the stream allocates. The servil single-threaded contender measures continuous load in the
lent-buffer tasks, where its synchronous calls serve the one-thread
column of the API plan.

## A batch

A program with a queue of small messages to hash (a Merkle tree's
nodes, a table of records) has two ways to spend a call: one message per
call of the plain entry point, or a batch per call where the
implementation offers that. The many-messages use case measures both as
they are, with messages of 64 bytes (a Merkle tree's inner node, two
32-byte children). A contender without a batch entry point loops its
plain entry point over the batch, one message per call: `for m in batch
{ hash(m) }`. Three have one. BLAKE3 servil's `hash_many(input, message_len, out)`
takes messages of one length back to back in one buffer and fills one
digest each; BLAKE3 servil mt's `hash_many_multithreaded` does the same
over the fork's worker threads. The crates.io BLAKE3 crate has a hidden
one, `blake3::platform::Platform::hash_many::<N>`, which programs that
want its batch speed call directly (WHIR's Merkle trees do): the bencher
calls it as they do, sixteen messages per call, with the flags that make
each digest the message's hash. A batch of one message is one call of
the batch entry point where a contender has one, else of its plain one.

The axis counts messages per batch: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32,
48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
131072, 262144 (16 MiB of input at the end). Powers of two up to 16 show a SIMD batch filling (four blocks at a time
on NEON, sixteen with AVX-512 or SME2). Counts of 3, 6, 12, 24, and 48
leave a group partly filled or leave a remainder past sixteen-message
groups. From 64 up the per-batch overhead amortises and the rate settles.
Results read in nanoseconds per message and million messages per second.

BLAKE3 official mt takes no part in this use case: a 64-byte message is
a call to `update_rayon` that no program would make, and the
crate has no multithreaded batch entry point. The bencher writes no wrapper of its own around any
contender; the contenders' own entry points are the whole of what it
calls.

Both scenarios apply unchanged: in the shared one each copy hashes its
own batch.

## Solo and shared, and the gap

A reader of these results wants to compare contenders on a load pattern,
to spot a regression, or to estimate speed in a system they are
designing. Each needs a few numbers per contender, so every sample
interval takes two samples of the same batch:

- **Solo**: one copy of the contender on one thread, no other program
  running at the same time. What a program gets with the machine to
  itself.
- **Shared**, for the tasks that hash one input after another: two
  independent copies at once, each on its own thread over its own input,
  released together; each copy's own time is a sample. They compete for
  cores, memory bandwidth, and any shared execution unit (such as Apple
  M4's SME unit). The calls after a gap run solo, since a second copy
  would warm the cache the gap was meant to cool.

In the calls after a gap, each sample is one call (or enough calls to
fill 2 µs when a single call is shorter than the clock tick), each
preceded by its own 1 ms gap and timed outside the gap. Before a sample's
first timed call, one untimed call with its own gap runs so the sample
starts from that cell's own state. After other work, the thread executes
1024 generated functions (about 1.1 MiB of machine code) to evict the
instruction cache, walks a 128 MiB working buffer at 64-byte intervals,
and spends any remaining millisecond on integer arithmetic. After idling,
the thread sleeps 1 ms. In both cases the thread then writes the input
into its buffer and hashes it, storing the digest in its output slot.

In both gap kinds, any worker pool's threads have gone to sleep. After
other work, the calling core is at full clock with its caches holding the
other program's code and data. After idling, the operating system has often lowered the core's clock or
moved the thread. On an Apple M4 Max, 1 ms of sleep lowers the clock
until roughly 85 µs of work raises it again, and multi-millisecond calls
run near 3.8–4.0 GHz after idling against 4.5 GHz after other work. The cell's mean weighs these
states by how often they occur.

A single-threaded hash costs about the same in both. A multithreaded one
shows in the shared scenario what its threads cost when the machine is
shared; an SME2 kernel shows what sharing its unit costs.

## Hash implementations

The `BLAKE3 official` and `BLAKE3 servil st` contenders call the one-shot `hash`
function, which is single-threaded on every platform.
BLAKE3 may still use SIMD parallelism within the calling thread; that
is single-threaded execution, not operating-system-level
multithreading.

The blake3 crate is built with its `rayon` feature so that the
`BLAKE3 official mt` contender can call `Hasher::update_rayon`; that feature
adds the method and leaves `blake3::hash` and every other API
single-threaded.

SHA-256 is provided by RustCrypto's sha2 crate (0.11), whose built-in
backends use the ARMv8 SHA-256 instructions on AArch64 and SHA-NI on
x86, selected at runtime; other targets use its portable code.

SHA-256 ring is provided by the ring crate: BoringSSL's assembly,
which interleaves the next block's message schedule with the current
block's rounds. That pipelining wins about 13% per byte over sha2's
straightforward per-block loop on Apple silicon, and costs a few
nanoseconds of setup that sha2 wins back on inputs of one or two
blocks. The two kernels are the two sides of one design trade-off, so
the crossover near 128–256 B is structural.

BLAKE3 servil is the same crate from the `servil` branch of
github.com/johnservil/BLAKE3, at the commit `Cargo.lock` pins, under
the crate name `blake3-servil` so it links beside the crates.io crate.
On AArch64 Linux and macOS the build includes the SME2 kernel when the
C compiler assembles SME2 (Clang 17 or later, Xcode 15 or later, or GCC
14 or later with binutils 2.41), and leaves it out with a warning
otherwise; only CPUs with SME2 run it. At run time the
fork reads the CPU: one that reports SME2 with a 512-bit streaming
vector length gets the SME2 group kernel for sixteen chunks and up;
every AArch64 core runs the scalar and integer + NEON hybrid kernels.
On x86-64 it runs the crates.io crate's assembly kernels (SSE4.1, AVX2,
AVX-512) and kernels of its own: passes that hash whole chunks and a
partial one together, and batches of messages of two to fifteen chunks
hashed across the messages. Its provenance line gives the repository,
branch, and commit. For a batch the fork's `hash_many`
hashes messages of one to sixteen blocks many lanes at a time (sixteen
per group on SME2, NEON below a group).

BLAKE3 official mt is the crates.io crate's own multithreading, called as a
program calls it by default: `Hasher::new().update_rayon(input)` on
Rayon's global pool, which Rayon sizes to one thread per logical CPU.
The method splits the tree recursively with `rayon::join` down to the
SIMD degree, so any input above one SIMD width of chunks may cross
threads, and idle pool threads steal the halves. The two shared copies are
two callers in one process sharing that one pool, the same situation
the servil fork's fair sharing addresses, so the two multithreaded
columns compare like for like.

BLAKE3 servil mt is the fork's `performance::hash_multithreaded`,
which returns the same hash as `convenience::hash`. Inputs below
its split (512 KiB) stay on the calling thread.
Larger inputs can split at subtree
boundaries across the calling thread and worker threads the fork starts
once per process and keeps; the caller merges the chaining values. How
many threads a call uses is the fork's decision from the input and the
machine, and concurrent callers in one process share the workers
fairly: two callers at once each get about half the machine. Across
processes the operating system's scheduler shares the workers' CPUs.

The benchmark touches each implementation only by listing it and calling
the entry points its users call, with no cap, pool, or wrapper of its
own. Those entry points are the one-message call (`hash`,
`hash_multithreaded`, `Hasher::update_rayon`, `digest`), the batch call
where one exists (`hash_many`, `hash_many_multithreaded`, `Platform::hash_many`),
the incremental API (`update`, `update_multithreaded`), and for servil mt
its `stream`. It asks for no machine capacity, sets no environment, and
checks no digests.

SHA-256 CommonCrypto, on Apple platforms only, calls the system's
libSystem through FFI using `CC_SHA256_Init`, `CC_SHA256_Update`, and
`CC_SHA256_Final`. This is the implementation most Apple software
reaches for, so it anchors the sha2 crate's number against the
platform's own. Its provenance is the running OS.

The three-call form is the fastest route into corecrypto. Measured on
an M4 Max, a 64-byte digest takes 51 ns through Init/Update/Final and
182 ns through the one-shot `CC_SHA256()`, whose finalisation spends
about 110 ns per compression; bulk throughput is identical on both.
Callers hashing small inputs through CommonCrypto gain most from the
streaming calls.

SHA-1DC is provided by RustCrypto's sha1-checked crate: SHA-1 with the
collision-detection pass that git applies to every object hash. The
detection is pure Rust and has no hardware path, so this contender shows
what git pays today.

SHA3-256 is provided by RustCrypto's sha3 crate, whose keccak backend
uses the ARMv8 SHA-3 instructions (EOR3, RAX1, XAR, BCAX) when the CPU
reports them at run time, and portable code elsewhere.

The resolved crate versions, sources, and registry checksums are included
in stdout, the text report, and the SVG metadata.

## Code paths by input size

Each contender may switch implementation as the input grows. BLAKE3
divides input into 1024-byte chunks; the crates.io crate runs a single
chunk through its one-chunk compressor and batches whole chunks into
the widest SIMD `hash_many` it can fill (four-way NEON on AArch64, so
four chunks at 4 KiB; AVX-512, AVX2, SSE4.1, or SSE2 on x86). On AArch64, `blake3-servil` runs an input of one chunk or less through
one call of its scalar kernel, two to fifteen chunks on integer + NEON
hybrid kernels, and groups of sixteen chunks on the SME2 kernel (16 KiB
and above). BLAKE3 official mt leaves the caller's thread above one SIMD
width of chunks; BLAKE3 servil mt splits across threads from 512 KiB.
SHA-256, SHA3-256, and SHA-1DC run one path at every size.

In the batch use cases, a contender without a batch call runs its
single-message kernel at every batch size. The crates.io crate's batch
call switches path at the platform's SIMD degree (four on NEON, sixteen
on AVX-512). `blake3-servil`'s batch call switches at two messages and at
sixteen, and servil mt splits larger batches across threads.

These paths explain a line's bends; the benchmark measures them and
reports none of them (Zooko, October 19: the paths of a stream or a
pool cannot be reported alike, so none are).

These inferences follow BLAKE3 v1.8.7's `src/platform.rs` and the
fork's `src/ffi_sme2.rs` and `src/ffi_neon_hybrid.rs`.

## Inputs, and what the benchmark leaves to others

The benchmark times hashes and checks no digests: each implementation's
own tests establish that it is correct. An input of `n` bytes for seed `s` consists of the little-endian 64-bit
words `s << 48 | 0, s << 48 | 1, ...` cut to `n` bytes. Every block of
every input differs, and seed 1 gives the second shared copy distinct
bytes. The timed loop hands every digest to `black_box`, so no hash can
be optimized away, and the two shared copies hash separate buffers, as
two programs would.

## Interleaving

The participating contenders in each use case run in a Williams design.
That schedule places every contender in every position equally often and
realises every "Y right after X" adjacency equally often (n orders for an
even count of contenders, 2n for odd). Point order rotates independently.
The run measures in three phases, each with its own calibration and
rounds: the nonstop use cases, then the calls after other work, then the
calls after idling. Because the operating system sets a core's clock from
the program's recent activity, each calling pattern runs among its own
kind. When nonstop and sleeping calls were mixed on an Apple M4 Max, nonstop
samples ran near 3.0 GHz (against 4.4 GHz alone) and sleeping calls ran
near full clock (against 3.8–4.0 GHz among other sleeping calls). Each contender/point combination is
calibrated separately: a continuous cell's timed samples last about 1 ms
each (and hold at least twice the buffers its program keeps in flight), a
synchronous cell's sum about 2 µs of calls after the gap.

A full run has 96 rounds, a `--quick` one 24 (and stops below 1 MiB and
10,000 messages). Each point samples at least 12 visits, rounded up to
whole cycles of its orders and spread over the rounds at an offset of its
own; every contender that takes part samples at each, long hashes
included, so each meets every position and every predecessor equally
often. A shared visit gives two samples, one per copy. The starting
point's rotation is even only when the round count is a multiple of the
point count.

With `--contenders ... --rounds N`, every cell samples every round.
Choose N as a multiple of each measured use case's order count for
complete balance; other counts give a partial design. For example, a
roster of eight has eight orders for messages, while seven participants
in its batch use case have fourteen: 56 rounds completes both designs.

Shorter samples (0.5 ms) were tried and rejected: every cell read 1.6%
slower, since a sample's fixed cost weighs twice as much. Before it sizes
a cell's samples, the run calls the cell's batch once, untimed, so that
one-time costs (a pool's threads starting, buffers touched for
the first time) leave the size alone.

Each dot is its cell's mean. The hover panel also gives the cell's
fastest and slowest samples, and the samples file holds every timing.
Most of a measurement's uncertainty lies between runs, not inside one:
where a program's code landed, where its threads were placed, which of
two speeds a cell settled into, all fixed for a whole process. Some
cells run at two speeds: two copies of an SME2 kernel run at full speed
when macOS places them on different P-clusters and at about half when it
places them on one, which shares its SME unit. The mean weighs each speed
by how often it came, which is what a caller pays, and the samples file
keeps every timing for a look at the speeds. A lead between two runs is
established by repeating them.

## Comparing runs

`bench-hashes compare OLD.tsv... -- NEW.tsv...` takes the runs in pairs
(the i-th old with the i-th new, run back to back in alternating order,
at least three pairs) and judges all their cells at once. Each cell uses
a paired t-test on the logarithm of each pair's ratio (new mean over
old), with the Benjamini-Hochberg procedure controlling the false
discovery rate at 5% across all cells; a cell that passes is marked
slower or faster beyond a 3% margin, and the rest are level. Beside each
cell it prints each side's geometric mean of run means, the ratio of the
two, and how many pairs ran faster. Any run whose load was busy or
unmeasured is noted beside the result.
Where both sides' samples carry CPU time, it adds each cell's CPU-time
ratio by the same rule (per pair, new CPU time per unit over old, all of
a run's samples' CPU time over their units), with no verdict.

**CPU time.** Each solo sample of a continuous or lent cell records the
process's CPU time across it, all its threads together
(`clocks::process_cpu_ns`: `CLOCK_PROCESS_CPUTIME_ID` on Linux and macOS,
read outside the timed interval with the thread's counts), in the samples
file's `cpu ns` column. It is what the call costs the machine, beside
what its caller waits for: the calling thread and every worker the call
kept busy, waiting, or spinning. A sample after gaps has none, since the
gap's other work would count as the call's, and a shared sample none,
since its two copies share the process. A b3sum row holds each run's CPU
time as the OS counts it for the child process.

**Energy.** The same samples record the CPU package's energy across each
(`clocks::energy`, RAPL's `energy-pkg` on Linux, read outside the timed
interval beside the CPU time), in the `energy nJ` column, and a b3sum row
each run's. It is the whole package's: the call's on a quiet machine
alone (every run's load is checked), and it counts the idle cores and the
uncore beside the busy ones. RAPL updates about once a millisecond, about
a sample's length, so one sample's reading is coarse; its rounding falls
either way, so a cell's energy, all its samples' over all their units, is
not biased by it. `compare` prints its ratio beside CPU time's, with no
verdict.

**Where each sample ran.** Every sample records the calling thread's
counts per core kind across it (`clocks::Counts`) in the `core counts`
column as `P time:P cycles:E time:E cycles`, or `-` where the platform
counts none. On Apple silicon and Linux x86-64 CPUs with performance
counters, the map's tooltip shows each point's core share and clock rate.
On Linux, cycles cover user space while running time also covers kernel
work, so the clock rate reads low across system calls. The pool's workers
are not counted yet.

**Threads.** BLAKE3 servil mt's pool holds one thread per CPU, its caller
among them, unless a run says `--threads N`, which sets it for the whole
run (the pool is the process's): the samples' `contenders` line records it,
and the map names the contender "BLAKE3 servil mt, N threads". `--threads 2`
measures the smallest pipeline, one worker beside the program's thread.

`bench-hashes check --fork DIR [--against REV]` checks whether a change to
`blake3-servil` slowed any cell. It builds the working tree and the
target commit, relinks each side with a fresh random function order for
each pair of runs, and runs the cells whose code the change touches
(`--cells all` runs every cell). A change to assembly or to the build
runs every cell.

Underneath, `bench-hashes regress` runs the pairs back to back in
alternating order (four pairs by default in `check`, eight in `regress`),
with 24 visits per run for cells after a gap and 6 for nonstop cells.
Before the pairs start, `regress` calibrates sample sizes once per side
so both sides perform the same work per sample and no timed run pays for
calibration. Giving each pair a fresh link layout averages out code
placement, which alone can move calls after a gap by several percent.

When the pairs find no cell slower, `check` takes a second stage. The
cells `regress` names as suspects (level, but the pairs' mean beyond the
margin; at most 8) run again on 16 fresh pairs, each side relinked anew,
and are judged among themselves. Judged among every cell a change
reaches, a slowdown at one point passes unseen unless it is many times
its pairs' spread; among 8 cells on 16 pairs the bar is about a third as
high. The stage adds about five minutes, most of it the relinks.

### Each part of the rule, and its evidence

| choice | evidence | what it gives up |
|---|---|---|
| A run's cell is its **mean**, total time over total work, every sample counted (`clocks::summary::mean`; Zooko, October 2 and 23) | Planted slowdowns on the Mac: at +6% a fast speed never caught 1 MiB, while the mean caught every cell nearly every time. At the check's budgets (6 and 24 samples per cell), setting the slowest sample aside erased a slow state occurring once in ten samples (`2026-10-22/mean-trim-controls`), whereas the untrimmed mean shows it in full. | A stall moves a cell's figure in full; a run whose load reading is busy carries no verdict. |
| **Pairs of runs**, alternating, a **fresh link layout** on each side per pair (October 2 and 16) | Most uncertainty lies between processes (placement, the speed a cell settles into): one binary's run means varied 1.49 times what its samples predict; 12 samples or 24 gave the same between-run spread. A layout moves the cells of calls made now and then by several percent, fixed for every run of that binary, so alternation cannot cancel it: two links of one source gave 27 false verdicts in 700 cells. | A second side's runs and a relink per pair. |
| **4 pairs** by default, each run with **the regression check's visits** (24 after a gap, 6 nonstop; October 20) | Calibrated on the i7-12700K (`data` branch `2026-10-20/check-validation`) and the M4 Max: on both, an A/A over every cell found no cell slower (the Mac at 8 pairs, 16 min; the i7 over 146 cells), and the two planted 6% slowdowns were caught at 4 pairs: `hash()`'s in servil st's cells (the Mac: +5% to +28% in the continuous cells, 11 min; the i7: 48 cells), the pool split's in servil mt's cells from 512 KiB, the cells after a gap among them (the Mac: +7% to +25%, 7 min; the i7: 22 of 28), in half the time of 8 pairs. At 12 visits after a gap instead of 24, the cells after a gap caught a plant 3 times in 10. Of the budget, pairs buy more than samples: 18 pairs of 4 samples judged 40% more narrowly than 8 of 12 in the same minutes. | A check takes 2 to 6 minutes. |
| **All cells judged at once**: a paired t-test per cell on the log ratios, Benjamini-Hochberg at a 5% false discovery rate (Zooko, October 16) | A per-cell test over 700 cells flags 15-27 cells of unchanged code; a sign test cannot go stricter with few pairs. With the rule: 0 of 700 in three unchanged-code controls, and the code's real changes found. | Floating point inside the t-test. |
| **A second stage** for the suspects, 16 fresh pairs, judged among themselves (Zooko, October 24; bench #66) | One input length planted slower in `hash()` (`data` branch, `2026-10-24/check-one-point` and `check-second-stage`): judged among 75 cells, +20% of calls (the cell +23% to +77% in every pair) was level. With the stage, +20% and +10% were caught, +6% not (cells after a gap swing 10-25% a pair); three A/A checks found no cell slower. | About five minutes a check; one cell beside the planted ones judged slower at +10%. |
| **A 3% margin** (October 2) | +3% planted was caught about half the time, +6% nearly always; one layout-only change in eight sat near 3% at lent 64 B. | Changes under 3% pass one commit at a time; the release check (`--against` the last release) sees their sum. |
| **One calibration a side a check** (October 17) | Each run calibrating its own sizes cost 18 s of a 41-s run; shared sizes judged every group of cells more narrowly in less time (about 2.2 times the information per minute), and both sides do the same work a sample. | Sizes chosen on the old side's build. |
| **Cells chosen by what the change touches** (October 20) | Coverage names the lines each cell executes; a change to `hash()` chose 73 points, a stream change 13, a comment none. The fixed five-cell list this replaced was blind to the cells after a gap, where #80's +18% sat, and to the stream. | A coverage build once per contender (cached); a change to assembly or the build checks every cell. |
| **No intervals as verdicts** (October 1) | 95% median bands held for a quarter of the cells across runs of identical code. | A printed range is a reading aid only. |

**What the SHA-256 cells can tell.** SHA-256's code is identical on both
sides of a servil comparison, so its cells show the instrument's noise, and
a SHA-256 verdict usually means the comparison misread something. One real
effect moves them too: the package's clock. A change to how BLAKE3's pool wakes or parks its workers changes how
many cores are active, which sets the clock every P core shares (on an
i7-12700K, 4.9 GHz with one or two active and 4.7 GHz with all). Because
SHA-256's samples are taken beside BLAKE3's, they inherit that clock
shift. A SHA-256 shift beside a change to the pool's waking is a finding
about that change, which programs pay too. A second interaction is
placement: after the pool's stream cells, Linux sometimes leaves the
caller on an E core, which costs BLAKE3 st (AVX2 at half width there) and
spares SHA-256 (as fast on either kind): bench #25.

## The map

`bench-hashes.map.html` draws every cell the run's samples file holds as
a small chart, its lines through each contender's means on log scales,
higher faster. The charts sit in a grid, first for one program (solo) and then for two
at once (shared). Its rows are what the program hashes: one message, a
batch of 64-byte messages, many messages at once in pieces, a collection
of messages, a message with its outboard, and a message verified as it
arrives. Its columns are how the program calls: after other work, after
a pause, waiting for each call, or pipelined. A cell the run did not measure stays
empty, in its place. Clicking a chart opens it over the cells, its row's
and column's headers lit, with its axes and, under the pointer, each
contender's rate and time per message (or batch, or run) at that point;
clicking again, or Escape, puts it back. A row's or column's name shows
what it means while the pointer rests on it, or after a click or tap, and
a hash's name in the legend hides or shows its lines in every chart.
Each chart names the call BLAKE3 servil mt makes there, a link to that
call's documentation. A link to `bench-hashes.map.html#solo|one|lent`
opens that chart. When the folder also
holds a b3sum run's samples, the map adds a section for it in the same
form: a file or a tree of files, in the page cache or read from storage.
Under each section's title one line says where and when the run was made
and whether other programs kept the machine busy. Opening "About this
run" shows the full provenance: the time, machine, power, load, versions,
commits, compiler, target, and samples file.
The map comes from the samples files alone (`bench-hashes map FOLDER`
draws it again), so a stored record draws the same map its run did.

## b3sum

`bench-hashes b3sum NAME=COMMAND...` measures builds of `b3sum` as a
person runs them. Each run is a new process, timed from start to exit (`clocks::child`),
along with the operating system's resource counts for the child. Those
counts include CPU time, peak memory, bytes read from storage, major page
faults (Linux and macOS), and on macOS cycles and instructions per core
kind. The samples file keeps them as comment
lines beside each run's time.

**Inputs.** The benchmark hashes four kinds of file input:
- Single files of 4 KiB, 64 KiB, 1 MiB, 16 MiB, 256 MiB, and 1 GiB.
- A tree of 1000 files of 16 KiB passed in one command (`b3sum $(find src -type f)`), and the same 1000 files hashed by one `b3sum` process per file.
- A mixed tree of 1000 files (74 MiB in all, from 1 KiB to 16 MiB with sizes interleaved like a source checkout) passed in one command.
- 16 files of 64 MiB (1 GiB in all; 4 of 16 MiB under `--quick`) passed in one command.

Each file contains BLAKE3's extended output of its relative path: the
same bytes on every machine, and incompressible so a compressing
filesystem or drive still reads them in full.

**Builds.** `tools/b3sum-contenders.py` builds each `b3sum` as its own
project recommends: official BLAKE3's as `cargo install b3sum` builds it
(dynamically linked), the fork's statically linked on Linux, as its
README recommends. A static build skips the dynamic loader, saving about 240 µs per process
on an i7-12700K (0.49 ms against 0.74 ms on a 64-byte file), which
contributes to the single-file process cells. With `LAYOUTS=N`, the
script relinks each `b3sum` binary `N` times with a random function
order, and `bench-hashes b3sum` rotates through those builds across
rounds so no single link layout dominates.

**Page cache.** *Warm*: the files were read moments before. *Cold*: each
file is evicted before each run, without root: Linux with
`posix_fadvise(DONTNEED)`, macOS with `msync(MS_INVALIDATE)`. The report
checks every cold run's reads from storage and names any cell the page
cache still served. Cold runs are skipped on Windows and on filesystems
kept in memory (tmpfs). In a virtual machine the host's own cache may
serve a guest's cold reads.

**Rounds.** Every contender runs once on every input, untimed, then 15
rounds (5 with `--quick`), the contenders in the main run's Williams
design (Interleaving), a row a round and input: every contender runs in
every position and right after every other equally often. A run's after-effects fall on the run that follows it. When contenders
merely rotated in a fixed order, each kept the same predecessor, which
moved cells by 7–10% on an i7-12700K; the Williams design balances those
predecessors. The balance is exact in whole cycles
of the design, a multiple of n rounds for n contenders when n is even and
of 2n when odd (18 rounds for three contenders); other round counts leave
a few rounds' remainder, a fraction of a percent at that effect's size.
Each cell's
figure is its mean time per run; a contender's `xN` against the first
contender is the ratio `compare` judges, each round a pair: the geometric
mean of the rounds' ratios, marked slower or faster where every cell
judged at once says so, beyond 3%. (A median of the rounds' ratios flips
with the share of rounds in each mode where a cell runs at two speeds: a
1 GiB file read ×0.62 or ×0.98 by that alone.)

The run writes `b3sum.result.txt` (the report) and `b3sum.samples.tsv`
(each run's time, and its counts as `# counts` lines), and draws the
folder's map again, with a b3sum section (above, "The map").

## Output

The run prints the text report on stdout and progress on stderr, and
writes its results to `benchmark-results/{CPU}.{OS}/`:
- `bench-hashes.result.txt`: the text report.
- `bench-hashes.map.html`: the interactive map.
- `bench-hashes.chart.svg`: one 1 MiB message as bars (drawn on full runs).
- `bench-hashes.samples.tsv`: every sample in the order taken (`ns/units`, start time, CPU time, energy, and core counts), preceded by `# key: value` header lines for provenance, CPU identity, and load windows. `bench-hashes compare` and `bench-hashes regress` read this file.

`bench-hashes chart SAMPLES.tsv` draws a samples file's chart again
beside it, and `bench-hashes map FOLDER` a folder's map; both come from
the samples alone, so a stored record draws what its run did. `bench-hashes
map FOLDER --beside OLD_FOLDER` draws the map with an earlier run's means
dashed under each chart's lines, for the contenders and points both runs
hold; it judges nothing (a single pair of runs is weak evidence of a
change: `bench-hashes compare` gives the verdicts).

## Load from other programs

While it measures, the run reads how much CPU time the whole machine
spent busy and how much this process used; the difference is CPU time
other programs took. Linux also reports steal time, CPU time a
hypervisor withheld from a virtual machine's CPUs for other work on the
host. The OS counts both in 10 ms ticks, so the run sums them over
windows of about a second, read between samples (a reading takes about
9 µs, once a second, outside every timed interval); on 16 CPUs a window
reads within 0.16 CPUs. The report, the samples file, and the line under
each of the map's titles give the run's average and its busiest window, in
CPUs kept busy, and call the run busy when a window reached a whole CPU
(other programs or steal), naming the busy windows. The samples file
lists every window and when each sample started, so a sample's window
is known. Measured in a quiet 16-CPU Linux
VM: 0.00 CPUs on average, 0.02 in the busiest window; two busy loops
beside the run read 1.98-2.01 in each of their windows. An
Apple M4 Max desktop running a Linux VM keeps 0.40-0.56 CPUs busy, and
its results then match a quieter run's to about 1%.

A program that runs others to measure them, `regress` and `bench-hashes
b3sum`, counts each child's CPU time as its own: reaped children through
the OS's count, a running child by reading its CPU time at each window's
boundary. So its windows stay about a second long while a child runs, and
a busy second inside a long run shows. `regress` judges each run by these
windows, not by the run's own, so a run needs no least length.

Hypervisors that report no steal time (Apple's Virtualization framework,
for one) keep a VM's guest from seeing load on the host, so a VM can
read quiet while the host is busy.

## Power

The run reads the machine's power state when it starts and when it
finishes measuring: whether it draws from a battery (and the charge),
and any power mode that trades speed for energy (macOS's Low Power Mode
and High Power mode, through `pmset`; Linux's ACPI platform profile).
The report, the samples file, and the line under each of the map's
titles give it. On battery an Apple M4 Max ran more of the calls that
follow a pause on its efficiency cores (233 of 400 calls after 1 ms of
sleep, against 32 of 400 on mains power). A virtual machine sees no
power supply, and its report says the OS reports none.

## Build settings and provenance

Every samples file records what built its machine code so another person
can reproduce it. The header lists `bench-hashes`'s repository, commit,
and working-tree state, the BLAKE3 digest of `Cargo.lock`, the Rust and C
compilers, `RUSTFLAGS`, the target and its features, and one `source
CONTENDER` line per contender (crate version and checksum, git commit, or
system library). The CPU's identity includes its
microcode. A b3sum contender's line names its build by the BLAKE3 digest
of its executable.

Release builds use optimization level 3, fat LTO, one codegen unit,
abort-on-panic, and no incremental compilation, for the target's
generic CPU, as programs are shipped: every contender chooses its code
path from the CPU it finds at run time.

The build script reads `Cargo.lock` and embeds each contender crate's
resolved version, registry checksum or git commit, and source, and this
repository's own commit and whether its tree was clean. The report and
the samples file carry them, so a result
names the exact code it measured. `Cargo.lock` is checked in, so every
build of one commit measures the same code.


## Produced leaves (`bench-hashes leaves`)

`bench-hashes leaves` times a program that computes a Merkle tree's
leaves and hashes each one, as a zero-knowledge prover's encoder does.
All four variants run the same producer (N 64-byte messages, 16,384 or
1,048,576, filled from a 64-bit counter) and differ in how the program
hands the messages to `blake3-servil`:
- `lend-oneshot`: produced into a buffer and lent to `hash_many_multithreaded`.
- `lend-stream`: produced into a buffer and lent to a stream's `submit_batch`.
- `copy-ring`: produced into a buffer and copied into the stream's ring.
- `write-ring`: produced straight into the stream's ring via `submit_batch_with`. A sample times whole jobs, from
producing the first message to having read every digest, nothing
subtracted. Each sample records, solo, the process's CPU time across it
(all its threads), as the main run's samples do; `solo` is one program,
`shared` two at once, each with its own samples (their CPU time is both
programs', so none is recorded). It writes `leaves.samples.tsv` in the
main run's samples format, which `bench-hashes compare` reads.

## Choices that stay settled, and why

**The bytes hashed.** In memory, any bytes the program has written: their
values change no contender's speed. The benchmark always writes its input buffers before timing. An
unwritten allocation can be backed by a single shared zero page in the
L1 cache, which would make a 64 MiB read artificially fast, and reading
uninitialised memory in Rust is undefined behaviour. The benchmark writes little-endian 64-bit
counter words (`seed << 48 | index`, `make_input`) once, outside the timed
work, so every block differs. Files on storage (`bench-hashes b3sum`)
hold BLAKE3's extended output of their names: a filesystem or a drive
that compresses would read zeros or counters from storage almost for free.

**No contender's correctness is checked here.** That is each contender's
own tests' business (the fork's `QUALITY.md`). The timed loop hands every
digest to `black_box`, and the harness's tests check its own work: that
each call's digest reaches its slot, and that each input of a collection
or batch reaches the hash, in order.

**Wall time is the basis.** Cycles per byte would miss time a core waits
on a shared unit (such as Apple's SME unit), and clock throttling caused
by a contender's power draw is a real cost its caller pays.
CPU time and energy are recorded beside wall time, never in its place.

**The default roster is fixed:** BLAKE3 servil st and mt, and SHA-256 by
two crates, sha2 and ring, since neither leads everywhere (on an Apple M4
Max sha2 leads at 64 B and 128 B and in 64-byte batches, ring from
256 B). `--all` adds the rest; the map opens showing servil mt, SHA-256
ring and official BLAKE3 (`SHOWN_AT_FIRST`), the others a click away.

## What a contender could do that the benchmark would reward

None of these is detected automatically; watch for them when reading a
result.

- **Caching across calls.** Every batch hashes the same buffer
  `iterations` times; a contender that memoised on pointer and length
  would score without limit. Inputs pass through `black_box`, but the
  bytes don't change. A defence, if one is ever needed: rotate among
  equal-size buffers, at a cache cost every contender would pay.
- **Knowing the harness's thread count.** The shared scenario runs exactly
  two copies; a multithreaded contender could detect that. More copies
  (devonjonte/bench-hashes#1) would make it harder.
- **Workers that stay hot.** A contender whose workers keep polling
  between calls looks better in a tight loop than in a program that hashes
  once a second; the after-idling cells (a 1 ms sleep before each call)
  and CPU time and energy beside wall time show what it costs.
