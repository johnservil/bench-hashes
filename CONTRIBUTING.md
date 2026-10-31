# Working on bench-hashes

This file holds what you need to change bench-hashes, or the BLAKE3 fork
it measures, and keep your results comparable with ours.
[METHODOLOGY.md](METHODOLOGY.md) explains the measurement design.

## Layout

- `src/main.rs`: the benchmark (contenders, timing, statistics, the text
  report); `src/map.rs` and `src/map.html`, the map; `src/b3sum.rs`,
  `bench-hashes b3sum`.
- `build.rs`: embeds the provenance (this repository's commit and state,
  and each contender crate's version and source).
- `tools/graph-check/`: `map.js` drives the map in Chromium (its README
  says how).
- `bench-hashes compare OLD.tsv... -- NEW.tsv...`: compares runs'
  samples files in pairs (the i-th old with the i-th new), each cell
  judged by its pairs' ratios as `regress` judges.

## Build and test

The tests run under [cargo-nextest](https://nexte.st), each in its own
process (`cargo test` stops at once and says so).

```sh
cargo nextest run --release
cargo run --release -- --quick     # seconds; a full run takes minutes
node tools/graph-check/map.js benchmark-results/FOLDER/bench-hashes.map.html
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
Then `cargo nextest run --release` and
`cargo run --release -- --contenders yours,sha256-ring,blake3-servil-mt`.
The harness does the rest: interleaving, the use cases and scenarios,
statistics, the report, and the map.

## Working on the fork alongside

The BLAKE3 servil contenders (`blake3-servil-st`, `blake3-servil-mt`) use
the crate `blake3-servil`, a git dependency on
[github.com/johnservil/BLAKE3](https://github.com/johnservil/BLAKE3) at
the commit whose full hash `Cargo.toml` names (`rev`). The separate
`clocks` pin names the instrument used on both sides of a comparison.
To measure your own checkout of the fork,
put this repository inside it and let the fork's tool build it there:

```sh
git clone --branch servil https://github.com/johnservil/BLAKE3
git clone https://github.com/johnservil/bench-hashes BLAKE3/bench-hashes
cd BLAKE3/bench-hashes && cargo run --release -- check   # the fork's working tree against its HEAD
```

`check` builds in directories of its own (the fork's `target/check/`), with a
copy of this repository whose `Cargo.lock` points at the fork checkout,
and leaves your checkouts' files as they were. It chooses the cells a
change touches from coverage, with the toolchain's `llvm-profdata` and
`llvm-cov` (`rustup component add llvm-tools`). Run the executable from a
scratch directory (it writes `benchmark-results/` there); the report
names the fork checkout's commit and whether its tree was clean. The fork's `CONTRIBUTING.md` covers the
fork's own tests and its performance-regression check, which runs this
benchmark.

To move either pin, set its `rev` in `Cargo.toml` to the full commit
hash and build: Cargo updates the lock. `build-against`, relinking and
coverage replace the fork while keeping the benchmark's clocks pin.

## Rules that keep results comparable

- **Contenders are black boxes.** The benchmark lists a contender, calls
  its plain entry point (single-threaded, multithreaded, a batch call, or
  its incremental API for messages in pieces; for the fork's multithreaded
  contender, its stream for inputs one after another) with no pool, cap,
  or wrapper of its own. A contender without a batch entry point hashes a
  batch as `for m in batch { hash(m) }`.
- **What the benchmark asks of the fork is frozen.** `FROZEN.md` lists
  each use case's calls and call pattern, and a test compares it with the
  code; changing it is Zooko's decision, recorded there.
- **The benchmark checks no digests.** Each crate's own tests establish
  that it is correct; the benchmark only keeps every digest from being
  optimized away (`black_box`).
- **Results name their code.** Commit before you publish a result: a
  report whose provenance says `dirty-…` measured uncommitted code.
- **The fork's regression check runs `bench-hashes regress OLD... -- NEW...`**,
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

`AGENTS.md` holds the maintainers' principles and the repository's shape;
the open work is in the issues. Contributing needs neither.
`METHODOLOGY.md` explains why the measurement works as it does,
should you want to change it; earlier findings are in `git log`, and in the
notes the tree held until commit `aadeca9d`.

## Releases

The overview shows one release a page, so a release is the moment every
machine runs again. In order:

1. **What it carries:** each entry under FROZEN.md's "Changes for the next
   release", and the two pins (BLAKE3 servil's commit, the clocks), each a
   commit that passed the fork's own checks.
2. **Checks on main:** `cargo nextest run --release`, `cargo fmt --check`,
   and a `--quick` run that reports no busy window.
3. **The tag:** `python3 tools/gen-ver.py X.Y.Z` from a clean tree: one
   commit setting the version to X.Y.Z+<HEAD's hash>, and its tag.
4. **Every machine runs again at the tag,** each on a quiet machine,
   plugged in, with the CPU governor at `performance` (a tuning service
   such as `tuned` can reset it mid-run). A machine's run is:
   - the fork's release check and, beside it, the same check with the same
     library on both sides (`--against` the measured commit, `--old-bench`
     the previous bench). A cell slower in the first and level in the
     second is the library's; slower in both, the bench's;
   - the full run, built against the fork's checkout, never through a
     `--config` patch (Cargo's lock can build the pin instead):
     `bench-hashes build-against ..`, then the executable it prints, with
     `--all --expect-servil COMMIT`, from a scratch directory;
   - the b3sum run, `LAYOUTS=4 python3 tools/b3sum-contenders.py`.

   Check each report names the commit you meant, before you read its numbers.
5. **The results:** each machine's folder under `benchmark-results/` as
   the run wrote it, with SHA-256 and BLAKE3 manifests.
6. **The overview**, once the machines are in: `cargo run --release --
   overview index.html benchmark-results` (its test fails while it is
   stale).
7. **FROZEN.md:** the released entries leave "Changes for the next
   release" and stand under the release's version.
