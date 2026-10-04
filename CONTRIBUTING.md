# Working on bench-hashes

This file holds what you need to change bench-hashes, or the BLAKE3 fork
it measures, and keep your results comparable with ours.
[METHODOLOGY.md](METHODOLOGY.md) explains the measurement design.

## Layout

- `src/main.rs`: the benchmark (contenders, timing, statistics, the text
  report); `src/map.rs` and `src/map.html`, the map; `src/b3sum.rs`,
  `bench-hashes b3sum`.
- `src/guide.html`: the offline API decision guide, generated with a full run.
- `build.rs`: embeds the provenance (this repository's commit and state,
  and each contender crate's version and source).
- `tools/graph-check/`: `map.js` drives the map in Chromium, and `guide.js`
  the guide (its README says how).
- `bench-hashes compare OLD.tsv... -- NEW.tsv...`: compares runs'
  samples files, each side's pooled, cell by cell, speed with speed and
  share with share.

## Build and test

```sh
cargo test --release
cargo run --release -- --quick     # seconds; a full run takes minutes
node tools/graph-check/map.js benchmark-results/FOLDER/bench-hashes.map.html
node tools/graph-check/guide-summary.js # the guide's sentences (Playwright)
```

## Adding a contender

Copy what SHA3-256 does: `grep -n Sha3_256 src/main.rs build.rs` lists
every place, and the compiler names any match you miss.

1. Add your crate to `Cargo.toml`, and its provenance to `build.rs`
   (`emit_required_package`, read back as a `*_SOURCE_INFO` constant).
2. Add a variant to `Algorithm` and to `Algorithm::ALL`, with its key
   (for `--contenders`), name, one-line description, colour, provenance,
   and mode (single-threaded or multithreaded). A hash with both modes
   is two contenders, as BLAKE3 servil st and mt are.
3. Give it three calls, each the plain entry point your users call: one
   message (`one_message_call`), a batch of 64-byte messages
   (`hash_batch`; a loop over its one-message call when it has no batch
   entry point), and its incremental API, for long messages read in
   pieces (`hash_stream`) and for many messages at once
   (`hash_interleaved`).
4. Describe its code paths for the report's kernel tables
   (`detect_kernels`); one path at every size is a fine start.

Then `cargo test --release` and
`cargo run --release -- --contenders yours,sha256-ring,blake3-servil-mt`.
The harness does the rest: interleaving, the use cases and scenarios,
statistics, the report, the map, and the guide.

## Working on the fork alongside

The BLAKE3 servil contenders (`blake3-servil-st`, `blake3-servil-mt`) use
the crate `blake3-servil`, a git dependency on the `servil` branch of
[github.com/johnservil/BLAKE3](https://github.com/johnservil/BLAKE3), at
the commit `Cargo.lock` pins. To measure your own checkout of the fork,
put this repository inside it and let the fork's tool build it there:

```sh
git clone --branch servil https://github.com/johnservil/BLAKE3
git clone https://github.com/johnservil/bench-hashes BLAKE3/bench-hashes
cd BLAKE3
pypy3 tools/perf_regress.py build     # prints the executable's path (python3 where PyPy is absent)
```

The tool builds in a directory of its own (`tmp/perf-ab/new/`), with a
copy of this repository whose `Cargo.lock` points at the fork checkout,
and leaves your checkouts' files as they were. Run the executable from a
scratch directory (it writes `benchmark-results/` there); the report
names the fork checkout's commit and whether its tree was clean. The fork's `CONTRIBUTING.md` covers the
fork's own tests and its performance-regression check, which runs this
benchmark.

`cargo update -p blake3-servil` moves the pin to the fork's newest
`servil` commit.

## Rules that keep results comparable

- **Contenders are black boxes.** The benchmark lists a contender, calls
  its plain entry point (single-threaded, multithreaded, a batch call, or
  its incremental API for messages in pieces; for the fork's multithreaded
  contender, its queue for inputs one after another) with no pool, cap,
  or wrapper of its own, and asks the fork for its
  `kernel_report()`. A contender without a batch entry point hashes a
  batch as `for m in batch { hash(m) }`.
- **What the benchmark asks of the fork is frozen.** `FROZEN.md` lists
  each use case's calls and call pattern, and a test compares it with the
  code; changing it is Zooko's decision, recorded there.
- **The benchmark checks no digests.** Each crate's own tests establish
  that it is correct; the benchmark only keeps every digest from being
  optimized away (`black_box`).
- **Results name their code.** Commit before you publish a result: a
  report whose provenance says `dirty-…` measured uncommitted code.
- **Consistency checks** (`bench-hashes.checks.txt`, METHODOLOGY.md):
  relations every contender keeps when the benchmark measures what it
  means to. A change to the benchmark keeps them holding, or explains
  in its commit message why one breaks.
- **The fork's regression check runs `bench-hashes regress OLD NEW`**,
  which runs the two executables with `--contenders`, `--points`, and
  `--rounds` and reads their samples files: the rule lives here, so a
  change to the format changes its reader in the same change.

## Results from other machines

Pull requests that add a machine's results are welcome: one folder under
`benchmark-results/`, the files a run writes there, from a clean commit
of this repository. A second machine of a kind we already have gets a
folder name of its own.

## Text in the map and the report

The map and the report are read by newcomers holding only the page as
well as by regulars: text in them uses words a newcomer knows or the page
introduces, describes the page as it is, and is computed from the run's
own data (it names only contenders the run has). Details go behind the
page's doors (an opened chart, its values under the pointer, the links to
the documentation).
`AGENTS.md`, "Presentation: write each page for a reader who holds only
the page", has the whole practice. Check a change with
`node tools/graph-check/map.js` and by looking at the page.

## Maintainers' notes

`AGENTS.md`, `PROCEDURES.md`, `NEXT-STEPS.md`, and `NOTES.md` are the maintainers' working
notes: environment, current work, and the reasoning behind past
decisions. Contributing needs none of them; `NOTES.md` explains why the
measurement works as it does, should you want to change it.
