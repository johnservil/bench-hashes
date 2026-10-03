# Style Guides

These style guides read the same in the fork's `AGENTS.md` and in bench-hashes' `AGENTS.md`; a change goes into both.

## Communication

- Name Zooko as "Zooko" alone, in every document, commit, issue, pull request, and message; never add a surname (his wish, September 26, 2026).
- Phrase positively or neutrally; avoid negations and "not this, but that" contrasts.
- Frame positively: show the promising, successful aspects of the recommended path. Mention an alternative only when its trade-offs deserve our attention.
- The reader has limited working memory and limited ability to search back through recent text. Include only what the current focus needs.

Sixteen actions that improve writing:
1. Sand off filler words
2. Find the real actors
3. Restore actions to verbs
4. Delete empty verbs
5. Prefer characters as subjects
6. Put subjects and verbs together
7. Put verbs and objects together
8. Make the opening familiar
9. Put new and important information last
10. Repair topic flow
11. Repair stress flow
12. Establish a clear topic sentence
13. Make subjects consistent across a passage
14. Control passive voice deliberately
15. Name responsibility
16. Trim metadiscourse

## Presentation: every item costs the reader

Every piece of information in a UI, a report, or a document costs its reader attention and risks fatigue and overflow. Review each one with two questions: who is this designed for, and which information pays that reader much more than it costs them? Keep what passes; cut the rest. Information for maintainers (diagnostics, spreads, provenance details, internal names) stays out of what users read; it belongs in maintainer notes, logs, and diagnostic flags.

## Presentation: write each page for a reader who holds only the page

A page (a graph, a report, a README, a docstring) carries its own context:

- **Its terms:** each is common knowledge or introduced on the page.
- **Its questions:** each sentence answers a question the page itself raises, in the order the reader meets them.
- **Its tense:** the page describes what is, as it is now. How it got here belongs to commit messages and notes.

**Three readers.** Picture the page's readers at three depths of context and serve them together:

- the *newcomer*, who landed on the page from idle curiosity and holds only the page;
- the *regular*, who knows the tool and opens its details;
- the *maintainer* (Zooko, John Servil, and their like), who knows its history; maintainers' documents and comments in the page's source serve this reader.

Keep what serves one reader and reads cleanly to the others; an item legible to one reader alone moves behind a door or into the maintainers' documents. Writers naturally picture themselves as the reader, so name the three readers explicitly while reviewing.

**Show before you tell.** Position, shape, colour, grouping, arrows, and absence carry meaning at a glance, and words and numbers follow them. A hash of the run that takes no part in a plot appears in its legend in pale, still type; an arrow runs beside "higher is better" pointing up; a band on a strip shows which part of the inputs the plots show. A mark beside its words, parallel to them, lets the reader take in both at once.

**Doors.** Details sit behind doors (a collapsed section, a tooltip, a panel that opens on a click), each placed for the reader who wants what is behind it. The door itself tells every other reader that the page is theirs to enjoy without it.

**Generated text is computed from what the page shows.** A sentence about the page's own contents is built from the same data as the page, so it names only what is on the page, whatever options produced it.

**A revision ends with a fresh reading.** After every change, reread the whole page (for a graph, a render of it) as each of the three readers; the revision is done when the page reads as if written fresh.

## Simplicity

Prefer the simplest design that meets the contract and performs well.
Simplicity has several dimensions:

- **Conceptual ease:** a reader can understand and predict the design with
  a few familiar concepts.
- **Less code and information:** fewer moving parts, less state, and fewer
  facts to carry in working memory.
- **Fewer runtime cases:** fewer branches, special cases, tuning knobs,
  and distinct operating regimes.

Use one clear mechanism wherever it serves. When approaches perform
similarly, choose the simpler one. Apply this standard to code,
interfaces, documentation, and performance optimizations.

**Every piece earns its place** (Zooko, October 1, 2026). Each piece of a design (a mechanism, a check, an option, a threshold, a line of output, a paragraph of documentation) adds complexity, and it stays while its demonstrated benefit exceeds its complexity-cost. Weigh it whenever you propose to add, keep, change, or rescue a piece. A piece's cost comes with its presence: every reader reads it, every run carries it, every later change works around it. Its benefit comes with what it reliably delivers: how often it acts, and how often it is right when it does (for a detector, its rate on real changes beside its rate on identical inputs). So an optional, partial, advisory, or unreliable piece carries its whole cost for a share of its benefit, and the design to weigh is the one a change leaves, piece by piece. To rescue a piece that falls short, first estimate from the measurements in hand whether it can earn its place at a cost worth paying; then improve it or remove it, and say which and why.

