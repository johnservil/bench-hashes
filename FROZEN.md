# What the benchmark asks of the servil fork (frozen)

The servil team's contract with itself (Zooko and John Servil): bench-hashes
measures the fork through exactly the calls and usage patterns below, which
are the API plan (the fork's `docs/api-design.md`) turned into
measurements. The fork's work is to be as fast as possible under them.
Changing what the benchmark asks is a decision of Zooko's: edit this file
with the change, the date, and the reason, and the code with it; the test
`frozen_contract_matches_frozen_md` compares the block below with the
code's own tables and fails when they differ.

**The whole benchmark is frozen** (Zooko, October 1, 2026, at bench-hashes
0.10.0, tag v0.10.0+e57316f, measuring the fork's `servil` at b132f8c), so
that a number, a verdict, and a picture from later mean what they mean
now, and people can compare results by eye: how samples are taken (the
rounds, the sample lengths, the gaps, the contender orders), the
contenders and their versions, the summary rule (each run's mean, pairs
of runs: `clocks::summary`), the load rule (`clocks::load`), the
regression check (`bench-hashes regress`: its points, its pairs, its
margin of 3%), and the
presentation (the graph, the guide, and the report, their layout and
their words). A change to any of these, a bug fix included, is Zooko's
decision, recorded here with its date and reason, and it starts a new
release, so results of different benchmarks are never read as alike. The
documents that explain the benchmark (README, METHODOLOGY, CONTRIBUTING,
the notes) may still change, to say it better.

**Changes since 0.11.0** (Zooko, October 2, 2026), for the next release,
from the regression check's calibration (the fork's NOTES, "The
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
- `bench-hashes b3sum` measures builds of b3sum (it was the fork's
  tools/b3sum-bench), in this samples format.

**Changes in 0.11.0** (Zooko, October 2, 2026):
- The fork's queue takes a mode and a handler (`Queue::messages(mode,
  handler)`): the fork removed its time-or-energy choice, whose
  complexity outweighed its likely use, and the benchmark called
  `Efficiency::Time`, the queue's only way of hashing now. The calls and
  their patterns are otherwise the same.
- The programs' kept buffers are written when made (a bug fix): the
  queue's batch buffers and digest space, its message buffers, and the
  lent pieces' read buffer came from `vec![0; len]`, zeroed pages the
  system maps only at their first write, so they faulted inside each
  cell's first sample, a first-use cost no kept buffer of a real program
  pays (330 against 217 us a queue batch in the fork's tmp/lentprobe).

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
  would otherwise decide what that was (bench-hashes NOTES, "Shared
  after a gap"). The producer writes the input after the gap, before the
  call; its write is timed separately and excluded from the hashing
  sample (Zooko, September 28, evening).
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
  faster than one program alone (NOTES, "Shared after a gap"). The graph
  plots solo and shared.
- **Planned, to add under this contract**: keyed and derive-key spot
  checks in perf_regress.

```frozen
use case OneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case ManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case IdleOneMessage: 64 B, 128 B, 256 B, 512 B, 1 KiB, 2 KiB, 2304 B, 3 KiB, 3839 B, 4 KiB, 4470 B, 7935 B, 8 KiB, 16 KiB, 32 KiB, 64 KiB, 128 KiB, 256 KiB, 512 KiB, 1 MiB, 2 MiB, 3 MiB, 4 MiB, 8 MiB, 32 MiB, 64 MiB, 128 MiB
use case IdleManyMessages: 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144
use case ContinuousMessages: 64 B, 256 B, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB
use case ContinuousBatches: 16, 64, 256, 1024, 4096, 16384, 65536
use case LentMessages: 64 B, 256 B, 1 KiB, 4 KiB, 16 KiB, 64 KiB, 256 KiB, 1 MiB, 4 MiB, 16 MiB, 64 MiB
use case LentPieces: 64 MiB
use case LentBatches: 16, 64, 256, 1024, 4096, 16384, 65536
scenarios: solo, shared
shared measures: ContinuousMessages, ContinuousBatches, LentMessages, LentPieces, LentBatches
blake3-servil-st OneMessage: hash(input), each call after other work
blake3-servil-st ManyMessages: hash_many(batch, 64, out), the padded batch contract, each call after other work
blake3-servil-mt OneMessage: hash_multithreaded(input), each call after other work
blake3-servil-mt ManyMessages: hash_many_multithreaded(batch, 64, out), the padded batch contract, each call after other work
blake3-servil-mt ContinuousMessages: Queue::messages(Mode::Hash) for messages of up to 64 KiB, Queue::pieces(Mode::Hash) in 64 KiB pieces for longer ones, one message after another, each read into free buffers of the program's, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept
blake3-servil-mt ContinuousBatches: Queue::fixed(64, Mode::Hash), one batch after another, each read into a free buffer of the program's, submitted with its digests' space, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept
blake3-servil-st LentMessages: hash(input), one message after another, each read into a kept buffer and lent until the call returns
blake3-servil-st LentPieces: Hasher::update per 64 KiB piece, then finalize, messages one after another, each piece read into a kept buffer and lent until the update returns
blake3-servil-st LentBatches: hash_many(batch, 64, out), the padded batch contract, batches one after another, each read into a kept buffer and lent with kept digests until the call returns
blake3-servil-mt LentMessages: hash_multithreaded(input), one message after another, each read into a kept buffer and lent until the call returns
blake3-servil-mt LentPieces: Hasher::update_multithreaded per 64 KiB piece, then finalize, messages one after another, each piece read into a kept buffer and lent until the update returns
blake3-servil-mt LentBatches: hash_many_multithreaded(batch, 64, out), the padded batch contract, batches one after another, each read into a kept buffer and lent with kept digests until the call returns
blake3-servil-st IdleOneMessage: hash(input), each call after idling
blake3-servil-st IdleManyMessages: hash_many(batch, 64, out), the padded batch contract, each call after idling
blake3-servil-mt IdleOneMessage: hash_multithreaded(input), each call after idling
blake3-servil-mt IdleManyMessages: hash_many_multithreaded(batch, 64, out), the padded batch contract, each call after idling
```
