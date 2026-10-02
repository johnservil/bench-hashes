# How bench-hashes measures

This file explains what a run measures, how it keeps the numbers honest,
and what each contender runs. [README.md](README.md) says how to run it.

Every run measures each contender in nine use cases. Four are calls made
now and then, each after a gap of one of two kinds: **after other
work**, after the program has run a fixed other program and read 128 MiB
of data, as a program hashes between its other tasks; and **after
idling**, after the program has slept 1 ms, as a server waits for its
next request. Each kind measures **a message in one buffer**, at
twenty-seven sizes from 64 B to 128 MiB, reported per byte, and **a
batch** of 64-byte messages, at twenty-four counts from 1 to 262144,
reported per message.

Five hash one input after another, as fast as the program can, each
input first read into a buffer (a memory copy, timed). With buffers the
program **owns** and hands over, filling the next while one is hashed:
**messages**, at eleven sizes from 64 B to 64 MiB, and **batches**, of 16
to 65536 messages. With buffers it **lends** to a call until the call
returns, so reading and hashing take turns: messages and batches at the
same sizes, and **64 MiB messages in 64 KiB pieces**, through each
contender's incremental API.

Each nonstop use case runs in two scenarios: **solo**, one copy of the
contender, and **shared**, two copies at once. The calls after a gap run
solo. The report shows each use case once per scenario, solo first; the
graph shows both.

## Contenders