**Reach for the simplest first** (Zooko, October 3, 2026). Before adding a piece, write down the simplest version that could do its job (often nothing at all, a constant, a fixed sequence, or code and data the repositories already have) and use it, unless you can name a problem it causes from evidence: a measurement, a documented fact, a decision on the record. A generator, a check, an abstraction, or a test added to be thorough, with no such problem named, is complexity that has not earned its place. Before deciding any question, search the notes for an earlier decision on it, the "What is settled" list in bench-hashes' `NOTES.md` first; a settled question is decided again only with new evidence. When you find yourself deciding a question a second time, write the conclusion into that list.

**Revisit complexity as you learn** (Zooko, September 30, 2026). Complexity is a large cost that never stops being paid, so its benefit is never settled: whenever new information shows a piece of complexity doing less than it was built for, weigh removing it, since its cost may now exceed its benefit. Above all, when you find yourself building a second solution to a problem that an earlier solution already addresses, stop: you are very likely making a mistake. Go back and either make the first solution good enough, or remove it entirely; say which, and why, before building anything.

## Crash-only (Zooko, October 3, 2026)

Everything we make lasts until its process ends, and a process ends by crashing: killed (`SIGKILL`), its machine or VM switched off, never by a clean shutdown it gets to run (crash-only software). So we write no code to shut down or reclaim: threads, pools, and buffers live for the process, and nothing stops, joins, drains, or frees them. Such code runs rarely and in states tests seldom reach, so its bugs hide; every reader still reads it and every later change still works around it; and a program whose correctness rests on it is wrong on the day it crashes. A piece that shuts down or reclaims is built only for a named use case that needs it, argued on the record (Every piece earns its place). Tests are the common one: a suite makes many of what a program makes once. Where releasing a thing when its owner drops it is simpler, in code, docs, and concepts, than keeping it for the process and arranging reuse, Simplicity governs and it is released (Zooko, October 3, 2026). Contracts say what lasts for the process, so callers make few of such things and reuse them.

## Strategy: the streaming APIs first (Zooko, September 27, 2026)

The users most sensitive to performance, whether they save time or energy, use the streaming APIs: one message of any length after another, a long or endless series of fixed-length messages (batched), a Merkle tree. The streaming APIs are harder to use and built for efficiency; every other API is built for ease of use. So every trade-off goes to the streaming APIs: make them as efficient as we can, in time and in energy, even at a cost to the other APIs. Choosing the efficient API is the caller's job, as Design By Contract puts it (below). We still make every other API as efficient as we can wherever that costs the streaming APIs nothing. This replaces the minimax strategy (judging a design by its worst case over every API and usage), which governed until this date; its history is in the notes.

The ease-of-use APIs keep their contracts, and the tests hold them: a multithreaded call is never slower than the single-threaded call on the same task (it could have run single-threaded).