A default run measures BLAKE3 servil, multithreaded (servil mt) and
single-threaded (servil st),
and SHA-256 from two crates, sha2 and ring, since each is the faster
SHA-256 at some sizes. `--all` adds every other contender the machine can
run: the crates.io BLAKE3 crate (BLAKE3 official), SHA3-256 (the `sha3`
crate, with the CPU's SHA-3 instructions where it has them), SHA-1DC
(`sha1dc`, SHA-1 with the collision detection git uses, far slower than
every other contender at every size; left out of `--quick --all`), and
CommonCrypto's SHA-256 on Apple platforms (`sha256-cc`), which the ring
and sha2 crates each beat at every size on Apple silicon. `--contenders`
names any set, including one that runs only when named: the crates.io
crate on its Rayon pool (`blake3-official-mt`), which BLAKE3 servil mt
beats at every point. `--list` shows every key.

## Input sizes

The one-message axis tests every power-of-two input size from 64 B to
128 MiB but 16 MiB, plus 3 KiB and 3 MiB, plus four sizes of real data
between 2 and 8 KiB: 2304 B (an 802.11 frame body at its maximum),
3839 and 7935 B (the 802.11n A-MSDU maxima), and 4470 B (the Packet over
SONET/SDH MTU). None of those four is a multiple of BLAKE3's 1 KiB
chunk, as real inputs seldom are. 16 MiB was dropped: its neighbours
predict it to within run-to-run noise.

Below 1 KiB a BLAKE3 input is one chunk; from 2 KiB to 16 KiB
its SIMD paths fill (4-way NEON at 4 KiB, a sixteen-lane SME2 group at
16 KiB); above that the bulk rate settles. 3 KiB is where the SME2 fork's
integer + NEON hybrid kernels first overtake hardware SHA-256. The sizes
past 1 MiB show the plateau: a contender whose 32, 64, and 128 MiB
means agree has levelled out. The multithreaded contenders take
longest to get there, since a pool hand-off or a subtree merge amortises
more slowly than one kernel call (the fork's was still climbing at
8 MiB, and Rayon's still is at 128 MiB in a Linux VM); from 8 MiB up an
input passes most machines' last-level cache (some hold 32 MiB or
more, and some server chips far more), and where it does, the rate is
the memory's. 3 MiB is to the plateau what 3 KiB is to the SIMD
ramp: a tree that is no power of two (a 2 MiB left subtree beside a
1 MiB right one), so a splitter that cuts at subtree boundaries hands
its threads unequal work there.

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

## A message in pieces

A program that reads a file or a socket hands a hash its input piece by
piece, so the hash never learns the total size in advance. This use case
measures that on 64 MiB messages, one after another, in pieces of 64 KiB
(a common read buffer). One length is enough. A message of up to 64 KiB
is a single piece, so it costs what the one-message call costs on it.
On one thread, a long message's pieces each cost about what the
one-message call costs at 64 KiB. A multithreaded incremental API can
spread a long message's pieces over threads, a rate no one-message
size predicts; one long message shows it. Each piece is read, timed, as
a memory copy from the input, the cheapest read there is (a read from
the operating system's page cache adds a system call per piece), and
every contender pays it once per byte.

Each piece is read into a 64 KiB buffer of the program's, kept from one
message to the next, and then handed to the contender's incremental API,
so reading and hashing take turns (`Hasher::update` in crates.io BLAKE3
and in both BLAKE3 servil contenders, `update_rayon` for BLAKE3 official mt, `Digest::update` in sha2 and
sha1-checked, ring's `Context::update`, CommonCrypto's
`CC_SHA256_Update`), and the message is finalized; the next message
follows at once. BLAKE3 servil mt calls `Hasher::update_multithreaded`.

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
case hashes it. BLAKE3 servil mt takes them through the fork's queue,
built for throughput: `Queue::messages` for messages of up to 64 KiB,
`Queue::pieces` in 64 KiB pieces for longer ones, and `Queue::fixed` for
batches. The program keeps enough buffers in flight to cover the queue's
round trip (about 1 MiB of them or 1024, whichever is fewer), reads each
input into a free one, hands it over, and gets it back with its digest
through the queue's handler, so reading and hashing overlap. The
program makes its queue once and keeps it, with a bounded channel (a ring
allocated when it is made) that carries the returned buffers and digests
from the handler to the program's thread, so after warm-up neither the
program nor the queue allocates. The servil single-threaded contender measures continuous load in the
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
131072, 262144 (16 MiB of input at the end). Powers of two up to 16 show a SIMD batch filling (the blake3
crate's `hash_many` takes four blocks at a time on NEON, sixteen with
AVX-512); 3, 6, 12, 24, and 48 leave a group partly filled or leave a
remainder past sixteen-message groups; from 64 up
the per-batch overhead amortises and the rate settles. Results read in
nanoseconds per message and million messages per second.

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
  released together; each copy's own time is a sample. What each of two
  users of the same code gets. They compete for every resource the code
  uses: cores and memory bandwidth, and for the SME2 fork an SME unit,
  which serves a whole cluster of cores. The calls after a gap run
  alone: there a copy met the code its twin had just run in a cache the
  gap left warm, and read faster than one program alone.

The synchronous use cases' samples are calls after the gap: one call, or
as many as fill 2 µs where a call is shorter (the clock ticks every 41.7
ns, so a single short call cannot be timed), each after its own 1 ms
of other work, timed alone, and summed. After other work, each thread runs a fixed other program (1024 generated
functions, about 1.1 MiB of distinct machine code, run once), walks its
own kept 128 MiB working buffer at 64-byte intervals, then spends any
remaining millisecond on integer arithmetic. The whole program runs even
past 1 ms, so the gap can last longer on slower machines. The other
code matters: a data walk alone would leave the hash's own code in
the core's instruction cache. After idling, the thread sleeps 1 ms.
Each timed call follows the same call: before a sample's first, one
untimed call, gap and all. A gap leaves some of what ran before it in
the caches, so without that call the benchmark's own schedule (another
contender, another size) would decide what the first found. Either way
the thread then writes the input, as a read or producer would, before the hash call.
That write is measured separately and excluded from the hash sample.
The working buffer's pages are written when it is made, so operating
systems that share untouched zero pages give it real physical memory.
Each thread keeps one work buffer and one producer buffer across samples.
`--trace-clocks` records the producer's wall time and counts on rows
labelled `preparation solo`; the
hashing rows describe the call alone. In both, a pool's workers have
fallen asleep. After other work the caller's core is busy and its caches
hold the other program's code and data. After idling the core may have
slowed or powered down, or the thread may wake on another core: an
Apple M4 Max meets full clock, its lowest, or a step between, for each
call and every contender alike, and in a VM two speeds about 3.5 times
apart, so these cells mix speeds, and their means weigh each by how
often it came. Longer calls meet the clock of a mostly idle core
too: on an Apple M4 Max, a call of several milliseconds after idling ran
15-50% slower than after other work (SHA-256 at 8 MiB 0.43-0.51 against
0.29-0.34 ns/B), its core near 3.8-4.0 GHz where after other work it ran
near 4.5, which is what a server that mostly waits gets.
`--trace-clocks` records each call's cycles and time, the clock it ran
at. A full run takes about a minute.

A single-threaded hash costs about the same in both. A multithreaded one
shows in the shared scenario what its threads cost when the machine is
shared; an SME2 kernel shows what sharing its unit costs.

## Consistency checks

Every run also writes `bench-hashes.checks.txt`, for people who maintain
the benchmark or a contender: relations that hold for every contender
alike when the benchmark measures what it means to. Nonstop calls are no slower than calls after other work for
messages of 64 B-4 KiB; two copies at once are no faster than one; a hash
that runs on its core alone (no shared SME unit, no helper threads) is no
slower beside a second copy; and more work within the first-level cache
(up to 32 KiB) is no slower per byte or message than a size that divides
it. Each is judged on the cells' means, over 10% apart. A broken relation names a bug in the
benchmark or in the contender, or a finding to explain; the file lists
each, or says that all hold.

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
On other CPUs it runs the kernels of the crates.io crate it forks
(SSE4.1, AVX2, AVX-512 on x86). The report's kernel table names the
platform the run measured. Its provenance line gives the repository,
branch, and commit instead of a registry checksum. For a batch the fork's `hash_many`
hashes messages of one to sixteen blocks many lanes at a time (sixteen
per group on SME2, NEON below a group), and
`kernel_report_many(message_len)` describes that by batch size.

BLAKE3 official mt is the crates.io crate's own multithreading, called as a
program calls it by default: `Hasher::new().update_rayon(input)` on
Rayon's global pool, which Rayon sizes to one thread per logical CPU.
The method splits the tree recursively with `rayon::join` down to the
SIMD degree, so any input above one SIMD width of chunks may cross
threads, and idle pool threads steal the halves. The two shared copies are
two callers in one process sharing that one pool, the same situation
the servil fork's fair sharing addresses, so the two multithreaded
columns compare like for like.

BLAKE3 servil mt is the fork's `blake3_servil::hash_multithreaded`,
which returns the same hash as `blake3_servil::hash`. Inputs below
the threshold shown in the kernel table stay on the calling thread.
Larger inputs can split at subtree
boundaries across the calling thread and worker threads the fork starts
once per process and keeps; the caller merges the chaining values. How
many threads a call uses is the fork's decision from the input and the
machine, and concurrent callers in one process share the workers
fairly: two callers at once each get about half the machine. Across
processes the operating system's scheduler shares the workers' CPUs.
The fork also offers `hash_multithreaded_with_budget(input,
max_threads)` to cap one call's threads; the contender measures the
uncapped call.

The benchmark touches each implementation in three ways only: it lists
it; it calls the entry points its users call, with no cap, pool, or
wrapper of its own: its one-message call (`hash`, `hash_multithreaded`,
`Hasher::update_rayon`, `digest`), its batch call where it has one
(`hash_many`, `hash_many_multithreaded`, the crates.io crate's hidden
`Platform::hash_many`), its incremental API (`update`,
`update_multithreaded`), and for servil mt its queue; and it asks the
servil fork to describe its kernels (`kernel_report()` and its `_many`
and `_multithreaded` forms). It asks for no machine capacity, sets no
environment, and checks no digests: each crate's own tests do.

SHA-256 CommonCrypto, on Apple platforms only, calls the system's
libSystem through FFI using `CC_SHA256_Init`, `CC_SHA256_Update`, and
`CC_SHA256_Final`. This is the implementation most Apple software
reaches for, so it anchors the sha2 crate's number against the
platform's own. Its provenance is the running OS rather than a crate
version.

The three-call form is the fastest route into corecrypto. Measured on
an M4 Max, a 64-byte digest takes 51 ns through Init/Update/Final and
182 ns through the one-shot `CC_SHA256()`, whose finalisation spends
about 110 ns per compression; bulk throughput is identical on both.
Callers hashing small inputs through CommonCrypto gain most from the
streaming calls.

SHA-1DC is provided by RustCrypto's sha1-checked crate: SHA-1 with the
collision-detection pass that git applies to every object hash. The
detection is pure Rust and has no hardware path, so this contender shows
what git pays today rather than what raw SHA-1 costs.

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
four chunks at 4 KiB; AVX-512, AVX2, SSE4.1, or SSE2 on x86). The SME2
fork runs an input of one chunk or less through one call of its scalar
kernel (every block including the root compression, with the state in
registers throughout), two to fifteen chunks on integer + NEON hybrid
kernels, and groups of sixteen on the SME2 kernel (16 KiB and above).
BLAKE3 official mt leaves the caller's thread above one SIMD width of chunks;
BLAKE3 servil mt can split over threads from the length its kernel table
shows (512 KiB on an Apple M4 Max), another path with its own dot shape. SHA-256, SHA3-256, and SHA-1DC run one path at every size.

In the many-messages use cases a contender looping one message per call
runs the kernel for its message length at every batch size; the
crates.io crate's batch function changes path at the platform's SIMD
degree (four on NEON); BLAKE3 servil's at two (the NEON hybrid parent
kernels) and at sixteen (the SME2 group kernel), and servil mt's again
where a batch may leave the calling thread (the kernel table says
where).

The text report lists the kernel at each point for every contender in
each use case (one line for a contender with a single kernel) and marks
where a new one begins. In the graph, dot shape carries the same information: a circle
for a contender's first kernel, a diamond for its second, a square for
its third, a triangle for a fourth. Hovering any dot names its kernel,
and hovering the first dot of a new kernel adds a sentence on why the
kernel changes there. A legend under the plot
explains the shapes. Colour stays with the contender, so a line keeps
one colour while its dots change shape.

These inferences follow BLAKE3 v1.8.7's `src/platform.rs` and the
fork's `src/ffi_sme2.rs` and `src/ffi_neon_hybrid.rs`.

## Inputs, and what the benchmark leaves to others

The benchmark times hashes and checks no digests: each implementation's
own tests establish that it is correct. An input of `n` bytes for seed
`s` is the little-endian 64-bit words `s << 48 | 0, s << 48 | 1, ...` cut
to `n` bytes, so every block of every input differs, and seed 1 gives the
second shared copy different bytes; hash speed does not depend on the
bytes. The timed loop hands every digest to `black_box`, so no hash can
be optimized away, and the two shared copies hash separate buffers, as
two programs would.