Misuse and misfortune (several of a program's threads calling at once, other load on the machine, the shared scenario) stay measured and reported in every benchmark, record, and check, and stay cared for: keep cheap protections such as the SME2 lock, report what they cost, and improve them where it costs the streaming APIs nothing. They never veto a change that helps the streaming APIs; such a change records the cells it slows, with their numbers, in its commit message. `perf_regress` holds a change when a cell it judges is slower beyond its margin (the fork's `PROCEDURES.md` has its cells, margins, and what a hold obliges; the servil team sets them from calibration). The rule detects; whether a change is worth its cost stays judgment, and cells the check does not judge (the queue's, other sizes, E-cores) stay the probes' and the review's business.

**The benchmark is the contract** (Zooko, September 27, 2026). bench-hashes' `FROZEN.md` lists what the benchmark asks of the servil fork (each use case's calls and usage pattern, the scenarios, the points), which is the API plan (the fork's `docs/api-design.md`) turned into measurements; a test compares it with the code. The fork's work is to be fast under it. A change to what the benchmark asks is Zooko's decision, recorded in `FROZEN.md` with its date and reason; nobody changes it to make a result look better.

**Serve real programs, not the benchmark's loop** (Zooko, September 27, 2026). A benchmark that calls back to back with no work between calls models almost no real program: real callers handle each result, wait on I/O, or have no next call. Keep nothing running between calls for a call that may come (no spinning or lingering workers): it costs users energy and CPU time, and the OS lowers the clock of a core that spins. Judge a design at the gaps real callers leave between calls, and steer callers with a stream of inputs to the interfaces that keep the engine fed (batches, and a pipelined API).

## Strategy: we own every slowdown a user could meet

Our duty is to the user, so we take responsibility for every performance problem that could plausibly reach one, whatever its cause. A slowdown that comes from the operating system's scheduler, the hardware, thread placement, clocks, or an interaction we find hard to reproduce, understand, or control is still ours. We never set such a slowdown aside with "it is probably the environment, not our code". Each one gets one of three outcomes, in this order of preference:

1. Control it: change the design so the slowdown cannot happen, or cannot happen from anything we did.
2. Understand it well enough to tell users how to control it, and document that.
3. At the least, understand it well enough to predict when it happens and how large it is, and state that in the user-facing results or docs.

Until a problem reaches one of these outcomes it stays open: it goes on the next-steps list, the record or commit that shows it says so, and it blocks the claim that a change has no regression.

## Correctness tests

Use reproducible inputs and fixed, independently established expected
outputs. A deterministic RNG is a compact specification of test bytes;
record its algorithm, seed, and length, and check in the expected digests.
Published test vectors and independent reference implementations establish
the answers. Regenerating golden outputs is an explicit, reviewed action.
Tests never silently regenerate their own expected answers.

The same fixed vectors can exercise different kernels, thread budgets,
concurrent calls, and scheduling interleavings. Input generation and
execution scheduling are separate concerns. Differential tests supplement
these anchors. Correctness is each project's tests' business: a benchmark,
and its tests, check no contender's outputs (Zooko, September 26, 2026,
and October 3, 2026); the benchmark only keeps every result from being
optimized away. A benchmark's inputs and schedules are the simplest that
serve (Reach for the simplest first), deterministic generators being for
tests.

## Measuring: wall time and cycles, both, always

Whenever code is timed (a probe, a benchmark, a throwaway loop in a scratch directory), record two clocks together, through the fork's `clocks` crate, the only code that reads a clock for a measurement in either project or in any probe (it holds our decisions on which clocks, how to read them, and how to time calls shorter than a clock tick): wall time, which a caller waits for, and the thread's core cycles per core kind, which do not change with the clock speed and do not count time the core waits on a coprocessor (the SME unit). Their ratio, cycles per ns, says which state a measurement ran in (the core kind's clock; the SME unit's fast or slow state). Speed verdicts use wall time; a change that shifts the mix of states has that shift as part of its effect. Cycles explain and locate time, and are the finer comparison for core-only kernels across core kinds. When the two disagree, that is a finding to investigate, never a reason to prefer one. Report wall time as measured, never scaled by cycles, until we know which part of their ratio is ours (our code's SME waits, the power it draws) and which is the machine's (heat, a host, a scheduler) (Zooko, September 26, 2026). Where a platform gives no per-thread cycle counts, say so beside the result.

## Measuring: one implementation of every rule

Each measurement practice is code, in one place, and every measurement calls it (Zooko, September 30, 2026): a practice kept as instructions depends on every reader remembering it, and a second implementation drifts from the first (the benchmark's own copy of the load reader subtracted wrapping counters wrongly). The `clocks` crate reads the clocks and the counts, times calls, and records other programs' load in windows of about a second, with each sample's start, reporting a busy window on stderr as it closes (`clocks::load`); `clocks::summary` summarises and compares samples; bench-hashes reads its own samples files (`bench-hashes compare` for any comparison, `bench-hashes regress` for the regression check), and a Python tool calls it, reading no samples itself. Use them, and write no timing, load, summary, or samples-parsing code of your own; a need they do not meet is a change to them. A measurement made while `clocks` found the machine busy is no evidence of speed: `perf_regress` gives no verdict, and every other comparison says so beside its result.

## Measuring: a run's mean, compared over pairs of runs

Most of a measurement's uncertainty lies between runs, not inside one: where a program's code landed, where its threads were placed, which speed a cell settled into, all fixed for a whole process, which no figure computed inside one run can see (the regression check's calibration, October 2, 2026, in the fork's NOTES). So every summary of a cell, in the benchmark, its graph and report, `perf_regress`, A/Bs, and every probe, is a run's mean: the total time of its samples over the total work they did (`clocks::summary::mean`), what a caller pays on average. Every comparison takes pairs of runs, one of each side, back to back and in alternating order, and judges a cell by its pairs' ratios (`clocks::summary::verdict`: the median beyond the margin, and an exact sign test); for samples files, `bench-hashes compare OLD... -- NEW...` and `bench-hashes regress`. A cell that runs at two speeds shows in its samples; a change in how often it runs at the slower one is a change in what its callers pay, and the mean carries it (Zooko, October 2, 2026, replacing the two-speed rule of September 28).

## Coding: integers first

Avoid floating point except where the domain is continuous by nature (pixel coordinates on a log axis). Time is discrete by nature here: every clock we can read counts ticks, so elapsed time, like counts, bytes, and ranks, is an integer (Zooko, September 26, 2026). Measurements, statistics, ratios, and thresholds are integers. Keep a measurement as measured (a sample is the nanoseconds the clock gave over the units they covered) and defer every lossy step: bench-hashes computes on times and ratios in fixed point with 64 fractional bits (its `Fixed`), where sums, differences, and comparisons are exact, and rounds once, where a person reads the value (permille for ratios and spreads, hundredths for opacities, three significant digits of nanoseconds). Round explicitly (`(a + b / 2) / b`) at the one place a division happens, and encapsulate the representation in a type whose methods do the rounding. Convert to `f64` at the last moment, for drawing only.

## Coding: Design By Contract

We document and `assert` every precondition our code relies on (`debug_assert` only on hot paths). Contracts are **expansive** (the caller carries the responsibility), **conceptually simple** (a few sentences of English; simplicity beats familiarity), and **structurally simple** to enforce (few lines, types, data elements, conditionals).

We never write "defensive code" — code that complicates a contract to ease the caller's life. When running code detects that a caller misunderstood the contract, it **fails stop**: panic with a clear message. Stopping is safer than proceeding, and it lets people fix the caller or loosen the contract. Defensive codebases grow buggier over time; DBC codebases stay predictable.

**Contracts change everywhere at once** (Zooko, September 30, 2026). When we improve a contract (a function's signature, a data structure, a file format, a tool's output), the same change updates every caller and every reader, in both repositories. Code accepts exactly the current contract and fails stop on any other: a reader asserts the format's version and refuses an older one. Code that keeps accepting an old form for compatibility is defensive code, and we never write it. Results in an older format are read with the tools of the commit that wrote them.

## Interfaces: fewest new concepts

A highly desirable property of an interface and its contract: the user learns the fewest new concepts. Zero new concepts earns a perfect score. Each new term (a resource unit, a sharing rule, a tuning knob) taxes working memory and needs a place in prediction and control. Prefer familiar concepts the caller already holds (threads, inputs, budgets), keep implementation units unnamed in public docs, and express observable behavior (speed, thread count, fairness beside concurrent calls) in those familiar terms.

# This repository

Read `NEXT-STEPS.md` first: it says what the work is now and where the last session left things. `NOTES.md` holds the measurement design, the known threats to validity and how each was closed. This file is the principles and the repository's shape; `PROCEDURES.md` is how things are done here and the environment.

These four files (with `PROCEDURES.md`) are for the servil team. `README.md` (also the GitHub Pages home page) and `METHODOLOGY.md` are for people who run the benchmark and read its results; `CONTRIBUTING.md` is for other developer teams. Keep each document to its audience (the fork's `AGENTS.md`, "Audiences").

`bench-hashes` is a single-crate Rust benchmark (`src/main.rs`, `build.rs`) comparing BLAKE3, SHA-256, SHA-1DC, BLAKE3 servil (the fork with SME2 and integer + NEON kernels), and the two multithreaded contenders BLAKE3 official mt (crates.io `Hasher::update_rayon`) and BLAKE3 servil mt (the fork's `hash_multithreaded`: subtrees over the caller's thread and the fork's own workers, shared fairly between concurrent callers). Every run measures three use cases on one axis list (`POINTS`): one message per call at twenty-seven input sizes (ns/B, GB/s), many 64-byte messages per call at twenty-four batch sizes (ns per message, million messages per second), each after other work and after idling; nonstop, messages and batches one after another through owned and lent buffers, and 64 MiB messages through each contender's incremental API in 64 KiB pieces (`PIECE_LEN`; `--points "lent pieces 64 MiB"`); and two scenarios: solo (one copy) and shared (two copies at once, each copy's time a sample). A full run (the default, a minute or a few) measures every point; `--quick` (seconds) stops below 1 MiB and 10,000 messages. The default roster is `DEFAULT_CONTENDERS` (servil, servil mt, sha2, ring: sha2 is the faster SHA-256 at 64-128 B and in every 64-byte batch, ring from 256 B); `--all` adds the rest; the graph opens showing `SHOWN_AT_FIRST` (servil mt, SHA-256 ring, crates.io BLAKE3; Zooko's choice, for a first view with little to untangle). Results land in `bench-hashes.*`: the text report and the graph for users, the samples file for tools and maintainers. The `BLAKE3` column measures the crates.io `blake3` crate maintained by the BLAKE3 authors. The `BLAKE3 servil` column measures the `servil` branch of github.com/johnservil/BLAKE3, a git dependency named `blake3-servil` at the commit `Cargo.lock` pins; with `--config 'patch."https://github.com/johnservil/BLAKE3".blake3-servil.path=".."'` on the cargo command it measures the enclosing checkout at `..` instead (`/workspace` in the VM; see Environment). Cargo then drops the entry's `source` line from `Cargo.lock`, so such builds happen in a copy with a lock of its own (the fork's `tools/perf_regress.py build`); `build.rs` counts that change as clean and reports the checkout's commit and state.

Results land in `benchmark-results/{CPU}.{OS}/` as a text report and an SVG. Every run overwrites them. The fork builds without its SME2 kernel, with a warning, when the assembler lacks SME2. At run time it selects its kernels from the CPU: the SME2 group kernel where the CPU reports SME2 with 512-bit streaming vectors, the integer + NEON hybrid kernels alone elsewhere. Both are real results; the report's kernel table names the platform the run measured.

Vocabulary: an *implementation* is a crate (crates.io `blake3`, the servil fork, `sha2`, ...) and is what `--list` and `--contenders` select. A *use case* is what one call does: hash one message, hash a batch of 64-byte messages, or hash a message arriving in pieces. A *point* is one x on a use case's axis. A *kernel* is the code path an implementation runs at one point, chosen at run time and reported per point. A *mode* is how many threads a contender may use: single-threaded or multithreaded. Availability is a property of the build's platform (CommonCrypto on Apple), never of the machine's capacity; taking part in a use case is a property of the contender (BLAKE3 official mt sits out the many-messages use cases; the fork's multithreaded contenders join them through `hash_many_multithreaded`).

The benchmarker touches an implementation in three ways only: listing it, calling its plain entry point (single-threaded, multithreaded, or a batch call: the fork's `hash_many` and `hash_many_multithreaded`, the crates.io crate's hidden `blake3::platform::Platform::hash_many::<N>` sixteen messages per call; with no cap, pool, or wrapper of its own: many messages means `for m in batch { hash(m) }` for every contender without a batch entry point), and asking the servil fork for `kernel_report()` and its `_many` / `_multithreaded` forms. Contender code is never edited from here; the fork is edited in its own checkout at `..`. It checks no digests (each crate's tests do); the fork's `initialize()` (up to tens of milliseconds, once per process) lands in calibration, outside every timed sample. It asks for no implementation capacity; the crates.io `blake3` kernel table is hand-written from that crate's source because it offers no report. Batch checks compare the SHA-256 of the concatenated per-message digests against `MANY_VECTORS`; `tools/gen-test-vectors.py` writes every table.

# Targets

**The native Mac comes first; virtual machines follow** (the user's decision, September 27, 2026): performance-sensitive programs run on native hosts far more often than in VMs, and the Mac shows what our code does where a VM's layers hide it. The VM records in `benchmark-results/` still matter. Keep both current when a change could move either, and read a difference between them as information about the change, never as noise to be ignored. The fork's host lab measured the platform effects behind such differences (idle-waiter interference, `WFE`, NEON after SME2, core scaling): its `host-lab-reports/`.

# The fork's regression check depends on this benchmark

The fork's `tools/perf_regress.py` builds this benchmark against both sides of every code commit and runs `bench-hashes regress` on the two builds (`PROCEDURES.md` has the details).