## Interleaving

The participating contenders in each use case run in a Williams design:
a set of orders that together place every contender in every position
equally often and realise every "Y right after X" adjacency equally
often (n orders for an even count of contenders, 2n for odd), built over
the contenders that take part in the use case. Point order rotates
independently. The run measures in three phases, each
with its own calibration and rounds: the nonstop use cases, then the
calls after other work, then the calls after idling. The operating
system sets a core's clock from the program's recent use of it, so each
way of calling is measured among its own kind: a program that hashes one
input after another never sleeps, and one that idles between requests
mostly sleeps. Mixed, the neighbours moved each other's clocks: nonstop
samples ran near 3.0 GHz on an Apple M4 Max beside the sleeps, alone near
4.4 GHz (SHA-256's 1 KiB messages 0.51 against 0.36 ns/B); calls after
idling ran near full clock beside the busy calls, and near 3.8-4.0 GHz
among other idle calls. Each contender/point combination is
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
one-time costs (a pool or a queue's thread starting, buffers touched for
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

`bench-hashes compare OLD.tsv... -- NEW.tsv...` gives each cell's mean on
each side (over several runs, the median of their means) and the ratio,
and says so beside any run whose load was busy or unmeasured.
`bench-hashes regress OLD_EXE NEW_EXE` judges two builds: eight pairs of
runs, one of each, back to back in alternating order, over the lent cells
(a program hands its buffer to a call and waits), one copy alone. Each
pair gives each cell one ratio, new mean over old; a cell is slower when
the median of its ratios exceeds 3% and an exact sign test says it was
slower in more pairs than chance would give (seven of eight). The queue's
cells and the shared scenario stay out of it: on identical code the
queue's means move 6-60% between processes, and two copies' 64 B cells
switch between states 20-30% apart, more than the check can judge. A
change that only moves the code's layout is held about one time in eight,
at the 64 B cell by about the margin, a cost of where code lands. The check, calibrated on an Apple M4 Max with planted slowdowns
and builds of identical code, is in the fork's NOTES ("The regression
check, calibrated").

## The graph

The SVG shows a plot for each use case and scenario the run has (the
calls after a gap solo, the nonstop ones solo and then shared), each with
lines through the means on a log-log grid.

A switch at the header's left, above the y axes' titles, flips every plot between rate (the
default; higher is better: GB/s above, million messages per second
below) and time (lower is better: ns/B above, ns per message below).
Rate is the reciprocal of time, so on the log axis each plot mirrors
through its middle: the switch animates each point along a straight
line to its mirrored position over 0.7 s while the axes cross-fade, and
every label, value, and hover figure follows the chosen unit. Ratios
between contenders are unitless and stay put.

Hovering a dot opens a panel for that point: the hovered
contender's mean, range, and method (its code path), then every visible contender
of that plot ranked fastest first with its time, rate, and speed
relative to the hovered one ("▲ 1.35× faster" in green, "about the same" in grey, "▼ 3.22×
slower" in red; contender colours stay away from those two hues).
Hidden contenders stay out of the ranking. On a touch screen, tapping a
dot pins the panel; tapping it again or the background clears it. Name
highlighting follows the mouse, since a finger has no way to leave.

The strip at the top narrows every plot, in lock step, to a range of
inputs. It has a tick for every input the plots have (a batch counts its
messages' bytes) on the plots' logarithmic spacing; its band marks the
range shown. The strip spans the plots' own x range, so at the full range
each tick stands over its input in the plots. The strip holds
no numbers, since the plots' axes name their inputs in bytes or messages.
Dragging either end of the band moves that end of the range, and
dragging the band moves both, from tick to tick and never past each
other. A slowly moving pointer moves the band's end at a third of its
travel, so a slow hand can settle on one of several close ticks, and an
end leaves its tick only once the pointer aims 3 px nearer another; the
ticks under the ends light up while dragging. "All", shown whenever the
range is narrowed, restores every input. The chips at the header's right
show and hide plots, in groups that follow the measurements: what is
hashed (messages, batches, pieces); how the program calls (after idling,
after other work, nonstop); and, joined under Nonstop, the two choices
only nonstop plots have (owned or lent buffers; solo or shared). A plot
shows when every chip that applies to it is pressed; the plots shown
close ranks. A chip whose press would change nothing, as the others
stand, is dimmed, and a press that would leave no plot is refused. The header (title, strip, chips, and rate/time
switch) sits at the top of the page.

The page is written for three readers at once: a newcomer who holds only
the page, a regular who knows the benchmark, and a maintainer. The header
says what the page shows and on which computer; "How to read this graph"
opens a panel on the lines and dot shapes; "About this run" at
the bottom opens section by section onto the machine, the run, the
sources, the method behind each dot shape, and each hash's version. A
hash of the run that takes no part in a plot (BLAKE3 official mt has no batch
function over threads) is
listed under that plot's legend in pale type, "not measured here", with
the reason as a tooltip; each name's tooltip says what the hash is. A
comment at the top of the SVG source points maintainers to the code and
data behind it.

The names at the right edge of each plot are toggles. Clicking one
hides that contender in every plot: its marks fade out, each y axis
rescales to the contenders still showing, and its provenance line drops
out of the block below. The name stays in
place, greyed with a hollow swatch and a "hidden · click to show" hint,
anchored toward where its line would sit on the current axis. Resting
the mouse on a name underlines it and fades the other contenders' marks;
the names themselves keep their look, so they always show which
contenders are hidden. A viewer
without script support shows every contender, laid out identically.

## b3sum

`bench-hashes b3sum NAME=COMMAND...` measures builds of `b3sum` as a
person runs them. Each run is a new process, timed from just before its
start to its exit (`clocks::child`), with the counts the operating
system keeps for it: CPU time, peak memory, bytes read from storage, and
major page faults (Linux and macOS), and on macOS its cycles and
instructions on each core kind. The samples file keeps them as comment
lines beside each run's time.

**Inputs.** Single files of 4 KiB, 64 KiB, 1 MiB, 16 MiB, 256 MiB, and
1 GiB; a tree of 1000 files of 16 KiB passed together, as `b3sum $(find
src -type f)` passes them; and a mixed tree of 1000 files, 74 MiB in all,
as a source checkout holds them (300 of 1 KiB, 300 of 4 KiB, 200 of
16 KiB, 120 of 64 KiB, 60 of 256 KiB, 15 of 1 MiB, 4 of 4 MiB, one of
16 MiB, their sizes interleaved), hashed in one run as `find . -type f
-print0 | xargs -0 b3sum` hashes them. Each file holds BLAKE3's extended
output of its name (its path under the files directory): the same files
on every machine, and incompressible, so a filesystem or drive that
compresses reads them in full.

**Page cache.** *Warm*: the files were read moments before. *Cold*: each
file is evicted before each run, without root: Linux with
`posix_fadvise(DONTNEED)`, macOS with `msync(MS_INVALIDATE)`. The report
checks every cold run's reads from storage and names any cell the page
cache still served. Cold runs are skipped on Windows and on filesystems
kept in memory (tmpfs). In a virtual machine the host's own cache may
serve a guest's cold reads.

**Rounds.** Every contender runs once on every input, untimed, then 15
rounds (5 with `--quick`), the contenders' order rotating. Each cell's
figure is its mean time per run; a contender's `xN` is the median, over
the rounds, of its time against the first contender's in the same round,
marked slower or faster by the rule `regress` uses (3%, an exact sign
test over the rounds).

## Output

The run prints the text report on stdout and progress on stderr (the
phase, a bar over the sample rounds, and the running mean of every
contender at the largest input size), and writes five files to
`benchmark-results/{CPU}.{OS}/`: `bench-hashes.result.txt` (the
report), `bench-hashes.graph.svg` (the graph), `bench-hashes.guide.html`
(the guide for programmers), `bench-hashes.checks.txt` (the consistency
checks, above), and `bench-hashes.samples.tsv` (every sample of every cell, every scenario,
in the order taken, each as `ns/units`, and in a last column the
millisecond each sample started, with the provenance, the CPU's identity,
and the load windows as `# key: value` lines). `bench-hashes compare` and `bench-hashes regress` read it. `--trace-clocks PATH` also writes each
sample's thread counts per core kind (cycles, instructions, time), the
clock each call ran at.

## Load from other programs

While it measures, the run reads how much CPU time the whole machine
spent busy and how much this process used; the difference is CPU time
other programs took. Linux also reports steal time, CPU time a
hypervisor withheld from a virtual machine's CPUs for other work on the
host. The OS counts both in 10 ms ticks, so the run sums them over
windows of about a second, read between samples (a reading takes about
9 µs, once a second, outside every timed interval); on 16 CPUs a window
reads within 0.16 CPUs. The report, the samples file, and the graph's
"About this run" section give the run's average and its busiest window, in
CPUs kept busy, and call the run busy when a window reached a whole CPU
(other programs or steal), naming the busy windows. The samples file
lists every window and when each sample started, so a sample's window
is known. Measured in a quiet 16-CPU Linux
VM: 0.00 CPUs on average, 0.02 in the busiest window; two busy loops
beside the run read 1.98-2.01 in each of their windows. An
Apple M4 Max desktop running a Linux VM keeps 0.40-0.56 CPUs busy, and
its results then match a quieter run's to about 1%.

Hypervisors that report no steal time (Apple's Virtualization framework,
for one) keep a VM's guest from seeing load on the host, so a VM can
read quiet while the host is busy.

## Power

The run reads the machine's power state when it starts and when it
finishes measuring: whether it draws from a battery (and the charge),
and any power mode that trades speed for energy (macOS's Low Power Mode
and High Power mode, through `pmset`; Linux's ACPI platform profile).
The report, the samples file, and the graph's "About this run" section
give it, and the graph's header says so when the run drew on a battery
or saved power. On battery an Apple M4 Max ran more of the calls that
follow a pause on its efficiency cores (233 of 400 calls after 1 ms of
sleep, against 32 of 400 on mains power). A virtual machine sees no
power supply, and its report says the OS reports none.

## Build settings and provenance

Release builds use optimization level 3, fat LTO, one codegen unit,
abort-on-panic, and no incremental compilation, for the target's
generic CPU, as programs are shipped: every contender chooses its code
path from the CPU it finds at run time (the kernel tables say which).

The build script reads `Cargo.lock` and embeds each contender crate's
resolved version, registry checksum or git commit, and source, and this
repository's own commit and whether its tree was clean. The report, the
samples file, and the graph's "About this run" section carry them, so a result
names the exact code it measured. `Cargo.lock` is checked in, so every
build of one commit measures the same code.


## Choosing a call and reading its speed

`bench-hashes.guide.html` asks how a program receives its data,
recommends one servil call, shows a complete program that calls it (one
of this crate's compiled examples, so it builds), and draws that call's
measured cells from this run beside the fastest SHA-256 present. Each
question ends with an "I'm not sure" answer: one thread, one buffer, a
thread that keeps up, a borrowed buffer, time.

The chart shows throughput on log scales, so every size gets the same
room, and chips above it switch between the ways the benchmark called
that function (after idling, after other work, nonstop; alone or beside
another program, where measured). A sentence above the chart says from
which size the call leads SHA-256, computed from the dots. Hovering a dot
gives what a caller waits for, the time of one call or batch to three
significant digits, with the rate and the code path; for the queue that
time is the stream's average per input with many in flight.
`Queue::messages` covers messages up to 64 KiB and `Queue::pieces` the
longer ones. For a message arriving in pieces now and then, the guide
shows `hash`'s cells, labelled by piece length: each piece costs about
what `hash` costs on a buffer that long. A run that lacks the
recommended cells says so. The chips keep the recommended call: on
several threads, nonstop pieces show `update_multithreaded`, and pieces
now and then show `update`, with `hash`'s cells standing in.
