use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::io::{IsTerminal, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::{CpuRefreshKind, RefreshKind, System};

#[cfg(target_arch = "wasm32")]
compile_error!("bench-hashes currently supports native targets only");

/*
 * Every (contender, size) cell collects complete cycles of contender
 * orders, with samples of about TARGET_SAMPLE_NS each. The two
 * knobs trade off differently:
 *
 * - Fewer rounds thin the evidence behind each median and its speeds: a
 *   second speed held by a few samples goes unseen.
 * - Shorter samples keep the sample count. Any disturbance (an interrupt, a
 *   clock step) is a larger share of a short sample, so it shows as a slow
 *   sample rather than averaging away inside one.
 *
 * So the runtime budget goes to rounds first. 1 ms is long enough that the
 * clock's own resolution (tens of nanoseconds) is under 0.01% of a sample.
 *
 * Sampled visits cycle through the participating contenders' orders and
 * rotate the point that starts a round. Default sample counts are complete
 * Williams cycles. Explicit --rounds samples every round; a multiple of
 * each measured use case's order count balances those orders too.
 */
const FULL_ROUNDS: usize = 96;
/// A quick run (`--quick`): seconds, not minutes, for a first look while
/// developing; a full run, the default, confirms. Its axes stop below
/// QUICK_BYTES and QUICK_MESSAGES.
const QUICK_ROUNDS: usize = 24;
const QUICK_BYTES: usize = 1 << 20;
const QUICK_MESSAGES: usize = 10_000;
const CALIBRATION_PROBE_NS: u128 = 250_000;
/// Measured: 0.5 ms samples ran a full --all in 62 s instead of 80 s but
/// read 1.6% slower across the board (each sample's fixed cost, the duo
/// release and the clock reads, weighs twice as much), so 1 ms stays.
const TARGET_SAMPLE_NS: u128 = 1_000_000;

/*
 * Every cell samples in a share of the rounds, spread over the run at an
 * offset of its own, so drift and other programs' load reach every cell
 * alike: the use case's visits (UseCase::visits).
 * Every participating contender at a point samples the same visits.
 * Thinning long contenders separately aliased their orders and compared
 * different moments.
 *
 * Measured on the VM (September 26, 2026), full default runs old / new /
 * new / old / old / new, cell medians' |log ratio| between runs, solo:
 * - 12 samples instead of 24, doubled while a cell's 95% interval was
 *   wider than 2%: 25 s instead of 47; new against new 1.6%, old against
 *   old 3.5% (median cell). A median varies between runs about as much at
 *   12 samples as at 24 (0.53% against 0.37%; the worst tenth 2.7%
 *   against 2.8%, set by how long a run spends in each state).
 * - Then no doubling: 18.8 s instead of 25; 1.36% against 1.94% (the
 *   worst tenth 7.1% against 5.7%); more cells marked poorly determined.
 *   On the VM the doubling measured the machine's spread of samples, a
 *   tenth to a third of them 10-14% slow, more than a median's
 *   uncertainty, and a 4% threshold saved 1.5 s of the 6.
 */
/*
 * The gap: how long the program does something else before each call of
 * a synchronous use case (FROZEN.md: a program that hashes now and then).
 * Two kinds, each measured (Zooko, September 30, 2026; clocks::Gap):
 * after other work, a fixed other program (about 1 MiB of distinct code),
 * a walk of GAP_WORK_BYTES, then integer work to 1 ms, as on a machine
 * busy with other programs; after idling, a 1 ms sleep, as a server
 * waiting for its next request. Longer than a pool keeps its workers
 * polling (BLAKE3-Servil's, 200 us), so each call meets them asleep.
 */
const GAP_NS: u64 = 1_000_000;
/// The data the other work reads, per measuring thread: larger than the
/// target Mac's caches, as other programs' data fill them. A complete
/// sweep is required even when it outlasts GAP_NS.
const GAP_WORK_BYTES: usize = 128 * 1024 * 1024;
/*
 * How much call time a synchronous cell's sample sums: one call when it
 * lasts this long or more, else as many calls as fill it, each after its
 * own gap and timed alone, after one untimed call
 * (clocks::measure_after_gaps_prepared; the crate's "Resolution" says why
 * a sum of single readings is exact on average). Each call costs a gap of
 * GAP_NS, so this sets the run time of the small cells.
 */
const GAP_SAMPLE_NS: u128 = 2_000;
/// Calls timed after the gap to size a synchronous cell's sample.
const CALIBRATION_GAPS: u64 = 4;
/*
 * Visits a cell takes in a run, by how the cell calls (each visit one solo
 * sample and, where the use case is measured shared, two shared samples
 * beside it), and by what the run is for. A run that measures speed (the
 * map, the report) and a run inside a check for performance regressions
 * (`regress`, which BLAKE3-Servil's perf_regress drives) spend their time
 * differently, and the two counts sit side by side so that a change to
 * one is made looking at the other (Zooko, October 19, 2026: the two had
 * drifted into one constant, tuned for the wrong one).
 *
 * Tuned from eight pairs of A/A runs on an i7-12700K (identical code,
 * fresh link layouts; the data branch of devonjonte/bench-hashes,
 * 2026-10-19/stat41/aa; bench-hashes-validation/bench-design/
 * tune-samples.py): a cell's spread between runs, by samples taken,
 * against its spread at 24 samples. Cells after a gap keep narrowing to
 * 24 (at 12, 1.29x the spread at 24; at 6, 1.75x); nonstop solo cells stop
 * narrowing at 6 (1.04x); nonstop shared cells at 12 samples (1.29x),
 * which 6 visits give.
 */
struct Visits {
    /// Visits of a cell that calls after a gap (after other work, after a pause).
    gap: usize,
    /// Visits of a cell that calls nonstop.
    nonstop: usize,
}

/// To measure speed: the map and the report, for a reader comparing
/// contenders' levels, who takes them as good to about 10%. Between two
/// runs the cells agree to 1-3% at the median and within 10% at the 90th
/// percentile (i7-12700K, eight contenders, about 4 minutes). More visits
/// buy agreement this reader does not need.
const TO_MEASURE_SPEED: Visits = Visits {
    gap: 12,
    nonstop: 2,
};

/// To check for performance regressions: `regress`'s runs, for a reader
/// who must see a 3% change between two builds and be spared false alarms.
/// The cells after a gap, where the noise is, take every sample that still
/// narrows them. On the planted slowdowns kept beside the A/A runs, 24 gap
/// visits caught the rare slow mode 88 times of 138 against 52 at 12,
/// false alarms 0. At 12, `bench-hashes check` (4 pairs) halved a pair's
/// time but caught a hash() plant in 3 of 10 cells after other work and 4
/// of 9 after a pause (October 20; the data branch, 2026-10-20/
/// check-validation).
const TO_CHECK_FOR_REGRESSIONS: Visits = Visits {
    gap: 24,
    nonstop: 2,
};

/*
 * A nonstop cell's visit (FROZEN.md, "Changes for 0.19": continuous means
 * continuous): the cell's own load, untimed, for `warm_ns` (one batch at
 * least), then `samples` samples back to back, each sized as calibration
 * sized it; solo, then the same for the two shared copies, each copy its
 * own train from one release. Each sample then meets the machine as the
 * cell's own continuous load leaves it, where the 1 ms samples of a visit
 * each (6 visits a cell, to 0.18) met it as the contender before left it:
 * a GPU's clock (30-40 ms of load to settle, M4 Max), a core's clock,
 * thread placement. The settling curves (`--train 0,N`, read by the
 * study's tools/settle.py, in the tree until 0.21) on the M4 Max, the
 * Linux VM and the EPYC 9255 set the values.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Train {
    warm_ns: u64,
    samples: usize,
}
const NONSTOP_TRAIN: Train = Train {
    warm_ns: 100_000_000,
    samples: 6,
};

/// Points on the one-message axis, and on each many-messages axis.
const INPUT_COUNT: usize = 28;
const BATCH_COUNT: usize = 24;
/// Points on the continuous axes: message lengths, and messages per batch.
const CONTINUOUS_MESSAGE_COUNT: usize = 13;
const CONTINUOUS_BATCH_COUNT: usize = 7;
/// Points on the axis of batches of 4 KiB messages (FROZEN.md).
const LONG_BATCH_COUNT: usize = 4;
/// The length of each message in a ManyLongMessages batch: four chunks,
/// within the 2 to 15 chunks hash_many hashes across messages.
const LONG_MESSAGE_LEN: usize = 4096;
/// Many messages at once is measured at one point (FROZEN.md).
const INTERLEAVED_COUNT: usize = 1;
/// Collections: git's objects, and the files of Nix store paths (FROZEN.md).
const COLLECTION_COUNT: usize = 2;
const POINT_COUNT: usize = 4 * (INPUT_COUNT + BATCH_COUNT)
    + 2 * CONTINUOUS_MESSAGE_COUNT
    + INTERLEAVED_COUNT
    + 2 * COLLECTION_COUNT
    + 2 * CONTINUOUS_BATCH_COUNT
    + LONG_BATCH_COUNT
    + MERKLE_COUNT
    + 4 * INTERLEAVED_COUNT;
/// A long message reaches a contender in pieces of this many bytes (a
/// typical read buffer), the last one shorter.
const PIECE_LEN: usize = 64 * 1024;

/*
 * Many messages at once (UseCase::Interleaved; Zooko, October 3, 2026): a
 * server receiving many messages from its connections at once, each in
 * pieces, interleaved. `open` messages are open at once and piece k goes
 * to open message k mod `open`; piece k's length is `pieces[k mod len]`,
 * cut at its message's end, and the n-th message opened has length
 * `messages[n mod len]`. When a message ends, its hash is used and the
 * next opens in its place. The pieces: mostly one TCP segment's payload
 * (1448 B), with a page-sized read (4 KiB) and a TLS record (16 KiB); the
 * messages: many short, a few long. The pieces arrive in turns of TURN,
 * as one wait of a server's event loop delivers them, each read into its
 * own buffer; a contender with calls for many messages at once (the
 * servil fork's throughput::update_each and finalize_each) makes one of each per
 * turn, every other one update per piece and one finish per message. The
 * schedule carries over from sample to sample, so samples see the steady
 * state. A sample's unit of work is `chunk` bytes of pieces.
 */
/*
 * A collection of messages of different lengths (UseCase::Collection; Zooko,
 * October 3, 2026): a program naming each item of a collection by its
 * hash, as a content-addressed store, git, Bazel, or Nix does, each item
 * hashed once, one after another, all in memory. Each collection is a
 * table of (count, length): the items of one octave of size at one length,
 * one and a half times the octave's lower bound (off chunk boundaries),
 * their counts in the octave's share of 2048 items. The shares come from
 * real collections (October 3, 2026): git/git's 422,404 objects at
 * c46c1e3 (blobs, trees, commits, tags), and the 192,213 files under 1 MiB
 * in 1000 store paths of nixos-25.05 (every 205th of its 205,358, at
 * nixpkgs ac62194; the files of 1 MiB and more, 1% of the files and 74% of
 * the bytes, are the one-message cells' sizes). The items follow one
 * another in the order a stride gives (item j takes the class of position
 * j x 389 mod 2048), so the sizes mix.
 */
const COLLECTIONS: [(&str, &[(usize, usize)]); COLLECTION_COUNT] = [
    (
        "small mixed",
        &[
            (64, 100),
            (64, 160),
            (64, 220),
            (64, 280),
            (64, 340),
            (64, 400),
            (64, 460),
            (64, 520),
            (64, 580),
            (64, 640),
            (64, 700),
            (64, 760),
            (64, 820),
            (64, 880),
            (64, 940),
            (64, 1000),
        ],
    ),
    (
        "Nix files",
        &[
            (6, 1),
            (1, 6),
            (1, 12),
            (4, 24),
            (11, 48),
            (44, 96),
            (139, 192),
            (133, 384),
            (439, 768),
            (428, 1536),
            (264, 3072),
            (179, 6144),
            (130, 12288),
            (96, 24576),
            (70, 49152),
            (46, 98304),
            (31, 196608),
            (17, 393216),
            (9, 786432),
        ],
    ),
];

/// A collection's messages as (offset, length) in one buffer, in the
/// stride's order, each starting its own whole blocks (UseCase::in_blocks).
fn collection_items(classes: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let lengths: Vec<usize> = classes
        .iter()
        .flat_map(|&(n, len)| std::iter::repeat_n(len, n))
        .collect();
    let count = lengths.len();
    assert!(
        count % 389 != 0 && count.is_power_of_two(),
        "a stride sharing no factor with the count takes every item once"
    );
    let mut offset = 0;
    (0..count)
        .map(|j| {
            let len = lengths[j * 389 % count];
            let at = offset;
            offset += len.next_multiple_of(BLOCK_LEN);
            (at, len)
        })
        .collect()
}

/// A collection's buffer: each message in its own whole blocks.
const fn collection_layout_bytes(classes: &[(usize, usize)]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < classes.len() {
        total += classes[i].0 * classes[i].1.next_multiple_of(BLOCK_LEN);
        i += 1;
    }
    total
}

const fn collection_bytes(classes: &[(usize, usize)]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < classes.len() {
        total += classes[i].0 * classes[i].1;
        i += 1;
    }
    total
}

const fn collection_count(classes: &[(usize, usize)]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < classes.len() {
        total += classes[i].0;
        i += 1;
    }
    total
}

struct InterleavedSpec {
    open: usize,
    pieces: &'static [usize],
    messages: &'static [usize],
    chunk: usize,
    /// One call's pieces after a gap (InterleavedTurn): a turn, up to
    /// TURN pieces in this many bytes (a turn of 64 averages about 306 KB).
    turn: usize,
}

const INTERLEAVED: InterleavedSpec = InterleavedSpec {
    open: 256,
    pieces: &[1448, 1448, 4096, 1448, 16 * 1024],
    messages: &[
        64,
        1000,
        4470,
        16 * 1024,
        100_000,
        1024 * 1024,
        16 * 1024 * 1024,
    ],
    chunk: 1024 * 1024,
    turn: 256 * 1024,
};
/*
 * A Merkle tree (UseCase::MerkleTree; Zooko, October 19, 2026): the
 * commitment a zero-knowledge prover makes to a Reed-Solomon codeword
 * (WHIR's, for one: BLAKE3-Servil's docs/whir-merkle-trees.md): the program
 * writes the codeword's leaves into its buffer, then hashes them, a
 * digest each, and each layer of parents, two children's digests side by
 * side, to the root, keeping every layer for the openings. Leaves of 256
 * bytes, as WHIR's tree of that note; the leaf counts powers of two.
 */
struct MerkleSpec {
    leaf_len: usize,
}
const MERKLE: MerkleSpec = MerkleSpec { leaf_len: 256 };
const MERKLE_COUNT: usize = 5;

/// Every message in the batches is one BLAKE3 block of 64 bytes, the size
/// of a Merkle tree's inner node (two 32-byte children).
const MESSAGE_LEN: usize = 64;

const BENCH_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SOURCE: &str = env!("BENCH_GIT_SOURCE");
const GIT_COMMIT: &str = env!("BENCH_GIT_COMMIT");
const GIT_CLEAN_STATUS: &str = env!("BENCH_GIT_CLEAN_STATUS");

const RUSTC_VERSION: &str = env!("BENCH_RUSTC_VERSION");
const BUILD_TARGET: &str = env!("BENCH_BUILD_TARGET");
const TARGET_FEATURES: &str = env!("BENCH_TARGET_FEATURES");
const LOCK_FINGERPRINT: &str = env!("BENCH_LOCK_FINGERPRINT");
const C_COMPILER: &str = env!("BENCH_C_COMPILER");
const RUSTFLAGS: &str = env!("BENCH_RUSTFLAGS");

const BLAKE3_SOURCE_INFO: &str = env!("BLAKE3_SOURCE_INFO");
const SHA2_SOURCE_INFO: &str = env!("SHA2_SOURCE_INFO");
const RING_SOURCE_INFO: &str = env!("RING_SOURCE_INFO");
const SHA1_CHECKED_SOURCE_INFO: &str = env!("SHA1_CHECKED_SOURCE_INFO");
const SHA3_SOURCE_INFO: &str = env!("SHA3_SOURCE_INFO");
const BLAKE3_SERVIL_SOURCE_INFO: &str = env!("BLAKE3_SERVIL_SOURCE_INFO");
/// The instrument's provenance: the clocks crate that timed the run (build.rs).
const CLOCKS_SOURCE_INFO: &str = env!("CLOCKS_SOURCE_INFO");

/*
 * Every power of two from 64 B to 128 MiB, plus 3 KiB and 3 MiB. Between
 * 64 B and 1 KiB BLAKE3 is inside one chunk; from 2 KiB to 16 KiB its SIMD
 * paths fill up (4-way NEON at 4 KiB, a sixteen-lane SME2 group at 16
 * KiB); above that the bulk rate settles. 3 KiB is where BLAKE3-Servil's integer
 * + NEON hybrid kernels first overtake hardware SHA-256: one chunk on the
 * integer ALUs beside a NEON pair costs the same as the pair alone. SHA-1DC
 * and SHA-256 are block-serial and have only the per-message overhead to
 * show.
 *
 * The sizes past 1 MiB are there to show the plateau: a contender whose
 * 32, 64, and 128 MiB medians agree has levelled out. They matter most
 * for the multithreaded contenders, whose per-call overhead (a pool
 * hand-off, a subtree merge) takes longest to amortise: at 8 MiB the
 * fork's multithreaded rate was still climbing. Everything from 8 MiB up
 * is past the last-level cache on every machine this benchmark targets,
 * so the plateau is the memory-resident one. 3 MiB is to the plateau what
 * 3 KiB is to the SIMD ramp: a tree that is no power of two, whose left
 * subtree is 2 MiB and right 1 MiB, so a splitter that cuts at subtree
 * boundaries hands its threads unequal work there. 16 MiB was measured
 * and dropped: interpolated from 8 and 32 MiB, every contender's median
 * fell within the difference between two runs, on both machines.
 *
 * Many messages at once is measured at one point, 256 messages open
 * (INTERLEAVED). It replaced one long message in pieces (Zooko, October 3,
 * 2026): with no thread lingering between updates, a piece lent to an
 * incremental call hashes as one message of its length does, which the
 * one-message cells show.
 *
 * The continuous messages axis takes a length every factor of four from
 * 64 B to 64 MiB: a program hashing messages of that length one after
 * another as fast as it can (files, records, network objects), each read
 * into a buffer of the program's (a copy) in pieces of up to PIECE_LEN;
 * a sample covers many of them, timed end to end. Every factor of two
 * would add cells for shapes the one-message axis already shows. The
 * continuous batches axis counts 64-byte messages per batch, every
 * factor of four from 16 to 65536 (4 MiB a batch).
 *
 * The many-messages axis counts 64-byte messages per batch, from one to
 * 262144 (16 MiB of input). Powers of two from 1 to 16 show a SIMD batch
 * filling up (the blake3 crate's hash_many takes four blocks at a time on
 * NEON, sixteen with AVX-512); 3, 6, 12, 24, and 48 leave a group
 * partly filled or leave a remainder past sixteen-message groups; from
 * 64 up the per-batch overhead amortises, and the batches past 16384 show
 * the multithreaded batch calls levelling out.
 */
const POINTS: [Point; POINT_COUNT] = [
    Point::one("64 B", 64),
    Point::one("128 B", 128),
    Point::one("256 B", 256),
    Point::one("512 B", 512),
    Point::one("1 KiB", 1024),
    Point::one("1536 B", 1536),
    Point::one("2 KiB", 2 * 1024),
    Point::one("2304 B", 2304),
    Point::one("3 KiB", 3 * 1024),
    Point::one("3839 B", 3839),
    Point::one("4 KiB", 4 * 1024),
    Point::one("4470 B", 4470),
    Point::one("7935 B", 7935),
    Point::one("8 KiB", 8 * 1024),
    Point::one("16 KiB", 16 * 1024),
    Point::one("32 KiB", 32 * 1024),
    Point::one("64 KiB", 64 * 1024),
    Point::one("128 KiB", 128 * 1024),
    Point::one("256 KiB", 256 * 1024),
    Point::one("512 KiB", 512 * 1024),
    Point::one("1 MiB", 1024 * 1024),
    Point::one("2 MiB", 2 * 1024 * 1024),
    Point::one("3 MiB", 3 * 1024 * 1024),
    Point::one("4 MiB", 4 * 1024 * 1024),
    Point::one("8 MiB", 8 * 1024 * 1024),
    Point::one("32 MiB", 32 * 1024 * 1024),
    Point::one("64 MiB", 64 * 1024 * 1024),
    Point::one("128 MiB", 128 * 1024 * 1024),
    Point::many("1", 1),
    Point::many("2", 2),
    Point::many("3", 3),
    Point::many("4", 4),
    Point::many("6", 6),
    Point::many("8", 8),
    Point::many("12", 12),
    Point::many("16", 16),
    Point::many("24", 24),
    Point::many("32", 32),
    Point::many("48", 48),
    Point::many("64", 64),
    Point::many("128", 128),
    Point::many("256", 256),
    Point::many("512", 512),
    Point::many("1024", 1024),
    Point::many("2048", 2048),
    Point::many("4096", 4096),
    Point::many("8192", 8192),
    Point::many("16384", 16384),
    Point::many("32768", 32768),
    Point::many("65536", 65536),
    Point::many("131072", 131072),
    Point::many("262144", 262144),
    // Batches of 4 KiB messages (Zooko, October 24): hash_many's path for
    // messages of 2 to 15 chunks, across the messages at every step of
    // their tree, which no other cell reaches.
    Point::many_long("4", 4),
    Point::many_long("16", 16),
    Point::many_long("64", 64),
    Point::many_long("256", 256),
    Point::one("64 B", 64).idle(),
    Point::one("128 B", 128).idle(),
    Point::one("256 B", 256).idle(),
    Point::one("512 B", 512).idle(),
    Point::one("1 KiB", 1024).idle(),
    Point::one("1536 B", 1536).idle(),
    Point::one("2 KiB", 2 * 1024).idle(),
    Point::one("2304 B", 2304).idle(),
    Point::one("3 KiB", 3 * 1024).idle(),
    Point::one("3839 B", 3839).idle(),
    Point::one("4 KiB", 4 * 1024).idle(),
    Point::one("4470 B", 4470).idle(),
    Point::one("7935 B", 7935).idle(),
    Point::one("8 KiB", 8 * 1024).idle(),
    Point::one("16 KiB", 16 * 1024).idle(),
    Point::one("32 KiB", 32 * 1024).idle(),
    Point::one("64 KiB", 64 * 1024).idle(),
    Point::one("128 KiB", 128 * 1024).idle(),
    Point::one("256 KiB", 256 * 1024).idle(),
    Point::one("512 KiB", 512 * 1024).idle(),
    Point::one("1 MiB", 1024 * 1024).idle(),
    Point::one("2 MiB", 2 * 1024 * 1024).idle(),
    Point::one("3 MiB", 3 * 1024 * 1024).idle(),
    Point::one("4 MiB", 4 * 1024 * 1024).idle(),
    Point::one("8 MiB", 8 * 1024 * 1024).idle(),
    Point::one("32 MiB", 32 * 1024 * 1024).idle(),
    Point::one("64 MiB", 64 * 1024 * 1024).idle(),
    Point::one("128 MiB", 128 * 1024 * 1024).idle(),
    Point::many("1", 1).idle(),
    Point::many("2", 2).idle(),
    Point::many("3", 3).idle(),
    Point::many("4", 4).idle(),
    Point::many("6", 6).idle(),
    Point::many("8", 8).idle(),
    Point::many("12", 12).idle(),
    Point::many("16", 16).idle(),
    Point::many("24", 24).idle(),
    Point::many("32", 32).idle(),
    Point::many("48", 48).idle(),
    Point::many("64", 64).idle(),
    Point::many("128", 128).idle(),
    Point::many("256", 256).idle(),
    Point::many("512", 512).idle(),
    Point::many("1024", 1024).idle(),
    Point::many("2048", 2048).idle(),
    Point::many("4096", 4096).idle(),
    Point::many("8192", 8192).idle(),
    Point::many("16384", 16384).idle(),
    Point::many("32768", 32768).idle(),
    Point::many("65536", 65536).idle(),
    Point::many("131072", 131072).idle(),
    Point::many("262144", 262144).idle(),
    Point::one("64 B", 64).brief_idle(),
    Point::one("128 B", 128).brief_idle(),
    Point::one("256 B", 256).brief_idle(),
    Point::one("512 B", 512).brief_idle(),
    Point::one("1 KiB", 1024).brief_idle(),
    Point::one("1536 B", 1536).brief_idle(),
    Point::one("2 KiB", 2 * 1024).brief_idle(),
    Point::one("2304 B", 2304).brief_idle(),
    Point::one("3 KiB", 3 * 1024).brief_idle(),
    Point::one("3839 B", 3839).brief_idle(),
    Point::one("4 KiB", 4 * 1024).brief_idle(),
    Point::one("4470 B", 4470).brief_idle(),
    Point::one("7935 B", 7935).brief_idle(),
    Point::one("8 KiB", 8 * 1024).brief_idle(),
    Point::one("16 KiB", 16 * 1024).brief_idle(),
    Point::one("32 KiB", 32 * 1024).brief_idle(),
    Point::one("64 KiB", 64 * 1024).brief_idle(),
    Point::one("128 KiB", 128 * 1024).brief_idle(),
    Point::one("256 KiB", 256 * 1024).brief_idle(),
    Point::one("512 KiB", 512 * 1024).brief_idle(),
    Point::one("1 MiB", 1024 * 1024).brief_idle(),
    Point::one("2 MiB", 2 * 1024 * 1024).brief_idle(),
    Point::one("3 MiB", 3 * 1024 * 1024).brief_idle(),
    Point::one("4 MiB", 4 * 1024 * 1024).brief_idle(),
    Point::one("8 MiB", 8 * 1024 * 1024).brief_idle(),
    Point::one("32 MiB", 32 * 1024 * 1024).brief_idle(),
    Point::one("64 MiB", 64 * 1024 * 1024).brief_idle(),
    Point::one("128 MiB", 128 * 1024 * 1024).brief_idle(),
    Point::many("1", 1).brief_idle(),
    Point::many("2", 2).brief_idle(),
    Point::many("3", 3).brief_idle(),
    Point::many("4", 4).brief_idle(),
    Point::many("6", 6).brief_idle(),
    Point::many("8", 8).brief_idle(),
    Point::many("12", 12).brief_idle(),
    Point::many("16", 16).brief_idle(),
    Point::many("24", 24).brief_idle(),
    Point::many("32", 32).brief_idle(),
    Point::many("48", 48).brief_idle(),
    Point::many("64", 64).brief_idle(),
    Point::many("128", 128).brief_idle(),
    Point::many("256", 256).brief_idle(),
    Point::many("512", 512).brief_idle(),
    Point::many("1024", 1024).brief_idle(),
    Point::many("2048", 2048).brief_idle(),
    Point::many("4096", 4096).brief_idle(),
    Point::many("8192", 8192).brief_idle(),
    Point::many("16384", 16384).brief_idle(),
    Point::many("32768", 32768).brief_idle(),
    Point::many("65536", 65536).brief_idle(),
    Point::many("131072", 131072).brief_idle(),
    Point::many("262144", 262144).brief_idle(),
    Point::one("64 B", 64).controlled_idle(),
    Point::one("128 B", 128).controlled_idle(),
    Point::one("256 B", 256).controlled_idle(),
    Point::one("512 B", 512).controlled_idle(),
    Point::one("1 KiB", 1024).controlled_idle(),
    Point::one("1536 B", 1536).controlled_idle(),
    Point::one("2 KiB", 2 * 1024).controlled_idle(),
    Point::one("2304 B", 2304).controlled_idle(),
    Point::one("3 KiB", 3 * 1024).controlled_idle(),
    Point::one("3839 B", 3839).controlled_idle(),
    Point::one("4 KiB", 4 * 1024).controlled_idle(),
    Point::one("4470 B", 4470).controlled_idle(),
    Point::one("7935 B", 7935).controlled_idle(),
    Point::one("8 KiB", 8 * 1024).controlled_idle(),
    Point::one("16 KiB", 16 * 1024).controlled_idle(),
    Point::one("32 KiB", 32 * 1024).controlled_idle(),
    Point::one("64 KiB", 64 * 1024).controlled_idle(),
    Point::one("128 KiB", 128 * 1024).controlled_idle(),
    Point::one("256 KiB", 256 * 1024).controlled_idle(),
    Point::one("512 KiB", 512 * 1024).controlled_idle(),
    Point::one("1 MiB", 1024 * 1024).controlled_idle(),
    Point::one("2 MiB", 2 * 1024 * 1024).controlled_idle(),
    Point::one("3 MiB", 3 * 1024 * 1024).controlled_idle(),
    Point::one("4 MiB", 4 * 1024 * 1024).controlled_idle(),
    Point::one("8 MiB", 8 * 1024 * 1024).controlled_idle(),
    Point::one("32 MiB", 32 * 1024 * 1024).controlled_idle(),
    Point::one("64 MiB", 64 * 1024 * 1024).controlled_idle(),
    Point::one("128 MiB", 128 * 1024 * 1024).controlled_idle(),
    Point::many("1", 1).controlled_idle(),
    Point::many("2", 2).controlled_idle(),
    Point::many("3", 3).controlled_idle(),
    Point::many("4", 4).controlled_idle(),
    Point::many("6", 6).controlled_idle(),
    Point::many("8", 8).controlled_idle(),
    Point::many("12", 12).controlled_idle(),
    Point::many("16", 16).controlled_idle(),
    Point::many("24", 24).controlled_idle(),
    Point::many("32", 32).controlled_idle(),
    Point::many("48", 48).controlled_idle(),
    Point::many("64", 64).controlled_idle(),
    Point::many("128", 128).controlled_idle(),
    Point::many("256", 256).controlled_idle(),
    Point::many("512", 512).controlled_idle(),
    Point::many("1024", 1024).controlled_idle(),
    Point::many("2048", 2048).controlled_idle(),
    Point::many("4096", 4096).controlled_idle(),
    Point::many("8192", 8192).controlled_idle(),
    Point::many("16384", 16384).controlled_idle(),
    Point::many("32768", 32768).controlled_idle(),
    Point::many("65536", 65536).controlled_idle(),
    Point::many("131072", 131072).controlled_idle(),
    Point::many("262144", 262144).controlled_idle(),
    Point::continuous("64 B", 64),
    // 128 B and 512 B too: the stream's kernels for tasks of one member
    // length run at 64 B, 128 B, 256 B, 512 B and 1 KiB (Zooko, October 26;
    // BLAKE3 #183).
    Point::continuous("128 B", 128),
    Point::continuous("256 B", 256),
    Point::continuous("512 B", 512),
    Point::continuous("1 KiB", 1024),
    Point::continuous("4 KiB", 4 * 1024),
    Point::continuous("16 KiB", 16 * 1024),
    Point::continuous("64 KiB", 64 * 1024),
    Point::continuous("256 KiB", 256 * 1024),
    Point::continuous("1 MiB", 1024 * 1024),
    Point::continuous("4 MiB", 4 * 1024 * 1024),
    Point::continuous("16 MiB", 16 * 1024 * 1024),
    Point::continuous("64 MiB", 64 * 1024 * 1024),
    Point::continuous_batch("16", 16),
    Point::continuous_batch("64", 64),
    Point::continuous_batch("256", 256),
    Point::continuous_batch("1024", 1024),
    Point::continuous_batch("4096", 4096),
    Point::continuous_batch("16384", 16384),
    Point::continuous_batch("65536", 65536),
    Point::lent("64 B", 64),
    Point::lent("128 B", 128),
    Point::lent("256 B", 256),
    Point::lent("512 B", 512),
    Point::lent("1 KiB", 1024),
    Point::lent("4 KiB", 4 * 1024),
    Point::lent("16 KiB", 16 * 1024),
    Point::lent("64 KiB", 64 * 1024),
    Point::lent("256 KiB", 256 * 1024),
    Point::lent("1 MiB", 1024 * 1024),
    Point::lent("4 MiB", 4 * 1024 * 1024),
    Point::lent("16 MiB", 16 * 1024 * 1024),
    Point::lent("64 MiB", 64 * 1024 * 1024),
    Point::interleaved("256", INTERLEAVED.chunk),
    Point::collection(0),
    Point::collection(1),
    Point::lent_batch("16", 16),
    Point::lent_batch("64", 64),
    Point::lent_batch("256", 256),
    Point::lent_batch("1024", 1024),
    Point::lent_batch("4096", 4096),
    Point::lent_batch("16384", 16384),
    Point::lent_batch("65536", 65536),
    Point::continuous_collection(0),
    Point::continuous_collection(1),
    Point::merkle("1024", 1 << 10),
    Point::merkle("4096", 1 << 12),
    Point::merkle("16384", 1 << 14),
    Point::merkle("65536", 1 << 16),
    Point::merkle("262144", 1 << 18),
    Point::interleaved_turn("256"),
    Point::interleaved_turn("256").idle(),
    Point::interleaved_turn("256").brief_idle(),
    Point::interleaved_turn("256").controlled_idle(),
];

/// results[contender_index][point_index], contenders in the roster's
/// order; None where the contender takes no part in the point's use case.
type Results = Vec<Vec<Option<Cell>>>;
/// Samples as measured, by contender and point.
type Samples = Vec<Vec<Vec<Measured>>>;
/// Every sample of a run: one solo sample per sample interval, and two
/// shared samples beside it, one per copy.
struct RunSamples {
    solo: Samples,
    /// The shared samples, by placement (Placement::index), then contender
    /// and point.
    shared: [Samples; PLACEMENTS],
    /// Each sample's start on the load windows' scale, by contender and
    /// point. Shared starts belong to the individual copies, after solo.
    solo_started_ns: Vec<Vec<Vec<u64>>>,
    shared_started_ns: [Vec<Vec<Vec<u64>>>; PLACEMENTS],
    /// Each solo sample's process CPU time (DuoCopy::cpu_ns), by contender
    /// and point. Shared samples have none: their two copies share the
    /// process, so its CPU time is both copies'.
    solo_cpu_ns: Vec<Vec<Vec<Option<u64>>>>,
    /// Each solo sample's CPU-package energy in nanojoules
    /// (DuoCopy::energy_nj), as solo_cpu_ns.
    solo_energy_nj: Vec<Vec<Vec<Option<u64>>>>,
    /// Each sample's calling-thread counts per core kind (clocks::Counts),
    /// solo and shared as their samples: which kind of core ran it, and
    /// at what clock. None where the platform counts none.
    solo_counts: Vec<Vec<Vec<Option<clocks::Counts>>>>,
    shared_counts: [Vec<Vec<Vec<Option<clocks::Counts>>>>; PLACEMENTS],
    /// The CPU each sample's thread ended on (DuoCopy::cpu), solo and
    /// shared as their samples.
    solo_cpu: Vec<Vec<Vec<Option<u32>>>>,
    shared_cpu: [Vec<Vec<Vec<Option<u32>>>>; PLACEMENTS],
}

/*
 * The use cases (FROZEN.md). Two synchronous calls, each made now and
 * then, after other work and, as their idle twins, after idling: one
 * message in one buffer; a batch of 64-byte messages (a Merkle tree's
 * inner nodes; a contender with a batch entry point hands it the batch,
 * see hash_batch, every other one loops its plain entry point over it).
 * Five nonstop ones, a program hashing one input after another as fast
 * as it can: messages and batches through buffers the program owns (the
 * queue), and messages, many messages at once in pieces, and batches
 * through buffers it lends to a synchronous call, each read into a buffer of the
 * program's first.
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UseCase {
    OneMessage,
    /// Batches of 64-byte messages.
    ManyMessages,
    /// Batches of LONG_MESSAGE_LEN-byte messages, after other work.
    ManyLongMessages,
    /// The same two calls, each after the program idled LONG_PAUSE_NS
    /// (the first two: each after other work; clocks::Gap).
    IdleOneMessage,
    IdleManyMessages,
    /// The same two calls, each after the program idled BRIEF_PAUSE_NS.
    BriefIdleOneMessage,
    BriefIdleManyMessages,
    /// The same two calls, each after the program idled LONG_PAUSE_NS with
    /// the core's deep idle states switched off (IdleControl), so the
    /// call meets the core awake and the cell measures the library alone;
    /// measured where the bench holds the switch (root).
    ControlledIdleOneMessage,
    ControlledIdleManyMessages,
    /// Messages of the point's length, one after another, each read into
    /// a buffer in pieces of up to PIECE_LEN, then hashed.
    ContinuousMessages,
    /// Batches of the point's count of 64-byte messages, one after
    /// another, each read into a buffer, then hashed.
    ContinuousBatches,
    /// Continuous synchronous calls: the producer lends each buffer until
    /// the call returns, so producing and hashing take turns.
    LentMessages,
    /// Many messages open at once, each arriving in pieces, interleaved
    /// (INTERLEAVED): a server receiving many messages from its
    /// connections, each piece copied as a receive would, then fed to that
    /// message's incremental API.
    Interleaved,
    /// A collection's messages, each hashed once, one after another, in
    /// memory (COLLECTIONS).
    Collection,
    LentBatches,
    /// A collection's messages one after another, each read into the
    /// program's buffer, then hashed (COLLECTIONS): the collection
    /// pipelined, as a program reads a store's files and hashes them.
    ContinuousCollection,
    /// A Merkle tree built over leaves the program has just written into
    /// place (MERKLE): the leaves hashed in one batch, then each layer of
    /// parents (two children's digests) in one batch, to the root, every
    /// layer kept, as a prover commits to a Reed-Solomon codeword.
    MerkleTree,
    /// Many messages at once (Interleaved), one turn a call (up to TURN
    /// pieces, INTERLEAVED.turn bytes), each after other work, as a server
    /// handles a turn of uploads between its other tasks.
    InterleavedTurn,
    /// The same turns, each after idling, as a server waits on the network.
    IdleInterleavedTurn,
    /// The same turns, each after a brief idle.
    BriefIdleInterleavedTurn,
    /// The same turns, each after idling with the deep states off.
    ControlledIdleInterleavedTurn,
}

/// The two pauses before a call "after idling" (Zooko, November 4, 2026;
/// bench-hashes #117): a brief one, where a core keeps its shallowest
/// idle state and its clock, and a long one, where every machine
/// measured has settled into its deep states (the i7-12700K's transition
/// is at 1 ms, the pause until 0.20; the Xeon's and the EPYC's deep
/// plateau begins before). Each is a real program's situation, and each
/// is a stable one. The bench sets its timer slack to 1 us
/// (clocks::tight_timers), so a sleep of 20 us lands at 23 us on Linux
/// where the default slack landed it at 72; the pause's own length is
/// outside the interval.
const BRIEF_PAUSE_NS: u64 = 20_000;
const LONG_PAUSE_NS: u64 = 5_000_000;

impl UseCase {
    const ALL: [UseCase; 21] = [
        UseCase::OneMessage,
        UseCase::ManyMessages,
        UseCase::ManyLongMessages,
        UseCase::IdleOneMessage,
        UseCase::IdleManyMessages,
        UseCase::BriefIdleOneMessage,
        UseCase::BriefIdleManyMessages,
        UseCase::ControlledIdleOneMessage,
        UseCase::ControlledIdleManyMessages,
        UseCase::ContinuousMessages,
        UseCase::ContinuousBatches,
        UseCase::LentMessages,
        UseCase::Interleaved,
        UseCase::Collection,
        UseCase::LentBatches,
        UseCase::ContinuousCollection,
        UseCase::MerkleTree,
        UseCase::InterleavedTurn,
        UseCase::IdleInterleavedTurn,
        UseCase::BriefIdleInterleavedTurn,
        UseCase::ControlledIdleInterleavedTurn,
    ];

    /// Whether each call comes after a gap (the synchronous use cases:
    /// after other work, or after idling), or one follows another (the
    /// continuous ones).
    fn after_gap(self) -> bool {
        matches!(
            self.call(),
            Self::OneMessage | Self::ManyMessages | Self::ManyLongMessages | Self::InterleavedTurn
        )
    }

    /// How the program calls, as the graph's chips name it.
    fn pattern_key(self) -> &'static str {
        if self.controlled() {
            "controlled"
        } else if self.pause_ns() == Some(BRIEF_PAUSE_NS) {
            "brief"
        } else if self.idle() {
            "idle"
        } else if self.after_gap() {
            "busy"
        } else {
            "nonstop"
        }
    }

    /// Whether the call is made with the core's deep idle states off.
    fn controlled(self) -> bool {
        matches!(
            self,
            Self::ControlledIdleOneMessage
                | Self::ControlledIdleManyMessages
                | Self::ControlledIdleInterleavedTurn
        )
    }

    /// Whether the program idles before each call.
    fn idle(self) -> bool {
        self.pause_ns().is_some()
    }

    /// How long the program idles before each call, where it idles.
    fn pause_ns(self) -> Option<u64> {
        match self {
            Self::IdleOneMessage
            | Self::IdleManyMessages
            | Self::IdleInterleavedTurn
            | Self::ControlledIdleOneMessage
            | Self::ControlledIdleManyMessages
            | Self::ControlledIdleInterleavedTurn => Some(LONG_PAUSE_NS),
            Self::BriefIdleOneMessage
            | Self::BriefIdleManyMessages
            | Self::BriefIdleInterleavedTurn => Some(BRIEF_PAUSE_NS),
            _ => None,
        }
    }

    /// The use case whose call this one makes: an idle use case makes its
    /// twin's (the same call, after another gap), every other its own.
    fn call(self) -> UseCase {
        match self {
            Self::IdleOneMessage | Self::BriefIdleOneMessage | Self::ControlledIdleOneMessage => {
                Self::OneMessage
            }
            Self::IdleManyMessages
            | Self::BriefIdleManyMessages
            | Self::ControlledIdleManyMessages => Self::ManyMessages,
            Self::IdleInterleavedTurn
            | Self::BriefIdleInterleavedTurn
            | Self::ControlledIdleInterleavedTurn => Self::InterleavedTurn,
            other => other,
        }
    }

    /// Visits a cell of this use case takes in a run (TO_MEASURE_SPEED, or TO_CHECK_FOR_REGRESSIONS).
    fn visits(self, visits: &Visits) -> usize {
        if self.after_gap() {
            visits.gap
        } else {
            visits.nonstop
        }
    }

    /// Whether the cell's inputs are laid out in blocks, each message
    /// starting a 64-byte boundary with whole 64-byte blocks behind it, for
    /// every contender alike: the batches, and every cell where servil's
    /// call is a throughput call that takes blocks (Zooko, October 24:
    /// align the inputs of every cell that has a performance-call
    /// contender). The pieces cells and the interleaved ones feed servil's
    /// Hasher, which takes any slice.
    fn in_blocks(self) -> bool {
        self.batch()
            || matches!(
                self.call(),
                Self::OneMessage
                    | Self::LentMessages
                    | Self::ContinuousMessages
                    | Self::Collection
                    | Self::ContinuousCollection
            )
    }

    /// Whether a call hashes a batch of messages.
    fn batch(self) -> bool {
        matches!(
            self.call(),
            Self::ManyMessages
                | Self::ManyLongMessages
                | Self::ContinuousBatches
                | Self::LentBatches
        )
    }

    /// The contiguous run of POINTS on this use case's axis.
    fn points(self) -> std::ops::Range<usize> {
        let start = POINTS
            .iter()
            .position(|point| point.use_case == self)
            .expect("each use case has points");
        let end = POINTS
            .iter()
            .rposition(|point| point.use_case == self)
            .unwrap()
            + 1;
        assert!(
            POINTS[start..end]
                .iter()
                .all(|point| point.use_case == self),
            "a use case's points are contiguous"
        );
        start..end
    }

    fn heading(self) -> &'static str {
        match self {
            Self::OneMessage => "A message in one buffer, after other work",
            Self::ManyMessages => "A batch of 64-byte messages, after other work",
            Self::ManyLongMessages => "A batch of 4 KiB messages, after other work",
            Self::IdleOneMessage => "A message in one buffer, after idling 5 ms",
            Self::IdleManyMessages => "A batch of 64-byte messages, after idling 5 ms",
            Self::BriefIdleOneMessage => "A message in one buffer, after idling 20 us",
            Self::BriefIdleManyMessages => "A batch of 64-byte messages, after idling 20 us",
            Self::ControlledIdleOneMessage => {
                "A message in one buffer, after idling 5 ms, deep idle states off"
            }
            Self::ControlledIdleManyMessages => {
                "A batch of 64-byte messages, after idling 5 ms, deep idle states off"
            }
            Self::ContinuousMessages => "Messages one after another, buffers owned",
            Self::ContinuousBatches => "Batches one after another, buffers owned",
            Self::LentMessages => "Messages one after another, buffers lent",
            Self::Interleaved => "Many messages at once, each arriving in pieces, buffers lent",
            Self::InterleavedTurn => {
                "Many messages at once, a turn of their pieces after other work"
            }
            Self::IdleInterleavedTurn => {
                "Many messages at once, a turn of their pieces after idling 5 ms"
            }
            Self::BriefIdleInterleavedTurn => {
                "Many messages at once, a turn of their pieces after idling 20 us"
            }
            Self::ControlledIdleInterleavedTurn => {
                "Many messages at once, a turn of their pieces after idling 5 ms, deep idle states off"
            }
            Self::Collection => "A collection's messages, each hashed once, in memory",
            Self::LentBatches => "Batches one after another, buffers lent",
            Self::ContinuousCollection => {
                "A collection's messages one after another, each read into a buffer, then hashed"
            }
            Self::MerkleTree => {
                "A Merkle tree over leaves just written into place, every layer kept"
            }
        }
    }

    /// The x column's header in the text report.
    fn column(self) -> &'static str {
        match self {
            Self::OneMessage
            | Self::IdleOneMessage
            | Self::BriefIdleOneMessage
            | Self::ControlledIdleOneMessage
            | Self::ContinuousMessages
            | Self::LentMessages => "size",
            Self::Interleaved
            | Self::InterleavedTurn
            | Self::IdleInterleavedTurn
            | Self::BriefIdleInterleavedTurn
            | Self::ControlledIdleInterleavedTurn => "messages open",
            Self::Collection | Self::ContinuousCollection => "collection",
            Self::MerkleTree => "leaves",
            Self::ManyMessages
            | Self::IdleManyMessages
            | Self::BriefIdleManyMessages
            | Self::ControlledIdleManyMessages
            | Self::ManyLongMessages
            | Self::ContinuousBatches
            | Self::LentBatches => "messages",
        }
    }

    /*
     * What a sample is divided by, and the units that follow. Messages:
     * bytes, so time is ns/B and rate GB/s. Batches: messages, so time is
     * ns per message and rate million messages per second. In both, rate
     * = rate_scale / time.
     */
    fn units(self, point: Point, iterations: usize) -> u64 {
        match self {
            Self::OneMessage
            | Self::IdleOneMessage
            | Self::BriefIdleOneMessage
            | Self::ControlledIdleOneMessage
            | Self::ContinuousMessages
            | Self::LentMessages
            | Self::Interleaved
            | Self::Collection
            | Self::ContinuousCollection
            | Self::MerkleTree
            | Self::InterleavedTurn
            | Self::IdleInterleavedTurn
            | Self::BriefIdleInterleavedTurn
            | Self::ControlledIdleInterleavedTurn => point.bytes as u64 * iterations as u64,
            Self::ManyMessages
            | Self::IdleManyMessages
            | Self::BriefIdleManyMessages
            | Self::ControlledIdleManyMessages
            | Self::ManyLongMessages
            | Self::ContinuousBatches
            | Self::LentBatches => point.messages as u64 * iterations as u64,
        }
    }

    /// The unit a sample is per, in the samples file.
    fn unit_key(self) -> &'static str {
        if self.batch() {
            "msg"
        } else {
            "B"
        }
    }

    fn time_unit(self) -> &'static str {
        if self.batch() {
            "ns/msg"
        } else {
            "ns/B"
        }
    }

    /// How the program calls, for a reader of the results.
    /// How the program calls, for the report's opening: the clause after
    /// the tables' names.
    fn pattern(self) -> &'static str {
        if self.controlled() {
            "the program hashes, sleeps 5 ms, and hashes again, with every idle state but the shallowest switched off (the bench runs as root), so the call meets the core awake and the cell measures the library alone; the second call, with the write of its input, is timed"
        } else if self.pause_ns() == Some(BRIEF_PAUSE_NS) {
            "the program hashes, sleeps 20 us, and hashes again, as a server handles a request, waits a moment, and handles the next; the second call, with the write of its input, is timed"
        } else if self.idle() {
            "the program hashes, sleeps 5 ms, and hashes again, as a server handles a request, waits, and handles the next; the second call, with the write of its input, is timed"
        } else if self.after_gap() {
            "the program hashes, runs other code and reads 128 MiB of memory (at least 1 ms), and hashes again, as a program hashes between its other tasks; the second call, with the write of its input, is timed"
        } else {
            "the program hashes one input after another, as fast as it can, alone and with a second program doing the same at once"
        }
    }

    /// The prefix that names this use case's points on the command line
    /// ("interleaved 256"), empty for the first two.
    fn label_prefix(self) -> &'static str {
        match self {
            Self::OneMessage | Self::ManyMessages => "",
            Self::ManyLongMessages => "4 KiB batch ",
            Self::IdleOneMessage | Self::IdleManyMessages => "idle ",
            Self::BriefIdleOneMessage | Self::BriefIdleManyMessages => "brief idle ",
            Self::ControlledIdleOneMessage | Self::ControlledIdleManyMessages => "controlled idle ",
            Self::ContinuousMessages => "continuous ",
            Self::ContinuousBatches => "continuous batch ",
            Self::LentMessages => "lent ",
            Self::Interleaved => "interleaved ",
            Self::InterleavedTurn => "turn ",
            Self::IdleInterleavedTurn => "idle turn ",
            Self::BriefIdleInterleavedTurn => "brief idle turn ",
            Self::ControlledIdleInterleavedTurn => "controlled idle turn ",
            Self::Collection => "collection ",
            Self::LentBatches => "lent batch ",
            Self::ContinuousCollection => "continuous collection ",
            Self::MerkleTree => "merkle ",
        }
    }
}

/// One x-axis point: a message length on the message axes, or a batch of
/// `messages` messages (`bytes` in all) on a batch axis.
#[derive(Clone, Copy)]
struct Point {
    label: &'static str,
    bytes: usize,
    messages: usize,
    use_case: UseCase,
}

impl Point {
    const fn one(label: &'static str, bytes: usize) -> Self {
        Self {
            label,
            bytes,
            messages: 1,
            use_case: UseCase::OneMessage,
        }
    }

    /// The same point, its call made after idling LONG_PAUSE_NS.
    const fn idle(self) -> Self {
        let use_case = match self.use_case {
            UseCase::OneMessage => UseCase::IdleOneMessage,
            UseCase::ManyMessages => UseCase::IdleManyMessages,
            UseCase::InterleavedTurn => UseCase::IdleInterleavedTurn,
            _ => panic!("only the calls after other work have idle twins"),
        };
        Self { use_case, ..self }
    }

    /// The same point, its call made after idling BRIEF_PAUSE_NS.
    const fn brief_idle(self) -> Self {
        let use_case = match self.use_case {
            UseCase::OneMessage => UseCase::BriefIdleOneMessage,
            UseCase::ManyMessages => UseCase::BriefIdleManyMessages,
            UseCase::InterleavedTurn => UseCase::BriefIdleInterleavedTurn,
            _ => panic!("only the calls after other work have idle twins"),
        };
        Self { use_case, ..self }
    }

    /// The same point, its call made after idling LONG_PAUSE_NS with the
    /// deep idle states off.
    const fn controlled_idle(self) -> Self {
        let use_case = match self.use_case {
            UseCase::OneMessage => UseCase::ControlledIdleOneMessage,
            UseCase::ManyMessages => UseCase::ControlledIdleManyMessages,
            UseCase::InterleavedTurn => UseCase::ControlledIdleInterleavedTurn,
            _ => panic!("only the calls after other work have idle twins"),
        };
        Self { use_case, ..self }
    }

    const fn continuous(label: &'static str, bytes: usize) -> Self {
        Self {
            label,
            bytes,
            messages: 1,
            use_case: UseCase::ContinuousMessages,
        }
    }

    const fn many(label: &'static str, messages: usize) -> Self {
        Self {
            label,
            bytes: messages * MESSAGE_LEN,
            messages,
            use_case: UseCase::ManyMessages,
        }
    }

    const fn continuous_batch(label: &'static str, messages: usize) -> Self {
        Self {
            label,
            bytes: messages * MESSAGE_LEN,
            messages,
            use_case: UseCase::ContinuousBatches,
        }
    }

    const fn many_long(label: &'static str, messages: usize) -> Self {
        Self {
            label,
            bytes: messages * LONG_MESSAGE_LEN,
            messages,
            use_case: UseCase::ManyLongMessages,
        }
    }

    /// Each message's length in a batch point: its bytes over its count.
    fn message_len(self) -> usize {
        assert!(
            self.use_case.batch(),
            "{:?} hashes messages of their own lengths",
            self.use_case
        );
        self.bytes / self.messages
    }

    const fn lent(label: &'static str, bytes: usize) -> Self {
        Self {
            label,
            bytes,
            messages: 1,
            use_case: UseCase::LentMessages,
        }
    }

    const fn collection(index: usize) -> Self {
        let (label, classes) = COLLECTIONS[index];
        Self {
            label,
            bytes: collection_bytes(classes),
            messages: collection_count(classes),
            use_case: UseCase::Collection,
        }
    }

    const fn continuous_collection(index: usize) -> Self {
        let mut point = Self::collection(index);
        point.use_case = UseCase::ContinuousCollection;
        point
    }

    const fn interleaved(label: &'static str, bytes: usize) -> Self {
        Self {
            label,
            bytes,
            messages: 1,
            use_case: UseCase::Interleaved,
        }
    }

    /// The same open messages, one turn of their pieces a call.
    const fn interleaved_turn(label: &'static str) -> Self {
        Self {
            label,
            bytes: INTERLEAVED.turn,
            messages: 1,
            use_case: UseCase::InterleavedTurn,
        }
    }

    /// A Merkle tree of `leaves` leaves of MERKLE.leaf_len bytes.
    const fn merkle(label: &'static str, leaves: usize) -> Self {
        Self {
            label,
            bytes: leaves * MERKLE.leaf_len,
            messages: leaves,
            use_case: UseCase::MerkleTree,
        }
    }

    const fn lent_batch(label: &'static str, messages: usize) -> Self {
        Self {
            label,
            bytes: messages * MESSAGE_LEN,
            messages,
            use_case: UseCase::LentBatches,
        }
    }

    /// Whether a quick run measures this point: inputs below QUICK_BYTES,
    /// batches below QUICK_MESSAGES.
    fn quick(&self) -> bool {
        if self.use_case.batch() {
            self.messages < QUICK_MESSAGES
        } else {
            self.bytes < QUICK_BYTES
        }
    }
}

/*
 * A contender is one hash implementation under test: which crate (the
 * crates.io blake3, BLAKE3-Servil, sha2, ...) in which mode
 * (single-threaded or multithreaded). Adding a contender means a variant
 * here, an entry in ALL, a key, a name, a color, a provenance string, and
 * an arm in hash_batch; the harness handles selection, interleaving,
 * and reporting for any count.
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Algorithm {
    Blake3,
    Sha256,
    Sha1Dc,
    /// BLAKE3-Servil's single-threaded hash; its kernels are chosen at
    /// run time (SME2 where the CPU has it, integer + NEON hybrids elsewhere).
    Blake3ServilSt,
    /// Apple's CommonCrypto SHA-256 through CC_SHA256_Init/Update/Final.
    Sha256CommonCrypto,
    /// ring's SHA-256: BoringSSL's assembly, with runtime CPU detection.
    Sha256Ring,
    /// crates.io blake3 through Hasher::update_rayon, the crate's own
    /// multithreading, on Rayon's global pool as Rayon sizes it.
    Blake3Rayon,
    /// BLAKE3-Servil's multithreaded calls and its queue: the caller's thread
    /// plus BLAKE3-Servil's own workers, which sleep between calls.
    Blake3ServilMt,
    /// RustCrypto's SHA3-256 (the sha3 crate), with the ARMv8 SHA-3
    /// instructions where the CPU has them (keccak's run-time detection).
    Sha3_256,
}

impl Algorithm {
    const ALL: [Algorithm; 9] = [
        Algorithm::Blake3,
        Algorithm::Sha256,
        Algorithm::Sha1Dc,
        Algorithm::Blake3ServilSt,
        Algorithm::Sha256CommonCrypto,
        Algorithm::Sha256Ring,
        Algorithm::Blake3Rayon,
        Algorithm::Blake3ServilMt,
        Algorithm::Sha3_256,
    ];

    /// Command-line key, as in `--contenders blake3,sha256-cc`.
    fn key(self) -> &'static str {
        match self {
            Self::Blake3 => "blake3-official",
            Self::Sha256 => "sha256",
            Self::Sha1Dc => "sha1dc",
            Self::Blake3ServilSt => "blake3-servil-st",
            Self::Sha256CommonCrypto => "sha256-cc",
            Self::Sha256Ring => "sha256-ring",
            Self::Blake3Rayon => "blake3-official-mt",
            Self::Blake3ServilMt => "blake3-servil-mt",
            Self::Sha3_256 => "sha3-256",
        }
    }

    /// The bytes of one digest: a Merkle tree's parent is two of them.
    fn digest_len(self) -> usize {
        if self == Self::Sha1Dc {
            20
        } else {
            32
        }
    }

    /// Whether this contender is measured in a use case. BLAKE3 mt stays
    /// out of the batch use cases: `update_rayon` exists for large inputs,
    /// and a 64-byte message is a call to it that no program would make.
    /// BLAKE3-Servil's multithreaded contender takes part through its batch
    /// entry point, `hash_many_multithreaded`, and its stream. BLAKE3
    /// servil st takes part in the continuous use cases as every hash
    /// without a stream does, on the program's thread: a program that keeps
    /// to one thread hashes its inputs so (FROZEN.md).
    fn takes_part(self, use_case: UseCase) -> bool {
        let use_case = use_case.call();
        match use_case {
            UseCase::ManyMessages
            | UseCase::ManyLongMessages
            | UseCase::IdleManyMessages
            | UseCase::BriefIdleManyMessages
            | UseCase::ControlledIdleManyMessages
            | UseCase::LentBatches => !matches!(self, Self::Blake3Rayon),
            UseCase::ContinuousBatches => !matches!(self, Self::Blake3Rayon),
            UseCase::ContinuousMessages => true,
            UseCase::OneMessage
            | UseCase::IdleOneMessage
            | UseCase::BriefIdleOneMessage
            | UseCase::ControlledIdleOneMessage
            | UseCase::LentMessages
            | UseCase::Interleaved
            | UseCase::Collection
            | UseCase::ContinuousCollection => true,
            /* A tree's layers are batches: BLAKE3 official mt has no batch call. */
            UseCase::MerkleTree => !matches!(self, Self::Blake3Rayon),
            UseCase::InterleavedTurn
            | UseCase::IdleInterleavedTurn
            | UseCase::BriefIdleInterleavedTurn
            | UseCase::ControlledIdleInterleavedTurn => true,
        }
    }

    /// Whether this contender can run in this build, or why not. A
    /// property of the target platform alone, so --list reads the same on
    /// every machine of one platform; machine capacity (CPU count, the
    /// CPU's instructions) shows up in the results, never here.
    fn availability(self) -> Result<(), String> {
        match self {
            Self::Blake3
            | Self::Sha256
            | Self::Sha1Dc
            | Self::Sha256Ring
            | Self::Blake3ServilSt
            | Self::Blake3Rayon
            | Self::Blake3ServilMt
            | Self::Sha3_256 => Ok(()),
            Self::Sha256CommonCrypto => {
                if cfg!(target_vendor = "apple") {
                    Ok(())
                } else {
                    Err("CommonCrypto is Apple's system library; this build is not for an Apple platform".to_owned())
                }
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Blake3 => "BLAKE3 official",
            Self::Sha256 => "SHA-256",
            Self::Sha1Dc => "SHA-1DC",
            Self::Blake3ServilSt => "BLAKE3-Servil st",
            Self::Sha256CommonCrypto => "SHA-256 CommonCrypto",
            Self::Sha256Ring => "SHA-256 ring",
            Self::Blake3Rayon => "BLAKE3 official mt",
            Self::Blake3ServilMt => "BLAKE3-Servil mt",
            Self::Sha3_256 => "SHA3-256",
        }
    }

    /*
     * Contender colours stay off pure green and pure red, which the hover
     * panel reserves for "faster" and "slower". A multithreaded contender
     * wears a darker shade of its single-threaded sibling's hue.
     */
    /// The colour of the contender's library: a library's single- and
    /// multi-threaded forms share it, and `striped` tells them apart
    /// (Zooko, October 24: one family, two forms, at a glance).
    fn color(self) -> &'static str {
        match self {
            Self::Blake3 | Self::Blake3Rayon => "#3b82f6",
            Self::Sha256 => "#e07a45",
            Self::Sha1Dc => "#8a7a1e",
            Self::Blake3ServilSt | Self::Blake3ServilMt => "#7c3aed",
            Self::Sha256CommonCrypto => "#0e9aa7",
            Self::Sha256Ring => "#c2410c",
            Self::Sha3_256 => "#db2777",
        }
    }

    /// Whether the contender is a library's multi-threaded form, drawn in
    /// its single-threaded form's colour with a pattern of its own, and
    /// the one kind the shared scenario measures (Scenario::measures).
    fn multithreaded(self) -> bool {
        matches!(self, Self::Blake3Rayon | Self::Blake3ServilMt)
    }

    /// The Cargo.lock description of the crate that implements this contender.
    fn source(self) -> &'static str {
        match self {
            Self::Blake3 => BLAKE3_SOURCE_INFO,
            Self::Sha256 => SHA2_SOURCE_INFO,
            Self::Sha1Dc => SHA1_CHECKED_SOURCE_INFO,
            Self::Blake3ServilSt => BLAKE3_SERVIL_SOURCE_INFO,
            Self::Sha256CommonCrypto => "CommonCrypto CC_SHA256_Init/Update/Final from the running macOS (libSystem); version follows the OS",
            Self::Sha256Ring => RING_SOURCE_INFO,
            Self::Blake3Rayon => BLAKE3_SOURCE_INFO,
            Self::Blake3ServilMt => BLAKE3_SERVIL_SOURCE_INFO,
            Self::Sha3_256 => SHA3_SOURCE_INFO,
        }
    }

    /// The contender's mode: how many threads it may use and how, for the
    /// report header.
    fn mode(self) -> &'static str {
        match self {
            Self::Blake3 => "single-threaded; blake3::hash for one message, the crate's hidden batch function blake3::platform::Platform::hash_many::<N> for a batch, sixteen messages per call",
            Self::Sha256
            | Self::Sha1Dc
            | Self::Sha256CommonCrypto
            | Self::Sha256Ring
            | Self::Sha3_256 => "single-threaded",
            Self::Blake3ServilSt => "single-threaded; convenience::hash for one message after a gap and throughput::hash for one after another, throughput::hash_many for a batch, throughput::Hasher::update per piece for many messages at once",
            Self::Blake3Rayon => "multithreaded; Hasher::update_rayon (per piece, for a stream) on Rayon's global pool, the crate's own multithreading as a program gets it by default: the tree splits recursively over the pool, and inputs under a few chunks stay on the caller's thread",
            Self::Blake3ServilMt => "multithreaded; throughput::hash_multithreaded for one message, throughput::hash_many_multithreaded for a batch, throughput::Hasher::update_multithreaded per piece for many messages at once; for continuous loads its stream: Stream::submit_with for messages, Stream::submit_batch_with for batches, each digest into a place the program keeps: BLAKE3-Servil chooses when to wake its worker threads",
        }
    }
}

/*
 * Time, kept as measured until a reader sees it. A sample is `Measured`:
 * the nanoseconds the clock gave and the units (bytes, or messages) they
 * covered, both exact; the samples file stores them as they are. Every
 * statistic works on `PerUnit`, nanoseconds per unit in fixed point with
 * 64 fractional bits (Q64.64 in a u128): the one division, rounded once,
 * leaves an error of 2^-64 relative, about 20 digits below what a 1 ns
 * clock resolves, and after it sums, differences, and ratios are exact
 * integer arithmetic. Rounding to a unit a person reads happens only in
 * Fixed's display methods. Bounds: a sample below 2^40 ns (18 minutes;
 * the longest recorded, 1.6 s) keeps every value below 2^104, so a
 * product with a permille factor stays within u128.
 */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Measured {
    ns: u64,
    units: u64,
}

impl Measured {
    fn new(ns: u64, units: u64) -> Self {
        assert!(
            ns > 0 && units > 0,
            "a sample covers some units in some time"
        );
        assert!(
            ns < 1 << 40,
            "a sample of {ns} ns exceeds 2^40 ns (18 minutes)"
        );
        Measured { ns, units }
    }
}

/*
 * A cell's figure: its mean, what a caller pays on average, the total time
 * of its retained samples over their total work (clocks::summary), kept
 * exactly as measured: Q64.64 is ample for arithmetic ([`Mean::fixed`]),
 * yet a half-way decimal can move one display tick if approximated first.
 */
#[derive(Clone, Copy)]
struct Mean {
    ns: u128,
    units: u128,
}

impl Mean {
    fn of(samples: &[Measured]) -> Self {
        let (ns, units) = clocks::summary::mean_totals(samples.iter().map(|m| (m.ns, m.units)));
        Self { ns, units }
    }

    fn fixed(self) -> PerUnit {
        Fixed(clocks::summary::mean([(
            u64::try_from(self.ns).expect("a cell's samples total under 2^64 ns"),
            u64::try_from(self.units).expect("a cell's units fit in u64"),
        )]))
    }

    /// The mean times `factor` (ns per `factor` units), rounded once for the
    /// reader ([`format_ns`]).
    fn format_ns(self, factor: u64) -> String {
        format_ns(
            self.ns
                .checked_mul(u128::from(factor))
                .expect("a scaled mean fits in u128"),
            self.units,
        )
    }
}

/// Nanoseconds `numerator / denominator` for a person: three decimals, and
/// more below 0.1 ns so that three significant digits show (six at most),
/// rounded half up once: "0.437", "0.0311", "121.362". The one display rule
/// for time per unit (the tables, compare, regress, leaves); b3sum's whole
/// runs, durations with their units, have their own (b3sum::time).
fn format_ns(numerator: u128, denominator: u128) -> String {
    assert!(denominator > 0, "a figure needs a denominator");
    let mut decimals = 3u32;
    while decimals < 6 && numerator * 10u128.pow(decimals - 2) < denominator {
        decimals += 1;
    }
    let scale = 10u128.pow(decimals);
    let rounded = numerator
        .checked_mul(scale)
        .and_then(|n| n.checked_add(denominator / 2))
        .expect("a rounded figure fits in u128")
        / denominator;
    format!(
        "{}.{:0width$}",
        rounded / scale,
        rounded % scale,
        width = decimals as usize
    )
}

/// A non-negative number in fixed point, 64 integer and 64 fractional bits
/// (Q64.64). Times are Fixed nanoseconds per unit (PerUnit), ratios of
/// times are plain Fixed; arithmetic on them is exact, and each method that
/// leaves Fixed for a person rounds once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Fixed(u128);

/// Nanoseconds per unit (see Measured).
type PerUnit = Fixed;

impl std::ops::Sub for Fixed {
    type Output = Fixed;
    fn sub(self, other: Fixed) -> Fixed {
        Fixed(
            self.0
                .checked_sub(other.0)
                .expect("a difference is never negative here"),
        )
    }
}

impl std::ops::Mul<u64> for Fixed {
    type Output = Fixed;
    fn mul(self, factor: u64) -> Fixed {
        Fixed(
            self.0
                .checked_mul(u128::from(factor))
                .expect("a scaled value fits in u128"),
        )
    }
}

impl Fixed {
    /// The value in nanoseconds for a person ([`format_ns`]).
    fn format_ns(self) -> String {
        format_ns(self.0, 1 << 64)
    }
}

/*
 * The two scenarios every run measures. Solo: one copy of the contender
 * on one thread, the machine otherwise idle. Shared: two independent
 * copies at once, each on its own thread over its own input, so two users
 * of the same code compete for every resource it uses, cores, memory, and
 * an SME unit alike; each copy's own time is a sample.
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scenario {
    Solo,
    Shared(Placement),
}

/// Where the shared scenario's two copies run (Zooko, November 2, 2026;
/// bench-hashes #116): what two programs share decides what they cost
/// each other, and the scheduler chose it for every sample until 0.20
/// (an SMT pair read x1.5, two L3 domains x0.6 of one, Sam Shackleton).
/// Each placement is two physical cores, never one core's two hardware
/// threads: under one last-level cache, or under two (two L3 domains, or
/// two of Apple's clusters, which also means two SME units). A machine
/// with one cache measures the first alone (Topology::pair).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Placement {
    OneCache,
    TwoCaches,
    /// Where the machine offers no placement (macOS, whose threads take no
    /// affinity; one core): the operating system places the copies.
    Os,
}

impl Placement {
    const ALL: [Placement; PLACEMENTS] = [Placement::OneCache, Placement::TwoCaches, Placement::Os];

    fn index(self) -> usize {
        self as usize
    }
}

/// How many placements a shared layer may have (Placement).
const PLACEMENTS: usize = 3;

impl Scenario {
    const ALL: [Scenario; 4] = [
        Scenario::Solo,
        Scenario::Shared(Placement::OneCache),
        Scenario::Shared(Placement::TwoCaches),
        Scenario::Shared(Placement::Os),
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::Shared(Placement::OneCache) => "shared-one-cache",
            Self::Shared(Placement::TwoCaches) => "shared-two-caches",
            Self::Shared(Placement::Os) => "shared-os",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Solo => "Solo",
            Self::Shared(Placement::OneCache) => "Shared, one cache",
            Self::Shared(Placement::TwoCaches) => "Shared, two caches",
            Self::Shared(Placement::Os) => "Shared, placed by the OS",
        }
    }

    /// What a reader of the results needs to know about the scenario.
    fn description(self) -> &'static str {
        match self {
            Self::Solo => "one copy of each contender, on one thread",
            Self::Shared(Placement::OneCache) => "the multithreaded contenders, two copies at once on two cores under one last-level cache, each hashing its own input on its own thread, inputs one after another; the time of each copy",
            Self::Shared(Placement::TwoCaches) => "the multithreaded contenders, two copies at once on two cores under two last-level caches, each hashing its own input on its own thread, inputs one after another; the time of each copy",
            Self::Shared(Placement::Os) => "the multithreaded contenders, two copies at once wherever the operating system runs them (it lets no program choose its CPUs here), each hashing its own input on its own thread, inputs one after another; the time of each copy",
        }
    }

    /// Whether this scenario measures `algorithm` at `use_case`. Two
    /// copies at once measure the multithreaded contenders alone, at the
    /// nonstop use cases alone (bench-hashes #108, November 2, 2026): a
    /// single-threaded hash cost the same beside a neighbour as alone on
    /// every machine measured (x1.00 at the median), apart from where the
    /// scheduler happened to place its two copies; a multithreaded one
    /// shows what its threads do beside a neighbour's, which nothing else
    /// measures (on an EPYC 9255 servil mt ran x0.59 beside one, its pool
    /// kept compact). After a gap, a copy whose twin had just run the same
    /// code met that code in a cache the gap left warm, and read faster
    /// than one program alone ("Shared after a gap"; Zooko, September 30).
    /// A placement the machine lacks measures nothing (Topology::cpus):
    /// where it offers none, the OS places the copies.
    fn measures(self, algorithm: Algorithm, use_case: UseCase) -> bool {
        match self {
            Self::Solo => true,
            Self::Shared(placement) => {
                algorithm.multithreaded()
                    && !use_case.after_gap()
                    && topology().cpus(placement).is_some()
            }
        }
    }
}

/// One (contender, point) cell: measured time per unit in each scenario
/// that measures its use case (Scenario::measures).
#[derive(Clone, Copy)]
struct Cell {
    solo: Mean,
    shared: [Option<Mean>; PLACEMENTS],
}

impl Cell {
    /// Requires `scenario` to measure the cell's use case.
    fn get(&self, scenario: Scenario) -> Mean {
        match scenario {
            Scenario::Solo => self.solo,
            Scenario::Shared(p) => self.shared[p.index()]
                .expect("a shared cell exists for the nonstop use cases alone"),
        }
    }
}

/*
 * The machine's cores and caches, for placing the shared scenario's two
 * copies (Placement). Linux: each CPU's core (topology/core_cpus_list)
 * and last-level cache (cache/index3/shared_cpu_list, or index2 where
 * there is no level 3), from sysfs. macOS: each performance cluster is a
 * cache (hw.perflevel0.cpusperl2 CPUs a cluster, in order); its CPUs
 * cannot be chosen (no affinity), so the pairs are None there and the
 * scheduler places the copies, which each sample's `cpu` records as
 * unknown. The pair for a placement: two CPUs on two different cores,
 * performance cores where the machine has both kinds, under one cache or
 * under two; None where the machine has no such pair.
 */
struct Topology {
    /// Each CPU's core, cache and memory node, by CPU number; empty where
    /// unknown.
    core: Vec<usize>,
    cache: Vec<usize>,
    node: Vec<usize>,
    /// Performance CPUs, where the machine has two kinds (Linux: the
    /// CPUs whose cpu_capacity is the largest); else every CPU.
    fast: Vec<usize>,
}

impl Topology {
    /// From the machine's tables: each CPU's core and cache, and the fast
    /// CPUs (tests).
    #[cfg(test)]
    fn from_tables(
        core: Vec<usize>,
        cache: Vec<usize>,
        node: Vec<usize>,
        fast: Vec<usize>,
    ) -> Topology {
        Topology {
            core,
            cache,
            node,
            fast,
        }
    }

    fn read() -> Topology {
        let mut core = Vec::new();
        let mut cache = Vec::new();
        let mut node = Vec::new();
        let mut capacity = Vec::new();
        for n in 0.. {
            let dir = format!("/sys/devices/system/cpu/cpu{n}");
            let Ok(cores) = fs::read_to_string(format!("{dir}/topology/core_cpus_list")) else {
                break;
            };
            let caches = fs::read_to_string(format!("{dir}/cache/index3/shared_cpu_list"))
                .or_else(|_| fs::read_to_string(format!("{dir}/cache/index2/shared_cpu_list")))
                .unwrap_or_default();
            // A group's identity: its lowest CPU.
            let first = |list: &str| {
                list.split([',', '-'])
                    .next()
                    .and_then(|x| x.trim().parse().ok())
                    .unwrap_or(n)
            };
            core.push(first(&cores));
            cache.push(first(&caches));
            // The CPU's memory node: its `nodeN` entry (one on most machines).
            node.push(
                fs::read_dir(&dir)
                    .ok()
                    .and_then(|entries| {
                        entries.flatten().find_map(|e| {
                            e.file_name().to_str()?.strip_prefix("node")?.parse().ok()
                        })
                    })
                    .unwrap_or(0),
            );
            capacity.push(
                fs::read_to_string(format!("{dir}/cpu_capacity"))
                    .ok()
                    .and_then(|c| c.trim().parse::<u64>().ok())
                    .unwrap_or(1024),
            );
        }
        let top = capacity.iter().copied().max().unwrap_or(1024);
        let fast = (0..core.len()).filter(|&i| capacity[i] == top).collect();
        Topology {
            core,
            cache,
            node,
            fast,
        }
    }

    /// Whether the machine has memory on several nodes (two sockets), so
    /// a copy's input belongs on its CPU's node.
    fn several_nodes(&self) -> bool {
        self.node.iter().any(|&n| n != self.node[0])
    }

    /// The two CPUs for `placement`, or None where the machine has no such
    /// pair (or its CPUs cannot be chosen, and for Os).
    fn pair(&self, placement: Placement) -> Option<[usize; 2]> {
        if placement == Placement::Os {
            return None;
        }
        let a = *self.fast.first()?;
        self.fast
            .iter()
            .copied()
            .find(|&b| {
                self.core[b] != self.core[a]
                    && match placement {
                        Placement::OneCache => self.cache[b] == self.cache[a],
                        Placement::TwoCaches => self.cache[b] != self.cache[a],
                        Placement::Os => unreachable!("Os has no pair"),
                    }
            })
            .map(|b| [a, b])
    }

    /// Where a placement's two copies run: Some(Some(pair)), pinned there;
    /// Some(None), placed by the OS (the Os placement, measured exactly
    /// where the machine offers no pair at all, so no copy thread was ever
    /// pinned); None, not measured.
    fn cpus(&self, placement: Placement) -> Option<Option<[usize; 2]>> {
        match placement {
            Placement::Os => (self.pair(Placement::OneCache).is_none()
                && self.pair(Placement::TwoCaches).is_none())
            .then_some(None),
            _ => self.pair(placement).map(Some),
        }
    }

    /// The CPU pair `pair` of a comparison (regress) pins its measuring
    /// thread to (bench-hashes #69, John Servil's design): one CPU a core,
    /// the fast cores alone, of those `allowed`; pair i takes a core whose
    /// place among them has the parity of i (odd cores pay the busy gap's
    /// DRAM fills that even ones do not, so a change that helps one parity
    /// and hurts the other still shows in half the pairs), the pairs of a
    /// parity turning through the cache domains and their cores. None where
    /// the machine names no cores (no topology, or no thread affinity).
    fn measuring_cpu(&self, pair: usize, allowed: impl Fn(usize) -> bool) -> Option<usize> {
        // Each fast core once, by its lowest allowed CPU, in order.
        let mut cores: Vec<(usize, usize)> = Vec::new(); // (core, CPU)
        for &cpu in &self.fast {
            if allowed(cpu) && !cores.iter().any(|&(core, _)| core == self.core[cpu]) {
                cores.push((self.core[cpu], cpu));
            }
        }
        cores.sort();
        let parity = pair % 2;
        let mut candidates: Vec<(usize, usize)> = cores
            .iter()
            .enumerate()
            .filter(|&(place, _)| place % 2 == parity)
            .map(|(_, &(_, cpu))| (self.cache[cpu], cpu))
            .collect();
        if candidates.is_empty() {
            candidates = cores
                .iter()
                .map(|&(_, cpu)| (self.cache[cpu], cpu))
                .collect();
        }
        if candidates.is_empty() {
            return None;
        }
        // The cache domains in order, each with its cores in order; the
        // pairs of a parity take domain k % domains, and within it, its
        // core (k / domains) % its cores.
        let mut domains: Vec<usize> = candidates.iter().map(|&(cache, _)| cache).collect();
        domains.sort();
        domains.dedup();
        let k = pair / 2;
        let domain = domains[k % domains.len()];
        let in_domain: Vec<usize> = candidates
            .iter()
            .filter(|&&(cache, _)| cache == domain)
            .map(|&(_, cpu)| cpu)
            .collect();
        Some(in_domain[(k / domains.len()) % in_domain.len()])
    }

    /// The header's line: each placement's pair, or "none".
    fn describe(&self) -> String {
        let one = |p: Placement| {
            self.pair(p)
                .map_or("none".to_owned(), |[a, b]| format!("CPUs {a} and {b}"))
        };
        let nodes = if self.several_nodes() {
            "; each copy's input on its CPU's memory node"
        } else {
            ""
        };
        let os = if self.cpus(Placement::Os).is_some() {
            "; the OS places the copies"
        } else {
            ""
        };
        format!(
            "one cache: {}; two caches: {}{nodes}{os}",
            one(Placement::OneCache),
            one(Placement::TwoCaches)
        )
    }
}

/// Whether this process may run on `cpu` (its thread's affinity mask,
/// Linux; elsewhere every CPU).
fn allowed_cpu() -> impl Fn(usize) -> bool {
    #[cfg(target_os = "linux")]
    let mask = {
        unsafe extern "C" {
            fn sched_getaffinity(pid: i32, size: usize, set: *mut u64) -> i32;
        }
        let mut mask = [0u64; 16];
        // Sound: `mask` is a cpu_set_t of 1024 bits, the calling thread (0).
        assert_eq!(
            unsafe { sched_getaffinity(0, core::mem::size_of_val(&mask), mask.as_mut_ptr()) },
            0,
            "read this process's mask"
        );
        Some(mask)
    };
    #[cfg(not(target_os = "linux"))]
    let mask: Option<[u64; 16]> = None;
    move |cpu: usize| mask.map_or(true, |m| cpu < 1024 && m[cpu / 64] >> (cpu % 64) & 1 == 1)
}

fn topology() -> &'static Topology {
    TOPOLOGY.get_or_init(Topology::read)
}

static TOPOLOGY: std::sync::OnceLock<Topology> = std::sync::OnceLock::new();

/// A test's machine, set before anything reads the topology (each test
/// is a process of its own): a Mac's or a one-core machine's, on Linux.
#[cfg(test)]
fn set_topology_for_tests(t: Topology) {
    assert!(
        TOPOLOGY.set(t).is_ok(),
        "a test sets the topology before anything reads it"
    );
}

struct MachineMetadata {
    timestamp: String,
    cpu_type: String,
    cpu_count: usize,
    os_type: String,
    /// What the OS says about the CPU beyond its brand, so two machines
    /// that report the same brand (every Linux VM on Apple silicon says
    /// "aarch64") can be told apart: see cpu_identity.
    cpu_identity: String,
    /// How the OS lets a core sleep when a thread waits, which sets what a
    /// call after a pause costs: see idle_states.
    idle_states: String,
    /// The idle states the core entered in fact under each pause at the
    /// run's start (effective_idle_states), beside the configured ones.
    effective_idle_states: String,
    /// The cpuidle switch the controlled cells use (IdleControl::describe);
    /// a command without controlled cells says so.
    idle_control: String,
    /// Where the shared scenario's two copies ran, by placement
    /// (Topology::describe): "one cache: CPUs 0 and 2; two caches: none".
    placements: String,
    /// Other programs' load during the measuring phase, window by window
    /// (clocks::load), where the OS reports it (Linux, macOS; empty
    /// elsewhere); set once measuring ends.
    load: Vec<clocks::load::Window>,
    /// The power state when the run starts and when measuring ends (set
    /// then), where the OS reports one: see Power.
    power: [Option<Power>; 2],
}

/*
 * The contenders selected for this run, in column order, with the
 * interleaving orders that balance them.
 *
 * Contract: one contender or more (a check of one contender's change runs
 * it alone), each available on this machine, no
 * duplicates.
 */
struct Roster {
    algorithms: Vec<Algorithm>,
    /// The points measured, as ascending indices into POINTS: every point,
    /// or the subset --points names.
    points: Vec<usize>,
    /// Sample rounds.
    rounds: usize,
    /// The run's visits: TO_MEASURE_SPEED, or TO_CHECK_FOR_REGRESSIONS in a
    /// run `regress` starts (--for-regression-check); each cell samples in
    /// its visits of the rounds (cell_wants_sample).
    visits: &'static Visits,
    /// A nonstop cell's visit: its warm-up and its samples (NONSTOP_TRAIN,
    /// or --train's, for the settling and variance studies).
    train: Train,
    /// A nonstop cell's visits when --nonstop-visits sets them.
    nonstop_visits: Option<usize>,
    /// Where the run's sample sizes came from, for its samples file's
    /// `sample sizes` line.
    sample_sizes: String,
    /// The cpuidle switch the controlled cells need; without it the run
    /// measures none of them.
    idle_control: Option<IdleControl>,
}

impl Roster {
    /*
     * `points` restricts the run to those POINTS indices; without it a
     * full run measures every point and a quick run the points below
     * QUICK_BYTES and QUICK_MESSAGES. `rounds` fixes the round count
     * (positive), every cell sampled in each, else FULL_ROUNDS or
     * QUICK_ROUNDS with each cell sampled in its share of them.
     */
    fn new(
        algorithms: Vec<Algorithm>,
        quick: bool,
        points: Option<Vec<usize>>,
        rounds: Option<usize>,
        visits: &'static Visits,
        idle_control: Option<IdleControl>,
    ) -> Self {
        assert!(
            (1..=Algorithm::ALL.len()).contains(&algorithms.len()),
            "a run measures one contender or more, up to every one there is; {} were selected",
            algorithms.len()
        );
        for (index, algorithm) in algorithms.iter().enumerate() {
            assert!(
                !algorithms[..index].contains(algorithm),
                "{} was selected twice",
                algorithm.name()
            );
            if let Err(reason) = algorithm.availability() {
                panic!("{} cannot run here: {reason}", algorithm.name());
            }
        }
        // The controlled cells are measured where the bench holds the
        // switch, as the shared cells where the machine has a pair; named
        // by hand without it, they are a mistake to stop at.
        let points = points.unwrap_or_else(|| {
            (0..POINT_COUNT)
                .filter(|&index| !quick || POINTS[index].quick())
                .filter(|&index| idle_control.is_some() || !POINTS[index].use_case.controlled())
                .collect()
        });
        assert!(
            !points.is_empty() && points.windows(2).all(|w| w[0] < w[1]),
            "points ascend, without repeats"
        );
        if idle_control.is_none() {
            if let Some(&index) = points.iter().find(|&&i| POINTS[i].use_case.controlled()) {
                panic!(
                    "--points: {}{} needs the cpuidle switch (run as root; the idle control line says what is missing)",
                    POINTS[index].use_case.label_prefix(),
                    POINTS[index].label
                );
            }
        }
        let rounds = rounds.unwrap_or(if quick { QUICK_ROUNDS } else { FULL_ROUNDS });
        assert!(rounds > 0, "--rounds must be positive");
        Self {
            algorithms,
            points,
            rounds,
            visits,
            train: NONSTOP_TRAIN,
            nonstop_visits: None,
            sample_sizes: "calibrated in this run".to_owned(),
            idle_control,
        }
    }

    /// Visits a cell of `use_case` takes in this run.
    fn visits_of(&self, use_case: UseCase) -> usize {
        match self.nonstop_visits {
            Some(n) if !use_case.after_gap() => n,
            _ => use_case.visits(self.visits),
        }
    }

    /// Whether this run measures POINTS[point_index].
    fn measures(&self, point_index: usize) -> bool {
        self.points.binary_search(&point_index).is_ok()
    }

    /// The point the progress line follows: the largest one-message
    /// point measured, else the largest point.
    fn progress_point(&self) -> usize {
        *self
            .points
            .iter()
            .rev()
            .find(|&&index| POINTS[index].use_case == UseCase::OneMessage)
            .unwrap_or_else(|| self.points.last().unwrap())
    }

    fn len(&self) -> usize {
        self.algorithms.len()
    }
}

/// Orders over the contenders actually called in a use case. Filtering a
/// larger design afterward can unbalance both positions and predecessors.
fn participating_orders(algorithms: &[Algorithm], use_case: UseCase) -> Vec<Vec<usize>> {
    let participants: Vec<usize> = algorithms
        .iter()
        .enumerate()
        .filter_map(|(index, algorithm)| algorithm.takes_part(use_case).then_some(index))
        .collect();
    match participants.len() {
        0 => Vec::new(),
        1 => vec![participants],
        n => williams_orders(n)
            .into_iter()
            .map(|order| order.into_iter().map(|rank| participants[rank]).collect())
            .collect(),
    }
}

/*
 * A Williams design on n contenders: n orders when n is even, 2n when odd.
 * Every contender takes every position equally often and every ordered
 * adjacency "Y right after X" occurs equally often.
 */
fn williams_orders(n: usize) -> Vec<Vec<usize>> {
    assert!(n >= 2, "a Williams design needs at least two contenders");
    let mut rows: Vec<Vec<usize>> = (0..n)
        .map(|start| {
            (0..n)
                .map(|k| {
                    let offset = if k % 2 == 1 { (k + 1) / 2 } else { n - k / 2 };
                    (start + offset) % n
                })
                .collect()
        })
        .collect();
    if n % 2 == 1 {
        let reversed: Vec<Vec<usize>> = rows
            .iter()
            .map(|row| row.iter().rev().copied().collect())
            .collect();
        rows.extend(reversed);
    }
    assert_orders_balanced(&rows, n);
    rows
}

/*
 * The default contenders: the fastest implementations of BLAKE3 and
 * SHA-256 on the machines this benchmark has measured. For BLAKE3 that is
 * BLAKE3-Servil, single-threaded and multithreaded. For SHA-256 neither
 * crate wins everywhere: sha2 is faster for one and two blocks (and so for
 * every batch of 64-byte messages), ring from 256 B up, so both run.
 */
const DEFAULT_CONTENDERS: [Algorithm; 4] = [
    Algorithm::Blake3ServilSt,
    Algorithm::Blake3ServilMt,
    Algorithm::Sha256,
    Algorithm::Sha256Ring,
];

/*
 * The contenders the graph shows when it opens; a click on a name shows
 * any other. Three lines, for a first view with little to untangle
 * (Zooko, September 25, 2026): the fastest BLAKE3 at every size (servil
 * mt matches servil below its threading threshold), SHA-256 ring (the
 * faster SHA-256 from 256 B up; sha2, faster below and in 64-byte
 * batches, is one click away), and the crates.io BLAKE3 most programs
 * use. A run with none of them opens with every contender shown.
 */
const SHOWN_AT_FIRST: [Algorithm; 3] = [
    Algorithm::Blake3ServilMt,
    Algorithm::Sha256Ring,
    Algorithm::Blake3,
];

/// How the user chose the contenders, for the report header.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    /// DEFAULT_CONTENDERS.
    Default,
    /// `--all`: every contender that can run here.
    All,
    /// `--contenders a,b,c`.
    Explicit,
}

const USAGE: &str = "\
bench-hashes: hash speed by input size and by messages per batch, alone
and, for the multithreaded contenders, as two programs at once

  bench-hashes                     BLAKE3-Servil st and mt
                                   and SHA-256 (sha2 and ring)
  bench-hashes --all               every contender this machine can run
  bench-hashes --contenders K,...  exactly these, in this column order
  bench-hashes --list              contenders and their availability here

Keys: blake3-servil-st, blake3-servil-mt, sha256, sha256-ring,
      blake3-official, blake3-official-mt,
      sha1dc, sha256-cc, sha3-256

A run takes under a minute (the default contenders) or two (--all): every
point, to 128 MiB inputs and batches of 262144 messages, 96 rounds, each
cell sampled in a share of them.

  --quick                          seconds: inputs to 512 KiB and batches to
                                   8192 messages, 24 rounds; may misread a
                                   cell now and then; leaves SHA-1DC out of
                                   --all
  --points LABEL,...               measure only these points (labels as in the
                                   report: \"64 B\", \"8 MiB\", \"1024\" messages,
                                   \"interleaved 256\", \"continuous 64 KiB\",
                                   \"continuous batch 1024\"); with
                                   --contenders only
  --rounds N                       N sample rounds (default 96; --quick 24)
  --for-regression-check           the visits a regression check's run takes
                                   (regress passes it): a cell after a gap in
                                   24 rounds, a nonstop cell in 6 (default:
                                   12 and 6, to measure speed)
  --train W_MS,S                   a nonstop visit's warm-up (ms) and samples
                                   (default 100,6), for settling and variance
                                   studies: 0,N gives settling curves
  --nonstop-visits N               a nonstop cell's visits (default 2)
  --threads N                      BLAKE3-Servil mt on N threads in all, its
                                   caller among them (default: one a CPU); the
                                   results name the count
  --expect-servil COMMIT           stop before measuring unless this build's
                                   BLAKE3-Servil is BLAKE3-Servil at COMMIT (a full
                                   hash), from a clean tree: a run says what it
                                   measures, and refuses to measure anything else

  bench-hashes compare OLD.tsv... -- NEW.tsv...
                                   compare runs' samples files in pairs (the
                                   i-th old with the i-th new), each cell's
                                   verdict as regress gives it, after each
                                   side's builds (as provenance prints them)
  bench-hashes provenance          this build's bench-hashes and BLAKE3-Servil
                                   commits, each with its tree's state
  bench-hashes regress [--full | --contenders A,B [--points P,Q]] OLD_EXE... -- NEW_EXE...
                                   whether the new build is slower: pair i
                                   runs the i-th of each side (fresh
                                   layouts: BLAKE3-Servil's tools/perf_regress.py),
                                   in alternating order; all cells judged at
                                   once; exit 0, 1 (slower), or 2 (no
                                   verdict); --full: every default cell;
                                   --contenders (and --points): the cells
                                   a run with them measures, every one judged

  bench-hashes check --fork DIR    whether a change to BLAKE3-Servil's checkout
                                   is slower than its base (CONTRIBUTING.md)
  bench-hashes build-against DIR   build against BLAKE3-Servil's checkout at DIR
  bench-hashes map DIR             draw a results folder's map again
  bench-hashes chart SAMPLES.tsv   draw a run's chart again
  bench-hashes overview OUT DIR    the overview page from every folder's run
  bench-hashes b3sum ...           time b3sum builds on files (README.md)
  bench-hashes crowd               many programs at once, one a CPU (README.md)
  bench-hashes leaves              a Merkle tree's leaves, four ways to hand
                                   them over (src/leaves.rs)
  Each says its own usage when run without what it needs.
";

struct Options {
    selection: Selection,
    explicit: Vec<Algorithm>,
    points: Option<Vec<usize>>,
    rounds: Option<usize>,
    quick: bool,
    sizes: Sizes,
    /// The file --sizes named, for the samples file's `sample sizes` line.
    sizes_file: Option<String>,
    /// BLAKE3-Servil mt's threads (--threads N; None: one a CPU).
    threads: Option<std::num::NonZeroUsize>,
    /// The run's visits: TO_CHECK_FOR_REGRESSIONS with --for-regression-check
    /// (regress passes it to the runs it starts), else TO_MEASURE_SPEED.
    visits: &'static Visits,
    /// A nonstop visit's warm-up and samples (--train W_MS,S), and a nonstop
    /// cell's visits (--nonstop-visits N): for the settling and variance studies.
    train: Train,
    nonstop_visits: Option<usize>,
}

/// How a run sizes each cell's samples. A comparison's runs share one set
/// of sizes (regress calibrates once, before its pairs), so both sides do
/// the same work a sample and no run pays a calibration of its own (about
/// 18 s of a default run's 24 at one round, i7-12700K).
enum Sizes {
    /// Calibrate, as a run on its own does.
    Calibrate,
    /// Calibrate, write each cell's size to this file, and exit.
    CalibrateOnly(std::path::PathBuf),
    /// Each cell's size as this file gives it, keyed by contender and
    /// point (a cell it lacks stops the run): no calibration, one untimed
    /// sample of each cell in its place, which warms it as calibration did.
    From(crate::Map<(String, usize), usize>),
}

/// A sizes file's first line: a file in another format is read with the
/// tools of the commit that wrote it.
const SIZES_VERSION: &str = "# bench-hashes sizes v1";

impl Sizes {
    fn read(path: &str) -> Sizes {
        let text = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("--sizes: cannot read {path}: {error}"));
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(SIZES_VERSION),
            "--sizes: {path} begins with {SIZES_VERSION:?}"
        );
        // A row for a point this bench lacks is another bench's (check
        // --old-bench merges both sides' sizes): skipped; a cell this run
        // measures without a size stops it below.
        Sizes::From(
            lines
                .filter(|line| !line.starts_with('#'))
                .filter_map(|line| {
                    let fields: Vec<&str> = line.split('\t').collect();
                    assert_eq!(
                        fields.len(),
                        3,
                        "--sizes: {path}: a row of contender, point, iterations: {line}"
                    );
                    Some((
                        (fields[0].to_owned(), point_if_named(fields[1])?),
                        fields[2]
                            .parse()
                            .expect("--sizes: iterations are a whole number"),
                    ))
                })
                .collect(),
        )
    }

    /// The sizes file for these cells' sizes, after its context lines.
    fn write(
        path: &std::path::Path,
        context: &[(String, String)],
        roster: &Roster,
        batch_iterations: &[Vec<usize>],
    ) {
        let mut out = format!("{SIZES_VERSION}\n");
        for (key, value) in context {
            writeln!(out, "# {key}: {value}").unwrap();
        }
        // The calibration's record (when, and of which fork): no part of
        // a cache file's key.
        writeln!(
            out,
            "## calibrated: {}; {BLAKE3_SERVIL_SOURCE_INFO}",
            utc_timestamp()
        )
        .unwrap();
        for (a, algorithm) in roster.algorithms.iter().enumerate() {
            for &p in roster
                .points
                .iter()
                .filter(|&&p| algorithm.takes_part(POINTS[p].use_case))
            {
                writeln!(
                    out,
                    "{}\t{}{}\t{}",
                    algorithm.key(),
                    POINTS[p].use_case.label_prefix(),
                    POINTS[p].label,
                    batch_iterations[a][p]
                )
                .unwrap();
            }
        }
        fs::write(path, out).unwrap_or_else(|error| {
            panic!("--calibrate-only: cannot write {}: {error}", path.display())
        });
    }
}

/*
 * The calibration cache (Zooko, October 31, 2026: calibrating took 54 s of
 * a full run's 230 on the Mac). A run that calibrates writes its sizes,
 * with what they depend on, to a file of the user's cache directory named
 * for that context; a later run in the same context takes its sizes from
 * it and calibrates nothing. A size is how many calls fill a sample, so
 * what makes one wrong is a change to what a cell does: the bench's own
 * source, its commit and clean status (a dirty tree's status fingerprints
 * its diff; the committed lock pins every contender's version). The
 * context is that, the machine (its CPU, their count and identity, the
 * OS), the power source and mode at the start, and the run's contenders, points and
 * threads; each context has a file of its own, kept, so a context that
 * comes back (battery, mains, battery) finds its sizes still there. The
 * code under test is no part of it (Zooko, October 31): a change to it
 * makes a cell faster or slower, a slower one's samples longer, which
 * costs nothing but time, and a faster one's shorter, which the run's own
 * check catches (sizes_stale): a nonstop cell whose median sample came out
 * under a quarter of its target deletes the file, so the next run
 * calibrates again.
 */
fn sizes_context(
    roster: &Roster,
    machine: &MachineMetadata,
    threads: Option<std::num::NonZeroUsize>,
) -> Vec<(String, String)> {
    [
        (
            "bench-hashes".to_owned(),
            format!("{BENCH_VERSION}; {GIT_COMMIT}; {GIT_CLEAN_STATUS}"),
        ),
        (
            "machine".to_owned(),
            format!(
                "{}; {} CPUs; {}; {}",
                machine.cpu_type, machine.cpu_count, machine.cpu_identity, machine.os_type
            ),
        ),
        (
            "power".to_owned(),
            // The source and the mode; the battery's charge changes by the
            // minute and sizes nothing.
            machine.power[0]
                .as_ref()
                .map_or("not reported".to_owned(), |p| {
                    Power {
                        battery_percent: None,
                        ..p.clone()
                    }
                    .describe()
                }),
        ),
        (
            "contenders".to_owned(),
            roster
                .algorithms
                .iter()
                .map(|a| a.key())
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "points".to_owned(),
            roster
                .points
                .iter()
                .map(|&p| format!("{}{}", POINTS[p].use_case.label_prefix(), POINTS[p].label))
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "threads".to_owned(),
            threads.map_or("one a CPU".to_owned(), |n| n.to_string()),
        ),
    ]
    .into()
}

/// The cache file for a context: in $XDG_CACHE_HOME/bench-hashes, or
/// ~/.cache/bench-hashes; None where neither is set.
fn sizes_cache_path(context: &[(String, String)]) -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    let text: String = context.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
    let name = &blake3::hash(text.as_bytes()).to_hex()[..16];
    Some(dir.join("bench-hashes").join(format!("sizes-{name}.tsv")))
}

/// The cached sizes for a context, where a file holds them with that very
/// context.
fn cached_sizes(path: &std::path::Path, context: &[(String, String)]) -> Option<(Sizes, String)> {
    let text = fs::read_to_string(path).ok()?;
    let record = text
        .lines()
        .find_map(|l| l.strip_prefix("## calibrated: "))
        .unwrap_or("at an unrecorded time")
        .to_owned();
    let lines: Vec<&str> = text
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("# "))
        .collect();
    let wanted: Vec<String> = context.iter().map(|(k, v)| format!("# {k}: {v}")).collect();
    (lines == wanted).then(|| (Sizes::read(path.to_str().expect("a UTF-8 path")), record))
}

/// Whether a run on cached sizes found them stale: a nonstop cell,
/// sampled more than one call or batch at a time, whose median sample came
/// out under a quarter of its target (the code under test some 4 times
/// faster than when the sizes were calibrated). A calibration's own cells
/// reach half their target (Mac jobs 1786 and 1791: the least 0.49 and
/// 0.67), so a quarter leaves a margin of two. A sample after a gap sums
/// calls each timed after its gap, whose length bears on nothing but the
/// run's time.
fn sizes_stale(roster: &Roster, samples: &RunSamples, iterations: &[Vec<usize>]) -> Option<String> {
    for a in 0..roster.len() {
        for &p in &roster.points {
            let cell = &samples.solo[a][p];
            if cell.is_empty() || iterations[a][p] <= 1 || POINTS[p].use_case.after_gap() {
                continue;
            }
            let mut ns: Vec<u64> = cell.iter().map(|m| m.ns).collect();
            ns.sort_unstable();
            let target = TARGET_SAMPLE_NS;
            let median = u128::from(ns[ns.len() / 2]);
            if median * 4 < target || median > target * 4 {
                return Some(format!(
                    "{} {}{}: median sample {} ns, target {target} ns, a quarter to four times it",
                    roster.algorithms[a].key(),
                    POINTS[p].use_case.label_prefix(),
                    POINTS[p].label,
                    ns[ns.len() / 2]
                ));
            }
        }
    }
    None
}

/// A build's provenance in one line: bench-hashes' commit and BLAKE3
/// servil's, each with its tree's state (`clean`, or a dirty tree's
/// fingerprint), as the build records them (`servil_info` is
/// BLAKE3_SERVIL_SOURCE_INFO, or "" where a run measured no servil).
/// `compare` and `regress` print each side's, and `build-against` names
/// its executable's directory after it (`bench-hashes provenance`).
fn provenance(bench_commit: &str, bench_status: &str, servil_info: &str) -> String {
    let servil = match servil_info
        .split("; ")
        .find_map(|f| f.strip_prefix("commit "))
    {
        Some(commit) => format!(
            "servil {} {}",
            &commit[..12],
            servil_info
                .rsplit("; ")
                .next()
                .expect("a status after the commit")
        ),
        None => "no servil".to_owned(),
    };
    format!("bench {} {bench_status}; {servil}", &bench_commit[..12])
}

/// A samples file's provenance (provenance), from its headers.
fn file_provenance(file: &SamplesFile) -> String {
    let header = |key: &str| {
        file.headers
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    };
    let servil = file
        .headers
        .iter()
        .find(|(k, _)| k.starts_with("source blake3-servil"))
        .map_or("", |(_, v)| v.as_str());
    provenance(header("git commit"), header("git clean status"), servil)
}

/// Each side's builds, as `compare` and `regress` print them (`tool: old
/// side: ...`), and a line saying so where both sides are one build: the
/// comparison is then an A/A, whether or not it was meant as one.
fn print_sides(tool: &str, old: &[String], new: &[String]) {
    print!("{}", sides(tool, old, new));
}

fn sides(tool: &str, old: &[String], new: &[String]) -> String {
    let mut text = format!(
        "{tool}: old side: {}\n{tool}: new side: {}\n",
        old.join(" | "),
        new.join(" | ")
    );
    if old == new {
        text += &format!("{tool}: both sides are one build: an A/A comparison\n");
    }
    text
}

/// Stops unless `info` (this build's BLAKE3_SERVIL_SOURCE_INFO) names the
/// fork at `commit` from a clean tree: `--expect-servil`.
fn expect_servil(commit: &str, info: &str) {
    assert!(
        commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "--expect-servil takes a full 40-hex commit, not {commit:?}"
    );
    let named = info
        .split("; ")
        .any(|field| field == format!("commit {commit}"));
    let clean = info.split("; ").any(|field| field == "clean");
    assert!(
        named && clean,
        "--expect-servil {commit}: this build's BLAKE3-Servil is not that commit from a clean tree; it is: {info}\n\
         (build against a checkout with `bench-hashes build-against DIR`; a `--config` patch Cargo leaves unused builds the pin)"
    );
}

fn parse_arguments() -> Options {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();

    /* --quick may accompany any selection. */
    let mut take_flag = |flag: &str| {
        arguments
            .iter()
            .position(|argument| argument == flag)
            .map(|index| {
                arguments.remove(index);
            })
            .is_some()
    };
    let quick = take_flag("--quick");
    let for_check = take_flag("--for-regression-check");

    /* --points LIST and --rounds N narrow a --contenders run. */
    let mut take_value = |flag: &str| {
        arguments
            .iter()
            .position(|argument| argument == flag)
            .map(|index| {
                assert!(
                    index + 1 < arguments.len(),
                    "{flag} needs a value\n\n{USAGE}"
                );
                let value = arguments[index + 1].clone();
                arguments.drain(index..=index + 1);
                value
            })
    };
    let points = take_value("--points").map(|list| {
        let mut indices: Vec<usize> = list.split(',').map(point_named).collect();
        indices.sort_unstable();
        indices.dedup();
        indices
    });
    let rounds =
        take_value("--rounds").map(|n| n.parse::<usize>().expect("--rounds takes a whole number"));
    let visits: &'static Visits = if for_check {
        &TO_CHECK_FOR_REGRESSIONS
    } else {
        &TO_MEASURE_SPEED
    };
    /* --train W_MS,S and --nonstop-visits N: the settling and variance studies. */
    let train = take_value("--train").map_or(NONSTOP_TRAIN, |value| {
        let (warm, samples) = value
            .split_once(',')
            .expect("--train takes W_MS,S: a warm-up in ms and samples a visit");
        Train {
            warm_ns: warm.parse::<u64>().expect("--train's warm-up: whole ms") * 1_000_000,
            samples: samples
                .parse::<usize>()
                .expect("--train's samples: a whole number"),
        }
    });
    assert!(
        train.samples > 0,
        "--train takes one sample a visit or more"
    );
    let nonstop_visits = take_value("--nonstop-visits").map(|n| {
        n.parse::<usize>()
            .expect("--nonstop-visits takes a whole number")
    });
    assert!(
        nonstop_visits != Some(0),
        "--nonstop-visits takes one visit or more"
    );
    /* --threads N: BLAKE3-Servil mt's pool, N threads in all (its caller among them). */
    let threads = take_value("--threads").map(|n| {
        n.parse::<std::num::NonZeroUsize>()
            .expect("--threads takes a whole number above 0")
    });
    /* --expect-servil COMMIT: this build's servil is BLAKE3-Servil at COMMIT,
    clean, or nothing is measured (a Cargo patch that goes unused builds the
    pin instead, and says so only in a warning: BLAKE3 #125, October 27). */
    if let Some(commit) = take_value("--expect-servil") {
        expect_servil(&commit, BLAKE3_SERVIL_SOURCE_INFO);
    }
    /* --calibrate-only PATH or --sizes PATH may accompany any selection. */
    let sizes_file = take_value("--sizes");
    let sizes = match (take_value("--calibrate-only"), &sizes_file) {
        (None, None) => Sizes::Calibrate,
        (Some(path), None) => Sizes::CalibrateOnly(path.into()),
        (None, Some(path)) => Sizes::read(path),
        (Some(_), Some(_)) => {
            panic!("--calibrate-only writes sizes and --sizes reads them; take one\n\n{USAGE}")
        }
    };

    let (selection, explicit) = parse_selection(&arguments);
    assert!(
        (points.is_none() && rounds.is_none()) || selection == Selection::Explicit,
        "--points and --rounds narrow a --contenders run\n\n{USAGE}"
    );
    Options {
        selection,
        explicit,
        points,
        rounds,
        quick,
        sizes,
        threads,
        visits,
        train,
        nonstop_visits,
        sizes_file,
    }
}

/*
 * The point a `--points` name names: a label's prefix names its use cases
 * ("interleaved 256", "idle 16", "continuous batch 1024"), the longest
 * prefix that matches, the plain label the calls after other work.
 */
fn point_named(name: &str) -> usize {
    point_if_named(name).unwrap_or_else(|| {
        let (prefix, _) = prefix_and_label(name.trim());
        let labels: Vec<String> = POINTS
            .iter()
            .filter(|point| point.use_case.label_prefix() == prefix)
            .map(|point| format!("{prefix}{}", point.label))
            .collect();
        panic!(
            "--points: no point {name:?}; the points named so are {}",
            labels.join(", ")
        )
    })
}

/// A `--points` name's use-case prefix (the longest that matches) and the
/// label after it.
fn prefix_and_label(name: &str) -> (&'static str, &str) {
    UseCase::ALL
        .into_iter()
        .map(UseCase::label_prefix)
        .filter(|prefix| name.starts_with(prefix))
        .max_by_key(|prefix| prefix.len())
        .map(|prefix| (prefix, &name[prefix.len()..]))
        .expect("the plain prefix matches every label")
}

/// The point a `--points` name names, or None where this bench has no
/// such point (a sizes file written beside another bench, for a cell this
/// one lacks: check --old-bench).
fn point_if_named(name: &str) -> Option<usize> {
    let name = name.trim();
    let (prefix, label) = prefix_and_label(name);
    POINTS
        .iter()
        .position(|point| point.label == label && point.use_case.label_prefix() == prefix)
}

fn parse_selection(arguments: &[String]) -> (Selection, Vec<Algorithm>) {
    match arguments {
        [] => (Selection::Default, Vec::new()),
        [flag] if flag == "--all" => (Selection::All, Vec::new()),
        [flag] if flag == "--list" => {
            for algorithm in Algorithm::ALL {
                let status = match algorithm.availability() {
                    Ok(()) => "available".to_owned(),
                    Err(reason) => format!("unavailable: {reason}"),
                };
                println!(
                    "  {:<20} {:<22} {status}",
                    algorithm.key(),
                    algorithm.name()
                );
            }
            std::process::exit(0);
        }
        [flag, keys] if flag == "--contenders" => {
            let algorithms = keys
                .split(',')
                .map(|key| {
                    Algorithm::ALL
                        .into_iter()
                        .find(|algorithm| algorithm.key() == key.trim())
                        .unwrap_or_else(|| {
                            eprintln!("unknown contender {key:?}\n\n{USAGE}");
                            std::process::exit(2);
                        })
                })
                .collect();
            (Selection::Explicit, algorithms)
        }
        [flag] if flag == "--help" || flag == "-h" => {
            print!("{USAGE}");
            std::process::exit(0);
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Hash maps and sets with one fixed seed, in place of std's per-process
/// random one: a run's tables, their growth and their iteration order, are
/// the same in every run, so neither a measurement nor its report differs
/// by a draw. WyHash, as zooko/smalloc's devutils hashes: seeded and fast.
/// Every map in the bench is one of these (the test
/// `every_map_is_seeded` keeps it so).
pub(crate) type Map<K, V> = std::collections::HashMap<K, V, Seeded>;
pub(crate) type Set<K> = std::collections::HashSet<K, Seeded>;

#[derive(Clone, Copy, Default)]
pub(crate) struct Seeded;

impl std::hash::BuildHasher for Seeded {
    type Hasher = wyhash::WyHash;
    fn build_hasher(&self) -> wyhash::WyHash {
        wyhash::WyHash::with_seed(0x6A09_E667_F3BC_C908)
    }
}

fn main() {
    // Once per process, before any other call: blake3-servil's contract.
    blake3_servil::initialize();
    // The pauses before calls land when asked (clocks::tight_timers; the
    // pool's threads, made later, inherit it).
    clocks::tight_timers();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("compare") {
        return compare_command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("provenance") {
        assert!(arguments.len() == 1, "usage: bench-hashes provenance");
        println!(
            "{}",
            provenance(GIT_COMMIT, GIT_CLEAN_STATUS, BLAKE3_SERVIL_SOURCE_INFO)
        );
        return;
    }
    if arguments.first().map(String::as_str) == Some("check") {
        std::process::exit(check::command(&arguments[1..]));
    }
    if arguments.first().map(String::as_str) == Some("build-against") {
        std::process::exit(check::build_against_command(&arguments[1..]));
    }
    if arguments.first().map(String::as_str) == Some("regress") {
        std::process::exit(regress_command(&arguments[1..]));
    }
    if arguments.first().map(String::as_str) == Some("b3sum") {
        return b3sum::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("leaves") {
        return leaves::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("crowd") {
        return crowd::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("chart") {
        return chart::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("overview") {
        overview::command(&arguments[1..]);
        return;
    }
    if arguments.first().map(String::as_str) == Some("map") {
        return map::command(&arguments[1..]);
    }
    let Options {
        selection,
        explicit,
        points,
        rounds,
        quick,
        sizes,
        threads,
        visits,
        train,
        nonstop_visits,
        sizes_file,
    } = parse_arguments();
    let mut machine = machine_metadata();

    let (algorithms, selection_note) = match selection {
        Selection::Default => (
            DEFAULT_CONTENDERS.to_vec(),
            String::from("the default contenders"),
        ),
        Selection::All => (
            /* SHA-1DC, the slowest by far, runs in quick runs only when named. */
            Algorithm::ALL
                .into_iter()
                .filter(|algorithm| algorithm.availability().is_ok())
                .filter(|&algorithm| !quick || algorithm != Algorithm::Sha1Dc)
                .collect(),
            String::from("every contender available on this machine"),
        ),
        Selection::Explicit => {
            let keys = explicit
                .iter()
                .map(|algorithm| algorithm.key())
                .collect::<Vec<_>>()
                .join(",");
            (explicit, format!("--contenders {keys}"))
        }
    };
    // The thread count is the run's, so its contenders' note names it.
    let selection_note = match threads {
        Some(n) => format!("{selection_note}; --threads {n}"),
        None => selection_note,
    };
    // A run measured another way than the frozen one says so (the settling
    // and variance studies: --train, --nonstop-visits).
    let selection_note = if train != NONSTOP_TRAIN || nonstop_visits.is_some() {
        format!(
            "{selection_note}; nonstop visits: {} ms warm-up, {} samples a visit{}",
            train.warm_ns / 1_000_000,
            train.samples,
            nonstop_visits.map_or(String::new(), |n| format!(", {n} visits a cell"))
        )
    } else {
        selection_note
    };
    // A program that calls BLAKE3-Servil's multithreaded entry points or its
    // queue starts their threads once, first: BLAKE3-Servil's contract.
    if algorithms.contains(&Algorithm::Blake3ServilMt) {
        blake3_servil::initialize_multithreaded(threads);
    }
    let idle_control = IdleControl::detect(std::path::Path::new("/sys/devices/system/cpu"));
    machine.idle_control = IdleControl::describe(idle_control.as_ref());
    let mut roster = Roster::new(algorithms, quick, points, rounds, visits, idle_control);
    roster.train = train;
    roster.nonstop_visits = nonstop_visits;
    // Sizes from the cache where a calibration in this very context left
    // them (sizes_context); a run given --sizes or --calibrate-only does
    // as it is told.
    let context = sizes_context(&roster, &machine, threads);
    let cache = matches!(sizes, Sizes::Calibrate)
        .then(|| sizes_cache_path(&context))
        .flatten();
    let cached = cache
        .as_deref()
        .and_then(|path| cached_sizes(path, &context));
    let from_cache = cached.is_some();
    let cache_name = cache
        .as_deref()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
    roster.sample_sizes = match (&cached, &sizes_file, &cache_name) {
        (Some((_, record)), _, Some(name)) => {
            format!("from the calibration cache ({name}), calibrated {record}")
        }
        (None, Some(file), _) => format!("from {file} (--sizes)"),
        (None, None, Some(name)) => format!("calibrated in this run, cached as {name}"),
        _ => "calibrated in this run".to_owned(),
    };
    let sizes = cached.map_or(sizes, |(sizes, _)| sizes);
    let (results, samples, load, iterations) = measure_all(&roster, &sizes);
    if let Some(path) = &cache {
        if from_cache {
            if let Some(why) = sizes_stale(&roster, &samples, &iterations) {
                eprintln!("bench-hashes: the cached sample sizes ran short ({why}); the next run calibrates again");
                let _ = fs::remove_file(path);
            }
        } else {
            fs::create_dir_all(path.parent().unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            // Written whole, then renamed into place: a run reading the
            // cache meanwhile finds the file complete or absent.
            let part = path.with_extension(format!("part{}", std::process::id()));
            Sizes::write(&part, &context, &roster, &iterations);
            fs::rename(&part, path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
    machine.load = load;
    machine.power[1] = Power::read();

    let text = generate_text(&roster, &results, &machine, &selection_note);
    print!("{text}");

    let directory = output_directory(&machine);

    fs::create_dir_all(&directory).unwrap_or_else(|error| {
        panic!(
            "failed to create output directory {}: {error}",
            directory.display(),
        )
    });

    let stem = "bench-hashes";
    let text_path = directory.join(format!("{stem}.result.txt"));
    let samples_path = directory.join(format!("{stem}.samples.tsv"));
    let samples = generate_samples_tsv(&roster, &samples, &machine, &selection_note);
    fs::write(&samples_path, &samples)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", samples_path.display()));

    fs::write(&text_path, &text)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", text_path.display()));

    println!("# Data results (text) are in \"{}\" .", text_path.display(),);
    println!("# Samples (TSV) are in \"{}\" .", samples_path.display());
    map::write(&directory, None);
    overview::refresh(&directory);
    let chart_path = directory.join(format!("{stem}.chart.svg"));
    match chart::from_samples(samples_path.to_str().expect("a path in UTF-8")) {
        Some(chart) => {
            fs::write(&chart_path, chart).unwrap_or_else(|error| {
                panic!("failed to write {}: {error}", chart_path.display())
            });
            println!("# Speed chart (SVG) is in \"{}\" .", chart_path.display());
        }
        None => {
            // A quick run has no 128 MiB cells: no chart, and none left from an older run.
            if chart_path.exists() {
                fs::remove_file(&chart_path).unwrap_or_else(|error| {
                    panic!("failed to remove stale {}: {error}", chart_path.display())
                });
            }
        }
    }
}

/*
 * The orders must place every contender in every position equally often
 * and realise every ordered adjacency equally often. This is what lets a
 * handful of orders stand in for all permutations.
 */
fn assert_orders_balanced(rows: &[Vec<usize>], n: usize) {
    assert_eq!(
        rows.len() % n,
        0,
        "the order count must be a multiple of the contender count"
    );
    let per_position = rows.len() / n;
    let per_adjacency = rows.len() / n;

    let mut positions = vec![vec![0usize; n]; n];
    let mut adjacencies = vec![vec![0usize; n]; n];

    for order in rows {
        assert_eq!(order.len(), n);
        for (position, &algorithm) in order.iter().enumerate() {
            positions[algorithm][position] += 1;
            if position > 0 {
                adjacencies[order[position - 1]][algorithm] += 1;
            }
        }
    }

    for algorithm in 0..n {
        for position in 0..n {
            assert_eq!(
                positions[algorithm][position], per_position,
                "contender {algorithm} must take position {position} {per_position} time(s) across the orders"
            );
        }
        for follower in 0..n {
            let expected = if follower == algorithm {
                0
            } else {
                per_adjacency
            };
            assert_eq!(
                adjacencies[algorithm][follower], expected,
                "contender {follower} must run right after {algorithm} {expected} time(s) across the orders"
            );
        }
    }
}

/// The measured cell of a contender that takes part at this point.
fn cell(results: &Results, algorithm_index: usize, point_index: usize) -> &Cell {
    results[algorithm_index][point_index]
        .as_ref()
        .expect("the contender takes part in this point's use case")
}

fn measure_all(
    roster: &Roster,
    sizes: &Sizes,
) -> (
    Results,
    RunSamples,
    Vec<clocks::load::Window>,
    Vec<Vec<usize>>,
) {
    /* Inputs for the points measured; an empty buffer stands in for the rest. */
    /*
     * The bytes are irrelevant to every contender's speed, so each point
     * hashes a prefix of one buffer, written once (every page backed by
     * memory of its own); the second copy in a duo sample hashes a prefix
     * of a second buffer, so two copies share no cache lines, for the use
     * cases measured with two copies alone. (A buffer per point held 3.3 GB
     * in a full run.)
     */
    let largest = |wanted: &dyn Fn(usize) -> bool| {
        (0..POINT_COUNT)
            .filter(|&index| roster.measures(index) && wanted(index))
            .map(|index| input_len(POINTS[index]))
            .max()
            .unwrap_or(0)
    };
    let own = make_input_seeded(largest(&|_| true), 0);
    let other = make_input_seeded(
        largest(&|index| {
            roster.algorithms.iter().any(|&a| {
                Placement::ALL
                    .into_iter()
                    .any(|pl| Scenario::Shared(pl).measures(a, POINTS[index].use_case))
            })
        }),
        1,
    );
    let prefixes = |buffer: &[u8]| -> Vec<std::ops::Range<usize>> {
        (0..POINT_COUNT)
            .map(|index| {
                if roster.measures(index) && input_len(POINTS[index]) <= buffer.len() {
                    0..input_len(POINTS[index])
                } else {
                    0..0
                }
            })
            .collect()
    };
    /* A point in blocks reads its prefix of an aligned copy (block_copy). */
    let block_bytes =
        |buffer: &[u8]| largest(&|index| POINTS[index].use_case.in_blocks()).min(buffer.len());
    let (own_batches, own_at) = block_copy(&own[..block_bytes(&own)]);
    let (other_batches, other_at) = block_copy(&other[..block_bytes(&other)]);
    let inputs: Vec<&[u8]> = prefixes(&own)
        .into_iter()
        .enumerate()
        .map(|(index, range)| {
            if POINTS[index].use_case.in_blocks() && !range.is_empty() {
                &own_batches[own_at..][range]
            } else {
                &own[range]
            }
        })
        .collect();
    let duo_inputs: Vec<&[u8]> = prefixes(&other)
        .into_iter()
        .enumerate()
        .map(|(index, range)| {
            if POINTS[index].use_case.in_blocks() && !range.is_empty() {
                &other_batches[other_at..][range]
            } else {
                &other[range]
            }
        })
        .collect();
    let mut progress = Progress::new(roster);
    let duo = Duo::new();

    /*
     * Each algorithm/point combination gets its own calibrated iteration
     * count so that timed blocks have approximately equal durations.
     * The digest checks have already called each contender at every point.
     * Startup and calibration both happen before the timed samples.
     */
    let empty = || -> Samples {
        (0..roster.len())
            .map(|_| {
                (0..POINT_COUNT)
                    .map(|_| Vec::with_capacity(2 * roster.rounds))
                    .collect()
            })
            .collect()
    };
    let mut samples = RunSamples {
        solo: empty(),
        shared: std::array::from_fn(|_| empty()),
        solo_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        shared_started_ns: std::array::from_fn(|_| {
            vec![vec![Vec::new(); POINT_COUNT]; roster.len()]
        }),
        solo_cpu_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        solo_energy_nj: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        solo_counts: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        shared_counts: std::array::from_fn(|_| vec![vec![Vec::new(); POINT_COUNT]; roster.len()]),
        solo_cpu: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        shared_cpu: std::array::from_fn(|_| vec![vec![Vec::new(); POINT_COUNT]; roster.len()]),
    };
    let mut batch_iterations: Vec<Vec<usize>> = vec![vec![1usize; POINT_COUNT]; roster.len()];
    let point_orders: Vec<_> = POINTS
        .iter()
        .map(|point| participating_orders(&roster.algorithms, point.use_case))
        .collect();
    /* Other programs' load: clocks reads it between samples, by itself. */
    clocks::load::tick();
    let measuring_from_ns = clocks::load::now_ns();
    let mut visits = vec![0usize; POINT_COUNT];

    /*
     * Each way of calling in a phase of its own, calibrated and sampled
     * apart: the OS sets a core's clock from its recent use, so neighbours
     * that call differently move a cell's clock. In one set of rounds with
     * the calls after a gap, the nonstop cells ran near 3.0 GHz where alone
     * they ran near 4.4 (SHA-256's 1 KiB messages 0.51 against 0.36 ns/B;
     * Mac jobs 738-739, September 28, 2026); beside the calls after other
     * work, the large calls after idling ran at 3.8-4.0 GHz where alone they
     * ran at 4.4 (SHA-256 ring 32 MiB 0.327 against 0.291 ns/B; jobs 818-819,
     * September 30). A program that hashes one input after another does not
     * sleep between them, and one that idles between calls mostly sleeps.
     */
    // Each way of calling its own set of rounds (the brief pause and the
    // long one apart too: a core's clock after 20 us differs from after 5 ms).
    const PATTERNS: [&str; 5] = ["nonstop", "busy", "brief", "idle", "controlled"];
    for pattern in PATTERNS {
        progress.phase("calibrating");

        for (point_index, point) in POINTS.iter().enumerate() {
            for algorithm_index in 0..roster.len() {
                let algorithm = roster.algorithms[algorithm_index];
                if algorithm.takes_part(point.use_case)
                    && roster.measures(point_index)
                    && point.use_case.pattern_key() == pattern
                    && lent_twin(roster, algorithm, point_index).is_none()
                {
                    if let Sizes::From(given) = sizes {
                        let iterations = *given
                            .get(&(algorithm.key().to_owned(), point_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "--sizes: no size for {} at {}{}",
                                    algorithm.key(),
                                    point.use_case.label_prefix(),
                                    point.label
                                )
                            });
                        // One untimed sample warms the cell, as calibration did;
                        // for a cell after gaps, one call (its calls meet a cold
                        // machine by design, and each takes a gap).
                        take_sample(
                            algorithm,
                            inputs[point_index],
                            *point,
                            if point.use_case.after_gap() {
                                1
                            } else {
                                iterations
                            },
                        );
                        batch_iterations[algorithm_index][point_index] = iterations;
                        continue;
                    }
                    let (iterations, per_iteration_ns) =
                        calibrate_batch(algorithm, inputs[point_index], *point);
                    /*
                     * A synchronous cell's sample: calls after the gap summing
                     * about GAP_SAMPLE_NS (one at least), from calls timed
                     * after the gap (sized from their time back to back, a
                     * 64-byte call's sample summed about 50 calls, each after
                     * its own 1 ms gap, where 7 fill it).
                     */
                    batch_iterations[algorithm_index][point_index] = if point.use_case.after_gap() {
                        let input = inputs[point_index];
                        let timed =
                            take_sample(algorithm, input, *point, CALIBRATION_GAPS as usize);
                        let per_call_ns =
                            u128::from(timed.elapsed_ns) / u128::from(CALIBRATION_GAPS);
                        GAP_SAMPLE_NS.div_ceil(per_call_ns.max(per_iteration_ns).max(1)) as usize
                    } else {
                        iterations
                    };
                }
            }
        }

        if let Sizes::CalibrateOnly(path) = sizes {
            if pattern == PATTERNS[PATTERNS.len() - 1] {
                twins_take_their_sizes(roster, &mut batch_iterations);
                Sizes::write(path, &[], roster, &batch_iterations);
                std::process::exit(0);
            }
            continue;
        }

        /*
         * No warm-up phase. Calibration has just run every contender at every
         * size, and a cold-start cost that survived it would be one sample
         * among the rounds, which the median drops. A separate warm-up would
         * change nothing measurable and cost a minute of run time.
         */

        /*
         * Point order rotates by round. All participating contenders sample
         * each selected visit, and every visit advances the Williams order.
         * Default sample counts complete the design; explicit --rounds
         * samples every round. No contender skips a position because its
         * calibrated call is slower.
         */
        progress.phase(match pattern {
            "nonstop" => "measuring one after another",
            "busy" => "measuring after other work",
            "brief" => "measuring after a brief idle",
            "controlled" => "measuring after idling, deep states off",
            _ => "measuring after idling",
        });
        // The controlled cells' rounds run with the deep states off; the
        // guard puts them back when the rounds end (IdleControlGuard).
        let _deep_states_off = (pattern == "controlled")
            .then(|| roster.idle_control.as_ref().map(IdleControl::apply))
            .flatten();

        for round in 0..roster.rounds {
            progress.round(round, &samples.solo);
            clocks::load::tick();

            for point_offset in 0..roster.points.len() {
                let size_index = roster.points[(point_offset + round) % roster.points.len()];
                let point = POINTS[size_index];
                let orders = &point_orders[size_index];
                if point.use_case.pattern_key() != pattern
                    || orders.is_empty()
                    || !cell_wants_sample(
                        round + size_index,
                        roster.rounds,
                        roster.visits_of(point.use_case),
                    )
                {
                    continue;
                }
                let algorithm_order = &orders[visits[size_index] % orders.len()];
                visits[size_index] += 1;

                let input = inputs[size_index];

                for &algorithm_index in algorithm_order {
                    let algorithm = roster.algorithms[algorithm_index];
                    // Measured as its lent twin (copied below).
                    if lent_twin(roster, algorithm, size_index).is_some() {
                        continue;
                    }
                    let iterations = batch_iterations[algorithm_index][size_index];
                    /* A call after a gap: one sample; a nonstop cell: its train. */
                    let train = if point.use_case.after_gap() {
                        Train {
                            warm_ns: 0,
                            samples: 1,
                        }
                    } else {
                        roster.train
                    };

                    /* The solo samples: this thread runs the batches, alone. */
                    clocks::load::tick();
                    warm_up(algorithm, input, point, iterations, train.warm_ns);
                    let solo: Vec<DuoCopy> = (0..train.samples)
                        .map(|_| {
                            clocks::load::tick();
                            take_sample(algorithm, input, point, iterations)
                        })
                        .collect();

                    /*
                     * The shared samples, under the same conditions, for the
                     * nonstop use cases: two copies, released together, each
                     * run its own train on its own thread, and each copy's
                     * own time is a sample; the copy that finishes first
                     * keeps hashing until the other has finished (#105).
                     */
                    // Each placement the machine has, its two copies on
                    // the pair of CPUs the topology names (Placement).
                    let copies: Vec<(Placement, [DuoWork; 2])> = Placement::ALL
                        .into_iter()
                        .filter(|&p| Scenario::Shared(p).measures(algorithm, point.use_case))
                        .map(|p| {
                            (
                                p,
                                duo.run(
                                    algorithm,
                                    input,
                                    duo_inputs[size_index],
                                    point,
                                    iterations,
                                    train,
                                    topology()
                                        .cpus(p)
                                        .expect("a placement measured has its CPUs, or the OS's"),
                                ),
                            )
                        })
                        .collect();

                    let total_units = point.use_case.units(point, iterations);
                    let per_unit = |ns: u64| Measured::new(ns, total_units);
                    for sample in &solo {
                        samples.solo[algorithm_index][size_index].push(per_unit(sample.elapsed_ns));
                        samples.solo_started_ns[algorithm_index][size_index]
                            .push(sample.started_ns);
                        samples.solo_cpu_ns[algorithm_index][size_index].push(sample.cpu_ns);
                        samples.solo_energy_nj[algorithm_index][size_index].push(sample.energy_nj);
                        samples.solo_counts[algorithm_index][size_index].push(sample.counts);
                        samples.solo_cpu[algorithm_index][size_index].push(sample.cpu);
                    }
                    // In pairs, copy 0's i-th sample beside copy 1's.
                    for (placement, [first, second]) in &copies {
                        let i = placement.index();
                        debug_assert!(
                            both_contended(first, second),
                            "each copy's last sample began before the other's work ended"
                        );
                        for copy in first
                            .samples
                            .iter()
                            .zip(&second.samples)
                            .flat_map(|(a, b)| [a, b])
                        {
                            samples.shared[i][algorithm_index][size_index]
                                .push(per_unit(copy.elapsed_ns));
                            samples.shared_started_ns[i][algorithm_index][size_index]
                                .push(copy.started_ns);
                            samples.shared_counts[i][algorithm_index][size_index].push(copy.counts);
                            samples.shared_cpu[i][algorithm_index][size_index].push(copy.cpu);
                        }
                    }
                }
            }
        }
    }

    progress.finish(&samples.solo);
    // Each owned cell measured as its lent twin takes the twin's samples.
    for a in 0..roster.len() {
        for p in 0..POINT_COUNT {
            if let Some(t) = roster
                .measures(p)
                .then(|| lent_twin(roster, roster.algorithms[a], p))
                .flatten()
            {
                samples.solo[a][p] = samples.solo[a][t].clone();
                samples.solo_started_ns[a][p] = samples.solo_started_ns[a][t].clone();
                samples.solo_cpu_ns[a][p] = samples.solo_cpu_ns[a][t].clone();
                samples.solo_energy_nj[a][p] = samples.solo_energy_nj[a][t].clone();
                samples.solo_counts[a][p] = samples.solo_counts[a][t].clone();
                for i in 0..PLACEMENTS {
                    samples.shared[i][a][p] = samples.shared[i][a][t].clone();
                    samples.shared_started_ns[i][a][p] = samples.shared_started_ns[i][a][t].clone();
                    samples.shared_counts[i][a][p] = samples.shared_counts[i][a][t].clone();
                    samples.shared_cpu[i][a][p] = samples.shared_cpu[i][a][t].clone();
                }
                samples.solo_cpu[a][p] = samples.solo_cpu[a][t].clone();
            }
        }
    }
    twins_take_their_sizes(roster, &mut batch_iterations);
    let load: Vec<clocks::load::Window> = clocks::load::windows()
        .into_iter()
        .filter(|window| window.end_ns > measuring_from_ns)
        .collect();

    let mut results: Results = vec![vec![None; POINT_COUNT]; roster.len()];

    for algorithm_index in 0..roster.len() {
        for (size_index, point) in POINTS.iter().enumerate() {
            if !roster.algorithms[algorithm_index].takes_part(point.use_case)
                || !roster.measures(size_index)
            {
                continue;
            }
            let solo = &samples.solo[algorithm_index][size_index];
            let train = if point.use_case.after_gap() {
                1
            } else {
                roster.train.samples
            };
            assert!(
                !solo.is_empty() && solo.len() <= roster.rounds * train,
                "one solo sample per round (a nonstop cell: one train) at most, and one at least"
            );
            let algorithm = roster.algorithms[algorithm_index];
            let shared_mean = |placement: Placement| {
                let shared = &samples.shared[placement.index()][algorithm_index][size_index];
                let copies = if Scenario::Shared(placement).measures(algorithm, point.use_case) {
                    2
                } else {
                    0
                };
                assert_eq!(
                    shared.len(),
                    copies * solo.len(),
                    "two shared samples, one per copy, beside every solo sample of a nonstop cell"
                );
                (copies > 0).then(|| Mean::of(shared))
            };
            results[algorithm_index][size_index] = Some(Cell {
                solo: Mean::of(solo),
                shared: Placement::ALL.map(shared_mean),
            });
        }
    }

    (results, samples, load, batch_iterations)
}

/*
 * Live progress on stderr, so stdout stays a clean report. Shows the phase,
 * a bar over the sample rounds, the elapsed and estimated remaining time,
 * and the running mean for every contender at the largest input size.
 * The line redraws in place on a terminal; elsewhere each update is its
 * own line, so a log still shows the run advancing.
 */
struct Progress<'a> {
    roster: &'a Roster,
    started: clocks::Stamp,
    measuring_started: Option<clocks::Stamp>,
    interactive: bool,
    last_width: usize,
}

impl<'a> Progress<'a> {
    const BAR_WIDTH: usize = 30;

    fn new(roster: &'a Roster) -> Self {
        let interactive = std::io::stderr().is_terminal();
        Self {
            roster,
            started: clocks::now(),
            measuring_started: None,
            interactive,
            last_width: 0,
        }
    }

    fn phase(&mut self, name: &str) {
        if name.starts_with("measuring") {
            self.measuring_started = Some(clocks::now());
        }
        self.draw(&format!(
            "[{:>5}s] {name}…",
            tenths_of_seconds(std::time::Duration::from_nanos(clocks::since_ns(
                self.started
            )))
        ));
    }

    /* Called at the start of each round; `samples` holds every round so far. */
    fn round(&mut self, round: usize, samples: &Samples) {
        self.round_inner(
            round,
            &running_means(self.roster, samples, self.roster.progress_point()),
        );
    }

    fn round_inner(&mut self, round: usize, medians: &str) {
        let measuring_started = self
            .measuring_started
            .expect("round() runs inside the measuring phase");

        let rounds = self.roster.rounds;
        let filled = (round * Self::BAR_WIDTH + rounds / 2) / rounds;
        let bar: String = "█".repeat(filled) + &"░".repeat(Self::BAR_WIDTH - filled);

        /* The rounds left at the rate so far: elapsed × left / done, in whole seconds. */
        let remaining = if round > 0 {
            let elapsed_ns = u128::from(clocks::since_ns(measuring_started));
            let left_ns = elapsed_ns * (rounds - round) as u128 / round as u128;
            format!("{:>3}s left", (left_ns + 500_000_000) / 1_000_000_000)
        } else {
            " estimating".to_owned()
        };

        self.draw(&format!(
            "[{:>5}s] measuring {bar} {:>3}/{rounds} rounds · {remaining} · {medians}",
            tenths_of_seconds(std::time::Duration::from_nanos(clocks::since_ns(
                self.started
            ))),
            round,
        ));
    }

    fn finish(&mut self, samples: &Samples) {
        self.finish_inner(&running_means(
            self.roster,
            samples,
            self.roster.progress_point(),
        ));
    }

    fn finish_inner(&mut self, medians: &str) {
        let bar = "█".repeat(Self::BAR_WIDTH);
        let rounds = self.roster.rounds;
        self.draw(&format!(
            "[{:>5}s] measured  {bar} {rounds}/{rounds} rounds · {medians}",
            tenths_of_seconds(std::time::Duration::from_nanos(clocks::since_ns(
                self.started
            ))),
        ));
        eprintln!();
    }

    fn draw(&mut self, line: &str) {
        let mut stderr = std::io::stderr().lock();
        if self.interactive {
            /* Return to column 0, overwrite, blank any leftover from a longer line. */
            let padding = self.last_width.saturating_sub(line.chars().count());
            let _ = write!(stderr, "\r{line}{}", " ".repeat(padding));
        } else {
            let _ = writeln!(stderr, "{line}");
        }
        let _ = stderr.flush();
        self.last_width = line.chars().count();
    }
}

/*
 * "BLAKE3 0.39 · SHA-256 0.33 · … ns/B at 1 MiB" from the samples collected
 * so far, or a placeholder before the first round completes.
 */
/// A duration in seconds with one decimal, rounded once: "12.3".
fn tenths_of_seconds(duration: std::time::Duration) -> String {
    let tenths = (duration.as_nanos() + 50_000_000) / 100_000_000;
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn running_means(roster: &Roster, samples: &Samples, size_index: usize) -> String {
    if samples
        .iter()
        .all(|contender| contender[size_index].is_empty())
    {
        return format!("means at {} pending", POINTS[size_index].label);
    }

    /* Contenders that take no part in this point's use case have no samples there. */
    let parts: Vec<String> = (0..roster.len())
        .filter(|&algorithm_index| !samples[algorithm_index][size_index].is_empty())
        .map(|algorithm_index| {
            format!(
                "{} {}",
                roster.algorithms[algorithm_index].name(),
                Mean::of(&samples[algorithm_index][size_index]).format_ns(1)
            )
        })
        .collect();

    format!("{} ns/B at {}", parts.join(" · "), POINTS[size_index].label)
}

fn make_input(size: usize) -> Vec<u8> {
    make_input_seeded(size, 0)
}

/// The input of `size` bytes for `seed`: `seed` 0 is make_input's buffer,
/// seed 1 the duo copy's.
fn make_input_seeded(size: usize, seed: u64) -> Vec<u8> {
    /*
     * Little-endian 64-bit words `seed << 48 | index`: every block of every
     * input differs, and the two seeds give the duo copies different
     * contents. Hash speed does not depend on the bytes. Generated
     * outside timed intervals.
     */
    let mut input: Vec<u8> = (0..size.div_ceil(8) as u64)
        .flat_map(|index| (seed << 48 | index).to_le_bytes())
        .collect();
    input.truncate(size);
    input
}

fn run_batch(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize) {
    hash_batch(algorithm, input, point, iterations, use_digest);
}

/*
 * The use of each hash, the same in every cell (FROZEN.md, Zooko, October
 * 3, 2026): the program stores it in its slot for the message, in a kept
 * array of DIGEST_SLOTS slots, wherever the design delivers it.
 */
const DIGEST_SLOTS: usize = 1024;

thread_local! {
    static DIGESTS: std::cell::RefCell<(Vec<[u8; 32]>, usize)> = std::cell::RefCell::new((vec![[0; 32]; DIGEST_SLOTS], 0));
}

/// `digests`, one digest or a batch's run of them (32 bytes each; a shorter
/// one, as SHA-1's 20, is one digest): each in its slot, in turn. A run is
/// copied in at most two moves, up to the array's end and on from its start,
/// so the use costs a batch what storing its digests costs (a move a digest
/// took a call to memcpy each: the multithreaded stream's caller paid it for
/// every message, in series with the workers).
fn use_digest(digests: &[u8]) {
    DIGESTS.with_borrow_mut(|(slots, next)| {
        if digests.len() <= 32 {
            let slot = &mut slots[*next % DIGEST_SLOTS];
            match <[u8; 32]>::try_from(digests) {
                Ok(digest) => *slot = digest,
                Err(_) => slot[..digests.len()].copy_from_slice(digests),
            }
            *next += 1;
            return;
        }
        let (mut run, rest) = digests.as_chunks::<32>();
        assert!(
            rest.is_empty(),
            "a run of 32-byte digests, {} bytes",
            digests.len()
        );
        while !run.is_empty() {
            let at = *next % DIGEST_SLOTS;
            let n = run.len().min(DIGEST_SLOTS - at);
            slots[at..at + n].copy_from_slice(&run[..n]);
            *next += n;
            run = &run[n..];
        }
    });
}

/// The cell's own load, untimed, for `warm_ns` (one batch at least; none
/// for 0): a nonstop visit's warm-up (Train).
fn warm_up(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize, warm_ns: u64) {
    if warm_ns == 0 {
        return;
    }
    let started = clocks::now();
    loop {
        run_batch(algorithm, input, point, iterations);
        if clocks::since_ns(started) >= warm_ns {
            break;
        }
    }
}

/// Pin the calling thread to `cpu` (Linux; elsewhere Topology gives no
/// pairs, so the OS places the copies and this is never reached).
fn pin_to_cpu(cpu: usize) {
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn sched_setaffinity(pid: i32, size: usize, set: *const u64) -> i32;
        }
        let mut set = [0u64; 16];
        set[cpu / 64] |= 1 << (cpu % 64);
        // Sound: `set` is a cpu_set_t of 1024 bits, the calling thread (0).
        let r = unsafe { sched_setaffinity(0, core::mem::size_of_val(&set), set.as_ptr()) };
        assert_eq!(r, 0, "pin the shared copy to CPU {cpu}");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = cpu;
}

/// Move the pages of `bytes` to memory node `node` (Linux mbind with
/// MPOL_BIND and MPOL_MF_MOVE over the whole pages inside `bytes`; the
/// partial pages at the ends stay). Called on machines with several
/// nodes, which have Linux and thread affinity, so a copy's pinned CPU
/// names its node.
fn bind_to_node(bytes: &[u8], node: usize) {
    const PAGE: usize = 4096;
    let start = bytes.as_ptr() as usize;
    let first = start.next_multiple_of(PAGE);
    let end = (start + bytes.len()) / PAGE * PAGE;
    if end <= first {
        return;
    }
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        unsafe extern "C" {
            fn syscall(number: i64, ...) -> i64;
        }
        #[cfg(target_arch = "x86_64")]
        const SYS_MBIND: i64 = 237;
        #[cfg(target_arch = "aarch64")]
        const SYS_MBIND: i64 = 235;
        const MPOL_BIND: i64 = 2;
        const MPOL_MF_MOVE: i64 = 2;
        let mut mask = [0u64; 16];
        mask[node / 64] |= 1 << (node % 64);
        // Sound: the range is whole pages of the caller's live slice, and
        // `mask` holds 1024 node bits.
        let r = unsafe {
            syscall(
                SYS_MBIND,
                first,
                end - first,
                MPOL_BIND,
                mask.as_ptr(),
                mask.len() * 64,
                MPOL_MF_MOVE,
            )
        };
        assert_eq!(r, 0, "move the shared copy's input to memory node {node}");
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    panic!("binding a copy's input to node {node} is written for Linux on x86-64 and aarch64");
}

/// The CPU the calling thread runs on now (Linux: sched_getcpu; elsewhere
/// None), read after a sample, outside its timing: a read before the call
/// would warm the core the gap just cooled (bench-hashes #106).
fn current_cpu() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn sched_getcpu() -> i32;
        }
        // Sound: a plain query of the calling thread.
        let cpu = unsafe { sched_getcpu() };
        return u32::try_from(cpu).ok();
    }
    #[allow(unreachable_code)]
    None
}

/*
 * One sample of `iterations` on this thread, as the point's use case
 * calls (FROZEN.md): for a synchronous use case, that many calls, each
 * after the gap and timed alone, summed; for a continuous one, a batch of
 * that many back to back, timed whole. The thread's counts bracket it,
 * outside the timed intervals.
 */
fn take_sample(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize) -> DuoCopy {
    if point.use_case.after_gap() {
        let pause_ns = point.use_case.pause_ns();
        if point.use_case.call() == UseCase::OneMessage {
            // Select the API before the gap and clocks. A cold call should
            // pay for its API, rather than the benchmark's dispatch tree.
            let call = one_message_call(algorithm);
            return take_prepared_sample(input, true, iterations, pause_ns, |input| {
                // The door, for every contender alike (UseCase::in_blocks).
                assert_blocks_aligned(input);
                call(input, point.bytes)
            });
        }
        if matches!(
            point.use_case.call(),
            UseCase::ManyMessages | UseCase::ManyLongMessages
        ) {
            let hash_many: Option<fn(&[u8], usize, &mut [[u8; 32]])> = match algorithm {
                Algorithm::Blake3ServilSt => Some(|i, l, o| {
                    blake3_servil::throughput::hash_many(
                        blake3_servil::Mode::Plain,
                        &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                        o,
                    )
                }),
                Algorithm::Blake3ServilMt => Some(|i, l, o| {
                    blake3_servil::throughput::hash_many_multithreaded(
                        blake3_servil::Mode::Plain,
                        &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                        o,
                    )
                }),
                _ => None,
            };
            if let Some(hash_many) = hash_many {
                let mut digests = take_batch_digests(point.messages);
                let measured = take_prepared_sample(input, true, iterations, pause_ns, |input| {
                    #[cfg(test)]
                    observe_call("hash_many");
                    assert_blocks_aligned(input);
                    hash_many(
                        black_box(input),
                        point.message_len(),
                        &mut digests[..point.messages],
                    );
                    black_box(digests[..point.messages].as_flattened());
                });
                keep_batch_digests(digests);
                return measured;
            }
        }
        return take_prepared_sample(
            input,
            point.use_case.in_blocks(),
            iterations,
            pause_ns,
            |input| run_batch(algorithm, input, point, 1),
        );
    }
    let started_ns = clocks::load::now_ns();
    let cpu0 = clocks::process_cpu_ns();
    let energy0 = clocks::energy::package_nj();
    let counts0 = clocks::Counts::read();
    let started = clocks::now();
    run_batch(algorithm, input, point, iterations);
    let elapsed_ns = clocks::since_ns(started);
    let counts = counts0
        .zip(clocks::Counts::read())
        .map(|(before, after)| after.since(before));
    let energy_nj = energy0
        .zip(clocks::energy::package_nj())
        .map(|(before, after)| after - before);
    // Unix always has the clock (clocks' process_cpu_ns, BLAKE3 #171).
    let cpu_ns = Some(clocks::process_cpu_ns() - cpu0);
    DuoCopy {
        elapsed_ns,
        counts,
        started_ns,
        cpu_ns,
        energy_nj,
        cpu: current_cpu(),
    }
}

/// Prepare and time an already-selected call, each after the program
/// idled (`idle`) or after its other work. The producer and work buffers
/// stay outside the API's interval, as does dispatch selection. An input
/// `in_blocks` is written from a 64-byte boundary of the producer's buffer,
/// whole blocks behind it (BLOCK_SPARE).
fn take_prepared_sample(
    input: &[u8],
    in_blocks: bool,
    iterations: usize,
    pause_ns: Option<u64>,
    call: impl FnMut(&[u8]),
) -> DuoCopy {
    let mask = pin_measuring_thread();
    let sample = take_prepared_sample_here(input, in_blocks, iterations, pause_ns, call);
    restore_mask(mask);
    sample
}

/// The environment variable by which `regress` names the CPU a run's
/// measuring thread takes each sample after a gap on (bench-hashes #69):
/// a pair's old and new runs on one core.
const PIN_CPU_VARIABLE: &str = "BENCH_HASHES_PIN_CPU";

/// The CPU this run pins its measuring thread to around each sample after
/// a gap (PIN_CPU_VARIABLE; Linux), if any.
fn pinned_cpu() -> Option<usize> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let value = std::env::var(PIN_CPU_VARIABLE).ok()?;
    Some(
        value
            .parse()
            .unwrap_or_else(|_| panic!("{PIN_CPU_VARIABLE}={value}: not a CPU number")),
    )
}

/// The samples header's line: where the measuring thread took its samples
/// after a gap.
fn describe_pinning() -> String {
    match pinned_cpu() {
        Some(cpu) => {
            format!("pinned to CPU {cpu} around each sample after a gap (bench-hashes regress)")
        }
        None => "placed by the OS".to_owned(),
    }
}

/// Pin the main thread to the run's CPU (pinned_cpu) for one sample: its
/// mask, to restore after. The main thread alone, and only around the
/// sample: pinned for the run, it would hand the pool's workers, started
/// later, its one CPU.
fn pin_measuring_thread() -> Option<[u64; 16]> {
    let cpu = pinned_cpu()?;
    if std::thread::current().name() != Some("main") {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" {
            fn sched_getaffinity(pid: i32, size: usize, set: *mut u64) -> i32;
        }
        let mut mask = [0u64; 16];
        // Sound: `mask` is a cpu_set_t of 1024 bits, the calling thread (0).
        assert_eq!(
            unsafe { sched_getaffinity(0, core::mem::size_of_val(&mask), mask.as_mut_ptr()) },
            0,
            "read the measuring thread's mask"
        );
        pin_to_cpu(cpu);
        return Some(mask);
    }
    #[allow(unreachable_code)]
    {
        let _ = cpu;
        None
    }
}

/// The measuring thread's mask as it was before pin_measuring_thread.
fn restore_mask(mask: Option<[u64; 16]>) {
    #[cfg(target_os = "linux")]
    if let Some(mask) = mask {
        unsafe extern "C" {
            fn sched_setaffinity(pid: i32, size: usize, set: *const u64) -> i32;
        }
        // Sound: as pin_measuring_thread.
        assert_eq!(
            unsafe { sched_setaffinity(0, core::mem::size_of_val(&mask), mask.as_ptr()) },
            0,
            "restore the measuring thread's mask"
        );
    }
    let _ = mask;
}

fn take_prepared_sample_here(
    input: &[u8],
    in_blocks: bool,
    iterations: usize,
    pause_ns: Option<u64>,
    mut call: impl FnMut(&[u8]),
) -> DuoCopy {
    GAP_BUFFERS.with(|kept| {
        let mut buffers = kept.borrow_mut();
        let (work, produced) = &mut *buffers;
        produced.clear();
        let at = if in_blocks {
            produced.reserve(input.len() + BLOCK_SPARE);
            let at = block_start(produced);
            produced.resize(at + input.len().next_multiple_of(BLOCK_LEN), 0);
            at
        } else {
            produced.resize(input.len(), 0);
            0
        };
        // The pause before each call: the use case's idle (brief or long),
        // or GAP_NS of other work.
        let (gap, gap_ns) = match pause_ns {
            Some(ns) => (clocks::Gap::Idle, ns),
            None => (clocks::Gap::Busy(work), GAP_NS),
        };
        let started_ns = clocks::load::now_ns();
        let measured = clocks::measure_after_gaps_prepared(
            iterations as u64,
            gap,
            gap_ns,
            // An input in blocks reaches the call with its whole blocks.
            &mut produced[at..],
            |produced| produced[..input.len()].copy_from_slice(black_box(input)),
            |produced| call(black_box(produced)),
        );
        // The write and the call, both the program's work (FROZEN.md, Zooko,
        // October 3, 2026).
        let counts = measured
            .calls
            .counts
            .zip(measured.preparation.counts)
            .map(|(call, write)| call.plus(write));
        DuoCopy {
            elapsed_ns: measured.calls.wall_ns + measured.preparation.wall_ns,
            counts,
            started_ns,
            cpu_ns: None,
            energy_nj: None,
            cpu: current_cpu(),
        }
    })
}

#[cfg(test)]
thread_local! {
    static CALLS: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn observe_call(call: &'static str) {
    CALLS.with(|calls| calls.borrow_mut().push(call));
}

/// One whole message through the contender's plain API: the message's
/// `len` bytes, from the whole blocks it comes in (UseCase::in_blocks), which
/// servil's throughput call takes as they are. Selection happens
/// once, before the gap; every result remains observable to the optimizer.
fn one_message_call(algorithm: Algorithm) -> fn(&[u8], usize) {
    match algorithm {
        Algorithm::Blake3 => |blocks, len| use_digest(blake3::hash(&blocks[..len]).as_bytes()),
        Algorithm::Blake3ServilSt => |blocks, len| {
            let input = &blocks[..len];
            #[cfg(test)]
            observe_call("hash");
            use_digest(blake3_servil::convenience::hash(input).as_bytes());
        },
        Algorithm::Blake3ServilMt => |blocks, len| {
            use_digest(
                blake3_servil::throughput::hash_multithreaded(
                    blake3_servil::Mode::Plain,
                    servil_blocks(blocks),
                    len,
                )
                .as_bytes(),
            )
        },
        Algorithm::Sha256 => |blocks, len| use_digest(&Sha256::digest(&blocks[..len])),
        Algorithm::Sha256Ring => |blocks, len| {
            use_digest(ring::digest::digest(&ring::digest::SHA256, &blocks[..len]).as_ref())
        },
        Algorithm::Sha256CommonCrypto => {
            |blocks, len| use_digest(&common_crypto::sha256(&blocks[..len]))
        }
        Algorithm::Sha3_256 => |blocks, len| use_digest(&sha3::Sha3_256::digest(&blocks[..len])),
        Algorithm::Sha1Dc => {
            |blocks, len| use_digest(sha1_checked::Sha1::try_digest(&blocks[..len]).hash())
        }
        Algorithm::Blake3Rayon => |blocks, len| {
            let input = &blocks[..len];
            use_digest(
                blake3::Hasher::new()
                    .update_rayon(input)
                    .finalize()
                    .as_bytes(),
            )
        },
    }
}

thread_local! {
    /// Written counter bytes give the working set physical pages, including
    /// on systems that share untouched zero pages. Keep one work buffer and
    /// one producer buffer per measuring thread, outside timed intervals.
    static GAP_BUFFERS: std::cell::RefCell<(Vec<u8>, Vec<u8>)> = std::cell::RefCell::new((make_input(GAP_WORK_BYTES), Vec::new()));
}

/*
 * The dispatch of every timed batch. The callback is monomorphized: timed
 * batches black-box each digest (a test observes them). Selection and
 * allocation stay outside the per-hash loop.
 *
 * `input` holds `messages` messages of the use case's length (for one
 * message, the whole slice). A contender with a batch entry point hands
 * it every batch of two messages or more: BLAKE3-Servil's hash_many, the
 * blake3 crate's hidden Platform::hash_many sixteen messages per call.
 * Every other contender, and every contender for one message,
 * hashes the messages one call each through its plain entry point. The
 * contender must take part in the point's use case.
 */
fn hash_batch(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    consume: impl FnMut(&[u8]),
) {
    assert!(iterations > 0, "batch size must be positive");
    let messages = point.messages;
    let message_len = if point.use_case.batch() {
        point.message_len()
    } else {
        input.len()
    };
    assert!(
        messages == 1
            || matches!(
                point.use_case,
                UseCase::Collection | UseCase::ContinuousCollection | UseCase::MerkleTree
            )
            || input.len() == messages * message_len,
        "a batch is {messages} messages of {message_len} bytes"
    );
    assert!(
        algorithm.takes_part(point.use_case),
        "{} takes no part in {:?}",
        algorithm.key(),
        point.use_case
    );

    match point.use_case {
        UseCase::Interleaved
        | UseCase::InterleavedTurn
        | UseCase::IdleInterleavedTurn
        | UseCase::BriefIdleInterleavedTurn
        | UseCase::ControlledIdleInterleavedTurn => {
            return hash_interleaved(algorithm, input, point.bytes, iterations, consume)
        }
        UseCase::Collection => {
            return hash_collection(algorithm, input, point, iterations, consume)
        }
        UseCase::ContinuousCollection => {
            return hash_continuous_collection(algorithm, input, point, iterations, consume)
        }
        UseCase::ContinuousMessages => {
            return hash_continuous_messages(algorithm, input, point.bytes, iterations, consume)
        }
        UseCase::ContinuousBatches => {
            return hash_continuous_batches(algorithm, input, point, iterations, consume)
        }
        UseCase::LentMessages | UseCase::LentBatches => {
            return hash_lent(algorithm, input, point, iterations, consume)
        }
        UseCase::MerkleTree => return hash_merkle(algorithm, input, point, iterations, consume),
        UseCase::OneMessage
        | UseCase::IdleOneMessage
        | UseCase::BriefIdleOneMessage
        | UseCase::ControlledIdleOneMessage
        | UseCase::ManyMessages
        | UseCase::ManyLongMessages
        | UseCase::IdleManyMessages
        | UseCase::BriefIdleManyMessages
        | UseCase::ControlledIdleManyMessages => {
            hash_in_memory(algorithm, input, point, iterations, consume)
        }
    }
}

/// hash_batch for the use cases whose input is in memory: one message, or
/// a batch of them.
fn hash_in_memory(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    consume: impl FnMut(&[u8]),
) {
    if point.use_case.in_blocks() {
        // The door, for every contender alike.
        assert_blocks_aligned(input);
    }
    let message_len = if point.use_case.batch() {
        point.message_len()
    } else {
        point.bytes
    };
    hash_messages(
        algorithm,
        input,
        point.messages,
        message_len,
        point.use_case.batch(),
        iterations,
        consume,
    );
}

/// `messages` messages of `message_len` bytes back to back in `input`,
/// each contender's way: through its batch call where `batch` and it has
/// one, else one call each; every digest into `consume`.
fn hash_messages(
    algorithm: Algorithm,
    input: &[u8],
    messages: usize,
    message_len: usize,
    batch: bool,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    // A single message comes in its whole blocks (UseCase::in_blocks):
    // BLAKE3-Servil's throughput calls take them as they are, every other
    // contender the message's bytes. These are the calls made one after
    // another (lent messages, a pipelined collection's items), so servil st
    // makes its throughput call, the one for call after call (John
    // Servil, November 6, 2026, BLAKE3-Servil #253; Zooko, November 5,
    // "Cold calls and warm calls, by their API"); the calls after a gap keep
    // convenience::hash (one_message_call).
    if !batch
        && matches!(
            algorithm,
            Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt
        )
    {
        let blocks = servil_blocks(input);
        for _ in 0..iterations {
            let hash = if algorithm == Algorithm::Blake3ServilMt {
                blake3_servil::throughput::hash_multithreaded(
                    blake3_servil::Mode::Plain,
                    black_box(blocks),
                    message_len,
                )
            } else {
                blake3_servil::throughput::hash(
                    blake3_servil::Mode::Plain,
                    black_box(blocks),
                    message_len,
                )
            };
            consume(hash.as_bytes());
        }
        return;
    }
    let input = if batch { input } else { &input[..message_len] };
    match algorithm {
        Algorithm::Blake3 => {
            // The crate's hidden batch function hashes each message as one
            // chunk: longer messages, as a program would, one call each.
            if !batch || message_len > blake3::guts::CHUNK_LEN {
                each_message(
                    input,
                    message_len,
                    iterations,
                    |m| *blake3::hash(m).as_bytes(),
                    consume,
                )
            } else {
                blake3_batch(input, message_len, None, iterations, consume)
            }
        }
        Algorithm::Sha256 => each_message(
            input,
            message_len,
            iterations,
            |m| Sha256::digest(m),
            consume,
        ),
        Algorithm::Sha3_256 => each_message(
            input,
            message_len,
            iterations,
            |m| {
                let digest: [u8; 32] = sha3::Sha3_256::digest(m).into();
                digest
            },
            consume,
        ),
        Algorithm::Sha1Dc => each_message(
            input,
            message_len,
            iterations,
            |m| {
                let result = sha1_checked::Sha1::try_digest(m);
                let mut digest = [0u8; 20];
                digest.copy_from_slice(result.hash());
                digest
            },
            consume,
        ),
        // A batch: a single message took its throughput call above.
        Algorithm::Blake3ServilSt => servil_batch(
            input,
            messages,
            message_len,
            iterations,
            |i, l, o| {
                blake3_servil::throughput::hash_many(
                    blake3_servil::Mode::Plain,
                    &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                    o,
                )
            },
            consume,
        ),
        Algorithm::Sha256CommonCrypto => each_message(
            input,
            message_len,
            iterations,
            |m| common_crypto::sha256(m),
            consume,
        ),
        Algorithm::Sha256Ring => each_message(
            input,
            message_len,
            iterations,
            |m| ring::digest::digest(&ring::digest::SHA256, m),
            consume,
        ),
        Algorithm::Blake3Rayon => {
            assert_eq!(
                messages, 1,
                "BLAKE3 mt takes no part in the many-messages use case"
            );
            each_message(
                input,
                message_len,
                iterations,
                |m| *blake3::Hasher::new().update_rayon(m).finalize().as_bytes(),
                consume,
            )
        }
        Algorithm::Blake3ServilMt => servil_batch(
            input,
            messages,
            message_len,
            iterations,
            |i, l, o| {
                blake3_servil::throughput::hash_many_multithreaded(
                    blake3_servil::Mode::Plain,
                    &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                    o,
                )
            },
            consume,
        ),
    }
}

/*
 * A collection (COLLECTIONS): `iterations` passes over its items, laid out
 * one after another in `input`, each item hashed by the contender's
 * one-message call (hash_in_memory), its hash to `consume`.
 */
/// A collection point's items: each (offset, length) in its input, made once.
fn items_of(point: Point, input: &[u8]) -> &'static [(usize, usize)] {
    static ITEMS: [std::sync::OnceLock<Vec<(usize, usize)>>; COLLECTION_COUNT] =
        [const { std::sync::OnceLock::new() }; COLLECTION_COUNT];
    let index = collection_index(point);
    assert_eq!(
        input.len(),
        input_len(point),
        "the collection's messages, each in its whole blocks, fill its input"
    );
    ITEMS[index].get_or_init(|| collection_items(COLLECTIONS[index].1))
}

/// The COLLECTIONS entry a collection point measures.
fn collection_index(point: Point) -> usize {
    COLLECTIONS
        .iter()
        .position(|(label, _)| *label == point.label)
        .expect("a collection's point")
}

/// The bytes a point's input takes: a collection's layout, each message in
/// its own whole blocks; any other point's bytes. (A point's bytes are its
/// work: what a speed divides by.)
fn input_len(point: Point) -> usize {
    match point.use_case {
        UseCase::Collection | UseCase::ContinuousCollection => {
            collection_layout_bytes(COLLECTIONS[collection_index(point)].1)
        }
        // A message in blocks comes with its whole blocks.
        use_case if use_case.in_blocks() => point.bytes.next_multiple_of(BLOCK_LEN),
        _ => point.bytes,
    }
}

/*
 * A collection pipelined (UseCase::ContinuousCollection): `iterations`
 * passes over its items, one after another, each read into a buffer of the
 * program's, then hashed, its digest into `consume`. BLAKE3-Servil mt reads
 * each straight into its stream's ring (submit_with) and hashes on its
 * workers while the next is read; every other hash, on the program's thread
 * after reading it, as it hashes one message in memory.
 */
fn hash_continuous_collection(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    let items = items_of(point, input);
    if algorithm == Algorithm::Blake3ServilMt {
        let mut places = take_places(STREAM_PLACES_PER_MESSAGE);
        let free = std::cell::RefCell::new(places.chunks_mut(1).collect::<Vec<_>>());
        blake3_servil::throughput::stream(
            blake3_servil::Mode::Plain,
            |place: &mut [blake3_servil::Hash]| {
                consume(place[0].as_bytes());
                free.borrow_mut().push(place);
            },
            |s| {
                for _ in 0..iterations {
                    for &(offset, len) in items {
                        let item = black_box(&input[offset..offset + len]);
                        let place = free_place(&free, s);
                        s.submit_with(
                            len,
                            |at, room| {
                                room.copy_from_slice(&item[at..][..room.len()]);
                                room.len()
                            },
                            &mut place[0],
                        );
                    }
                }
                s.finish();
            },
        );
        drop(free);
        return keep_places(places);
    }
    let longest = items.iter().map(|&(_, len)| len).max().unwrap_or(0);
    let mut buffers = take_buffers(1, longest + BLOCK_SPARE);
    for _ in 0..iterations {
        for &(offset, len) in items {
            let buffer = &mut buffers[0];
            let at = block_message(buffer, black_box(&input[offset..offset + len]));
            hash_in_memory(
                algorithm,
                &buffer[at..][..len.next_multiple_of(BLOCK_LEN)],
                Point::one("", len),
                1,
                &mut consume,
            );
        }
    }
    keep_buffers(1, longest + BLOCK_SPARE, buffers);
}

fn hash_collection(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    let items = items_of(point, input);
    if matches!(
        algorithm,
        Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt
    ) {
        // BLAKE3-Servil's call for a collection: every item in one call.
        let each = if algorithm == Algorithm::Blake3ServilSt {
            blake3_servil::throughput::hash_each
        } else {
            blake3_servil::throughput::hash_each_multithreaded
        };
        // Each item in its whole blocks (the collection's layout).
        assert_blocks_aligned(input);
        let slices: Vec<(&[blake3_servil::throughput::Block], usize)> = items
            .iter()
            .map(|&(offset, len)| {
                (
                    servil_blocks(&input[offset..offset + len.next_multiple_of(BLOCK_LEN)]),
                    len,
                )
            })
            .collect();
        let mut digests = vec![[0u8; 32]; slices.len()];
        for _ in 0..iterations {
            each(blake3_servil::Mode::Plain, black_box(&slices), &mut digests);
            for digest in &digests {
                consume(digest);
            }
        }
        return;
    }
    for _ in 0..iterations {
        for &(offset, len) in items {
            hash_in_memory(
                algorithm,
                &input[offset..offset + len.next_multiple_of(BLOCK_LEN)],
                Point::one("", len),
                1,
                &mut consume,
            );
        }
    }
}

/*
 * A long message in pieces: one message produced in PIECE_LEN pieces, each
 * copied from `input` as a read would (one copy when the message is
 * shorter, none when empty), then finalized; one digest per pass into
 * `consume`. The copy stands for a read, the cheapest one there is. Every
 * contender gets each piece read into a PIECE_LEN buffer and then hashes
 * it through its incremental API, so reading and hashing take turns.
 */
fn hash_stream(algorithm: Algorithm, input: &[u8], iterations: usize, consume: impl FnMut(&[u8])) {
    use sha1_checked::digest::Update as _;
    use sha2::Digest as _;
    match algorithm {
        Algorithm::Blake3 => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = blake3::Hasher::new();
                pieces(&mut |piece| {
                    hasher.update(piece);
                });
                *hasher.finalize().as_bytes()
            },
            consume,
        ),
        Algorithm::Blake3Rayon => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = blake3::Hasher::new();
                pieces(&mut |piece| {
                    hasher.update_rayon(piece);
                });
                *hasher.finalize().as_bytes()
            },
            consume,
        ),
        Algorithm::Blake3ServilSt => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = blake3_servil::throughput::Hasher::new(blake3_servil::Mode::Plain);
                pieces(&mut |piece| {
                    hasher.update(piece);
                });
                *hasher.finalize().as_bytes()
            },
            consume,
        ),
        Algorithm::Blake3ServilMt => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = blake3_servil::throughput::Hasher::new(blake3_servil::Mode::Plain);
                pieces(&mut |piece| {
                    hasher.update_multithreaded(piece);
                });
                *hasher.finalize().as_bytes()
            },
            consume,
        ),
        Algorithm::Sha256 => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = Sha256::new();
                pieces(&mut |piece| hasher.update(piece));
                let digest: [u8; 32] = hasher.finalize().into();
                digest
            },
            consume,
        ),
        Algorithm::Sha256Ring => each_stream(
            input,
            iterations,
            |pieces| {
                let mut context = ring::digest::Context::new(&ring::digest::SHA256);
                pieces(&mut |piece| context.update(piece));
                let mut digest = [0u8; 32];
                digest.copy_from_slice(context.finish().as_ref());
                digest
            },
            consume,
        ),
        Algorithm::Sha256CommonCrypto => each_stream(
            input,
            iterations,
            |pieces| common_crypto::sha256_pieces(pieces),
            consume,
        ),
        Algorithm::Sha1Dc => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = sha1_checked::Sha1::new();
                pieces(&mut |piece| hasher.update(piece));
                let mut digest = [0u8; 20];
                digest.copy_from_slice(hasher.try_finalize().hash());
                digest
            },
            consume,
        ),
        Algorithm::Sha3_256 => each_stream(
            input,
            iterations,
            |pieces| {
                let mut hasher = sha3::Sha3_256::new();
                pieces(&mut |piece| sha3::Digest::update(&mut hasher, piece));
                let digest: [u8; 32] = hasher.finalize().into();
                digest
            },
            consume,
        ),
    }
}

/*
 * Many messages at once (INTERLEAVED): `iterations` units of
 * INTERLEAVED.chunk bytes of pieces, each piece copied from `input` into
 * the program's piece buffer (as a receive writes it), then fed to its
 * message's incremental API; each message's hash, when it ends, goes to
 * `consume`.
 */
fn hash_interleaved(
    algorithm: Algorithm,
    input: &[u8],
    budget: usize,
    iterations: usize,
    consume: impl FnMut(&[u8]),
) {
    use sha1_checked::digest::Update as _;
    use sha2::Digest as _;
    let key = algorithm.key();
    match algorithm {
        Algorithm::Blake3 => each_interleaved(
            key,
            input,
            budget,
            iterations,
            blake3::Hasher::new,
            per_piece(|h: &mut blake3::Hasher, p| {
                h.update(p);
            }),
            per_message(blake3::Hasher::new, |h| *h.finalize().as_bytes()),
            consume,
        ),
        Algorithm::Blake3Rayon => each_interleaved(
            key,
            input,
            budget,
            iterations,
            blake3::Hasher::new,
            per_piece(|h: &mut blake3::Hasher, p| {
                h.update_rayon(p);
            }),
            per_message(blake3::Hasher::new, |h| *h.finalize().as_bytes()),
            consume,
        ),
        Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt => {
            let update = if algorithm == Algorithm::Blake3ServilSt {
                blake3_servil::throughput::update_each
            } else {
                blake3_servil::throughput::update_each_multithreaded
            };
            let mut digests = Vec::new();
            each_interleaved(
                key,
                input,
                budget,
                iterations,
                || blake3_servil::throughput::Hasher::new(blake3_servil::Mode::Plain),
                update,
                move |hashers: &mut [blake3_servil::throughput::Hasher],
                      ended: &[usize],
                      consume: &mut dyn FnMut(&[u8])| {
                    digests.resize(ended.len(), [0u8; 32]);
                    blake3_servil::throughput::finalize_each(hashers, ended, &mut digests);
                    for (&i, digest) in ended.iter().zip(&digests) {
                        consume(digest);
                        hashers[i].reset();
                    }
                },
                consume,
            )
        }
        Algorithm::Sha256 => each_interleaved(
            key,
            input,
            budget,
            iterations,
            Sha256::new,
            per_piece(|h: &mut Sha256, p| sha2::Digest::update(h, p)),
            per_message(Sha256::new, |h| -> [u8; 32] { h.finalize().into() }),
            consume,
        ),
        Algorithm::Sha256Ring => {
            let new = || ring::digest::Context::new(&ring::digest::SHA256);
            each_interleaved(
                key,
                input,
                budget,
                iterations,
                new,
                per_piece(|h: &mut ring::digest::Context, p| h.update(p)),
                per_message(new, |h| {
                    let mut digest = [0u8; 32];
                    digest.copy_from_slice(h.finish().as_ref());
                    digest
                }),
                consume,
            )
        }
        Algorithm::Sha256CommonCrypto => each_interleaved(
            key,
            input,
            budget,
            iterations,
            common_crypto::Sha256State::new,
            per_piece(|h: &mut common_crypto::Sha256State, p| h.update(p)),
            per_message(common_crypto::Sha256State::new, |h| h.finish()),
            consume,
        ),
        Algorithm::Sha1Dc => each_interleaved(
            key,
            input,
            budget,
            iterations,
            sha1_checked::Sha1::new,
            per_piece(|h: &mut sha1_checked::Sha1, p| h.update(p)),
            per_message(sha1_checked::Sha1::new, |h| {
                let mut digest = [0u8; 20];
                digest.copy_from_slice(h.try_finalize().hash());
                digest
            }),
            consume,
        ),
        Algorithm::Sha3_256 => each_interleaved(
            key,
            input,
            budget,
            iterations,
            sha3::Sha3_256::new,
            per_piece(|h: &mut sha3::Sha3_256, p| sha3::Digest::update(h, p)),
            per_message(sha3::Sha3_256::new, |h| -> [u8; 32] { h.finalize().into() }),
            consume,
        ),
    }
}

/// A turn's pieces through a contender's incremental API, one update each.
fn per_piece<S>(update: impl Fn(&mut S, &[u8])) -> impl Fn(&mut [S], &[(usize, &[u8])]) {
    move |states, pieces| {
        for &(i, piece) in pieces {
            update(&mut states[i], piece);
        }
    }
}

/// The messages that ended in a turn finished one at a time, each slot
/// left with a fresh state.
fn per_message<S, D: AsRef<[u8]>>(
    new: impl Fn() -> S,
    finish: impl Fn(S) -> D,
) -> impl FnMut(&mut [S], &[usize], &mut dyn FnMut(&[u8])) {
    move |states, ended, consume| {
        for &i in ended {
            consume(finish(std::mem::replace(&mut states[i], new())).as_ref());
        }
    }
}

/// The open messages of many messages at once and where the schedule
/// stands, kept from sample to sample on each thread, one per contender.
struct Schedule<S> {
    states: Vec<S>,
    left: Vec<usize>,
    piece: usize,
    opened: usize,
    source: usize,
}

thread_local! {
    static SCHEDULES: std::cell::RefCell<Vec<(&'static str, Box<dyn std::any::Any>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The pieces of many messages at once arrive in turns of this many, as a
/// server's event loop takes them (one wait on epoll or io_uring), each in
/// its own buffer.
const TURN: usize = 64;

#[allow(clippy::too_many_arguments)]
fn each_interleaved<S: 'static>(
    key: &'static str,
    input: &[u8],
    budget: usize,
    iterations: usize,
    new: impl Fn() -> S,
    update: impl Fn(&mut [S], &[(usize, &[u8])]),
    mut finish: impl FnMut(&mut [S], &[usize], &mut dyn FnMut(&[u8])),
    mut consume: impl FnMut(&[u8]),
) {
    let spec = &INTERLEAVED;
    let longest = spec.pieces.iter().copied().max().unwrap();
    assert!(
        input.len() >= budget && input.len() > longest,
        "the pieces come from a source of a call's budget, longer than a piece"
    );
    assert!(
        TURN <= spec.open,
        "a turn holds at most one piece of each open message"
    );
    let kept = SCHEDULES.with(|all| {
        let mut all = all.borrow_mut();
        all.iter()
            .position(|(k, _)| *k == key)
            .map(|i| all.swap_remove(i).1)
    });
    let mut schedule: Schedule<S> = match kept {
        Some(any) => *any.downcast().expect("one schedule type per contender"),
        None => Schedule {
            states: (0..spec.open).map(|_| new()).collect(),
            left: (0..spec.open)
                .map(|n| spec.messages[n % spec.messages.len()])
                .collect(),
            piece: 0,
            opened: spec.open,
            source: 0,
        },
    };
    let mut buffer = STREAM_BUFFER.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    if buffer.len() < TURN * longest {
        buffer = written(TURN * longest, 1u8);
    }
    let mut turn: Vec<(usize, usize, usize)> = Vec::with_capacity(TURN);
    let mut ended: Vec<usize> = Vec::with_capacity(TURN);
    for _ in 0..iterations {
        let mut budget = budget;
        while budget > 0 {
            // A turn: up to TURN pieces, each read into its own buffer.
            turn.clear();
            ended.clear();
            let mut at = 0;
            while turn.len() < TURN && budget > 0 {
                let slot = schedule.piece % spec.open;
                let len = spec.pieces[schedule.piece % spec.pieces.len()]
                    .min(schedule.left[slot])
                    .min(budget);
                schedule.piece += 1;
                if schedule.source + len > input.len() {
                    schedule.source = 0;
                }
                buffer[at..at + len]
                    .copy_from_slice(&black_box(input)[schedule.source..schedule.source + len]);
                schedule.source += len;
                turn.push((slot, at, len));
                at += len;
                schedule.left[slot] -= len;
                budget -= len;
                if schedule.left[slot] == 0 {
                    ended.push(slot);
                    schedule.left[slot] = spec.messages[schedule.opened % spec.messages.len()];
                    schedule.opened += 1;
                }
            }
            let pieces: Vec<(usize, &[u8])> = turn
                .iter()
                .map(|&(slot, at, len)| (slot, &buffer[at..at + len]))
                .collect();
            update(&mut schedule.states, black_box(&pieces));
            finish(&mut schedule.states, &ended, &mut consume);
        }
    }
    STREAM_BUFFER.with(|kept| *kept.borrow_mut() = buffer);
    SCHEDULES.with(|all| all.borrow_mut().push((key, Box::new(schedule))));
}

/// The places a ContinuousMessages stream keeps for its digests: the
/// stream's own limit of messages in flight, so that places never bound
/// it (32 bytes each).
const STREAM_PLACES_PER_MESSAGE: usize = 1024;

thread_local! {
    /// The stream cells' places, made once and kept (by size).
    static STREAM_PLACES: std::cell::RefCell<crate::Map<usize, Vec<blake3_servil::Hash>>> = std::cell::RefCell::new(crate::Map::default());
}

fn take_places(n: usize) -> Vec<blake3_servil::Hash> {
    STREAM_PLACES
        .with(|kept| kept.borrow_mut().remove(&n))
        .unwrap_or_else(|| vec![blake3_servil::Hash::from_bytes([0; 32]); n])
}

fn keep_places(places: Vec<blake3_servil::Hash>) {
    STREAM_PLACES.with(|kept| kept.borrow_mut().insert(places.len(), places));
}

/// A free place from `free`, handing places back (collect) until one is.
fn free_place<'a, D: FnMut(&'a mut [blake3_servil::Hash])>(
    free: &std::cell::RefCell<Vec<&'a mut [blake3_servil::Hash]>>,
    s: &mut blake3_servil::throughput::Stream<'a, D>,
) -> &'a mut [blake3_servil::Hash] {
    loop {
        if let Some(place) = free.borrow_mut().pop() {
            return place;
        }
        s.collect();
    }
}

/*
 * The continuous messages use case: `iterations` messages, each a copy of
 * `input` read into a buffer of the program's (a memory copy, the
 * cheapest read) in pieces of up to PIECE_LEN, then hashed, its digest
 * into `consume`. Every contender but BLAKE3-Servil's multithreaded one
 * hashes each message after reading it, one call for a message of up to
 * PIECE_LEN, its incremental API per piece for a longer one, so reading
 * and hashing take turns; BLAKE3-Servil's queue takes the program's buffers
 * and hashes while the next are read.
 */
/// THE BENCHMARKS REPRESENT THIS IN TWO DIFFERENT WAYS TO USERS (the
/// map's and the report's "buffers owned" and "buffers lent" rows), so if
/// you change the way these cells call a contender (this predicate, or
/// hash_continuous_messages, hash_continuous_batches, hash_lent), you need
/// to update the benchmarks: FROZEN.md's calls, and how the two rows are
/// measured and drawn. Where it holds, the run measures such an owned cell
/// once, as its lent twin (lent_twin).
///
/// Whether a contender's continuous cell with buffers owned runs the same
/// calls on the same input as its cell with buffers lent, at a message of
/// `len` bytes (a batch: `None`): every contender but BLAKE3-Servil's stream,
/// at messages that arrive in one buffer (up to PIECE_LEN) and at batches.
/// The owned cells dispatch through it (hash_continuous_messages,
/// hash_continuous_batches), so a contender that gains a path of its own
/// for owned buffers changes it.
fn owned_runs_lent(algorithm: Algorithm, len: Option<usize>) -> bool {
    algorithm != Algorithm::Blake3ServilMt && len.is_none_or(|len| len <= PIECE_LEN)
}

/// Each owned cell measured as its lent twin (lent_twin) takes the twin's
/// sample size: its calls, and so its work a sample, are the twin's. A
/// sizes file (--calibrate-only) written before this held 1 for those
/// cells, which `regress` gave both sides (merge_sizes: the old side's
/// size): a new side that measures such a cell itself (a contender whose
/// owned calls changed) then timed one call a sample, a stream opened and
/// finished per message (bench #133: +72% read where it was -93%).
fn twins_take_their_sizes(roster: &Roster, batch_iterations: &mut [Vec<usize>]) {
    for (a, sizes) in batch_iterations.iter_mut().enumerate() {
        for p in 0..POINT_COUNT {
            if let Some(t) = roster
                .measures(p)
                .then(|| lent_twin(roster, roster.algorithms[a], p))
                .flatten()
            {
                sizes[p] = sizes[t];
            }
        }
    }
}

/// The lent cell whose samples stand for an owned cell in this run: where
/// the two run the same calls (owned_runs_lent) and the run measures the
/// lent one, the owned cell is not measured again, and its samples are the
/// lent cell's (Zooko, October 31, 2026: one measurement, shown in both
/// rows, where two measurements of it differed by up to 40% in a run).
fn lent_twin(roster: &Roster, algorithm: Algorithm, point_index: usize) -> Option<usize> {
    let point = POINTS[point_index];
    let (lent, len) = match point.use_case {
        UseCase::ContinuousMessages => (UseCase::LentMessages, Some(point.bytes)),
        UseCase::ContinuousBatches => (UseCase::LentBatches, None),
        _ => return None,
    };
    if !owned_runs_lent(algorithm, len) {
        return None;
    }
    let twin = POINTS
        .iter()
        .position(|p| p.use_case == lent && p.label == point.label)?;
    roster.measures(twin).then_some(twin)
}

fn hash_continuous_messages(
    algorithm: Algorithm,
    blocks: &[u8],
    len: usize,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    // The message's `len` bytes; `blocks`, its whole blocks (UseCase::in_blocks).
    let input = &blocks[..len];
    if algorithm == Algorithm::Blake3ServilMt {
        // Each message read straight into the stream's ring (submit_with),
        // its digest into a kept place, used as the place comes back and
        // the place used again.
        let mut places = take_places(STREAM_PLACES_PER_MESSAGE);
        let free = std::cell::RefCell::new(places.chunks_mut(1).collect::<Vec<_>>());
        blake3_servil::throughput::stream(
            blake3_servil::Mode::Plain,
            |place: &mut [blake3_servil::Hash]| {
                consume(place[0].as_bytes());
                free.borrow_mut().push(place);
            },
            |s| {
                for _ in 0..iterations {
                    let message = black_box(input);
                    let place = free_place(&free, s);
                    s.submit_with(
                        message.len(),
                        |at, room| {
                            room.copy_from_slice(&message[at..][..room.len()]);
                            room.len()
                        },
                        &mut place[0],
                    );
                }
                s.finish();
            },
        );
        drop(free);
        return keep_places(places);
    }
    if !owned_runs_lent(algorithm, Some(len)) {
        return hash_stream(algorithm, input, iterations, consume);
    }
    // The lent cell's calls (owned_runs_lent): each message read into a
    // kept buffer, then hashed through the one-shot call.
    let one = Point {
        label: "",
        bytes: len,
        messages: 1,
        use_case: UseCase::LentMessages,
    };
    hash_lent(algorithm, input, one, iterations, consume)
}

/*
 * The continuous batches use case: `iterations` batches, each a copy of
 * `input` read into a buffer of the program's, then hashed, the batch's
 * digests into `consume`. Every contender but BLAKE3-Servil's
 * multithreaded one hashes each batch after reading it, as it hashes a
 * batch in memory (hash_batch); BLAKE3-Servil's queue of fixed-length messages
 * takes the program's buffers and hashes while the next are read.
 */
fn hash_continuous_batches(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    if !owned_runs_lent(algorithm, None) {
        // Each batch read straight into the stream's ring (submit_batch_with),
        // its digests into a kept place of the batch's size, handed to
        // `consume` whole as the place comes back, as the queue's batch is,
        // and the place used again. Sound: a Hash is its 32 bytes
        // (repr(transparent)).
        let batches = ((8 << 20) / input.len().max(1)).clamp(2, 1024);
        let mut places = take_places(batches * point.messages);
        let free =
            std::cell::RefCell::new(places.chunks_mut(point.messages.max(1)).collect::<Vec<_>>());
        blake3_servil::throughput::stream(
            blake3_servil::Mode::Plain,
            |place: &mut [blake3_servil::Hash]| {
                consume(unsafe {
                    core::slice::from_raw_parts(place.as_ptr().cast::<u8>(), place.len() * 32)
                });
                free.borrow_mut().push(place);
            },
            |s| {
                for _ in 0..iterations {
                    let batch = black_box(input);
                    let place = free_place(&free, s);
                    s.submit_batch_with(
                        MESSAGE_LEN,
                        |at, room| {
                            room.copy_from_slice(&batch[at..][..room.len()]);
                            room.len()
                        },
                        &mut place[..point.messages],
                    );
                }
                s.finish();
            },
        );
        drop(free);
        return keep_places(places);
    }
    // The lent cell's calls (owned_runs_lent): each batch read into a kept
    // buffer, then hashed through the batch call (the point's own use
    // case: in blocks and a batch, as the lent one is).
    hash_lent(algorithm, input, point, iterations, consume)
}

/// The producer lends a whole message or batch for one synchronous call.
/// Each read is timed; the kept buffer and digest space avoid allocation
/// after calibration, just as the queue's producer does.
fn hash_lent(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    // Messages and batches lent alike are in blocks (UseCase::in_blocks),
    // and hash_in_memory reads nothing else of the use case.
    let mut buffers = take_buffers(1, input.len() + BLOCK_SPARE);
    for _ in 0..iterations {
        let buffer = &mut buffers[0];
        let at = block_message(buffer, black_box(input));
        hash_in_memory(
            algorithm,
            &buffer[at..][..input.len().next_multiple_of(BLOCK_LEN)],
            point,
            1,
            &mut consume,
        );
    }
    keep_buffers(1, input.len() + BLOCK_SPARE, buffers);
}

/*
 * A Merkle tree (UseCase::MerkleTree, MERKLE): `iterations` trees, each
 * over the point's leaves written into the program's kept buffer (the
 * prover producing its codeword in place), then hashed layer by layer
 * with the contender's batch call (hash_messages): the leaves, then each
 * layer of parents, a parent two children's digests side by side, to the
 * root, every layer kept in buffers of the program's; the root into
 * `consume`.
 */
/*
 * A tree's two kinds of node hash, kept apart (Zooko, October 19): were a
 * leaf hashed as a parent is, a parent's two child digests would pass for
 * a leaf, and a proof could open a leaf the tree never held (the standard
 * Merkle tree vulnerability). The BLAKE3 contenders hash leaves and
 * parents as keyed hashes under two keys derived from their contexts,
 * BLAKE3's own domain separation, at a batch call's speed; every other
 * hash prefixes a byte, 0x00 to a leaf and 0x01 to a parent (RFC 6962).
 */
#[derive(Clone, Copy, PartialEq, Debug)]
enum Node {
    Leaf,
    Parent,
}

impl Node {
    fn key(self) -> &'static [u8; 32] {
        static KEYS: std::sync::LazyLock<[[u8; 32]; 2]> = std::sync::LazyLock::new(|| {
            [
                blake3::derive_key("bench-hashes 2026-10-19 Merkle tree leaf", b""),
                blake3::derive_key("bench-hashes 2026-10-19 Merkle tree parent", b""),
            ]
        });
        &KEYS[self as usize]
    }

    fn prefix(self) -> u8 {
        self as u8
    }
}

/// One layer of a tree: `input`'s messages of `message_len` bytes hashed as
/// `node`s, each contender's way (Node), every digest into `consume`.
fn hash_tree_layer(
    algorithm: Algorithm,
    input: &[u8],
    message_len: usize,
    node: Node,
    consume: impl FnMut(&[u8]),
) {
    use sha1_checked::digest::Update as _;
    use sha2::Digest as _;
    assert_blocks_aligned(input);
    let messages = input.len() / message_len;
    let prefix = [node.prefix()];
    match algorithm {
        Algorithm::Blake3 => blake3_batch(input, message_len, Some(node.key()), 1, consume),
        Algorithm::Blake3ServilSt => servil_batch(
            input,
            messages,
            message_len,
            1,
            |i, l, o| {
                blake3_servil::throughput::hash_many(
                    blake3_servil::Mode::Keyed(node.key()),
                    &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                    o,
                )
            },
            consume,
        ),
        Algorithm::Blake3ServilMt => servil_batch(
            input,
            messages,
            message_len,
            1,
            |i, l, o| {
                blake3_servil::throughput::hash_many_multithreaded(
                    blake3_servil::Mode::Keyed(node.key()),
                    &blake3_servil::throughput::Batch::from_blocks(servil_blocks(i), l),
                    o,
                )
            },
            consume,
        ),
        Algorithm::Sha256 => each_message(
            input,
            message_len,
            1,
            |m| -> [u8; 32] {
                Sha256::new()
                    .chain_update(prefix)
                    .chain_update(m)
                    .finalize()
                    .into()
            },
            consume,
        ),
        Algorithm::Sha3_256 => each_message(
            input,
            message_len,
            1,
            |m| -> [u8; 32] {
                sha3::Sha3_256::new()
                    .chain_update(prefix)
                    .chain_update(m)
                    .finalize()
                    .into()
            },
            consume,
        ),
        Algorithm::Sha256Ring => each_message(
            input,
            message_len,
            1,
            |m| {
                let mut context = ring::digest::Context::new(&ring::digest::SHA256);
                context.update(&prefix);
                context.update(m);
                context.finish()
            },
            consume,
        ),
        Algorithm::Sha256CommonCrypto => each_message(
            input,
            message_len,
            1,
            |m| {
                common_crypto::sha256_pieces(&mut |piece: &mut dyn FnMut(&[u8])| {
                    piece(&prefix);
                    piece(m);
                })
            },
            consume,
        ),
        Algorithm::Sha1Dc => each_message(
            input,
            message_len,
            1,
            |m| {
                let mut hasher = sha1_checked::Sha1::new();
                hasher.update(&prefix);
                hasher.update(m);
                let mut digest = [0u8; 20];
                digest.copy_from_slice(hasher.try_finalize().hash());
                digest
            },
            consume,
        ),
        Algorithm::Blake3Rayon => {
            panic!("{} takes no part in a Merkle tree", algorithm.key())
        }
    }
}

fn hash_merkle(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    assert!(
        point.messages.is_power_of_two() && input.len() == point.messages * MERKLE.leaf_len,
        "a tree's leaves are a power of two of MERKLE.leaf_len bytes"
    );
    let depth = point.messages.trailing_zeros() as usize;
    /* The leaves, then each layer's digests: a buffer each, of its size. */
    let size = |k: usize| {
        if k == 0 {
            input.len()
        } else {
            (point.messages >> (k - 1)) * algorithm.digest_len()
        }
    };
    /* Every layer is a batch: each buffer starts its layer at a 64-byte
     * boundary (batch_start), at `at[k]`. */
    let mut buffers: Vec<Vec<u8>> = (0..depth + 2)
        .map(|k| take_buffers(1, size(k) + BLOCK_SPARE).pop().unwrap())
        .collect();
    let mut at = vec![0; depth + 2];
    for _ in 0..iterations {
        at[0] = block_start(&mut buffers[0]);
        buffers[0].extend_from_slice(black_box(input));
        let mut message_len = MERKLE.leaf_len;
        for layer in 0..=depth {
            let (done, rest) = buffers.split_at_mut(layer + 1);
            let (from, to) = (&done[layer][at[layer]..], &mut rest[0]);
            at[layer + 1] = block_start(to);
            let node = if layer == 0 { Node::Leaf } else { Node::Parent };
            hash_tree_layer(algorithm, from, message_len, node, |digest| {
                to.extend_from_slice(digest)
            });
            message_len = 2 * algorithm.digest_len();
        }
        consume(&buffers[depth + 1][at[depth + 1]..]);
    }
    for (k, buffer) in buffers.into_iter().enumerate() {
        keep_buffers(1, size(k) + BLOCK_SPARE, vec![buffer]);
    }
}

thread_local! {
    /*
     * The continuous use cases' read buffers, kept from one sample to the
     * next on each thread, a set per count and length: a fresh allocation
     * of a large buffer would put its page faults inside the timed sample,
     * and one set shared by every cell had each cell free or grow the
     * last one's inside its own sample (SHA-256's 1 KiB messages took 2-5%
     * more cycles per byte after BLAKE3-Servil's queue's 1024 buffers,
     * September 28, 2026).
     */
    static INPUT_BUFFERS: std::cell::RefCell<crate::Map<(usize, usize), Vec<Vec<u8>>>> = std::cell::RefCell::new(crate::Map::default());
}

/// `count` empty buffers with room for `len` bytes, this thread's kept set
/// for the pair (made on first use); give them back with keep_buffers.
fn take_buffers(count: usize, len: usize) -> Vec<Vec<u8>> {
    let mut kept = INPUT_BUFFERS
        .with(|kept| kept.borrow_mut().remove(&(count, len)))
        .unwrap_or_default();
    assert!(
        kept.len() <= count,
        "a kept set holds the buffers taken for it"
    );
    kept.resize_with(count, || {
        let mut buffer = written(len, 1u8);
        buffer.clear();
        buffer
    });
    for buffer in &mut kept {
        buffer.clear();
    }
    kept
}

/*
 * A buffer of `len` bytes, every byte written, so its pages are mapped
 * before any sample: a kept buffer of a real program has met its first
 * use long before. vec![0; len] is no substitute: it takes zeroed pages
 * the system maps only at their first write, inside the first sample
 * (fork tmp/lentprobe: 330 against 217 us a queue batch).
 */
fn written<T: Clone>(len: usize, value: T) -> Vec<T> {
    vec![value; len]
}

/*
 * Inputs in blocks (UseCase::in_blocks): every batch, and every message of
 * a cell where servil's call is a throughput call, starts at a 64-byte
 * boundary with whole 64-byte blocks behind it, for every contender alike
 * (Zooko, October 24: BLAKE3-Servil's throughput calls take their inputs as
 * Blocks, and the bench aligns the inputs of every cell that has a
 * performance-call contender). A message of the other cells starts where
 * the allocator puts it (glibc places a large one 16 bytes into a line), as
 * a real program's does. A kept buffer for blocks has BLOCK_SPARE bytes to
 * spare: the message is written from the first boundary within
 * (block_start) and padded with zeros to its last block's end
 * (block_message).
 */
const BLOCK_LEN: usize = 64;
const BLOCK_SPARE: usize = 2 * (BLOCK_LEN - 1);

/// Empties `buffer` and fills it up to its first 64-byte boundary, where
/// its message or batch is then written: the offset. With BLOCK_SPARE
/// bytes beyond the input in its capacity, the writes never move it.
fn block_start(buffer: &mut Vec<u8>) -> usize {
    buffer.clear();
    let at = buffer.as_ptr().align_offset(BLOCK_LEN);
    assert!(
        at < BLOCK_LEN,
        "a byte buffer reaches a 64-byte boundary within 63 bytes"
    );
    buffer.resize(at, 0);
    at
}

/// `input` written into `buffer` in blocks: from its first boundary, then
/// zeros to its last block's end; the offset.
fn block_message(buffer: &mut Vec<u8>, input: &[u8]) -> usize {
    let at = block_start(buffer);
    buffer.extend_from_slice(input);
    buffer.resize(at + input.len().next_multiple_of(BLOCK_LEN), 0);
    at
}

/// `input` copied into blocks (block_message): the copy and its offset.
fn block_copy(input: &[u8]) -> (Vec<u8>, usize) {
    let mut copy = Vec::with_capacity(input.len() + BLOCK_SPARE);
    let at = block_message(&mut copy, input);
    (copy, at)
}

/// The placement of an input in blocks, at the contenders' door.
fn assert_blocks_aligned(input: &[u8]) {
    // An empty input's pointer says nothing of placement.
    assert!(
        input.is_empty() || input.as_ptr().align_offset(BLOCK_LEN) == 0,
        "an input in blocks starts at a 64-byte boundary"
    );
}

/// An input in blocks (a batch, or a message with its whole blocks) as
/// servil's throughput calls take it, Blocks: the door
/// (assert_blocks_aligned) has checked its placement for every contender
/// alike, so servil's calls carry no check of their own.
fn servil_blocks(blocks: &[u8]) -> &[blake3_servil::throughput::Block] {
    debug_assert!(
        blake3_servil::throughput::Block::from_bytes(blocks).is_some(),
        "an input in whole aligned blocks"
    );
    if blocks.is_empty() {
        // An empty slice's pointer is no Block's.
        return &[];
    }
    // Sound: at a 64-byte boundary (the door) and whole blocks (the slice
    // covers them: input_len, block_message, the collection's layout); a
    // Block is 64 bytes, no padding.
    unsafe { core::slice::from_raw_parts(blocks.as_ptr().cast(), blocks.len() / BLOCK_LEN) }
}

fn keep_buffers(count: usize, len: usize, buffers: Vec<Vec<u8>>) {
    INPUT_BUFFERS.with(|kept| kept.borrow_mut().insert((count, len), buffers));
}

/*
 * How many buffers a continuous cell's warm-up covers (FROZEN.md): about
 * IN_FLIGHT_BYTES in all, or IN_FLIGHT_BUFFERS buffers, whichever is
 * fewer, and two at least: what a pipelined API holds in flight to cover
 * its round trip (Little's law: in flight = rate x round trip).
 */
const IN_FLIGHT_BYTES: usize = 1 << 20;
const IN_FLIGHT_BUFFERS: usize = 1024;

fn in_flight(buffer_len: usize) -> usize {
    (IN_FLIGHT_BYTES / buffer_len.max(1)).clamp(2, IN_FLIGHT_BUFFERS)
}

/// The pieces of one stream, each copied into a PIECE_LEN buffer (the
/// producer's read) and handed to the callback in order.
type Pieces<'a> = &'a mut dyn FnMut(&mut dyn FnMut(&[u8]));

/// `iterations` streams of `input` through `hash`, which receives the
/// pieces, copied, in order; the digest of each goes to `consume`.
#[inline(always)]
fn each_stream<D: AsRef<[u8]>>(
    input: &[u8],
    iterations: usize,
    hash: impl Fn(Pieces) -> D,
    mut consume: impl FnMut(&[u8]),
) {
    /* The program's read buffer, kept from call to call (a fresh one per
     * message would put its allocation and zeroing, microseconds after the
     * gap, inside every short stream's time). */
    let mut buffer = STREAM_BUFFER.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    if buffer.len() < PIECE_LEN {
        buffer = written(PIECE_LEN, 1u8);
    }
    for _ in 0..iterations {
        let mut pieces = |each: &mut dyn FnMut(&[u8])| {
            for piece in black_box(input).chunks(PIECE_LEN) {
                buffer[..piece.len()].copy_from_slice(piece);
                each(black_box(&buffer[..piece.len()]));
            }
        };
        consume(hash(&mut pieces).as_ref());
    }
    STREAM_BUFFER.with(|kept| *kept.borrow_mut() = buffer);
}

thread_local! {
    /// The streaming use case's read buffer, kept as INPUT_BUFFERS are.
    static STREAM_BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/*
 * What the benchmark asks of BLAKE3-Servil, by contender and use case:
 * BLAKE3-Servil's call and its usage pattern. FROZEN.md lists the same, with the
 * reasons; a test compares them, so a change here needs FROZEN.md changed,
 * with Zooko's decision. Keep the dispatch (hash_batch) calling exactly
 * these.
 */
#[cfg(test)]
const SERVIL_CALLS: [(Algorithm, UseCase, &str); 30] = [
    (Algorithm::Blake3ServilSt, UseCase::InterleavedTurn, "the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after other work: throughput::update_each, then throughput::finalize_each for the messages that ended in it"),
    (Algorithm::Blake3ServilSt, UseCase::IdleInterleavedTurn, "the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after idling: throughput::update_each, then throughput::finalize_each for the messages that ended in it"),
    (Algorithm::Blake3ServilMt, UseCase::InterleavedTurn, "the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after other work: throughput::update_each_multithreaded, then throughput::finalize_each for the messages that ended in it"),
    (Algorithm::Blake3ServilMt, UseCase::IdleInterleavedTurn, "the Hashers of the 256 open messages; one turn a call (up to 64 pieces, 256 KiB), each after idling: throughput::update_each_multithreaded, then throughput::finalize_each for the messages that ended in it"),
    (Algorithm::Blake3ServilSt, UseCase::MerkleTree, "throughput::hash_many(Mode::Keyed(key), &Batch::from_blocks(layer, len), out) for each layer, a tree a sample: the leaves (256 B) written into a kept buffer and hashed under the leaf key, then each layer of parents (64 B, two children's digests) under the parent key into a kept buffer of its own, to the root"),
    (Algorithm::Blake3ServilMt, UseCase::MerkleTree, "throughput::hash_many_multithreaded(Mode::Keyed(key), &Batch::from_blocks(layer, len), out) for each layer, a tree a sample: the leaves (256 B) written into a kept buffer and hashed under the leaf key, then each layer of parents (64 B, two children's digests) under the parent key into a kept buffer of its own, to the root"),
    (Algorithm::Blake3ServilSt, UseCase::OneMessage, "convenience::hash(input), each call after other work"),
    (Algorithm::Blake3ServilSt, UseCase::ManyMessages, "throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::OneMessage, "throughput::hash_multithreaded(Mode::Plain, blocks, len), each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ManyMessages, "throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after other work"),
    (Algorithm::Blake3ServilSt, UseCase::ManyLongMessages, "throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 4096), out), the batch borrowed in place, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ManyLongMessages, "throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 4096), out), the batch borrowed in place, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousMessages, "throughput::stream(Mode::Plain, handler, body), one a sample: one message after another, each read straight into the stream's ring (submit_with), its digest into one of 1024 kept places, used as the place comes back and the place used again"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousBatches, "throughput::stream(Mode::Plain, handler, body), one a sample: one batch after another, each read straight into the stream's ring (submit_batch_with), its digests into a kept place of the batch's size (enough that the ring fills first, up to 1024), the place used whole as it comes back and used again"),
    (Algorithm::Blake3ServilSt, UseCase::ContinuousMessages, "throughput::hash(Mode::Plain, blocks, len), one message after another, each read into a kept buffer of the program's in whole blocks and hashed on its thread (a Hasher fed 64 KiB pieces above 64 KiB)"),
    (Algorithm::Blake3ServilSt, UseCase::ContinuousBatches, "throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, one batch after another, each read into a kept buffer of the program's and hashed on its thread"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousCollection, "throughput::stream(Mode::Plain, handler, body), one a sample: the collection's messages one after another, each read straight into the stream's ring (submit_with), its digest into one of 1024 kept places, used as the place comes back and the place used again"),
    (Algorithm::Blake3ServilSt, UseCase::ContinuousCollection, "throughput::hash(Mode::Plain, blocks, len), the collection's messages one after another, each read into a kept buffer of the program's in whole blocks and hashed on its thread"),
    (Algorithm::Blake3ServilSt, UseCase::LentMessages, "throughput::hash(Mode::Plain, blocks, len), one message after another, each read into a kept buffer in whole blocks and lent until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::Interleaved, "a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; throughput::update_each per turn, then throughput::finalize_each for the messages that ended in it, each buffer lent until the calls return"),
    (Algorithm::Blake3ServilSt, UseCase::Collection, "throughput::hash_each(Mode::Plain, items, out), every item of the collection in one call, in memory"),
    (Algorithm::Blake3ServilSt, UseCase::LentBatches, "throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, batches one after another, each read into a kept buffer and lent with kept digests until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::LentMessages, "throughput::hash_multithreaded(Mode::Plain, blocks, len), one message after another, each read into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::Interleaved, "a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; throughput::update_each_multithreaded per turn, then throughput::finalize_each for the messages that ended in it, each buffer lent until the calls return"),
    (Algorithm::Blake3ServilMt, UseCase::Collection, "throughput::hash_each_multithreaded(Mode::Plain, items, out), every item of the collection in one call, in memory"),
    (Algorithm::Blake3ServilMt, UseCase::LentBatches, "throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, batches one after another, each read into a kept buffer and lent with kept digests until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::IdleOneMessage, "convenience::hash(input), each call after idling"),
    (Algorithm::Blake3ServilSt, UseCase::IdleManyMessages, "throughput::hash_many(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after idling"),
    (Algorithm::Blake3ServilMt, UseCase::IdleOneMessage, "throughput::hash_multithreaded(Mode::Plain, blocks, len), each call after idling"),
    (Algorithm::Blake3ServilMt, UseCase::IdleManyMessages, "throughput::hash_many_multithreaded(Mode::Plain, &Batch::from_blocks(blocks, 64), out), the batch borrowed in place, each call after idling"),
];

/// The frozen contract as text, from the code's own tables: FROZEN.md's
/// block must read the same (the test frozen_contract_matches_frozen_md).
#[cfg(test)]
fn frozen_contract() -> String {
    let mut text = String::new();
    for use_case in UseCase::ALL {
        let labels: Vec<&str> = POINTS
            .iter()
            .filter(|point| point.use_case == use_case)
            .map(|point| point.label)
            .collect();
        text += &format!("use case {use_case:?}: {}\n", labels.join(", "));
    }
    let keys = |scenarios: &[Scenario]| {
        scenarios
            .iter()
            .map(|scenario| scenario.key())
            .collect::<Vec<_>>()
            .join(", ")
    };
    text += &format!("scenarios: {}\n", keys(&Scenario::ALL));
    let shared: Vec<String> = UseCase::ALL
        .into_iter()
        .filter(|&use_case| Algorithm::Blake3ServilMt.multithreaded() && !use_case.after_gap())
        .map(|use_case| format!("{use_case:?}"))
        .collect();
    text += &format!("shared measures: {}\n", shared.join(", "));
    text += "shared placements: two cores under one last-level cache; two cores under two (where the machine has two); where it offers no pair (macOS), wherever the OS runs them\n";
    let multithreaded: Vec<&str> = Algorithm::ALL
        .iter()
        .filter(|a| a.multithreaded())
        .map(|a| a.key())
        .collect();
    text += &format!("shared contenders: {}\n", multithreaded.join(", "));
    for (algorithm, use_case, call) in SERVIL_CALLS {
        assert!(
            algorithm.takes_part(use_case),
            "{} takes part in {use_case:?}",
            algorithm.key()
        );
        text += &format!("{} {use_case:?}: {call}\n", algorithm.key());
    }
    text += &overview::frozen_lines();
    text
}

/// `iterations` passes over the batch through one of BLAKE3-Servil's batch entry
/// points, which take the messages back to back in one buffer with their
/// length and fill a slice of digests; the whole batch's digests go to
/// `consume` per pass.
#[inline(always)]
fn servil_batch(
    input: &[u8],
    messages: usize,
    message_len: usize,
    iterations: usize,
    hash_many: impl Fn(&[u8], usize, &mut [[u8; 32]]),
    mut consume: impl FnMut(&[u8]),
) {
    assert_eq!(input.len(), messages * message_len);
    let mut digests = take_batch_digests(messages);
    for _ in 0..iterations {
        hash_many(black_box(input), message_len, &mut digests[..messages]);
        consume(digests[..messages].as_flattened());
    }
    keep_batch_digests(digests);
}

thread_local! {
    /* The batches' digest space, kept from call to call: a fresh one per
     * call would put its allocation and zeroing (page faults for the
     * largest batches) inside every call after the gap. */
    static BATCH_DIGESTS: std::cell::RefCell<Vec<[u8; 32]>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// This thread's kept digest space, at least `messages` digests long.
/// Keep its initialized length when a smaller cell borrows a prefix: growing
/// a truncated vector would zero the outputs again inside a later sample.
fn take_batch_digests(messages: usize) -> Vec<[u8; 32]> {
    let mut digests = BATCH_DIGESTS.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    if digests.len() < messages {
        digests.resize(messages, [0; 32]);
    }
    digests
}

fn keep_batch_digests(digests: Vec<[u8; 32]>) {
    BATCH_DIGESTS.with(|kept| *kept.borrow_mut() = digests);
}

/// `iterations` passes over the batch through the blake3 crate's hidden
/// batch function, `blake3::platform::Platform::hash_many::<N>`, sixteen
/// messages per call, with the flags and counter that make each digest the
/// message's hash: how a program reaches the official crate's batch speed
/// (WHIR's Merkle trees call it this way). The whole batch's digests go to
/// `consume` per pass.
#[inline(always)]
fn blake3_batch(
    input: &[u8],
    message_len: usize,
    key: Option<&[u8; 32]>,
    iterations: usize,
    consume: impl FnMut(&[u8]),
) {
    match message_len {
        MESSAGE_LEN => blake3_batch_of::<MESSAGE_LEN>(input, key, iterations, consume),
        256 => blake3_batch_of::<256>(input, key, iterations, consume),
        other => panic!("no blake3 batch of {other}-byte messages"),
    }
}

#[inline(always)]
fn blake3_batch_of<const N: usize>(
    input: &[u8],
    key: Option<&[u8; 32]>,
    iterations: usize,
    mut consume: impl FnMut(&[u8]),
) {
    /* BLAKE3's IV and flags, private in the crate. */
    const IV: [u32; 8] = [
        0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB,
        0x5BE0CD19,
    ];
    const CHUNK_START: u8 = 1 << 0;
    const CHUNK_END: u8 = 1 << 1;
    const ROOT: u8 = 1 << 3;
    const KEYED_HASH: u8 = 1 << 4;
    /* A keyed hash's key words, little-endian, in place of the IV. */
    let (words, keyed) = match key {
        Some(k) => (
            core::array::from_fn(|i| u32::from_le_bytes(k[4 * i..4 * i + 4].try_into().unwrap())),
            KEYED_HASH,
        ),
        None => (IV, 0),
    };
    let (messages, rest) = input.as_chunks::<N>();
    assert!(rest.is_empty(), "a batch is whole {N}-byte messages");
    let platform = blake3::platform::Platform::detect();
    let mut digests = take_batch_digests(messages.len());
    for _ in 0..iterations {
        for (group, out) in black_box(messages)
            .chunks(16)
            .zip(digests[..messages.len()].chunks_mut(16))
        {
            let mut table = [&group[0]; 16];
            for (slot, message) in table.iter_mut().zip(group) {
                *slot = message;
            }
            platform.hash_many::<N>(
                &table[..group.len()],
                &words,
                0,
                blake3::IncrementCounter::No,
                keyed,
                CHUNK_START,
                CHUNK_END | ROOT,
                out.as_flattened_mut(),
            );
        }
        consume(digests[..messages.len()].as_flattened());
    }
    keep_batch_digests(digests);
}

/// `iterations` passes over the input, each hashing every message with one
/// call of `hash`, digest by digest into `consume`.
#[inline(always)]
fn each_message<D: AsRef<[u8]>>(
    input: &[u8],
    message_len: usize,
    iterations: usize,
    hash: impl Fn(&[u8]) -> D,
    mut consume: impl FnMut(&[u8]),
) {
    for _ in 0..iterations {
        if message_len == input.len() {
            consume(hash(black_box(input)).as_ref());
        } else {
            for message in black_box(input).chunks_exact(message_len) {
                consume(hash(message).as_ref());
            }
        }
    }
}

/*
 * The duo measurement: two independent copies of a contender run at once,
 * each on its own thread over its own input, and each sample is one copy's own elapsed time. A hash that takes the whole
 * machine to go faster alone runs beside a copy of itself here and shows
 * what that costs; a hash that leaves room finishes at its solo speed.
 * Every run measures duo; with --solo every sample interval takes a solo
 * sample and then a duo sample of the same batch, so each cell reports
 * both, side by side, from the same moment of the run.
 *
 * The two copy threads persist for the run. The caller posts a job, and
 * each copy takes it and polls a generation counter (yielding between
 * polls) until the caller has seen both arrive and flips it. Both copies
 * are then on a CPU, in the instruction stream, when the release happens,
 * and each starts its sample as its own first act (take_sample): for a
 * continuous use case it reads the sample clock, and the later finish is
 * the later of the two finish times, each measured from that copy's own
 * start; for a synchronous one it walks its own work buffer, prepares the input,
 * and times each call alone. Both gaps begin at the shared release; each
 * call follows its own preparation. A release through a barrier or condition variable would
 * instead need the operating system to wake a sleeping thread, which on a
 * busy machine can take hundreds of microseconds (a two-CPU virtual
 * machine measured 300 µs when the caller's own thread had just finished
 * a batch on that CPU); a copy that starts late finishes late through no
 * fault of the contender, and the pair's later finish would carry the
 * wake, not the hash. A polling copy needs no wake.
 *
 * The caller then sleeps on a condition variable until both copies have
 * finished. The finishes are sample-clock reads the copies take
 * themselves, so the caller's own wake, however slow, enters no sample;
 * and a sleeping caller holds no CPU, which matters on a machine with as
 * many CPUs as copies. Between jobs the copies sleep the same way, so an
 * idle run holds no CPU either.
 *
 * Reported per byte per copy: a duo sample over N bytes per copy is
 * divided by N, so the number reads as "the time one hash costs when
 * another runs beside it".
 */
struct Duo {
    /// A job for both threads, or None between jobs.
    job: std::sync::Mutex<Option<DuoJob>>,
    posted: std::sync::Condvar,
    /// Copies that hold the job and are spinning, ready to start.
    ready: std::sync::atomic::AtomicUsize,
    /// Advances once both copies are ready: the release.
    generation: std::sync::atomic::AtomicU64,
    /// Copies still taking their samples. The copy that finishes first
    /// keeps hashing, unsampled, until this reaches zero, so the other's
    /// last samples are taken under contention like its first (#105: the
    /// tail ran alone, about 1% optimistic).
    sampling: std::sync::atomic::AtomicUsize,
    /// Each copy's finished work; None while a copy is still running.
    finished: std::sync::Mutex<[Option<DuoWork>; 2]>,
    done: std::sync::Condvar,
}

/// One copy's work for a job: its train's samples, in order, and when its
/// work ended (the load windows' scale), after the other copy's last
/// sample began.
struct DuoWork {
    samples: Vec<DuoCopy>,
    worked_until_ns: u64,
}

/// Whether each copy's last sample began while the other copy still
/// worked: the shared contract (#105), which Duo keeps by having the copy
/// that finishes first hash on until the other has finished.
fn both_contended(first: &DuoWork, second: &DuoWork) -> bool {
    let began_under = |copy: &DuoWork, other: &DuoWork| {
        copy.samples
            .last()
            .is_none_or(|last| last.started_ns <= other.worked_until_ns)
    };
    began_under(first, second) && began_under(second, first)
}

/// One copy's part of a duo sample: nanoseconds from its own start to its
/// finish, and its thread's counts across the sample (read outside the
/// timed interval; None where the platform counts none).
#[derive(Clone, Copy)]
struct DuoCopy {
    /// Start of this copy's sample on the load windows' scale, outside timing.
    started_ns: u64,
    elapsed_ns: u64,
    counts: Option<clocks::Counts>,
    /// The CPU this copy's thread was on when the sample ended, read after
    /// the timed interval (bench-hashes #108: where the two shared copies
    /// ran, an SMT pair, one L3 domain, one SME cluster or two, decides
    /// what they share; a reader can now tell). None where the OS says none.
    cpu: Option<u32>,
    /// The process's CPU time across the sample, all its threads (read
    /// outside the timed interval): None for a sample after gaps (their
    /// other work is no part of the call's), or where the platform has no
    /// process CPU clock.
    cpu_ns: Option<u64>,
    /// The CPU package's energy across the sample, in nanojoules
    /// (clocks::energy, read outside the timed interval beside the CPU
    /// time): the whole package's, so a quiet machine's alone means the
    /// call's. None where cpu_ns is, or where the platform counts none.
    energy_nj: Option<u64>,
}

#[derive(Clone, Copy)]
struct DuoJob {
    algorithm: Algorithm,
    inputs: [*const [u8]; 2],
    point: Point,
    iterations: usize,
    /// Each copy's warm-up and samples, back to back from the release.
    train: Train,
    /// The CPU each copy's thread runs on for this job, or None where the
    /// OS places them (Placement).
    cpus: Option<[usize; 2]>,
    /// Which workers have taken this job (a bit each).
    taken: u8,
}

// The input pointers are borrows of the caller's buffers, which outlive the
// job: run() returns only after both finishes are read.
unsafe impl Send for DuoJob {}

impl Duo {
    fn new() -> &'static Self {
        use std::sync::atomic::{AtomicU64, AtomicUsize};
        let duo: &'static Self = Box::leak(Box::new(Self {
            job: std::sync::Mutex::new(None),
            posted: std::sync::Condvar::new(),
            ready: AtomicUsize::new(0),
            generation: AtomicU64::new(0),
            sampling: AtomicUsize::new(0),
            finished: std::sync::Mutex::new([None, None]),
            done: std::sync::Condvar::new(),
        }));
        for copy in 0..2 {
            std::thread::Builder::new()
                .name(format!("duo-copy-{copy}"))
                .spawn(move || duo.worker(copy))
                .expect("spawning a duo copy thread");
        }
        duo
    }

    /// Run `train` of `algorithm`'s batches of `iterations` on both threads
    /// at once, copy 0 over `input` and copy 1 over `other`, each copy its
    /// warm-up and then its samples back to back from one release; returns
    /// each copy's samples, each its time from its own start and its counts.
    fn run(
        &self,
        algorithm: Algorithm,
        input: &[u8],
        other: &[u8],
        point: Point,
        iterations: usize,
        train: Train,
        cpus: Option<[usize; 2]>,
    ) -> [DuoWork; 2] {
        use std::sync::atomic::Ordering;
        assert_eq!(
            input.len(),
            other.len(),
            "the two copies hash inputs of one size"
        );
        {
            let mut job = self.job.lock().unwrap();
            assert!(job.is_none(), "one duo job at a time");
            *job = Some(DuoJob {
                algorithm,
                inputs: [input, other],
                point,
                iterations,
                train,
                cpus,
                taken: 0,
            });
            self.sampling.store(2, Ordering::Release);
            self.posted.notify_all();
        }
        /*
         * Both copies spinning: release. The caller yields between polls
         * so that on a machine with as many CPUs as copies the copies get
         * theirs; a copy is spinning within microseconds of the post.
         */
        while self.ready.load(Ordering::Acquire) < 2 {
            std::thread::yield_now();
        }
        self.ready.store(0, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
        /* Sleep until both copies have finished. */
        let mut finished = self.finished.lock().unwrap();
        while finished.iter().any(Option::is_none) {
            finished = self.done.wait(finished).unwrap();
        }
        std::mem::take(&mut *finished).map(|copy| copy.unwrap())
    }

    fn worker(&self, copy: usize) {
        use std::sync::atomic::Ordering;
        let mut seen = self.generation.load(Ordering::Acquire);
        loop {
            let job = {
                let mut job = self.job.lock().unwrap();
                loop {
                    if let Some(current) = job.as_mut() {
                        if current.taken & (1 << copy) == 0 {
                            current.taken |= 1 << copy;
                            let taken = *current;
                            if taken.taken == 0b11 {
                                *job = None;
                            }
                            break taken;
                        }
                    }
                    job = self.posted.wait(job).unwrap();
                }
            };
            /*
             * Arrive, then poll until the caller releases, yielding between
             * polls. A copy that spun without yielding would hold its CPU
             * for a scheduler quantum, and on a machine with as many CPUs
             * as copies the other copy's worker threads (a multithreaded
             * contender's) would wait that quantum to start: measured, a
             * 128 KiB multithreaded hash beside two hard spinners took 2 ms in
             * place of 29 µs. A yielding poll still has the copy in the
             * instruction stream when the release comes, within a
             * microsecond of it.
             */
            // On this placement's CPU before arriving, so the release finds
            // the copy there. The copy's thread alone is pinned; a
            // multithreaded contender's workers run where its pool puts them.
            if let Some(cpus) = job.cpus {
                pin_to_cpu(cpus[copy]);
            }
            // Sound: run() holds the borrows until both finishes are read.
            let input = unsafe { &*job.inputs[copy] };
            // A program's input is in its own node's memory: the pages of
            // this copy's input move to its CPU's node (the main thread
            // first wrote them, where it happened to run; two-caches on a
            // two-socket machine puts copy 1 on the other socket).
            if let Some(cpus) = job.cpus.filter(|_| topology().several_nodes()) {
                bind_to_node(input, topology().node[cpus[copy]]);
            }
            self.ready.fetch_add(1, Ordering::AcqRel);
            while self.generation.load(Ordering::Acquire) == seen {
                std::thread::yield_now();
            }
            seen = self.generation.load(Ordering::Acquire);
            warm_up(
                job.algorithm,
                input,
                job.point,
                job.iterations,
                job.train.warm_ns,
            );
            let samples: Vec<DuoCopy> = (0..job.train.samples)
                .map(|_| take_sample(job.algorithm, input, job.point, job.iterations))
                .collect();
            self.sampling.fetch_sub(1, Ordering::AcqRel);
            while self.sampling.load(Ordering::Acquire) > 0 {
                take_sample(job.algorithm, input, job.point, job.iterations);
            }
            let worked_until_ns = clocks::load::now_ns();
            let mut finished = self.finished.lock().unwrap();
            finished[copy] = Some(DuoWork {
                samples,
                worked_until_ns,
            });
            self.done.notify_all();
        }
    }
}

/*
 * Apple's CommonCrypto SHA-256, linked from libSystem, through the
 * streaming Init/Update/Final calls: the fastest route into corecrypto.
 * Measured on an M4 Max, a 64-byte digest takes 51 ns this way and 182 ns
 * through the one-shot CC_SHA256(), whose finalisation spends about
 * 110 ns per compression; bulk throughput is identical on both. Keep the
 * three-call form.
 */
#[cfg(target_vendor = "apple")]
mod common_crypto {
    pub const DIGEST_LEN: usize = 32;

    /// CC_SHA256_CTX: two 32-bit counters, eight state words, a 64-byte
    /// block buffer. Layout fixed by <CommonCrypto/CommonDigest.h>.
    #[repr(C)]
    struct Context {
        count: [u32; 2],
        hash: [u32; 8],
        wbuf: [u32; 16],
    }

    unsafe extern "C" {
        fn CC_SHA256_Init(ctx: *mut Context) -> i32;
        /// CC_LONG is uint32_t, so one Update takes at most 4 GiB; every
        /// input here is at most 128 MiB.
        fn CC_SHA256_Update(ctx: *mut Context, data: *const u8, len: u32) -> i32;
        fn CC_SHA256_Final(md: *mut u8, ctx: *mut Context) -> i32;
    }

    pub fn sha256(input: &[u8]) -> [u8; DIGEST_LEN] {
        sha256_pieces(&mut |each: &mut dyn FnMut(&[u8])| each(input))
    }

    /// A SHA-256 in progress, for many messages at once.
    pub struct Sha256State(Context);

    impl Sha256State {
        pub fn new() -> Self {
            let mut context = Context {
                count: [0; 2],
                hash: [0; 8],
                wbuf: [0; 16],
            };
            // Safe: `context` is a valid CC_SHA256_CTX.
            assert_eq!(
                unsafe { CC_SHA256_Init(&mut context) },
                1,
                "CC_SHA256_Init failed"
            );
            Sha256State(context)
        }

        pub fn update(&mut self, piece: &[u8]) {
            let len = u32::try_from(piece.len()).expect("CC_SHA256_Update takes a 32-bit length");
            // Safe: the context is valid and `piece` holds `len` bytes.
            assert_eq!(
                unsafe { CC_SHA256_Update(&mut self.0, piece.as_ptr(), len) },
                1,
                "CC_SHA256_Update failed"
            );
        }

        pub fn finish(mut self) -> [u8; DIGEST_LEN] {
            let mut digest = [0u8; DIGEST_LEN];
            // Safe: Final writes exactly 32 bytes to `digest`.
            assert_eq!(
                unsafe { CC_SHA256_Final(digest.as_mut_ptr(), &mut self.0) },
                1,
                "CC_SHA256_Final failed"
            );
            digest
        }
    }

    /// One Update per piece, in order.
    pub fn sha256_pieces(pieces: super::Pieces) -> [u8; DIGEST_LEN] {
        let mut context = Context {
            count: [0; 2],
            hash: [0; 8],
            wbuf: [0; 16],
        };
        let mut digest = [0u8; DIGEST_LEN];
        // Safe: `context` is a valid CC_SHA256_CTX for every call, each
        // piece is valid for its `len` bytes, and Final writes exactly 32
        // bytes to `digest`. Each call returns 1 on success.
        unsafe {
            assert_eq!(CC_SHA256_Init(&mut context), 1, "CC_SHA256_Init failed");
            pieces(&mut |piece| {
                let len =
                    u32::try_from(piece.len()).expect("CC_SHA256_Update takes a 32-bit length");
                assert_eq!(
                    CC_SHA256_Update(&mut context, piece.as_ptr(), len),
                    1,
                    "CC_SHA256_Update failed"
                );
            });
            assert_eq!(
                CC_SHA256_Final(digest.as_mut_ptr(), &mut context),
                1,
                "CC_SHA256_Final failed"
            );
        }
        digest
    }
}

#[cfg(not(target_vendor = "apple"))]
mod common_crypto {
    /// Never called: Roster::new rejects the contender off Apple.
    pub fn sha256(_input: &[u8]) -> [u8; 32] {
        unreachable!("CommonCrypto SHA-256 is an Apple-only contender")
    }

    pub fn sha256_pieces(_pieces: super::Pieces) -> [u8; 32] {
        unreachable!("CommonCrypto SHA-256 is an Apple-only contender")
    }

    pub struct Sha256State;

    impl Sha256State {
        pub fn new() -> Self {
            unreachable!("CommonCrypto SHA-256 is an Apple-only contender")
        }

        pub fn update(&mut self, _piece: &[u8]) {}

        pub fn finish(self) -> [u8; 32] {
            unreachable!()
        }
    }
}

/*
 * The machine's power state: whether it draws from a battery, the
 * battery's charge, and a power mode that trades speed for energy. It
 * changes where the OS runs threads: an M4 Max on battery ran 233 of 400
 * calls that each followed 1 ms of sleep on its efficiency cores, against
 * 32 of 400 on mains power (fork runner jobs 351 and 357, September 27,
 * 2026). macOS reports it through pmset, Linux through
 * /sys/class/power_supply and the ACPI platform profile; a VM reports
 * none.
 */
#[derive(Clone, PartialEq, Eq)]
struct Power {
    on_battery: bool,
    /// The battery's charge in percent, where there is a battery.
    battery_percent: Option<u64>,
    /// The power mode as the OS names it ("Low Power Mode", "High Power
    /// mode", "platform profile balanced"), where it reports one.
    mode: Option<String>,
    /// The mode trades speed for energy.
    low_power: bool,
}

impl Power {
    /// The power state now, or None where the OS reports none.
    fn read() -> Option<Power> {
        if cfg!(target_os = "macos") {
            Self::read_macos()
        } else {
            Self::read_linux()
        }
    }

    fn read_macos() -> Option<Power> {
        let pmset = |args: &[&str]| {
            let out = std::process::Command::new("pmset")
                .args(args)
                .output()
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        Self::parse_pmset(
            &pmset(&["-g", "batt"])?,
            &pmset(&["-g"]).unwrap_or_default(),
        )
    }

    /// The state from `pmset -g batt` and `pmset -g` (the settings in use).
    fn parse_pmset(batt: &str, settings: &str) -> Option<Power> {
        let source = batt.lines().next()?.split('\'').nth(1)?.to_owned();
        let battery_percent = batt.split_whitespace().find_map(|word| {
            word.strip_suffix("%;")
                .or_else(|| word.strip_suffix('%'))?
                .parse()
                .ok()
        });
        /* The settings in use: "lowpowermode 1", or on Macs with a High Power mode "powermode 0|1|2". */
        let setting = |key: &str| {
            settings.lines().find_map(|line| {
                let mut words = line.split_whitespace();
                (words.next() == Some(key))
                    .then(|| words.next()?.parse::<u64>().ok())
                    .flatten()
            })
        };
        let (mode, low_power) = match (setting("powermode"), setting("lowpowermode")) {
            (Some(1), _) | (None, Some(1)) => (Some("Low Power Mode".to_owned()), true),
            (Some(2), _) => (Some("High Power mode".to_owned()), false),
            (Some(0), _) => (Some("automatic power mode".to_owned()), false),
            _ => (None, false),
        };
        Some(Power {
            on_battery: source == "Battery Power",
            battery_percent,
            mode,
            low_power,
        })
    }

    fn read_linux() -> Option<Power> {
        let read = |path: std::path::PathBuf| {
            fs::read_to_string(path)
                .ok()
                .map(|text| text.trim().to_owned())
        };
        let supplies: Vec<(String, Option<String>, Option<u64>)> =
            fs::read_dir("/sys/class/power_supply")
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    Some((
                        read(path.join("type"))?,
                        read(path.join("status")),
                        read(path.join("capacity")).and_then(|c| c.parse().ok()),
                    ))
                })
                .collect();
        // Every CPU's frequency policy, as each names it.
        let every_cpu = |file: &str| -> Vec<String> {
            let mut values: Vec<String> = fs::read_dir("/sys/devices/system/cpu/cpufreq")
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| read(e.path().join(file)))
                .collect();
            values.sort();
            values.dedup();
            values
        };
        Self::parse_linux(
            &supplies,
            read("/sys/firmware/acpi/platform_profile".into()).as_deref(),
            &every_cpu("scaling_governor"),
            &every_cpu("energy_performance_preference"),
        )
    }

    /// The state from Linux's power supplies (type, status, capacity), its
    /// ACPI platform profile, and its CPU frequency governors and
    /// energy-performance preferences (each distinct value once): None
    /// where it reports none of them.
    fn parse_linux(
        supplies: &[(String, Option<String>, Option<u64>)],
        profile: Option<&str>,
        governors: &[String],
        preferences: &[String],
    ) -> Option<Power> {
        let batteries: Vec<_> = supplies
            .iter()
            .filter(|(kind, _, _)| kind == "Battery")
            .collect();
        if supplies.is_empty()
            && profile.is_none()
            && governors.is_empty()
            && preferences.is_empty()
        {
            return None;
        }
        let on_battery = batteries
            .iter()
            .any(|(_, status, _)| status.as_deref() == Some("Discharging"));
        let battery_percent = batteries.iter().find_map(|(_, _, capacity)| *capacity);
        let mut mode = Vec::new();
        if batteries.is_empty() {
            mode.push("no battery".to_owned());
        }
        if let Some(p) = profile {
            mode.push(format!("platform profile {p}"));
        }
        if !governors.is_empty() {
            mode.push(format!("CPU frequency governor {}", governors.join("/")));
        }
        if !preferences.is_empty() {
            mode.push(format!(
                "energy-performance preference {}",
                preferences.join("/")
            ));
        }
        let low_power = profile
            .is_some_and(|p| p.starts_with("low-power") || p == "quiet" || p == "cool")
            || preferences.iter().any(|p| p == "power");
        Some(Power {
            on_battery,
            battery_percent,
            mode: Some(mode.join(", ")),
            low_power,
        })
    }

    /// Whether this state can make results read slower than the machine
    /// runs on mains power.
    fn slowing(&self) -> bool {
        self.on_battery || self.low_power
    }

    /// "mains power, High Power mode", "battery power (47% charged), Low Power Mode".
    fn describe(&self) -> String {
        let mut line = if self.on_battery {
            "battery power".to_owned()
        } else {
            "mains power".to_owned()
        };
        if let (true, Some(percent)) = (self.on_battery, self.battery_percent) {
            write!(line, " ({percent}% charged)").unwrap();
        }
        if let Some(mode) = &self.mode {
            write!(line, ", {mode}").unwrap();
        }
        line
    }
}

impl MachineMetadata {
    /// Whether the power state at the start or the end can make results
    /// read slow.
    fn power_slowing(&self) -> bool {
        self.power.iter().flatten().any(Power::slowing)
    }

    /// One line for readers: the power state, and its change where it
    /// changed during the run; "not reported by this OS" where there is none.
    fn describe_power(&self) -> String {
        let [start, end] = &self.power;
        let line = match (start, end) {
            (None, None) => return "not reported by this OS".to_owned(),
            (Some(start), Some(end)) if start.describe() != end.describe() => {
                format!(
                    "{} at the start, {} at the end",
                    start.describe(),
                    end.describe()
                )
            }
            (Some(one), _) | (None, Some(one)) => one.describe(),
        };
        if self.power_slowing() {
            line + "; results may differ from a run on mains power in the normal mode"
        } else {
            line
        }
    }
}

/// Spread `visits` of a point over `rounds`, at the point's offset
/// (UseCase::visits). The visits walk the Williams orders in turn; a count
/// that is no multiple of the order count leaves some orders one visit
/// ahead, a position effect of a fraction of a visit per contender, which
/// is accepted (Zooko, October 19, 2026) for the time it saves: rounding
/// up to whole cycles cost the nonstop phase a third with eight
/// contenders. Requires positive counts; explicit --rounds bypasses
/// thinning.
fn cell_wants_sample(slot: usize, rounds: usize, visits: usize) -> bool {
    assert!(rounds > 0);
    let target = visits.min(rounds);
    slot * target % rounds < target
}

/// (iterations per sample, nanoseconds per iteration measured).
/*
 * The fewest inputs a continuous cell's sample holds: twice the buffers
 * its program keeps in flight (FROZEN.md), so the sample times the
 * queue's steady flow over many inputs rather than one filling and
 * draining. A queue's first inputs cost tens of microseconds to start
 * (its pool and delivery thread wake), so a sample calibrated from one
 * input held a dozen (September 28, 2026: 12 messages of 64 B, far short
 * of the 1024 in flight).
 */
fn continuous_min_inputs(input: &[u8], point: Point) -> usize {
    match point.use_case {
        UseCase::ContinuousMessages => {
            let pieces = input.len().div_ceil(PIECE_LEN).max(1);
            2 * in_flight(input.len().min(PIECE_LEN)).div_ceil(pieces)
        }
        UseCase::ContinuousBatches => 2 * in_flight(input.len()),
        UseCase::OneMessage
        | UseCase::IdleOneMessage
        | UseCase::BriefIdleOneMessage
        | UseCase::ControlledIdleOneMessage
        | UseCase::ManyMessages
        | UseCase::ManyLongMessages
        | UseCase::IdleManyMessages
        | UseCase::BriefIdleManyMessages
        | UseCase::ControlledIdleManyMessages
        | UseCase::LentMessages
        | UseCase::Interleaved
        | UseCase::Collection
        | UseCase::LentBatches
        | UseCase::MerkleTree
        | UseCase::InterleavedTurn
        | UseCase::IdleInterleavedTurn
        | UseCase::BriefIdleInterleavedTurn
        | UseCase::ControlledIdleInterleavedTurn => 1,
        /* A pass over the collection keeps its many items in flight. */
        UseCase::ContinuousCollection => 1,
    }
}

/// Untimed batches before a cell's calibration, and probes it takes the
/// fastest of (calibrate_batch).
const CALIBRATION_WARMUPS: usize = 3;
const CALIBRATION_PROBES: usize = 3;

fn calibrate_batch(algorithm: Algorithm, input: &[u8], point: Point) -> (usize, u128) {
    let fewest = continuous_min_inputs(input, point);
    let mut iterations = fewest;
    /*
     * A cell's first calls carry one-time costs (a pool or a queue's
     * delivery thread starting, a self-test, first-touched buffers), which
     * would size its samples short: untimed batches first (Devon Jonte,
     * bench-hashes#4: the queue's 64 B samples 154-199 us against the 1 ms
     * target; the Mac's 176 us), and each probe the fastest of three. One
     * untimed batch and one probe left Interleaved 256 still warming
     * in some builds (i7-12700K: probes 0.57-0.79 ms against 0.28 ms an
     * iteration steady), which halved its samples and read it 4-10% slower
     * with no slower hashing (servil-software-factory/bench-hashes#11).
     */
    for _ in 0..CALIBRATION_WARMUPS {
        run_batch(algorithm, input, point, fewest);
    }

    loop {
        let elapsed_ns = (0..CALIBRATION_PROBES)
            .map(|_| {
                let started = clocks::now();
                run_batch(algorithm, input, point, iterations);
                u128::from(clocks::since_ns(started))
            })
            .min()
            .expect("a probe at least");

        /*
         * A sufficiently short interval can be below a platform timer's
         * effective resolution. Increase the batch until it is measurable.
         */
        if elapsed_ns == 0 {
            iterations = iterations
                .checked_mul(2)
                .expect("calibration iteration count overflowed");

            continue;
        }

        if elapsed_ns >= CALIBRATION_PROBE_NS {
            let scaled = (iterations as u128 * TARGET_SAMPLE_NS + elapsed_ns / 2) / elapsed_ns;

            let scaled = scaled.max(fewest as u128);

            assert!(
                scaled <= usize::MAX as u128,
                "calibrated iteration count must fit in usize"
            );

            return (scaled as usize, elapsed_ns / iterations as u128);
        }

        let growth = (CALIBRATION_PROBE_NS + elapsed_ns - 1) / elapsed_ns;

        assert!(
            growth >= 2,
            "a sub-probe measurement must require a larger batch"
        );

        iterations = iterations
            .checked_mul(
                usize::try_from(growth).expect("calibration growth factor must fit in usize"),
            )
            .expect("calibration iteration count overflowed");
    }
}

/// A cell's summary from its samples: their mean (clocks::summary), kept
/// exact for display.

/*
 * Every sample, for tools that do their own statistics (BLAKE3-Servil's
 * performance-regression check reads this file). Lines starting with '#'
 * carry the provenance and machine identity as `key: value`; then one row
 * per measured cell and scenario: contender key, scenario (`solo`,
 * `shared-one-cache` or `shared-two-caches`), use case, point label, the
 * unit a sample is per (`B` or `msg`), and the samples as measured,
 * comma-separated, each `ns/units` (the nanoseconds the clock gave over
 * the units they covered), in the order taken (shared: the two copies of
 * each interval in turn); then each sample's start, CPU time, energy,
 * core counts and CPU (SAMPLES_COLUMNS).
 */
/// The samples file's first line and its column row, which the reader
/// (read_samples) requires: a file in another format is read with the
/// tools of the commit that wrote it.
/// The samples format, and with it the rule the samples were taken under:
/// v10 (0.21) adds each sample's instructions retired, last-level-cache
/// references and misses beside its cycles, per core kind, and reads the
/// raw counter (Zooko, November 4: the most the counters give, kept, and
/// no clock a time daemon slews); v8 (0.20)
/// added each sample's CPU, its shared rows the multithreaded
/// contenders' alone, placed under one cache or two (bench-hashes #108,
/// #116); v7 marked samples that 0.17's summary reads with the declared
/// trim (Zooko, October 23).
const SAMPLES_VERSION: &str = "# bench-hashes samples v10";
const SAMPLES_COLUMNS: &str = "contender\tscenario\tuse_case\tpoint\tunit\tns/units\tstart ms\tcpu ns\tenergy nJ\tcore counts\tcpu";

/// The numbers one core kind contributes to a `core counts` sample:
/// time, cycles, instructions, cache references, cache misses.
const LEVEL_FIELDS: usize = 5;

/// A row's `core counts` field: each sample's calling-thread counts per
/// core kind, `P time ns:P cycles:P instructions:P cache references:P
/// cache misses:E time ns:E cycles:E instructions:E cache references:E
/// cache misses` (clocks::Level), or `-` where a cell's samples carry
/// none (a platform without per-thread counts).
fn counts_field(counts: &[Option<clocks::Counts>]) -> String {
    match counts
        .iter()
        .copied()
        .collect::<Option<Vec<clocks::Counts>>>()
    {
        Some(c) if !c.is_empty() => c
            .iter()
            .map(|c| {
                format!(
                    "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                    c.p.time_ns,
                    c.p.cycles,
                    c.p.instructions,
                    c.p.cache_references,
                    c.p.cache_misses,
                    c.e.time_ns,
                    c.e.cycles,
                    c.e.instructions,
                    c.e.cache_references,
                    c.e.cache_misses
                )
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => "-".to_string(),
    }
}

/// A row's `cpu` field: the CPU each sample's thread ended on, or `-`
/// where the OS names none.
fn cpu_of_field(cpus: &[Option<u32>]) -> String {
    match cpus.iter().copied().collect::<Option<Vec<u32>>>() {
        Some(c) if !c.is_empty() => c.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
        _ => "-".to_string(),
    }
}

/// A row's `cpu ns` or `energy nJ` field: each sample's, or `-` where a
/// cell has none (a sample after gaps, a shared one, no such counter).
fn cpu_field(cpu: &[Option<u64>]) -> String {
    match cpu.iter().copied().collect::<Option<Vec<u64>>>() {
        Some(ns) if !ns.is_empty() => ns.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
        _ => "-".to_string(),
    }
}

/// A samples file read back: its `# load:` line (the run's own verdict,
/// "quiet: ..." or "busy: ..."), and each cell's samples as measured,
/// keyed "contender|scenario|use_case|point", in the file's order.
#[derive(Clone)]
struct SamplesFile {
    /// Its `# key: value` lines, in order.
    headers: Vec<(String, String)>,
    load: String,
    /// Its `# power:` line.
    power: String,
    cells: Vec<(String, Vec<Measured>)>,
    /// The cells whose samples carry the process's CPU time: each sample's,
    /// in nanoseconds, in the order of its wall-time samples.
    cpu: Vec<(String, Vec<u64>)>,
    /// The cells whose samples carry the CPU package's energy: each
    /// sample's, in nanojoules, in the same order.
    energy: Vec<(String, Vec<u64>)>,
    /// The cells whose samples carry the calling thread's counts per core
    /// kind: each sample's [P time ns, P cycles, E time ns, E cycles].
    cores: Vec<(String, Vec<[u64; LEVEL_FIELDS * 2]>)>,
    /// The cells whose samples name the CPU each sample's thread ended on.
    cpus: Vec<(String, Vec<u32>)>,
}

fn read_samples(path: &str) -> SamplesFile {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(SAMPLES_VERSION),
        "{path}: a samples file of this version begins with {SAMPLES_VERSION:?}"
    );
    let mut load = None;
    let mut power = None;
    let mut headers = Vec::new();
    let mut columns = false;
    let mut cells = Vec::new();
    let mut cpu = Vec::new();
    let mut energy = Vec::new();
    let mut cores = Vec::new();
    let mut cpus = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("# load: ") {
            load = Some(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix("# power: ") {
            power = Some(rest.to_owned());
        } else if line.starts_with('#') || line.is_empty() {
            if let Some((key, value)) = line
                .strip_prefix("# ")
                .and_then(|rest| rest.split_once(": "))
            {
                headers.push((key.to_owned(), value.to_owned()));
            }
        } else if !columns {
            assert_eq!(line, SAMPLES_COLUMNS, "{path}: the column row");
            columns = true;
        } else {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 11, "{path}: a row of eleven fields: {line}");
            let samples: Vec<Measured> = fields[5]
                .split(',')
                .map(|sample| {
                    let (ns, units) = sample.split_once('/').expect("ns/units");
                    Measured::new(ns.parse().expect("ns"), units.parse().expect("units"))
                })
                .collect();
            let key = fields[..4].join("|");
            assert!(
                cells.iter().all(|(k, _)| *k != key),
                "{path}: a samples file holds each cell once: {key} twice"
            );
            if fields[7] != "-" {
                let ns: Vec<u64> = fields[7]
                    .split(',')
                    .map(|ns| ns.parse().expect("cpu ns"))
                    .collect();
                assert_eq!(
                    ns.len(),
                    samples.len(),
                    "{path}: a CPU time for every sample of {key}"
                );
                cpu.push((key.clone(), ns));
            }
            if fields[8] != "-" {
                let nj: Vec<u64> = fields[8]
                    .split(',')
                    .map(|nj| nj.parse().expect("energy nJ"))
                    .collect();
                assert_eq!(
                    nj.len(),
                    samples.len(),
                    "{path}: an energy for every sample of {key}"
                );
                energy.push((key.clone(), nj));
            }
            if fields[9] != "-" {
                let c: Vec<[u64; LEVEL_FIELDS * 2]> = fields[9]
                    .split(',')
                    .map(|sample| {
                        let v: Vec<u64> = sample
                            .split(':')
                            .map(|x| x.parse().expect("a count"))
                            .collect();
                        v.try_into().expect(
                            "P time, cycles, instructions, cache references, cache misses; then E's",
                        )
                    })
                    .collect();
                assert_eq!(
                    c.len(),
                    samples.len(),
                    "{path}: counts for every sample of {key}"
                );
                cores.push((key.clone(), c));
            }
            if fields[10] != "-" {
                let c: Vec<u32> = fields[10]
                    .split(',')
                    .map(|x| x.parse().expect("a CPU number"))
                    .collect();
                assert_eq!(
                    c.len(),
                    samples.len(),
                    "{path}: a CPU for every sample of {key}"
                );
                cpus.push((key.clone(), c));
            }
            cells.push((key, samples));
        }
    }
    SamplesFile {
        headers,
        load: load.unwrap_or_else(|| panic!("{path}: a load line")),
        power: power.unwrap_or_else(|| panic!("{path}: a power line")),
        cells,
        cpu,
        energy,
        cores,
        cpus,
    }
}

/*
 * `bench-hashes compare OLD... -- NEW...`: the runs in pairs, the i-th old
 * with the i-th new (as they ran, back to back in alternating order), and
 * every cell both measured judged at once (clocks::summary::discoveries:
 * a paired t-test per cell, the false discovery rate held to
 * COMPARE_Q_PERMILLE over all cells, the regression check's margin): each
 * side's mean (the geometric mean of its runs'), the ratio judged (the
 * geometric mean of the pairs' ratios, new over old, which is the two
 * sides' figures' ratio), the verdict, and how many pairs ran faster. At least
 * three pairs. For two builds, `regress` (through BLAKE3-Servil's
 * tools/perf_regress.py, which relinks each side with a fresh layout per
 * pair) runs and judges the pairs itself: link layout moves cold calls'
 * cells by itself (audit/variance-on-unchanged-code.md at aadeca9d).
 */
/// The false discovery rate `compare` holds its verdicts to, over all the
/// cells it judges.
const COMPARE_Q_PERMILLE: u64 = 50;

/// Each cell's mean wall time per unit (fixed point) and, where its
/// samples carry them, its CPU time and energy per unit: all the
/// samples' time over their units.
fn cell_means(file: SamplesFile) -> Vec<(String, u128, Option<u128>, Option<u128>)> {
    let cpu: crate::Map<String, Vec<u64>> = file.cpu.into_iter().collect();
    let energy: crate::Map<String, Vec<u64>> = file.energy.into_iter().collect();
    file.cells
        .into_iter()
        .map(|(key, samples)| {
            let units: u128 = samples.iter().map(|m| u128::from(m.units)).sum();
            let per_unit = |column: &crate::Map<String, Vec<u64>>| {
                column
                    .get(&key)
                    .map(|v| (v.iter().map(|&n| u128::from(n)).sum::<u128>() << 64) / units.max(1))
                    .filter(|&v| v > 0)
            };
            let (c, e) = (per_unit(&cpu), per_unit(&energy));
            (key, Mean::of(&samples).fixed().0, c, e)
        })
        .collect()
}

/// Each cell's instructions retired a unit (fixed point), both kinds of
/// core summed over all its samples, for the cells whose samples carry
/// counts: what names a level cell a suspect (instructions_moved).
fn cell_instructions(file: &SamplesFile) -> crate::Map<String, u128> {
    let cores: crate::Map<&String, &Vec<[u64; LEVEL_FIELDS * 2]>> =
        file.cores.iter().map(|(k, c)| (k, c)).collect();
    file.cells
        .iter()
        .filter_map(|(key, samples)| {
            let units: u128 = samples.iter().map(|m| u128::from(m.units)).sum();
            let instructions: u128 = cores
                .get(key)?
                .iter()
                .map(|s| u128::from(s[2] + s[LEVEL_FIELDS + 2]))
                .sum();
            Some((key.clone(), (instructions << 64) / units.max(1)))
        })
        .collect()
}

fn compare_command(arguments: &[String]) {
    let usage = "usage: bench-hashes compare OLD.tsv... -- NEW.tsv... (one new run for each old run, in the order they ran)";
    let split = arguments
        .iter()
        .position(|argument| argument == "--")
        .expect(usage);
    let (old, new) = (&arguments[..split], &arguments[split + 1..]);
    assert!(
        old.len() >= 3 && old.len() == new.len(),
        "{usage}; at least three pairs"
    );
    // Each cell's mean wall time per unit, and (where its samples carry
    // them) its CPU time and energy per unit: all the samples' CPU time, or
    // energy, over their units, in the same fixed point.
    let (mut old_builds, mut new_builds) = (Vec::new(), Vec::new());
    let means = |path: &String, builds: &mut Vec<String>| {
        let file = read_samples(path);
        let build = file_provenance(&file);
        if !builds.contains(&build) {
            builds.push(build);
        }
        if clocks::load::MEASURED && !file.load.starts_with("quiet") {
            println!(
                "{path}: {}: its samples are no evidence of speed",
                file.load
            );
        }
        cell_means(file)
    };
    // Every cell's means a pair, wall time and (where both sides carry it)
    // CPU time, in the first old run's order.
    type Pairs = Vec<(u128, u128)>;
    let mut order: Vec<String> = Vec::new();
    let mut cells: crate::Map<String, (Pairs, Pairs, Pairs)> = crate::Map::default();
    for (old_path, new_path) in old.iter().zip(new) {
        let new_means: crate::Map<String, (u128, Option<u128>, Option<u128>)> =
            means(new_path, &mut new_builds)
                .into_iter()
                .map(|(k, m, c, e)| (k, (m, c, e)))
                .collect();
        for (key, old_mean, old_cpu, old_energy) in means(old_path, &mut old_builds) {
            let Some(&(new_mean, new_cpu, new_energy)) = new_means.get(&key) else {
                continue;
            };
            if !cells.contains_key(&key) {
                order.push(key.clone());
            }
            let (wall, cpu, energy) = cells.entry(key).or_default();
            wall.push((old_mean, new_mean));
            if let (Some(o), Some(n)) = (old_cpu, new_cpu) {
                cpu.push((o, n));
            }
            if let (Some(o), Some(n)) = (old_energy, new_energy) {
                energy.push((o, n));
            }
        }
    }
    print_sides("compare", &old_builds, &new_builds);
    // Only cells every pair measured.
    order.retain(|key| cells[key].0.len() == old.len());
    let tested: Vec<Pairs> = order.iter().map(|key| cells[key].0.clone()).collect();
    let (judged, _) = judge_once(&tested);
    let times = |permille: u64| format!("x{}.{:03}", permille / 1000, permille % 1000);
    for (key, judgment) in order.into_iter().zip(judged) {
        let (wall, cpu, energy) = &cells[&key];
        let verdict = match judgment.verdict {
            clocks::summary::Verdict::Slower => "slower",
            clocks::summary::Verdict::Faster => "faster",
            clocks::summary::Verdict::Level => "level",
        };
        let faster = wall.iter().filter(|(old, new)| new < old).count();
        // CPU time beside wall time, by the same ratio, with no verdict (the
        // rule judges wall time), where every pair has it.
        let beside = |name: &str, pairs: &Pairs| {
            if pairs.len() == wall.len() {
                format!(
                    "; {name} {}",
                    times(clocks::summary::geometric_mean_permille(pairs))
                )
            } else {
                String::new()
            }
        };
        let cpu = beside("CPU time", cpu) + &beside("energy", energy);
        let (a, b) = (
            geometric_mean(wall.iter().map(|p| p.0)),
            geometric_mean(wall.iter().map(|p| p.1)),
        );
        println!(
            "{key}: {} -> {} ns/unit  {} {verdict}, {faster} of {} pairs faster{cpu}",
            Fixed(a).format_ns(),
            Fixed(b).format_ns(),
            times(judgment.ratio_permille),
            wall.len()
        );
    }
}

/// Every cell judged at once (clocks::summary::discoveries), each
/// measurement once: cells whose means agree in every pair, on both sides,
/// are one measurement (an owned cell measured as its lent twin,
/// lent_twin), judged as one and given its verdict on each row, so the
/// family the false discovery rate is controlled over counts it once
/// (bench-hashes #98, Devon Jonte). Returns each row's judgment and the
/// first row holding its measurement.
fn judge_once(tested: &[Vec<(u128, u128)>]) -> (Vec<clocks::summary::Judgment>, Vec<usize>) {
    let mut first: std::collections::BTreeMap<&Vec<(u128, u128)>, usize> = Default::default();
    let mut unique: Vec<Vec<(u128, u128)>> = Vec::new();
    let mut of = Vec::new();
    let mut held = Vec::new();
    for (i, pairs) in tested.iter().enumerate() {
        let k = *first.entry(pairs).or_insert_with(|| {
            unique.push(pairs.clone());
            held.push(i);
            unique.len() - 1
        });
        of.push(k);
    }
    let judged = clocks::summary::discoveries(&unique, COMPARE_Q_PERMILLE, REGRESS_MARGIN_PERMILLE);
    (
        of.iter().map(|&k| judged[k]).collect(),
        of.iter().map(|&k| held[k]).collect(),
    )
}

/// The geometric mean of some positive Q64.64 values: each side's figure in
/// compare, whose ratio is the ratio judged. For display only.
fn geometric_mean(values: impl Iterator<Item = u128>) -> u128 {
    let logs: Vec<f64> = values.map(|v| (v as f64).ln()).collect();
    assert!(!logs.is_empty(), "a mean needs values");
    (logs.iter().sum::<f64>() / logs.len() as f64).exp().round() as u128
}

/*
 * `bench-hashes regress [--contenders A,B [--points P,Q]] OLD... -- NEW...`:
 * whether the new build is slower than the old, measured on this machine.
 * `bench-hashes check` builds the two sides and runs the cells its caller
 * names, then calls this (src/check.rs); it is also callable by hand with builds of
 * one's own. REGRESS_PAIRS pairs of runs, pair i running the i-th
 * executable of each side (round robin), back to back in alternating order
 * (A B, B A, ...), each at the regression check's visits
 * (TO_CHECK_FOR_REGRESSIONS) over every cell of the default contenders,
 * or with --contenders (and --points) what a run with them measures, every
 * cell judged. Each pair gives each cell one ratio, new mean over old; the
 * cells are judged all at once by clocks::summary::discoveries (a false
 * discovery rate of 5% over them, a 3% margin). Calibrated on the Mac (fork
 * NOTES, "The regression check, calibrated"). Exit 0: no cell slower; 1: a
 * cell slower; 2: no verdict, when a run's load was busy or unobserved
 * (clocks::load).
 */
const REGRESS_ROUNDS: usize = 24;
const REGRESS_PAIRS: usize = 8;
/// How many times a pair runs again when one of its runs is no evidence.
const REGRESS_REPEATS: usize = 3;

/// The margin a solo cell's verdict uses.
const REGRESS_MARGIN_PERMILLE: u64 = 30;
/// The most level cells a regression run names as suspects for a second
/// stage, and so the largest family that stage judges (bench-hashes #66).
const REGRESS_SUSPECTS: usize = 8;
/// The margin beyond which a level cell's instructions, moved, name it a
/// suspect ahead of the wall-ranked ones (Zooko, November 4, 2026, on
/// John's rule; bench-hashes #127): instructions retired are the same
/// count whatever the core's clock or state, so a move is the code's,
/// and #119's cold code moved them 1.2% in every pair where wall missed
/// it three times. Wall still decides, on the second stage's fresh pairs.
/// The rule reads the solo cells of the single-threaded contenders
/// alone, where the calling thread does the whole work: a pool's caller
/// spins while it waits, and its count moved 1-9% between two runs of
/// the same code (data branch, 2026-11-04/judge2-i7).
const INSTRUCTIONS_MARGIN_PERMILLE: u64 = 5;

/// What each of the check's runs measures, and which of its cells are
/// judged: the regress points' solo cells; a default run's every cell
/// (--full); or the cells a run with the caller's --contenders (and
/// --points) measures, every one judged.
enum RegressScope {
    Full,
    Chosen {
        contenders: String,
        points: Option<String>,
    },
}

impl RegressScope {
    /// The run's arguments, `rounds` rounds (0: a default run, its own).
    fn arguments(&self, rounds: usize) -> Vec<String> {
        let count = rounds;
        let rounds = || {
            vec![
                "--for-regression-check".to_string(),
                "--rounds".to_string(),
                count.to_string(),
            ]
        };
        match self {
            RegressScope::Full => {
                let contenders: Vec<&str> = DEFAULT_CONTENDERS.iter().map(|a| a.key()).collect();
                [vec!["--contenders".into(), contenders.join(",")], rounds()].concat()
            }
            RegressScope::Chosen { contenders, points } => {
                let points = points
                    .iter()
                    .flat_map(|p| ["--points".to_string(), p.clone()])
                    .collect();
                [
                    vec!["--contenders".into(), contenders.clone()],
                    points,
                    rounds(),
                ]
                .concat()
            }
        }
    }

    fn describe(&self) -> String {
        match self {
            RegressScope::Full => "every cell of a default run".to_string(),
            RegressScope::Chosen { contenders, points } => format!(
                "--contenders {contenders}{}",
                points
                    .as_ref()
                    .map_or(String::new(), |p| format!(" --points {p:?}"))
            ),
        }
    }
}

/// Two sizes files as one: every cell of `old`, with its size, and the cells
/// only `new` has, with theirs.
fn merge_sizes(old: &str, new: &str) -> String {
    let cell = |line: &str| {
        line.rsplit_once('\t')
            .map(|(cell, _)| cell.to_owned())
            .unwrap_or_default()
    };
    let old_cells: crate::Set<String> = old.lines().skip(1).map(cell).collect();
    old.lines()
        .chain(
            new.lines()
                .skip(1)
                .filter(|line| !old_cells.contains(&cell(line))),
        )
        .map(|line| format!("{line}\n"))
        .collect()
}

/// One run of `exe` over `scope`, `rounds` rounds (0: a default run's
/// own), its samples sized by the file at `sizes`: its samples file, also
/// copied to `keep`, and the load this process saw while it ran
/// ([`regress_load`]). The run is this process's
/// child (clocks::child::run), so its CPU time is this process's own and
/// the windows around it stay a second long: a run needs no least length,
/// and its own load line (of windows inside it) goes unread.
fn regress_run(
    exe: &str,
    rounds: usize,
    scope: &RegressScope,
    sizes: &std::path::Path,
    keep: Option<&std::path::Path>,
    pin: Option<usize>,
) -> (SamplesFile, String) {
    let directory = std::env::temp_dir().join(format!(
        "bench-hashes-regress-{}-{}",
        std::process::id(),
        clocks::load::now_ns()
    ));
    fs::create_dir_all(&directory).unwrap();
    let stderr_path = directory.join("stderr.txt");
    let stderr = fs::File::create(&stderr_path).unwrap();
    let mut command = std::process::Command::new(exe);
    match pin {
        Some(cpu) => command.env(PIN_CPU_VARIABLE, cpu.to_string()),
        None => command.env_remove(PIN_CPU_VARIABLE),
    };
    let run = clocks::child::run(
        command
            .args(scope.arguments(rounds))
            .arg("--sizes")
            .arg(sizes)
            .current_dir(&directory)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(stderr),
    );
    assert!(
        run.success,
        "{exe} failed\n{}",
        fs::read_to_string(&stderr_path)
            .unwrap_or_default()
            .trim_end()
    );
    let found: Vec<_> = fs::read_dir(directory.join("benchmark-results"))
        .unwrap()
        .map(|entry| entry.unwrap().path().join("bench-hashes.samples.tsv"))
        .collect();
    assert_eq!(found.len(), 1, "one samples file");
    let file = read_samples(found[0].to_str().unwrap());
    // Both sides of a pair on its CPU, or neither: a build that ignores the
    // variable (one older than pinning) would measure unpinned beside a
    // pinned one, a difference between the sides that is no change of theirs.
    if let Some(cpu) = pin {
        let text = fs::read_to_string(&found[0]).unwrap();
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("# measuring thread: "))
            .unwrap_or("not stated");
        assert!(
            line.starts_with(&format!("pinned to CPU {cpu} ")),
            "{exe}: asked to pin its measuring thread to CPU {cpu}, its samples say \"{line}\" (a bench older than pinning: run regress with --unpinned)"
        );
    }
    let load = regress_load(run.started_ns, run.started_ns + run.wall_ns);
    if let Some(keep) = keep {
        // The kept file says what regress judged the run by: its load line
        // the windows around the run, which compare reads in turn (the
        // windows listed stay the run's own, on its samples' clock).
        let text = fs::read_to_string(&found[0]).unwrap();
        let kept: String = text.lines().map(|line| match line.strip_prefix("# load: ") {
            Some(_) => format!("# load: {load}\n# load measured by: bench-hashes regress, around the run (the windows below are the run's own)\n"),
            None => format!("{line}\n"),
        }).collect();
        fs::write(keep, kept)
            .unwrap_or_else(|error| panic!("cannot keep {}: {error}", keep.display()));
    }
    fs::remove_dir_all(&directory).unwrap();
    (file, load)
}

/// The load in the windows of this process overlapping `start_ns` to
/// `end_ns` (clocks::load's line for them), once a window covers the end:
/// the running one closes at half a window, so this waits that long at
/// most.
fn regress_load(start_ns: u64, end_ns: u64) -> String {
    loop {
        let windows = clocks::load::windows();
        // Where the OS counts CPU time, a window covers the end within half
        // a window; elsewhere none ever closes (describe says so).
        let counted = clocks::load::MEASURED;
        if windows
            .last()
            .map_or(!counted, |last| last.end_ns >= end_ns)
        {
            let overlapping: Vec<clocks::load::Window> = windows
                .into_iter()
                .filter(|w| w.end_ns > start_ns && w.start_ns < end_ns)
                .collect();
            return clocks::load::describe(&overlapping);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn regress_command(arguments: &[String]) -> i32 {
    let usage = "usage: bench-hashes regress [--contenders A,B [--points P,Q]] [--judge CELL;CELL] [--pairs N] [--rounds K] [--keep DIR] [--unpinned] OLD_EXE... -- NEW_EXE... (pair i runs the i-th of each, round robin; bench-hashes check builds the sides and runs the cells its caller names; each pair's runs take their samples after a gap on one CPU, pairs alternating core parity, unless --unpinned)";
    let mut arguments = arguments;
    let (mut contenders, mut points, mut pairs, mut chosen_rounds, mut keep) =
        (None, None, REGRESS_PAIRS, None, None);
    let mut only: Option<Vec<String>> = None;
    let mut unpinned = false;
    while let Some(name) = arguments
        .first()
        .filter(|a| a.starts_with("--") && *a != "--")
    {
        if name == "--unpinned" {
            unpinned = true;
            arguments = &arguments[1..];
            continue;
        }
        assert!(arguments.len() > 1, "{name} needs a value\n{usage}");
        let value = arguments[1].clone();
        let count = || {
            value
                .parse::<usize>()
                .ok()
                .filter(|&n| n > 0)
                .unwrap_or_else(|| panic!("{name} takes a positive count\n{usage}"))
        };
        match name.as_str() {
            "--contenders" => contenders = Some(value.clone()),
            "--points" => points = Some(value.clone()),
            "--pairs" => pairs = count(),
            "--rounds" => chosen_rounds = Some(count()),
            "--keep" => keep = Some(std::path::PathBuf::from(&value)),
            "--judge" => only = Some(value.split(';').map(str::to_string).collect()),
            _ => panic!("unknown option {name}\n{usage}"),
        }
        arguments = &arguments[2..];
    }
    assert!(
        pairs >= 3,
        "--pairs: the rule needs at least three pairs\n{usage}"
    );
    // The machine first: a run on a busy machine abstains at its end; ten
    // quiet seconds now say so before it (check asks the same before its
    // build), tried for up to two minutes, since check's builds leave load
    // behind them for a few seconds.
    if let Err(reason) = clocks::load::quiet_within(120) {
        println!(
            "regress: {reason}. No verdict (exit 2); run when nothing else runs on the machine."
        );
        return 2;
    }
    if let Some(keep) = &keep {
        fs::create_dir_all(keep)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", keep.display()));
    }
    assert!(
        points.is_none() || contenders.is_some(),
        "--points narrows a --contenders check\n{usage}"
    );
    let scope = match contenders {
        Some(contenders) => RegressScope::Chosen { contenders, points },
        None => RegressScope::Full,
    };
    let split = arguments.iter().position(|a| a == "--").expect(usage);
    let (old_exes, new_exes) = (&arguments[..split], &arguments[split + 1..]);
    assert!(!old_exes.is_empty() && !new_exes.is_empty(), "{usage}");
    let rounds = chosen_rounds.unwrap_or(REGRESS_ROUNDS);
    // Each pair's CPU (bench-hashes #69): its two runs take their samples
    // after a gap there, so the core is no difference between the sides.
    let allowed = allowed_cpu();
    let pins: Vec<Option<usize>> = (0..pairs)
        .map(|pair| {
            if unpinned {
                None
            } else {
                topology().measuring_cpu(pair, &allowed)
            }
        })
        .collect();
    match (unpinned, pins.iter().flatten().count()) {
        (true, _) => eprintln!("regress: --unpinned: the OS places every run's measuring thread"),
        (false, 0) => eprintln!("regress: no CPU to pin to (no thread affinity or topology here): the OS places every run's measuring thread"),
        (false, _) => {
            let mut cpus: Vec<usize> = pins.iter().flatten().copied().collect();
            cpus.sort();
            cpus.dedup();
            eprintln!("regress: each pair's runs take their samples after a gap on one CPU, the pairs alternating core parity over the cache domains: CPUs {}", cpus.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(","));
        }
    }
    eprintln!(
        "regress: {} new builds against {} old, {pairs} pairs over {}{}",
        new_exes.len(),
        old_exes.len(),
        scope.describe(),
        chosen_rounds.map_or(String::new(), |k| format!(", {k} rounds a run"))
    );
    let mut run_index = 0;
    // One calibration a side, by its first build, for the whole check: every
    // run takes the sizes (Sizes). A cell both sides measure takes the old
    // side's size, so both do the same work a sample; a cell only one side
    // measures (a --bench comparison whose cells changed), its own side's.
    // Each side reads a file of its own: the old side's builds its own
    // calibration (an older bench rejects a size for a cell it lacks, such
    // as one this bench added), the new side's the merged file.
    let sizes = std::env::temp_dir().join(format!(
        "bench-hashes-regress-sizes-{}.tsv",
        std::process::id()
    ));
    let old_sizes = std::env::temp_dir().join(format!(
        "bench-hashes-regress-sizes-{}-old-side.tsv",
        std::process::id()
    ));
    let calibrate = |exe: &str, side: &str| -> String {
        let path = std::env::temp_dir().join(format!(
            "bench-hashes-regress-sizes-{}-{side}.tsv",
            std::process::id()
        ));
        let calibration = std::process::Command::new(exe)
            .args(scope.arguments(rounds))
            .arg("--calibrate-only")
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .output()
            .unwrap_or_else(|error| panic!("failed to run {exe}: {error}"));
        assert!(
            calibration.status.success(),
            "{exe} --calibrate-only failed\n{}",
            String::from_utf8_lossy(&calibration.stderr).trim_end()
        );
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();
        text
    };
    let old_calibration = calibrate(&old_exes[0], "old");
    let merged = merge_sizes(&old_calibration, &calibrate(&new_exes[0], "new"));
    fs::write(&sizes, merged)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", sizes.display()));
    fs::write(&old_sizes, old_calibration)
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", old_sizes.display()));
    if let Some(keep) = &keep {
        fs::copy(&sizes, keep.join("sizes.tsv"))
            .unwrap_or_else(|error| panic!("cannot keep the sizes: {error}"));
    }
    let mut unreliable = Vec::new();
    let mut powers: Vec<String> = Vec::new();
    let (mut old_builds, mut new_builds): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut ratios: std::collections::BTreeMap<String, Vec<u64>> =
        std::collections::BTreeMap::new();
    let mut pairs_means: std::collections::BTreeMap<String, Vec<(u128, u128)>> =
        std::collections::BTreeMap::new();
    // A run's means, and its load line when clocks found it no evidence
    // (other programs busy, or no load window).
    let mut instruction_ratios: std::collections::BTreeMap<String, Vec<u64>> =
        std::collections::BTreeMap::new();
    // A run's means (wall, and instructions a unit where the machine
    // counts them), and its load line when clocks found it no evidence
    // (other programs busy, or no load window).
    let mut means = |exe: &str,
                     side: &str,
                     pin: Option<usize>|
     -> (
        crate::Map<String, u128>,
        crate::Map<String, u128>,
        Option<String>,
    ) {
        run_index += 1;
        let kept = keep
            .as_ref()
            .map(|dir| dir.join(format!("run-{run_index:03}-{side}.tsv")));
        let side_sizes = if side == "old" { &old_sizes } else { &sizes };
        let (file, load) = regress_run(exe, rounds, &scope, side_sizes, kept.as_deref(), pin);
        let builds = if side == "old" {
            &mut old_builds
        } else {
            &mut new_builds
        };
        let build = file_provenance(&file);
        if !builds.contains(&build) {
            builds.push(build);
        }
        if !powers.contains(&file.power) {
            powers.push(file.power.clone());
        }
        // Where clocks measures no load (macOS: clocks::load's doc), a run's
        // line says "not measured", and the run is evidence: keep that
        // machine quiet by hand while it measures.
        let busy = (clocks::load::MEASURED && !load.starts_with("quiet")).then_some(load);
        let instructions = cell_instructions(&file);
        (
            file.cells
                .into_iter()
                .map(|(key, samples)| (key, Mean::of(&samples).fixed().0))
                .collect(),
            instructions,
            busy,
        )
    };
    for pair in 0..pairs {
        let began = clocks::now();
        // A pair with a run that is no evidence runs again, in the same
        // order, up to REGRESS_REPEATS times (a process launch on macOS
        // now and then costs other programs about 1.7 CPU-seconds inside
        // a run's window: fork runner jobs 1304-1305, October 5, 2026).
        let mut attempt = 0;
        let (old_exe, new_exe) = (
            &old_exes[pair % old_exes.len()],
            &new_exes[pair % new_exes.len()],
        );
        let (old, old_instructions, new, new_instructions) = loop {
            let pin = pins[pair];
            let ((old, oi, a), (new, ni, b)) = if pair % 2 == 0 {
                let o = means(old_exe, "old", pin);
                (o, means(new_exe, "new", pin))
            } else {
                let n = means(new_exe, "new", pin);
                (means(old_exe, "old", pin), n)
            };
            let busy: Vec<String> = a.into_iter().chain(b).collect();
            if busy.is_empty() {
                break (old, oi, new, ni);
            }
            attempt += 1;
            if attempt > REGRESS_REPEATS {
                unreliable.extend(busy);
                break (old, oi, new, ni);
            }
            eprintln!("regress: pair {} again: {}", pair + 1, busy.join("; "));
        };
        for (key, old_count) in &old_instructions {
            if let Some(new_count) = new_instructions.get(key) {
                instruction_ratios
                    .entry(key.clone())
                    .or_default()
                    .push(clocks::summary::ratio_permille(*new_count, *old_count));
            }
        }
        for (key, old_mean) in &old {
            let Some(&new_mean) = new.get(key) else {
                continue;
            };
            ratios
                .entry(key.clone())
                .or_default()
                .push(clocks::summary::ratio_permille(new_mean, *old_mean));
            pairs_means
                .entry(key.clone())
                .or_default()
                .push((*old_mean, new_mean));
        }
        let tenths = (clocks::since_ns(began) + 50_000_000) / 100_000_000;
        eprintln!(
            "regress: pair {} of {pairs} in {}.{} s",
            pair + 1,
            tenths / 10,
            tenths % 10
        );
    }
    print_sides("regress", &old_builds, &new_builds);
    let power = format!("regress: power during the check: {}", powers.join("; "));
    if !unreliable.is_empty() {
        println!("regress: other programs kept the machine busy, or clocks saw no load window, during {} runs. No verdict (exit 2); run again when nothing else runs on the machine.", unreliable.len());
        for load in &unreliable {
            println!("  {load}");
        }
        println!("{power}");
        return 2;
    }
    let percent = |permille: u64| {
        let d = permille as i64 - 1000;
        format!(
            "{}{}.{}%",
            if d < 0 { "-" } else { "+" },
            d.abs() / 10,
            d.abs() % 10
        )
    };
    // The cells judged (the points' solo cells; with --full or a chosen
    // selection, every cell every pair measured), all at once
    // (clocks::summary::discoveries).
    // With --judge, those cells alone: a second stage's family (check).
    let judged: Vec<&String> = pairs_means
        .iter()
        .filter(|(_, m)| m.len() == pairs)
        .map(|(k, _)| k)
        .filter(|k| only.as_ref().is_none_or(|only| only.contains(k)))
        .collect();
    if let Some(only) = &only {
        assert_eq!(
            judged.len(),
            only.len(),
            "--judge: every cell named is measured in every pair"
        );
    }
    let tested: Vec<Vec<(u128, u128)>> = judged.iter().map(|k| pairs_means[*k].clone()).collect();
    let (judgments, measurement) = judge_once(&tested);
    let mut slower = 0;
    let mut suspects: Vec<(usize, &String)> = Vec::new();
    // The level cells whose instructions moved beyond their margin: the
    // code's doing, so the strongest suspects (rank 0), ahead of the
    // wall-ranked ones.
    let moved = instructions_moved(&instruction_ratios, pairs);
    for (i, (key, judgment)) in judged.into_iter().zip(judgments).enumerate() {
        // A row whose measurement an earlier row holds (an owned cell
        // measured as its lent twin): its verdict printed, not counted again.
        let first = measurement[i] == i;
        if judgment.verdict == clocks::summary::Verdict::Level {
            if first && moved.iter().any(|(k, _)| k == key) {
                // Its place among the moved, the largest move first.
                let place = moved.iter().position(|(k, _)| k == key).unwrap();
                suspects.push((place, key));
            } else if first && judgment.ratio_permille > 1000 + REGRESS_MARGIN_PERMILLE {
                suspects.push((moved.len() + judgment.rank, key));
            }
            continue;
        }
        let word = if judgment.verdict == clocks::summary::Verdict::Slower {
            slower += usize::from(first);
            "SLOWER"
        } else {
            "faster"
        };
        let pairs: Vec<String> = ratios[key].iter().map(|&r| percent(r)).collect();
        println!(
            "  {word} {key}: {} (the pairs' geometric mean; {} pairs: {})",
            percent(judgment.ratio_permille),
            pairs.len(),
            pairs.join(" ")
        );
    }
    if slower > 0 {
        println!("regress: REGRESSION: {slower} cells slower");
    } else {
        println!("regress: no cell slower");
        // The level cells slower beyond the margin, the strongest evidence
        // first: what a second stage confirms on fresh pairs (check). At 8
        // pairs, ranked by ratio, #119's slowed cells were among the 8 in
        // 42 resamplings of 55, ranked by evidence in 52 (the data branch,
        // 2026-10-31/check-errors-i7).
        suspects.sort();
        if instruction_ratios.is_empty() {
            println!("regress: this machine counts no instructions; the suspects are wall's alone");
        } else if moved.is_empty() {
            println!(
                "regress: no level cell's instructions moved beyond {}",
                percent(1000 + INSTRUCTIONS_MARGIN_PERMILLE)
            );
        } else {
            let listed: Vec<String> = moved
                .iter()
                .map(|(k, r)| format!("{k} {}", percent(*r)))
                .collect();
            println!(
                "regress: instructions moved beyond {} in {} level cells, suspects first: {}",
                percent(1000 + INSTRUCTIONS_MARGIN_PERMILLE),
                moved.len(),
                listed.join("; ")
            );
        }
        if !suspects.is_empty() {
            let named: Vec<&str> = suspects
                .iter()
                .take(REGRESS_SUSPECTS)
                .map(|(_, k)| k.as_str())
                .collect();
            println!("regress: suspects: {}", named.join(";"));
        }
    }
    println!("{power}");
    i32::from(slower > 0)
}

/// The cells whose instructions a unit moved beyond INSTRUCTIONS_MARGIN_
/// PERMILLE the same way in every pair, the largest median move first:
/// the solo cells of single-threaded contenders (whose calling thread
/// does the whole work), each counted in every pair (a cell a side lacks
/// counts for says nothing). Every pair, since a change in the code moves
/// the count the same way whatever the layout (#119's cells: x1.0119 in
/// five pairs to four places), where a count that moves with the link
/// layout or the run (the interleaved turns, 0.95-1.02 between pairs of
/// one build) moves both ways (data branch, 2026-11-04/judge3-i7 and
/// judge-i7, aa_instructions.py).
fn instructions_moved(
    ratios: &std::collections::BTreeMap<String, Vec<u64>>,
    pairs: usize,
) -> Vec<(String, u64)> {
    let mut moved: Vec<(String, u64)> = ratios
        .iter()
        .filter(|(key, r)| r.len() == pairs && instructions_tell(key))
        .filter_map(|(key, r)| {
            let mut sorted = r.clone();
            sorted.sort_unstable();
            let median = sorted[sorted.len() / 2];
            let (least, most) = (sorted[0], sorted[sorted.len() - 1]);
            let all_more = least > 1000 + INSTRUCTIONS_MARGIN_PERMILLE;
            let all_fewer = most < 1000 - INSTRUCTIONS_MARGIN_PERMILLE;
            (all_more || all_fewer).then(|| (key.clone(), median))
        })
        .collect();
    moved.sort_by_key(|(key, median)| (std::cmp::Reverse(median.abs_diff(1000)), key.clone()));
    moved
}

/// Whether a cell's calling-thread instructions are the code's alone: a
/// solo cell of a contender without a pool.
fn instructions_tell(key: &str) -> bool {
    let mut fields = key.split('|');
    let (Some(contender), Some(scenario)) = (fields.next(), fields.next()) else {
        return false;
    };
    scenario == "solo"
        && Algorithm::ALL
            .into_iter()
            .find(|a| a.key() == contender)
            .is_some_and(|a| !a.multithreaded())
}

/*
 * A samples file's header, the same in every samples file the benchmark
 * writes (the main run's, b3sum's, leaves'): the format's version; what measured
 * (this build of the benchmark, its compiler and target) and on what
 * machine; then `run`'s own keys, what this run measured; then the
 * machine's power and other programs' load over the run.
 */
fn samples_header(machine: &MachineMetadata, run: &[(String, String)], load: &str) -> String {
    let mut out = String::new();
    writeln!(out, "{SAMPLES_VERSION}").unwrap();
    for (key, value) in [
        ("timestamp", machine.timestamp.as_str()),
        ("bench-hashes version", BENCH_VERSION),
        ("bench-hashes source", GIT_SOURCE),
        ("git commit", GIT_COMMIT),
        ("git clean status", GIT_CLEAN_STATUS),
        ("dependency lock", LOCK_FINGERPRINT),
        ("rust compiler", RUSTC_VERSION),
        ("c compiler", C_COMPILER),
        ("rustflags", RUSTFLAGS),
        ("build target", BUILD_TARGET),
        ("target features", TARGET_FEATURES),
        ("sample clock", clocks::WALL_CLOCK),
        ("clocks source", CLOCKS_SOURCE_INFO),
        ("cpu type", machine.cpu_type.as_str()),
        ("cpu count", machine.cpu_count.to_string().as_str()),
        ("os type", machine.os_type.as_str()),
        ("cpu identity", machine.cpu_identity.as_str()),
        ("idle states", machine.idle_states.as_str()),
        (
            "effective idle states",
            machine.effective_idle_states.as_str(),
        ),
        ("idle control", machine.idle_control.as_str()),
        ("shared placements", machine.placements.as_str()),
    ]
    .into_iter()
    .chain(
        run.iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    ) {
        writeln!(out, "# {key}: {value}").unwrap();
    }
    writeln!(out, "# power: {}", machine.describe_power()).unwrap();
    if let Some(allocator) = diag_alloc::DESCRIPTION {
        writeln!(out, "# global allocator: {allocator}").unwrap();
    }
    writeln!(out, "# load: {load}").unwrap();
    let windows: Vec<String> = machine
        .load
        .iter()
        .map(|w| {
            format!(
                "{}-{}:{}:{}",
                w.start_ns / 1_000_000,
                w.end_ns / 1_000_000,
                w.other_milli_cpus,
                w.steal_milli_cpus
            )
        })
        .collect();
    writeln!(
        out,
        "# load windows (start ms-end ms:other milli-CPUs:steal milli-CPUs): {}",
        windows.join(",")
    )
    .unwrap();
    out
}

fn generate_samples_tsv(
    roster: &Roster,
    samples: &RunSamples,
    machine: &MachineMetadata,
    selection_note: &str,
) -> String {
    // Each contender's source, every one the run measured (a contender added
    // later has its line without anyone remembering to add it).
    let mut run: Vec<(String, String)> = roster
        .algorithms
        .iter()
        .map(|a| (format!("source {}", a.key()), a.source().to_owned()))
        .collect();
    run.extend([
        ("contenders".to_owned(), selection_note.to_owned()),
        ("rounds".to_owned(), roster.rounds.to_string()),
        ("measuring thread".to_owned(), describe_pinning()),
        ("sample sizes".to_owned(), roster.sample_sizes.clone()),
        (
            "buffers owned".to_owned(),
            "where a contender makes the same calls as with buffers lent, one measurement, shown in both rows".to_owned(),
        ),
        (
            "nonstop train".to_owned(),
            format!(
                "{} ms warm-up, {} samples a visit",
                roster.train.warm_ns / 1_000_000,
                roster.train.samples
            ),
        ),
        (
            "points".to_owned(),
            roster
                .points
                .iter()
                .map(|&index| POINTS[index].label)
                .collect::<Vec<_>>()
                .join(","),
        ),
    ]);
    let mut out = samples_header(machine, &run, &clocks::load::describe(&machine.load));
    writeln!(out, "{SAMPLES_COLUMNS}").unwrap();
    for (algorithm_index, &algorithm) in roster.algorithms.iter().enumerate() {
        for scenario in Scenario::ALL {
            let rows = match scenario {
                Scenario::Solo => &samples.solo,
                Scenario::Shared(p) => &samples.shared[p.index()],
            };
            for (point_index, point) in POINTS.iter().enumerate() {
                let cell_samples = &rows[algorithm_index][point_index];
                if cell_samples.is_empty() {
                    continue;
                }
                let values: Vec<String> = cell_samples
                    .iter()
                    .map(|m| format!("{}/{}", m.ns, m.units))
                    .collect();
                let timestamps = match scenario {
                    Scenario::Solo => &samples.solo_started_ns,
                    Scenario::Shared(p) => &samples.shared_started_ns[p.index()],
                };
                let cell_starts = &timestamps[algorithm_index][point_index];
                assert_eq!(
                    cell_starts.len(),
                    cell_samples.len(),
                    "each sample has its own load timestamp"
                );
                let starts: Vec<String> = cell_starts
                    .iter()
                    .map(|ns| (ns / 1_000_000).to_string())
                    .collect();
                let (cpu, energy, counts, on) = match scenario {
                    Scenario::Solo => (
                        cpu_field(&samples.solo_cpu_ns[algorithm_index][point_index]),
                        cpu_field(&samples.solo_energy_nj[algorithm_index][point_index]),
                        counts_field(&samples.solo_counts[algorithm_index][point_index]),
                        cpu_of_field(&samples.solo_cpu[algorithm_index][point_index]),
                    ),
                    Scenario::Shared(p) => (
                        "-".to_string(),
                        "-".to_string(),
                        counts_field(
                            &samples.shared_counts[p.index()][algorithm_index][point_index],
                        ),
                        cpu_of_field(&samples.shared_cpu[p.index()][algorithm_index][point_index]),
                    ),
                };
                writeln!(
                    out,
                    "{}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    algorithm.key(),
                    scenario.key(),
                    point.use_case,
                    point.label,
                    point.use_case.unit_key(),
                    values.join(","),
                    starts.join(","),
                    cpu,
                    energy,
                    counts,
                    on,
                )
                .unwrap();
            }
        }
    }
    out
}

/*
 * The text report: results first, one table per scenario and use case,
 * each cell's mean; then where the numbers came from, for whoever needs
 * to trust or reproduce them.
 */
fn generate_text(
    roster: &Roster,
    results: &Results,
    machine: &MachineMetadata,
    selection_note: &str,
) -> String {
    let mut output = String::new();

    writeln!(
        output,
        "Hash speed on {} ({}, {} CPUs), {}",
        machine.cpu_type, machine.os_type, machine.cpu_count, machine.timestamp
    )
    .unwrap();
    writeln!(
        output,
        "{} run: {} rounds{}. Each cell is the mean time per unit (total time over total work), lower is better.",
        if roster.points.iter().all(|&index| POINTS[index].quick()) { "Quick" } else { "Full" },
        roster.rounds,
        if roster.points.iter().all(|&index| POINTS[index].quick()) { "; a full run confirms and adds the largest inputs and batches" } else { "" },
    )
    .unwrap();
    writeln!(output).unwrap();

    /* How the program calls, for the tables this run shows. */
    for key in ["busy", "brief", "idle", "controlled", "nonstop"] {
        let names: Vec<String> = UseCase::ALL
            .into_iter()
            .filter(|&use_case| {
                use_case.pattern_key() == key
                    && roster
                        .points
                        .iter()
                        .any(|&index| POINTS[index].use_case == use_case)
            })
            .map(|use_case| format!("\u{201c}{}\u{201d}", use_case.heading()))
            .collect();
        let Some(pattern) = UseCase::ALL
            .into_iter()
            .find(|use_case| use_case.pattern_key() == key)
            .map(UseCase::pattern)
        else {
            continue;
        };
        let list = match names.as_slice() {
            [] => continue,
            [one] => one.clone(),
            [first, second] => format!("{first} and {second}"),
            [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
        };
        writeln!(output, "In {list}, {pattern}.").unwrap();
    }
    writeln!(output).unwrap();

    for scenario in Scenario::ALL {
        writeln!(
            output,
            "{}: {}.",
            scenario.heading().to_uppercase(),
            scenario.description()
        )
        .unwrap();
        writeln!(output).unwrap();
        for use_case in UseCase::ALL {
            append_table(&mut output, roster, results, scenario, use_case);
        }
    }

    writeln!(output, "PROVENANCE").unwrap();
    writeln!(
        output,
        "  bench-hashes {BENCH_VERSION}, {GIT_SOURCE}, commit {GIT_COMMIT} ({GIT_CLEAN_STATUS})"
    )
    .unwrap();
    writeln!(output, "  contenders: {selection_note}").unwrap();
    for algorithm in &roster.algorithms {
        writeln!(
            output,
            "  {}: {}; {}",
            algorithm.name(),
            algorithm.source(),
            algorithm.mode()
        )
        .unwrap();
    }
    writeln!(output, "  CPU: {}", machine.cpu_identity).unwrap();
    writeln!(
        output,
        "  {RUSTC_VERSION}; target {BUILD_TARGET}; features {TARGET_FEATURES}"
    )
    .unwrap();
    if let Some(allocator) = diag_alloc::DESCRIPTION {
        writeln!(output, "  global allocator: {allocator}").unwrap();
    }
    writeln!(output, "  clock: {}", clocks::WALL_CLOCK).unwrap();
    writeln!(output, "  clocks: {CLOCKS_SOURCE_INFO}").unwrap();
    writeln!(
        output,
        "  load during the run: {}",
        clocks::load::describe(&machine.load)
    )
    .unwrap();
    writeln!(output, "  power: {}", machine.describe_power()).unwrap();

    output
}

/// One results table: a row per point, a column per contender taking part.
fn append_table(
    output: &mut String,
    roster: &Roster,
    results: &Results,
    scenario: Scenario,
    use_case: UseCase,
) {
    let contenders: Vec<usize> = (0..roster.len())
        .filter(|&index| {
            roster.algorithms[index].takes_part(use_case)
                && scenario.measures(roster.algorithms[index], use_case)
        })
        .collect();
    let points: Vec<usize> = use_case
        .points()
        .filter(|&index| roster.measures(index))
        .collect();
    if points.is_empty() || contenders.is_empty() {
        return;
    }
    writeln!(
        output,
        "  {} ({})",
        use_case.heading(),
        use_case.time_unit()
    )
    .unwrap();
    write!(output, "  {:<8}", use_case.column()).unwrap();
    for &algorithm_index in &contenders {
        write!(
            output,
            "  {:>14}",
            column_heading(roster.algorithms[algorithm_index])
        )
        .unwrap();
    }
    writeln!(output).unwrap();
    for &point_index in &points {
        write!(output, "  {:<8}", POINTS[point_index].label).unwrap();
        for &algorithm_index in &contenders {
            let statistics = cell(results, algorithm_index, point_index).get(scenario);
            write!(output, "  {:>13} ", statistics.format_ns(1)).unwrap();
        }
        writeln!(output).unwrap();
    }
    writeln!(output).unwrap();
}

/// Column heading that fits the 13-character summary columns.
fn column_heading(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::Blake3 => "B3 official",
        Algorithm::Blake3Rayon => "B3 official mt",
        Algorithm::Sha256CommonCrypto => "SHA-256 CC",
        Algorithm::Sha256Ring => "SHA-256 ring",
        Algorithm::Blake3ServilSt => "B3 servil st",
        Algorithm::Blake3ServilMt => "B3 servil mt",
        other => other.name(),
    }
}

fn machine_metadata() -> MachineMetadata {
    // CPUs alone: every process's name (a kworker's changes as it works)
    // went into the heap before the measurement and moved the buffers
    // after it to a new offset each run (bench-hashes #65).
    let system =
        System::new_with_specifics(RefreshKind::new().with_cpu(CpuRefreshKind::everything()));

    let cpus = system.cpus();

    assert!(
        !cpus.is_empty(),
        "the operating system must report at least one logical CPU"
    );

    let cpu_type = cpus
        .iter()
        .map(|cpu| cpu.brand().trim())
        .find(|brand| !brand.is_empty())
        .unwrap_or(std::env::consts::ARCH)
        .to_owned();

    let kernel = System::kernel_version().unwrap_or_else(|| "unreported".to_owned());

    let os_type = if std::env::consts::OS == "macos" {
        let kernel_major = kernel
            .split('.')
            .next()
            .expect("kernel version must have a first component");

        format!("darwin{kernel_major}")
    } else {
        format!("{}{}", std::env::consts::OS, kernel)
    };

    MachineMetadata {
        timestamp: utc_timestamp(),
        cpu_type,
        cpu_count: cpus.len(),
        os_type,
        cpu_identity: cpu_identity(),
        idle_states: idle_states(),
        effective_idle_states: effective_idle_states(),
        idle_control: "not used by this command".to_owned(),
        placements: topology().describe(),
        load: Vec::new(),
        power: [Power::read(), None],
    }
}

/*
 * The CPU's identity for comparing runs across machines. Linux: the first
 * processor's implementer, variant, part, and revision (a hypervisor may
 * hide the part), its feature list (which pins an ARM core's generation),
 * and the machine model the device tree or DMI names (a VM says so). macOS:
 * the brand string and the core count of each performance level. Empty
 * where neither source exists.
 */
/*
 * The machine's idle states (bench-hashes #106): a call after a pause pays
 * for the depth the core slept at, which Linux's idle governor chooses
 * from recent history. On the i7-12700K a 16 x 64 B batch after 1 ms
 * costs 13.5 ns a message after C1E, 25.8 after C6, 58.2 after C8; the
 * EPYC 9255 runs no cpuidle driver and has no such modes; the Xeon Gold
 * 6330 is 14 times slower than the EPYC in that cell, mostly this. Linux
 * reports the driver and governor, and CPU 0's states as "NAME latency
 * LAT us residency RES us" with "disabled" where a state is; "none listed"
 * where no driver runs (the kernel's default idle, the EPYC 9255); other systems report no states ("not reported by this
 * OS"), as the power line does.
 */
fn idle_states() -> String {
    describe_idle_states(&|path| fs::read_to_string(path).ok())
}

/// The idle states a core enters in fact under the after-pause cells'
/// pauses: at the run's start this thread sleeps the brief pause and the
/// long one, each many times over, and its core's hardware residency
/// counters say what share of each it spent in each C-state (Linux,
/// Intel's `cstate_core` events; bench-hashes #117 and #128: a Xeon Gold
/// 6330 enters its C6 for 66% of a long idle and seldom within 1 ms,
/// and the i7-12700K's deep pauses sit in core C7, which no software
/// counter shows). Elsewhere, "not measured".
fn effective_idle_states() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Some(text) = effective_idle_states_linux() {
            return text;
        }
    }
    "not measured (no cstate_core counters readable here)".to_owned()
}

/// A pause as the headings name it: "20 us", "5 ms".
fn describe_pause(ns: u64) -> String {
    if ns % 1_000_000 == 0 {
        format!("{} ms", ns / 1_000_000)
    } else {
        format!("{} us", ns / 1_000)
    }
}

/// The pauses the line measures, each repeated to about 100 ms of sleep.
const EFFECTIVE_IDLE_PAUSES: [(u64, u32); 2] = [(BRIEF_PAUSE_NS, 1000), (LONG_PAUSE_NS, 20)];

#[cfg(target_os = "linux")]
fn effective_idle_states_linux() -> Option<String> {
    use std::ffi::{c_int, c_long, c_void};
    #[repr(C)]
    struct Attr {
        kind: u32,
        size: u32,
        config: u64,
        sample_period: u64,
        sample_type: u64,
        read_format: u64,
        flags: u64,
        reserved: [u64; 2],
    }
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
        fn read(fd: c_int, buffer: *mut c_void, size: usize) -> isize;
        fn close(fd: c_int) -> c_int;
        fn sched_getcpu() -> c_int;
    }
    let dir = "/sys/bus/event_source/devices/cstate_core";
    let kind: u32 = fs::read_to_string(format!("{dir}/type"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let cpu = unsafe { sched_getcpu() };
    if cpu < 0 {
        return None;
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(format!("{dir}/events")).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(state) = name.strip_suffix("-residency") {
            let spec = fs::read_to_string(entry.path()).ok()?;
            let config = u64::from_str_radix(spec.trim().strip_prefix("event=0x")?, 16).ok()?;
            names.push((state.to_uppercase(), config));
        }
    }
    names.sort();
    // A counter of each state on this CPU, system-wide (the core's time
    // in the state, whoever idles it).
    let mut fds = Vec::new();
    for &(_, config) in &names {
        let attr = Attr {
            kind,
            size: 64,
            config,
            sample_period: 0,
            sample_type: 0,
            read_format: 0,
            flags: 0,
            reserved: [0; 2],
        };
        // Sound: perf_event_open reads the attr given; any pid, this CPU, no group, CLOEXEC.
        let fd = unsafe {
            syscall(
                298,
                &attr as *const Attr,
                -1 as c_long,
                c_long::from(cpu),
                -1 as c_long,
                8 as c_long,
            )
        } as c_int;
        if fd < 0 {
            for fd in fds {
                unsafe { close(fd) };
            }
            return None;
        }
        fds.push(fd);
    }
    let read_all = |fds: &[c_int]| -> Vec<u64> {
        fds.iter()
            .map(|&fd| {
                let mut v = 0u64;
                // Sound: reads 8 bytes into v.
                unsafe { read(fd, &mut v as *mut u64 as *mut c_void, 8) };
                v
            })
            .collect()
    };
    // Each pause as the cells take it (clocks::Gap::Idle: a sleep of its
    // length), repeated; the counters read around the whole.
    let mut measured = Vec::new();
    for (pause_ns, repeats) in EFFECTIVE_IDLE_PAUSES {
        let before = read_all(&fds);
        let started = clocks::now();
        for _ in 0..repeats {
            std::thread::sleep(std::time::Duration::from_nanos(pause_ns));
        }
        let elapsed_ns = clocks::since_ns(started);
        let after = read_all(&fds);
        let ticks: Vec<u64> = before
            .iter()
            .zip(&after)
            .map(|(b, a)| a.saturating_sub(*b))
            .collect();
        measured.push((pause_ns, repeats, elapsed_ns, ticks));
    }
    for fd in fds {
        unsafe { close(fd) };
    }
    // The counters tick at the TSC's rate: the pause in ticks from the
    // machine's rate where it says it (tsc_freq_khz, or the model name's
    // GHz), else from the states' sum, a core idle all along.
    let tsc_hz: Option<u64> = fs::read_to_string("/sys/devices/system/cpu/cpu0/tsc_freq_khz")
        .ok()
        .and_then(|k| k.trim().parse::<u64>().ok())
        .map(|k| k * 1000)
        .or_else(|| {
            fs::read_to_string("/proc/cpuinfo").ok().and_then(|info| {
                info.lines()
                    .find(|l| l.starts_with("model name"))
                    .and_then(|l| {
                        let ghz = l
                            .rsplit('@')
                            .next()?
                            .trim()
                            .strip_suffix("GHz")?
                            .trim()
                            .parse::<f64>()
                            .ok()?;
                        Some((ghz * 1e9) as u64)
                    })
            })
        });
    let names: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    let lines: Vec<String> = measured
        .iter()
        .map(|(pause_ns, repeats, elapsed_ns, ticks)| {
            let total: u128 = ticks.iter().map(|&t| u128::from(t)).sum();
            let pause_ticks = tsc_hz.map_or(total, |hz| {
                u128::from(hz) * u128::from(*elapsed_ns) / 1_000_000_000
            });
            format!(
                "over {repeats} x {}: {}",
                describe_pause(*pause_ns),
                describe_effective_idle_states(&names, ticks, pause_ticks)
            )
        })
        .collect();
    Some(lines.join("; "))
}

/// The line: each state's share of the pause's ticks (the TSC's), and the
/// rest, which the core spent awake or in a state no counter names:
/// "C1 5%, C6 95%, C7 0%, awake or unnamed 0%".
fn describe_effective_idle_states(names: &[&str], ticks: &[u64], pause_ticks: u128) -> String {
    let total: u128 = ticks.iter().map(|&t| u128::from(t)).sum();
    let pause_ticks = pause_ticks.max(total).max(1);
    let share = |t: u128| (t * 100 + pause_ticks / 2) / pause_ticks;
    let mut parts: Vec<String> = names
        .iter()
        .zip(ticks)
        .map(|(n, &t)| format!("{n} {}%", share(u128::from(t))))
        .collect();
    parts.push(format!("awake or unnamed {}%", share(pause_ticks - total)));
    parts.join(", ")
}

/// The machine's idle states under the bench's control (bench-hashes #117,
/// Sam Shackleton's protocol): where every cpuidle `disable` file is
/// writable (the bench runs as root), the after-pause cells run once
/// more (the ControlledIdle* use cases) with every state but POLL and C1
/// switched off, so a call meets the core awake and the cell measures
/// the library alone; the shipped block, what users meet, stays the
/// first. The states are chosen by name: POLL and C1 kept, or POLL alone
/// where the machine has no plain C1 (the i7-12700K: C1E, C6, C8, C10).
/// Applied as a guard that writes each file's old value back when it
/// drops, so a run that ends early leaves the machine as it found it (a
/// run killed outright cannot; the header names the files).
struct IdleControl {
    /// Each state's `disable` file, with the value it held and whether
    /// the block keeps the state.
    files: Vec<(std::path::PathBuf, String, bool)>,
    /// Whether the machine has a plain C1 (kept beside POLL).
    has_c1: bool,
}

impl IdleControl {
    /// The control for the cpuidle tree at `root` (`/sys/devices/system/cpu`):
    /// None where there are no states, or any `disable` file is not
    /// writable.
    fn detect(root: &std::path::Path) -> Option<IdleControl> {
        let mut files = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for cpu in fs::read_dir(root).ok()?.flatten() {
            let name = cpu.file_name().to_string_lossy().into_owned();
            let Some(number) = name.strip_prefix("cpu") else {
                continue;
            };
            if number.parse::<u32>().is_err() {
                continue;
            }
            let Ok(states) = fs::read_dir(cpu.path().join("cpuidle")) else {
                continue;
            };
            for state in states.flatten() {
                let state_name = fs::read_to_string(state.path().join("name"))
                    .ok()?
                    .trim()
                    .to_owned();
                let disable = state.path().join("disable");
                let held = fs::read_to_string(&disable).ok()?.trim().to_owned();
                // Writable: open for writing succeeds (nothing written).
                fs::OpenOptions::new().write(true).open(&disable).ok()?;
                let keep = state_name == "POLL" || state_name == "C1";
                names.insert(state_name);
                files.push((disable, held, keep));
            }
        }
        let has_c1 = names.contains("C1");
        (!files.is_empty() && files.iter().any(|(_, _, keep)| !keep))
            .then_some(IdleControl { files, has_c1 })
    }

    /// The states the block keeps, "POLL and C1" or "POLL alone".
    fn kept(&self) -> &'static str {
        if self.has_c1 {
            "POLL and C1"
        } else {
            "POLL alone"
        }
    }

    /// Switch every other state off, until the guard drops.
    fn apply(&self) -> IdleControlGuard<'_> {
        for (path, _, keep) in &self.files {
            if !keep {
                fs::write(path, "1\n")
                    .unwrap_or_else(|e| panic!("switching off {}: {e}", path.display()));
            }
        }
        IdleControlGuard { control: self }
    }

    /// Write every file's old value back.
    fn restore(&self) {
        for (path, held, _) in &self.files {
            let _ = fs::write(path, format!("{held}\n"));
        }
    }

    /// The header's line: what the controlled cells keep, or why there
    /// are none.
    fn describe(control: Option<&IdleControl>) -> String {
        match control {
            Some(control) => format!(
                "the controlled cells keep {} and switch the other states off; run as root",
                control.kept()
            ),
            None => "none (no cpuidle states the bench can switch off: run as root for the controlled cells)".to_owned(),
        }
    }
}

struct IdleControlGuard<'a> {
    control: &'a IdleControl,
}

impl Drop for IdleControlGuard<'_> {
    fn drop(&mut self) {
        self.control.restore();
    }
}

fn describe_idle_states(read: &dyn Fn(&str) -> Option<String>) -> String {
    let dir = "/sys/devices/system/cpu";
    let Some(driver) = read(&format!("{dir}/cpuidle/current_driver")) else {
        return "not reported by this OS".to_owned();
    };
    let driver = driver.trim();
    if driver == "none" {
        return "none listed (no cpuidle driver; the kernel's default idle)".to_owned();
    }
    let governor = read(&format!("{dir}/cpuidle/current_governor"))
        .map_or("?".to_owned(), |g| g.trim().to_owned());
    let mut states = Vec::new();
    for i in 0.. {
        let state = format!("{dir}/cpu0/cpuidle/state{i}");
        let Some(name) = read(&format!("{state}/name")) else {
            break;
        };
        let field =
            |f: &str| read(&format!("{state}/{f}")).map_or("?".to_owned(), |v| v.trim().to_owned());
        states.push(format!(
            "{} latency {} us residency {} us{}",
            name.trim(),
            field("latency"),
            field("residency"),
            if field("disable") == "1" {
                " disabled"
            } else {
                ""
            }
        ));
    }
    format!(
        "driver {driver}, governor {governor}; {}",
        states.join(", ")
    )
}

fn cpu_identity() -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        let first = cpuinfo.split("\n\n").next().unwrap_or("");
        for key in [
            "CPU implementer",
            "CPU variant",
            "CPU part",
            "CPU revision",
            "model name",
            "microcode",
            "Features",
            "flags",
        ] {
            if let Some(line) = first
                .lines()
                .find(|line| line.split(':').next().is_some_and(|k| k.trim() == key))
            {
                parts.push(format!("{key}: {}", line.split_once(':').unwrap().1.trim()));
            }
        }
        for path in [
            "/sys/firmware/devicetree/base/compatible",
            "/sys/class/dmi/id/product_name",
        ] {
            if let Ok(model) = fs::read(path) {
                let model = String::from_utf8_lossy(&model)
                    .replace('\0', " ")
                    .trim()
                    .to_owned();
                if !model.is_empty() {
                    parts.push(format!("machine: {model}"));
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        for key in [
            "machdep.cpu.brand_string",
            "hw.perflevel0.physicalcpu",
            "hw.perflevel1.physicalcpu",
        ] {
            if let Ok(out) = std::process::Command::new("sysctl")
                .args(["-n", key])
                .output()
            {
                let value = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if out.status.success() && !value.is_empty() {
                    parts.push(format!("{key}: {value}"));
                }
            }
        }
    }
    parts.join(" · ")
}

fn utc_timestamp() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch");

    assert!(
        duration.as_secs() <= i64::MAX as u64,
        "system time must fit in the Gregorian conversion"
    );

    let total_seconds = duration.as_secs() as i64;
    let days = total_seconds.div_euclid(86_400);
    let seconds_in_day = total_seconds.rem_euclid(86_400);

    let hour = seconds_in_day / 3600;
    let minute = (seconds_in_day % 3600) / 60;
    let second = seconds_in_day % 60;

    let (year, month, day) = civil_date_from_unix_days(days);

    format!(
        "{year:04}-{month:02}-{day:02} \
         {hour:02}:{minute:02}:{second:02} UTC"
    )
}

fn civil_date_from_unix_days(unix_days: i64) -> (i64, i64, i64) {
    let shifted = unix_days + 719_468;

    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;

    let day_of_era = shifted - era * 146_097;

    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;

    let mut year = year_of_era + era * 400;

    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);

    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;

    let month = month_prime + if month_prime < 10 { 3 } else { -9 };

    if month <= 2 {
        year += 1;
    }

    (year, month, day)
}

fn sanitize_alphanumeric(input: &str) -> String {
    let sanitized: String = input.chars().filter(char::is_ascii_alphanumeric).collect();

    assert!(
        !sanitized.is_empty(),
        "sanitized identifier must not be empty; input was {input:?}"
    );

    sanitized
}

fn output_directory(machine: &MachineMetadata) -> std::path::PathBuf {
    let cpu = sanitize_alphanumeric(&machine.cpu_type);
    let os = sanitize_alphanumeric(&machine.os_type);

    std::path::PathBuf::from("benchmark-results").join(format!("{cpu}.{os}"))
}

/* Provenance that describes the run as a whole. */

/* Provenance that belongs to one contender and hides with it: the
implementation and its mode. */

fn xml_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());

    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }

    escaped
}

mod b3sum;
mod chart;
mod check;
mod diag_alloc;
#[cfg(test)]
mod harness_tests;
mod samecode;
#[cfg(test)]
mod require_nextest {
    include!("../tools/require_nextest.rs");
}
mod crowd;
mod leaves;
mod map;
mod overview;

#[cfg(test)]
mod correctness_tests {
    /// No std map with its per-process random seed: each draw would lay a
    /// run's heap out its own way (bench-hashes #65 found the main
    /// thread's kept buffers at a new offset in every process).
    #[test]
    fn every_map_is_seeded() {
        for (name, source) in [
            ("main.rs", include_str!("main.rs")),
            ("b3sum.rs", include_str!("b3sum.rs")),
        ] {
            let std_maps = source
                .lines()
                .filter(|l| {
                    l.contains(concat!("std::collections::", "HashMap"))
                        || l.contains(concat!("std::collections::", "HashSet"))
                })
                .count();
            assert_eq!(
                std_maps,
                if name == "main.rs" { 2 } else { 0 },
                "{name}: a std map outside the Map and Set aliases"
            );
        }
    }

    use super::*;

    /// pmset's outputs on an M4 Max (fork runner jobs 350 and 351, September 27, 2026).
    #[test]
    fn power_from_pmset() {
        let settings = "System-wide power settings:\nCurrently in use:\n standby              1\n sleep                1 (sleep prevented by powerd)\n powermode            2\n womp                 0\n";
        let battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=35389539)\t47%; discharging; 2:36 remaining present: true\n";
        let power = Power::parse_pmset(battery, settings).unwrap();
        assert!(power.on_battery && power.slowing());
        assert_eq!(
            power.describe(),
            "battery power (47% charged), High Power mode"
        );
        let mains = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=35389539)\t48%; charging; 1:10 remaining present: true\n";
        let power = Power::parse_pmset(
            mains,
            &settings.replace("powermode            2", "lowpowermode         1"),
        )
        .unwrap();
        assert!(!power.on_battery && power.low_power);
        assert_eq!(power.describe(), "mains power, Low Power Mode");
        // Linux: a desktop with no battery and no platform profile reports
        // its CPU frequency policy; a laptop its battery; "power" is low power.
        let (gov, epp) = (vec!["powersave".to_owned()], vec!["performance".to_owned()]);
        let linux = Power::parse_linux(&[], None, &gov, &epp).unwrap();
        assert_eq!(linux.describe(), "mains power, no battery, CPU frequency governor powersave, energy-performance preference performance");
        assert!(!linux.slowing());
        let laptop = Power::parse_linux(
            &[
                (
                    "Battery".to_owned(),
                    Some("Discharging".to_owned()),
                    Some(61),
                ),
                ("Mains".to_owned(), None, None),
            ],
            Some("balanced"),
            &[],
            &["power".to_owned()],
        )
        .unwrap();
        assert_eq!(laptop.describe(), "battery power (61% charged), platform profile balanced, energy-performance preference power");
        assert!(laptop.slowing());
        assert!(Power::parse_linux(&[], None, &[], &[]).is_none());
        let desktop = Power::parse_pmset("Now drawing from 'AC Power'\n", "").unwrap();
        assert!(!desktop.slowing());
        assert_eq!(desktop.describe(), "mains power");
    }

    /// Fixed and Measured: the one division rounds half up at 2^-64, and the
    /// display rounds once, with three significant digits at every size.
    /// Expected answers worked by hand.
    #[test]
    fn fixed_point_rounds_once_and_late() {
        let t = |ns, units| Fixed(clocks::summary::mean([(ns, units)]));
        assert_eq!(t(3, 1), Fixed(3 << 64));
        assert_eq!(t(1, 3), Fixed(((1u128 << 64) + 1) / 3)); // 2^64 / 3 rounds up
        assert_eq!(t(2, 3), Fixed(((2u128 << 64) + 1) / 3));
        for (ns, units, shown) in [
            (437, 1000, "0.437"),
            (3114, 100_000, "0.0311"),
            (311, 100_000, "0.00311"),
            (121_362, 1000, "121.362"),
            (1, 3, "0.333"),
            (2, 3, "0.667"),
            (1, 30, "0.0333"),
            (99_995, 1_000_000, "0.1000"), // 0.099995 keeps four decimals, rounds up
            (1_000_000, 1_000_000_000, "0.00100"),
            (1, 1_000_000, "0.000001"),
        ] {
            assert_eq!(t(ns, units).format_ns(), shown, "{ns}/{units}");
        }
    }

    fn point(label: &str, use_case: UseCase) -> usize {
        POINTS
            .iter()
            .position(|point| point.label == label && point.use_case == use_case)
            .unwrap()
    }

    /// Results and samples for two contenders over `rounds` rounds: each
    /// cell's sample in round r is `value(contender, point, r)` ns over one
    /// unit, solo and both shared copies alike (the shared ones for the
    /// nonstop use cases alone), summarised as measure_all does.
    fn run(
        roster: &Roster,
        rounds: usize,
        value: impl Fn(usize, usize, usize) -> u64,
    ) -> (Results, RunSamples) {
        let empty = || -> Samples { vec![vec![Vec::new(); POINT_COUNT]; roster.len()] };
        let mut samples = RunSamples {
            solo: empty(),
            shared: std::array::from_fn(|_| empty()),
            solo_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            shared_started_ns: std::array::from_fn(|_| {
                vec![vec![Vec::new(); POINT_COUNT]; roster.len()]
            }),
            solo_cpu_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            solo_energy_nj: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            solo_counts: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            shared_counts: std::array::from_fn(|_| {
                vec![vec![Vec::new(); POINT_COUNT]; roster.len()]
            }),
            solo_cpu: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            shared_cpu: std::array::from_fn(|_| vec![vec![Vec::new(); POINT_COUNT]; roster.len()]),
        };
        let mut results: Results = vec![vec![None; POINT_COUNT]; roster.len()];
        for a in 0..roster.len() {
            for &p in &roster.points {
                for r in 0..rounds {
                    let v = Measured::new(value(a, p, r), 1);
                    samples.solo[a][p].push(v);
                    for pl in Placement::ALL {
                        if Scenario::Shared(pl).measures(roster.algorithms[a], POINTS[p].use_case) {
                            let i = pl.index();
                            samples.shared[i][a][p].extend([v, v]);
                            // Separate starts cross load-window boundaries, and
                            // distinguish the two copies from one another.
                            samples.shared_started_ns[i][a][p].extend([
                                10_000_000 + r as u64 * 1_000_000,
                                11_000_000 + r as u64 * 1_000_000,
                            ]);
                            // Copy 0 on CPU 2, copy 1 on CPU 5.
                            samples.shared_cpu[i][a][p].extend([Some(2), Some(5)]);
                        }
                    }
                    samples.solo_started_ns[a][p].push(r as u64 * 1_000_000);
                    samples.solo_cpu[a][p].push(Some(3));
                    samples.solo_cpu_ns[a][p].push(Some(2 * value(a, p, r)));
                    samples.solo_energy_nj[a][p].push(Some(3 * value(a, p, r)));
                    let v = value(a, p, r);
                    samples.solo_counts[a][p].push(Some(clocks::Counts {
                        p: clocks::Level {
                            cycles: 4 * v,
                            instructions: 5 * v,
                            cache_references: 6 * v,
                            cache_misses: 7 * v,
                            time_ns: v,
                        },
                        e: clocks::Level::default(),
                    }));
                }
                results[a][p] = Some(Cell {
                    solo: Mean::of(&samples.solo[a][p]),
                    shared: Placement::ALL.map(|pl| {
                        Scenario::Shared(pl)
                            .measures(roster.algorithms[a], POINTS[p].use_case)
                            .then(|| Mean::of(&samples.shared[pl.index()][a][p]))
                    }),
                });
            }
        }
        (results, samples)
    }

    include!("fingerprint.rs");

    /// The fingerprint build.rs writes, on a fixed hash: Sigil's own vector,
    /// BLAKE3 of the empty input (johnservil/sigil's README), and it reads
    /// back to the same bytes.
    #[test]
    fn a_fingerprint_is_the_sigil_of_the_hash() {
        let hash = *blake3::hash(b"").as_bytes();
        assert_eq!(fingerprint(&hash), "dirty-ᢳᕌᔃጾዿᑂഔጕꔍⶰᣟꕄꕛቴⴲజᐽᙿᕯⶂጁᑃሃᛜጩ");
        let sigil = fingerprint(&hash)["dirty-".len()..].to_owned();
        assert_eq!(sigilcode::decode(&sigil, 32).unwrap(), hash);
    }

    /// This build's own status: `clean`, or a fingerprint (above) that
    /// decodes to 32 bytes.
    /// A build's line names both commits and both trees' states, and the
    /// A/A line appears exactly where the two sides' builds are one.
    #[test]
    fn each_side_names_its_builds() {
        let (bench, servil) = (
            "1c1958de1234567890abcdef1234567890abcdef",
            "68fd3e57d4a805dff77981d5ac223b309c97989f",
        );
        let info = |status: &str| {
            format!("blake3-servil 0.8.0 (local checkout); source ..; branch HEAD; commit {servil}; {status}")
        };
        let clean = provenance(bench, "clean", &info("clean"));
        assert_eq!(clean, "bench 1c1958de1234 clean; servil 68fd3e57d4a8 clean");
        let dirty = provenance(bench, "clean", &info("dirty-ᕖᣇጎ"));
        assert_eq!(
            dirty,
            "bench 1c1958de1234 clean; servil 68fd3e57d4a8 dirty-ᕖᣇጎ"
        );
        assert_eq!(
            provenance(bench, "clean", ""),
            "bench 1c1958de1234 clean; no servil"
        );
        let one = sides("compare", &[clean.clone()], &[clean.clone()]);
        assert!(one.ends_with("compare: both sides are one build: an A/A comparison\n"));
        let two = sides("compare", &[clean.clone()], &[dirty.clone()]);
        assert_eq!(
            two,
            format!("compare: old side: {clean}\ncompare: new side: {dirty}\n")
        );
    }

    #[test]
    fn a_dirty_status_is_a_sigil_of_its_hash() {
        if let Some(sigil) = GIT_CLEAN_STATUS.strip_prefix("dirty-") {
            assert_eq!(sigil.chars().count(), 25, "{GIT_CLEAN_STATUS}");
            assert_eq!(sigilcode::decode(sigil, 32).map(|d| d.len()), Ok(32));
        } else {
            assert_eq!(GIT_CLEAN_STATUS, "clean");
        }
    }

    #[test]
    fn expect_servil_takes_only_the_named_clean_commit() {
        let c = "934dee8902889186e3da9b62edf43699dd1c6305";
        let patched = format!("blake3-servil 0.8.0+f7ce (local checkout); source https://github.com/johnservil/BLAKE3; branch HEAD; commit {c}; clean; single-threaded");
        expect_servil(c, &patched);
        let pin = "blake3-servil 0.3.0+2e4f; source https://github.com/johnservil/BLAKE3; commit 49b43b1d317305dbd25e1aa93d56f307430a1205; clean";
        let dirty = patched.replace("; clean;", "; dirty-00ab;");
        for (expected, info) in [(c, pin), (c, dirty.as_str()), (&c[..12], patched.as_str())] {
            let r = std::panic::catch_unwind(|| expect_servil(expected, info));
            assert!(r.is_err(), "{expected} accepted {info}");
        }
    }

    #[test]
    fn regress_merges_the_sides_sizes() {
        let old =
            format!("{SIZES_VERSION}\nsha256\t64 B\t40\nblake3-servil-mt\tcontinuous 64 B\t9\n");
        let new = format!("{SIZES_VERSION}\nsha256\t64 B\t44\nblake3-servil-mt\tcontinuous 64 B\t7\nblake3-servil-mt\tcontinuous batch 16\t3\n");
        assert_eq!(merge_sizes(&old, &new), format!("{SIZES_VERSION}\nsha256\t64 B\t40\nblake3-servil-mt\tcontinuous 64 B\t9\nblake3-servil-mt\tcontinuous batch 16\t3\n"));
    }

    /// Every point hashes a prefix of one buffer: the input of each size is
    /// the first bytes of every larger one.
    #[test]
    fn inputs_are_prefixes_of_the_largest() {
        let largest = make_input_seeded(1 << 20, 1);
        for bytes in [0, 1, 63, 64, 4096, 4470, 1 << 20] {
            assert_eq!(
                make_input_seeded(bytes, 1),
                largest[..bytes],
                "{bytes} bytes"
            );
        }
    }

    /// The benchmark asks of BLAKE3-Servil exactly what FROZEN.md says it does.
    #[test]
    fn frozen_contract_matches_frozen_md() {
        let manifest = include_str!("../FROZEN.md");
        let start = manifest
            .find("```frozen\n")
            .expect("FROZEN.md has a frozen block")
            + "```frozen\n".len();
        let end = start
            + manifest[start..]
                .find("```")
                .expect("the frozen block ends");
        assert_eq!(
            &manifest[start..end],
            frozen_contract(),
            "the benchmark's contract with BLAKE3-Servil changed: change FROZEN.md with it, with Zooko's decision and its reason"
        );
    }

    #[test]
    fn mean_is_the_shared_rule_every_sample_counted() {
        // 99 samples of 1000 ns and one of 1,000,000 over one unit each: the
        // slow one counts in full, 10,990 ns a unit, as clocks::summary says
        // (a rule that set it aside would read 1000: bench #60, October 23).
        let mut samples = vec![Measured::new(1000, 1); 99];
        samples.push(Measured::new(1_000_000, 1));
        assert_eq!(
            clocks::summary::mean(samples.iter().map(|m| (m.ns, m.units))),
            10_990u128 << 64
        );
        assert_eq!(Mean::of(&samples).fixed().0, 10_990u128 << 64);
        assert_eq!(Mean::of(&samples).format_ns(1), "10990.000");
    }

    #[test]
    fn means_at_decimal_halfways_round_the_original_measurement_once() {
        // Independently established rational values: 2135/400 = 5.3375,
        // 20555/400 = 51.3875; half-up decimal rounding gives these anchors.
        for (ns, expected) in [(2135, "5.338"), (20555, "51.388")] {
            let measured = [Measured::new(ns, 400), Measured::new(ns, 400)];
            let statistics = Mean::of(&measured);
            assert_eq!(statistics.format_ns(1), expected);
            assert_eq!(statistics.format_ns(400), format!("{ns}.000"));
        }
        // Total time over total work: 4000 ns over 800 units, 5 ns a unit.
        assert_eq!(
            Mean::of(&[Measured::new(3000, 400), Measured::new(1000, 400)]).format_ns(1),
            "5.000"
        );
    }

    #[test]
    fn prepared_samples_select_the_promised_api_and_keep_digest_space() {
        blake3_servil::initialize();
        CALLS.with(|calls| calls.borrow_mut().clear());
        let input = make_input(MESSAGE_LEN);
        take_sample(
            Algorithm::Blake3ServilSt,
            &input,
            Point::one("", MESSAGE_LEN),
            2,
        );
        take_sample(Algorithm::Blake3ServilSt, &input, Point::many("", 1), 3);
        /* One untimed call before each sample's timed ones (clocks: each timed call follows the same call). */
        assert_eq!(
            CALLS.with(|calls| calls.borrow().clone()),
            [
                "hash",
                "hash",
                "hash",
                "hash_many",
                "hash_many",
                "hash_many",
                "hash_many"
            ]
        );
    }

    /// compare's quantity to judge by (bench-hashes #127): a file whose
    /// cell took the same wall time and cycles a unit as another's but
    /// more instructions reads level by wall and by cycles and slower by
    /// instructions; a cell without counts is judged by wall alone.
    /// A level solo cell of a single-threaded contender whose instructions
    /// moved beyond the margin the same way in every pair is named (either
    /// way, the larger move first); one at the margin in any pair, counted
    /// in fewer pairs than the run had, multithreaded, or shared, is not.
    /// A cell's visits spread over the rounds: `visits` of `rounds` slots
    /// take a sample, evenly, and a slot past the rounds (the round plus
    /// the point's offset) wraps; visits at or past the rounds take every
    /// slot.
    #[test]
    fn cell_visits_spread_over_the_rounds() {
        let taken = |rounds: usize, visits: usize| -> Vec<usize> {
            (0..rounds)
                .filter(|&slot| cell_wants_sample(slot, rounds, visits))
                .collect()
        };
        assert_eq!(taken(12, 4), vec![0, 3, 6, 9]);
        assert_eq!(taken(12, 5), vec![0, 3, 5, 8, 10]);
        assert_eq!(taken(12, 12).len(), 12);
        assert_eq!(taken(12, 40).len(), 12);
        assert_eq!(taken(7, 1), vec![0]);
        for slot in 0..36 {
            assert_eq!(
                cell_wants_sample(slot, 12, 5),
                cell_wants_sample(slot % 12, 12, 5),
                "slot {slot} wraps"
            );
        }
    }

    /// The sizes cache's context names the power source and mode, and
    /// leaves the battery's charge out: it changes by the minute and sizes
    /// nothing.
    #[test]
    fn the_sizes_context_ignores_the_batterys_charge() {
        let roster = Roster::new(
            vec![Algorithm::Sha256],
            true,
            None,
            Some(1),
            &TO_MEASURE_SPEED,
            None,
        );
        let mut machine = machine_metadata();
        let power = |percent| {
            Some(Power {
                on_battery: true,
                battery_percent: percent,
                mode: Some("Low Power Mode".to_owned()),
                low_power: true,
            })
        };
        machine.power = [power(Some(80)), None];
        let at_80 = sizes_context(&roster, &machine, None);
        machine.power = [power(Some(35)), None];
        let at_35 = sizes_context(&roster, &machine, None);
        assert_eq!(at_80, at_35, "the charge sizes nothing");
        let power_line = |context: &[(String, String)]| {
            context
                .iter()
                .find(|(k, _)| k == "power")
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(power_line(&at_80), "battery power, Low Power Mode");
        machine.power = [
            power(Some(80)).map(|p| Power {
                on_battery: false,
                ..p
            }),
            None,
        ];
        assert_ne!(
            power_line(&sizes_context(&roster, &machine, None)),
            power_line(&at_80),
            "the source sizes"
        );
    }

    #[test]
    fn instructions_moved_names_cells_beyond_the_margin_in_every_pair() {
        let ratios: std::collections::BTreeMap<String, Vec<u64>> = [
            (
                "blake3-servil-st|solo|OneMessage|3839 B",
                vec![1012, 1011, 1013, 1012],
            ),
            (
                "blake3-servil-st|solo|OneMessage|64 B",
                vec![990, 991, 989, 990],
            ),
            (
                "blake3-servil-st|solo|OneMessage|1 KiB",
                vec![1005, 1005, 1005, 1005],
            ),
            (
                "blake3-servil-st|solo|OneMessage|2 KiB",
                vec![1000, 1000, 1000, 1300],
            ),
            // Beyond in the median, at the margin in one pair: a count that
            // moves with the layout, no change in the code.
            (
                "blake3-servil-st|solo|InterleavedTurn|256",
                vec![1012, 1011, 1004, 1013],
            ),
            (
                "blake3-servil-st|solo|OneMessage|4 KiB",
                vec![1012, 1012, 1012],
            ),
            // A pool's caller spins while it waits: its count is no evidence.
            (
                "blake3-servil-mt|solo|OneMessage|1 MiB",
                vec![1030, 1040, 1050, 1060],
            ),
            (
                "blake3-servil-st|shared-one-cache|LentMessages|64 B",
                vec![1030, 1040, 1050, 1060],
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        assert_eq!(
            instructions_moved(&ratios, 4),
            vec![
                ("blake3-servil-st|solo|OneMessage|3839 B".to_owned(), 1012),
                ("blake3-servil-st|solo|OneMessage|64 B".to_owned(), 990),
            ],
            "beyond the margin the same way in every pair, the larger move first; st solo cells alone"
        );
        assert!(instructions_moved(&Default::default(), 4).is_empty());
    }

    /// A cell's instructions a unit sum both kinds of core over every
    /// sample, over the units; a cell without counts has none; the wall
    /// mean is the samples' regardless.
    #[test]
    fn cell_instructions_sum_both_kinds_over_the_units() {
        let dir =
            std::env::temp_dir().join(format!("bench-cell-instructions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, instructions: u64, counts: bool| -> String {
            let core = if counts {
                (0..3)
                    .map(|i| {
                        // P and E both counted: the reader sums the kinds.
                        format!(
                            "{}:{}:{}:7:1:{}:{}:{}:2:1",
                            1000 + i,
                            4000 + 4 * i,
                            instructions + i,
                            500 + i,
                            1000 + 2 * i,
                            instructions / 10
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                "-".to_owned()
            };
            let text = format!(
                "{SAMPLES_VERSION}\n# bench-hashes version: test\n# load: quiet\n# power: mains power\n{SAMPLES_COLUMNS}\nblake3-servil-st\tsolo\tOneMessage\t64 B\tB\t1000/64,1001/64,1002/64\t0,1,2\t-\t-\t{core}\t3,3,3\n"
            );
            let path = dir.join(name);
            std::fs::write(&path, text).unwrap();
            path.to_str().unwrap().to_owned()
        };
        let key = "blake3-servil-st|solo|OneMessage|64 B";
        let (old, new) = (
            read_samples(&write("old.tsv", 20_000, true)),
            read_samples(&write("new.tsv", 21_000, true)),
        );
        let (o, n) = (cell_instructions(&old)[key], cell_instructions(&new)[key]);
        assert!(
            n > o && (49..=51).contains(&((n - o) * 1000 / o)),
            "5% more instructions a unit: {o} to {n}"
        );
        // P (20000 + 20001 + 20002) + E (3 x 2000), over 192 B.
        assert_eq!(o >> 64, (60_003 + 6_000) / 192);
        assert_eq!(
            cell_means(old)[0].1,
            cell_means(new)[0].1,
            "the same wall time"
        );
        let none = read_samples(&write("none.tsv", 0, false));
        assert!(cell_instructions(&none).is_empty(), "no counts: none");
        assert_eq!(cell_means(none).len(), 1);
        // A CPU-time column of zeros (a platform that counts none writes
        // "-", but a zero is the same absence) reads as none.
        let zero_cpu = format!(
            "{SAMPLES_VERSION}\n# bench-hashes version: test\n# load: quiet\n# power: mains power\n{SAMPLES_COLUMNS}\nblake3-servil-st\tsolo\tOneMessage\t64 B\tB\t1000/64,1001/64,1002/64\t0,1,2\t0,0,0\t-\t-\t3,3,3\n"
        );
        let path = dir.join("zero-cpu.tsv");
        std::fs::write(&path, zero_cpu).unwrap();
        let (_, _, cpu, energy) = cell_means(read_samples(path.to_str().unwrap())).remove(0);
        assert_eq!((cpu, energy), (None, None), "zero CPU time a unit: none");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn samples_file_places_each_sample_in_the_load_windows() {
        // A machine with one cache (the i7's table), so the shared rows read
        // the same on every machine CI runs (a Mac offers no pair).
        set_topology_for_tests(Topology::from_tables(
            (0..20)
                .map(|c| if c < 16 { c / 2 * 2 } else { c })
                .collect(),
            vec![0; 20],
            vec![0; 20],
            (0..16).collect(),
        ));
        blake3_servil::initialize();
        let roster = Roster::new(
            vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt],
            true,
            Some(vec![
                point("64 B", UseCase::OneMessage),
                point("128 B", UseCase::OneMessage),
                point("64 B", UseCase::LentMessages),
                point("256 B", UseCase::LentMessages),
            ]),
            Some(3),
            &TO_CHECK_FOR_REGRESSIONS,
            None,
        );
        let (_, samples) = run(&roster, 3, |_, _, r| 10_000 + r as u64);
        let mut machine = machine_metadata();
        machine.load = vec![
            clocks::load::Window {
                start_ns: 0,
                end_ns: 1_500_000,
                other_milli_cpus: 1200,
                steal_milli_cpus: 0,
            },
            clocks::load::Window {
                start_ns: 1_500_000,
                end_ns: 3_000_000,
                other_milli_cpus: 100,
                steal_milli_cpus: 7,
            },
        ];
        let tsv = generate_samples_tsv(&roster, &samples, &machine, "test");
        assert!(tsv.starts_with("# bench-hashes samples v10\n"));
        // The main run's own file carries the shared provenance first.
        assert_eq!(
            &harness_tests::header_keys(&tsv)[..harness_tests::PROVENANCE.len()],
            harness_tests::PROVENANCE,
            "{tsv}"
        );
        assert!(tsv.contains("\n# load: busy: "), "{tsv}");
        assert!(tsv.contains("\n# load windows (start ms-end ms:other milli-CPUs:steal milli-CPUs): 0-1:1200:0,1-3:100:7\n"), "{tsv}");
        assert!(tsv.contains("\ncontender\tscenario\tuse_case\tpoint\tunit\tns/units\tstart ms\tcpu ns\tenergy nJ\tcore counts\tcpu\n"));
        assert!(tsv.contains("\nblake3-servil-st\tsolo\tOneMessage\t64 B\tB\t10000/1,10001/1,10002/1\t0,1,2\t20000,20002,20004\t30000,30003,30006\t10000:40000:50000:60000:70000:0:0:0:0:0,10001:40004:50005:60006:70007:0:0:0:0:0,10002:40008:50010:60012:70014:0:0:0:0:0\t3,3,3\n"), "{tsv}");
        // The multithreaded contender's shared rows, the two copies in turn,
        // each sample's CPU with it; a single-threaded contender has none.
        assert!(tsv.contains("\nblake3-servil-mt\tshared-one-cache\tLentMessages\t64 B\tB\t10000/1,10000/1,10001/1,10001/1,10002/1,10002/1\t10,11,11,12,12,13\t-\t-\t-\t2,5,2,5,2,5\n"), "{tsv}");
        // The second placement where the machine has two caches alone.
        assert_eq!(
            tsv.contains("\tshared-two-caches\t"),
            topology().pair(Placement::TwoCaches).is_some(),
            "{tsv}"
        );
        assert!(
            !tsv.contains("\nblake3-servil-st\tshared")
                && !tsv.contains("\tshared-one-cache\tOneMessage\t"),
            "the single-threaded contender, and every call after a gap, run alone: {tsv}"
        );
    }

    /// Rows whose means agree in every pair are one measurement: judged
    /// once, each given its verdict, the first row holding it named.
    #[test]
    fn identical_measurements_are_judged_once() {
        let q = |x: u128| x << 64;
        let slower: Vec<(u128, u128)> = [110, 111, 109, 112]
            .iter()
            .map(|&n| (q(100), q(n)))
            .collect();
        let level: Vec<(u128, u128)> = [100, 101, 99, 100]
            .iter()
            .map(|&n| (q(100), q(n)))
            .collect();
        let (judged, first) = judge_once(&[slower.clone(), level, slower]);
        assert_eq!(first, vec![0, 1, 0]);
        assert_eq!(judged[0], judged[2]);
        assert_eq!(judged[0].verdict, clocks::summary::Verdict::Slower);
        assert_eq!(judged[1].verdict, clocks::summary::Verdict::Level);
    }

    /// The pairs a placement takes on three machines (#116): the i7-12700K
    /// (8 P cores of 2 threads, 4 E cores, one L3: CPUs 0 and 2, and no
    /// two-caches pair), an EPYC 9255 cut to two L3 domains of 6 cores
    /// with SMT (0 and 1 under one, 0 and 6 under two), and one core alone
    /// (no pair at all).
    /// A comparison's pairs pin their measuring thread one CPU a core, the
    /// fast cores alone, alternating the core's parity (its place among
    /// the cores) and turning through the cache domains (bench-hashes #69),
    /// on the tables above; within the CPUs this process may use; and
    /// nowhere where the machine names no cores (macOS).
    #[test]
    fn a_comparisons_pairs_alternate_core_parity_over_the_cache_domains() {
        let epyc = Topology::from_tables(
            (0..24).map(|c| c % 12).collect(),
            (0..24).map(|c| (c % 12) / 6 * 6).collect(),
            vec![0; 24],
            (0..24).collect(),
        );
        let all = |_: usize| true;
        let seq = |t: &Topology, allowed: &dyn Fn(usize) -> bool, n: usize| -> Vec<Option<usize>> {
            (0..n).map(|pair| t.measuring_cpu(pair, allowed)).collect()
        };
        // Even cores in one domain, odd there, then the other domain's.
        assert_eq!(
            seq(&epyc, &all, 14),
            [0, 1, 6, 7, 2, 3, 8, 9, 4, 5, 10, 11, 0, 1].map(Some)
        );
        // Its hardware threads' second CPUs alone: each core by them.
        let siblings = |cpu: usize| cpu >= 12;
        assert_eq!(seq(&epyc, &siblings, 4), [12, 13, 18, 19].map(Some));
        // The i7: its P cores (CPUs 0-15, two a core), never an E core.
        let i7 = Topology::from_tables(
            (0..20)
                .map(|c| if c < 16 { c / 2 * 2 } else { c })
                .collect(),
            vec![0; 20],
            vec![0; 20],
            (0..16).collect(),
        );
        assert_eq!(seq(&i7, &all, 9), [0, 2, 4, 6, 8, 10, 12, 14, 0].map(Some));
        // One core: both parities take it.
        let one_core = Topology::from_tables(vec![0, 0], vec![0, 0], vec![0, 0], vec![0, 1]);
        assert_eq!(seq(&one_core, &all, 2), [Some(0), Some(0)]);
        let mac = Topology::from_tables(Vec::new(), Vec::new(), Vec::new(), Vec::new());
        assert_eq!(seq(&mac, &all, 2), [None, None]);
    }

    #[test]
    fn placements_take_two_cores_never_a_hardware_thread_pair() {
        let i7 = Topology::from_tables(
            (0..20)
                .map(|c| if c < 16 { c / 2 * 2 } else { c })
                .collect(),
            vec![0; 20],
            vec![0; 20],
            (0..16).collect(),
        );
        assert!(!i7.several_nodes());
        assert_eq!(i7.pair(Placement::OneCache), Some([0, 2]));
        assert_eq!(i7.pair(Placement::TwoCaches), None);
        assert_eq!(i7.describe(), "one cache: CPUs 0 and 2; two caches: none");
        // 24 CPUs: cores 0-11, thread c and c + 12 a core; L3s of cores 0-5 and 6-11.
        let epyc = Topology::from_tables(
            (0..24).map(|c| c % 12).collect(),
            (0..24).map(|c| (c % 12) / 6 * 6).collect(),
            vec![0; 24],
            (0..24).collect(),
        );
        assert_eq!(epyc.pair(Placement::OneCache), Some([0, 1]));
        assert_eq!(epyc.pair(Placement::TwoCaches), Some([0, 6]));
        let one_core = Topology::from_tables(vec![0, 0], vec![0, 0], vec![0, 0], vec![0, 1]);
        assert_eq!(one_core.pair(Placement::OneCache), None);
        // Where the machine offers no pair (one core here; macOS, whose
        // table is empty), the OS places the copies, a layer of its own;
        // where it offers one, that layer is never measured.
        let mac = Topology::from_tables(Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for no_pair in [&one_core, &mac] {
            assert_eq!(no_pair.cpus(Placement::Os), Some(None));
            assert_eq!(no_pair.cpus(Placement::OneCache), None);
            assert_eq!(no_pair.cpus(Placement::TwoCaches), None);
        }
        assert_eq!(
            mac.describe(),
            "one cache: none; two caches: none; the OS places the copies"
        );
        for pairs in [&i7, &epyc] {
            assert_eq!(pairs.cpus(Placement::Os), None);
        }
        assert_eq!(i7.cpus(Placement::OneCache), Some(Some([0, 2])));
        // Two sockets of two cores, a memory node each: the two-caches pair
        // crosses nodes, and the line says the inputs follow.
        let xeon = Topology::from_tables(
            vec![0, 1, 2, 3],
            vec![0, 0, 2, 2],
            vec![0, 0, 1, 1],
            vec![0, 1, 2, 3],
        );
        assert_eq!(xeon.pair(Placement::TwoCaches), Some([0, 2]));
        assert!(xeon.several_nodes());
        assert_eq!(
            xeon.describe(),
            "one cache: CPUs 0 and 1; two caches: CPUs 0 and 2; each copy's input on its CPU's memory node"
        );
    }

    /// The idle control on a faked cpuidle tree: every state but POLL and
    /// C1 switched off while the guard lives, every file's value back when
    /// it drops; none where a file is read-only, or no state is there to
    /// switch off.
    #[test]
    fn idle_control_switches_the_deep_states_off_and_back() {
        let root = std::env::temp_dir().join(format!("bench-cpuidle-{}", std::process::id()));
        let make = |cpu: usize, states: &[(&str, &str)]| {
            for (i, (name, disable)) in states.iter().enumerate() {
                let dir = root.join(format!("cpu{cpu}/cpuidle/state{i}"));
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join("name"), format!("{name}\n")).unwrap();
                fs::write(dir.join("disable"), format!("{disable}\n")).unwrap();
            }
        };
        // The i7's ladder on two CPUs (no plain C1), C6 already off on cpu1.
        make(0, &[("POLL", "0"), ("C1E", "0"), ("C6", "0"), ("C8", "0")]);
        make(1, &[("POLL", "0"), ("C1E", "0"), ("C6", "1"), ("C8", "0")]);
        fs::create_dir_all(root.join("cpufreq")).unwrap(); // a sibling that is no CPU
        let control = IdleControl::detect(&root).expect("writable states");
        assert_eq!(control.kept(), "POLL alone");
        let read = |cpu: usize, state: usize| {
            fs::read_to_string(root.join(format!("cpu{cpu}/cpuidle/state{state}/disable")))
                .unwrap()
                .trim()
                .to_owned()
        };
        {
            let _guard = control.apply();
            assert_eq!(
                [read(0, 0), read(0, 1), read(0, 2), read(0, 3)],
                ["0", "1", "1", "1"],
                "POLL kept, the rest off"
            );
            assert_eq!([read(1, 1), read(1, 2)], ["1", "1"]);
        }
        assert_eq!(
            [read(0, 0), read(0, 1), read(0, 2), read(0, 3)],
            ["0", "0", "0", "0"],
            "back as found"
        );
        assert_eq!(read(1, 2), "1", "cpu1's C6 was off before, and is again");
        // A machine with a plain C1 keeps it.
        let root2 = root.join("with-c1");
        for (i, name) in ["POLL", "C1", "C6"].iter().enumerate() {
            let dir = root2.join(format!("cpu0/cpuidle/state{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            fs::write(dir.join("disable"), "0\n").unwrap();
        }
        let control2 = IdleControl::detect(&root2).expect("writable states");
        assert_eq!(control2.kept(), "POLL and C1");
        {
            let _guard = control2.apply();
            let read2 = |state: usize| {
                fs::read_to_string(root2.join(format!("cpu0/cpuidle/state{state}/disable")))
                    .unwrap()
                    .trim()
                    .to_owned()
            };
            assert_eq!([read2(0), read2(1), read2(2)], ["0", "0", "1"]);
        }
        // A read-only disable file: no control (the bench lacks root). Root
        // writes a read-only file anyway, so under root (Sam's machines,
        // the switch's only user) the file stays writable and detect
        // rightly finds control; the check holds where the mode binds.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = root.join("cpu0/cpuidle/state2/disable");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o444)).unwrap();
            let mode_binds = fs::OpenOptions::new().write(true).open(&locked).is_err();
            assert_eq!(
                IdleControl::detect(&root).is_none(),
                mode_binds,
                "a file the bench cannot write: no control"
            );
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
        }
        // No cpuidle at all: no control.
        assert!(IdleControl::detect(&root.join("nowhere")).is_none());
        // Only the states the block keeps: nothing to switch off, no control.
        let shallow = root.join("shallow");
        for (i, name) in ["POLL", "C1"].iter().enumerate() {
            let dir = shallow.join(format!("cpu0/cpuidle/state{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            fs::write(dir.join("disable"), "0\n").unwrap();
        }
        assert!(
            IdleControl::detect(&shallow).is_none(),
            "POLL and C1 alone: nothing to switch off"
        );
        // The header's line, with the switch and without.
        assert_eq!(
            IdleControl::describe(Some(&control2)),
            "the controlled cells keep POLL and C1 and switch the other states off; run as root"
        );
        assert_eq!(
            IdleControl::describe(Some(&control)),
            "the controlled cells keep POLL alone and switch the other states off; run as root"
        );
        assert_eq!(
            IdleControl::describe(None),
            "none (no cpuidle states the bench can switch off: run as root for the controlled cells)"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// The effective idle states' line: each state's share of the pause's
    /// ticks, and the rest awake or unnamed; a sum past the pause (a
    /// rate read low) is the whole.
    #[test]
    fn effective_idle_states_line_shares_the_pause() {
        assert_eq!(
            describe_effective_idle_states(&["C1", "C6", "C7"], &[5, 95, 0], 100),
            "C1 5%, C6 95%, C7 0%, awake or unnamed 0%"
        );
        assert_eq!(
            describe_effective_idle_states(&["C1", "C6"], &[20, 30], 100),
            "C1 20%, C6 30%, awake or unnamed 50%"
        );
        assert_eq!(
            describe_effective_idle_states(&["C1"], &[120], 100),
            "C1 100%, awake or unnamed 0%"
        );
        // Rounded to the nearest percent: 1 of 200 is 1%, 199 of 200 is 100%.
        assert_eq!(
            describe_effective_idle_states(&["C1"], &[1], 200),
            "C1 1%, awake or unnamed 100%"
        );
    }

    /// The idle-states line from the three kinds of machine (#106): Linux
    /// with a driver (the i7), Linux with none (the EPYC 9255), and a
    /// system without cpuidle (the Mac).
    #[test]
    fn idle_states_line_from_each_kind_of_machine() {
        let i7 = |path: &str| -> Option<String> {
            let f = path.rsplit('/').next().unwrap();
            if path.ends_with("cpuidle/current_driver") {
                return Some("intel_idle\n".into());
            }
            if path.ends_with("cpuidle/current_governor") {
                return Some("menu\n".into());
            }
            let states = [
                ("POLL", "0", "0", "0"),
                ("C1E", "2", "4", "0"),
                ("C6", "220", "600", "1"),
            ];
            let n: usize = path
                .split("state")
                .nth(1)?
                .split('/')
                .next()?
                .parse()
                .ok()?;
            let (name, lat, res, dis) = states.get(n)?;
            Some(
                match f {
                    "name" => name,
                    "latency" => lat,
                    "residency" => res,
                    "disable" => dis,
                    _ => return None,
                }
                .to_string()
                    + "\n",
            )
        };
        assert_eq!(
            describe_idle_states(&i7),
            "driver intel_idle, governor menu; POLL latency 0 us residency 0 us, C1E latency 2 us residency 4 us, C6 latency 220 us residency 600 us disabled"
        );
        let epyc = |path: &str| -> Option<String> {
            path.ends_with("current_driver")
                .then(|| "none\n".to_owned())
        };
        assert_eq!(
            describe_idle_states(&epyc),
            "none listed (no cpuidle driver; the kernel's default idle)"
        );
        let mac = |_: &str| -> Option<String> { None };
        assert_eq!(describe_idle_states(&mac), "not reported by this OS");
    }

    /// The calibration cache gives back a file's sizes for its very
    /// context, and nothing for another; each context names its own file.
    #[test]
    fn cached_sizes_serve_only_their_context() {
        let roster = Roster::new(
            vec![Algorithm::Sha256],
            true,
            Some(vec![point("64 B", UseCase::OneMessage)]),
            Some(2),
            &TO_MEASURE_SPEED,
            None,
        );
        let context = vec![
            ("machine".to_owned(), "this one".to_owned()),
            ("power".to_owned(), "mains power".to_owned()),
        ];
        let mut other = context.clone();
        other[1].1 = "battery power".to_owned();
        assert_ne!(sizes_cache_path(&context), sizes_cache_path(&other));
        let path = std::env::temp_dir().join(format!(
            "bench-hashes-cache-test-{}.tsv",
            std::process::id()
        ));
        let mut iterations = vec![vec![0; POINT_COUNT]];
        iterations[0][point("64 B", UseCase::OneMessage)] = 7;
        Sizes::write(&path, &context, &roster, &iterations);
        let Some((Sizes::From(given), _)) = cached_sizes(&path, &context) else {
            panic!("its own context's sizes")
        };
        assert_eq!(
            given.get(&("sha256".to_owned(), point("64 B", UseCase::OneMessage))),
            Some(&7)
        );
        assert!(
            cached_sizes(&path, &other).is_none(),
            "another context's file serves nothing"
        );
        fs::remove_file(&path).unwrap();
    }

    /// An owned cell measured as its lent twin takes the twin's sample size
    /// (twins_take_their_sizes), which the sizes file then holds: servil st's
    /// continuous 4 KiB beside its lent 4 KiB; a cell with no twin in the run
    /// (servil mt's stream; servil st's continuous 64 B, its lent cell not
    /// measured) keeps its own.
    #[test]
    fn owned_cells_measured_as_twins_take_the_twins_sizes() {
        let mut points = vec![
            point("4 KiB", UseCase::ContinuousMessages),
            point("4 KiB", UseCase::LentMessages),
            point("64 B", UseCase::ContinuousMessages),
        ];
        points.sort_unstable();
        let roster = Roster::new(
            vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt],
            true,
            Some(points),
            Some(2),
            &TO_MEASURE_SPEED,
            None,
        );
        let (owned, lent, small) = (
            point("4 KiB", UseCase::ContinuousMessages),
            point("4 KiB", UseCase::LentMessages),
            point("64 B", UseCase::ContinuousMessages),
        );
        let mut sizes = vec![vec![1usize; POINT_COUNT]; 2];
        for (a, row) in sizes.iter_mut().enumerate() {
            row[owned] = 1;
            row[lent] = 1000 + a;
            row[small] = 77;
        }
        twins_take_their_sizes(&roster, &mut sizes);
        assert_eq!(
            sizes[0][owned], 1000,
            "servil st's owned 4 KiB is its lent twin: the twin's size"
        );
        assert_eq!(
            sizes[1][owned], 1,
            "servil mt's stream is measured on its own: its own size"
        );
        assert_eq!(sizes[0][small], 77, "no twin in the run: its own size");
        let path = std::env::temp_dir().join(format!("twin-sizes-{}.tsv", std::process::id()));
        Sizes::write(&path, &[], &roster, &sizes);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(
            text.lines()
                .any(|l| l == "blake3-servil-st\tcontinuous 4 KiB\t1000"),
            "the sizes file holds the twin's size:\n{text}"
        );
    }

    /// An owned cell whose calls are its lent twin's (owned_runs_lent) is
    /// measured once, as the twin: its samples are the twin's; BLAKE3-Servil's
    /// stream, whose owned calls differ, is measured on its own.
    #[test]
    fn owned_cells_with_lent_calls_take_the_lent_samples() {
        blake3_servil::initialize();
        let mut points = vec![
            point("4 KiB", UseCase::ContinuousMessages),
            point("4 KiB", UseCase::LentMessages),
        ];
        points.sort_unstable();
        let mut roster = Roster::new(
            vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt],
            true,
            Some(points),
            Some(2),
            &TO_MEASURE_SPEED,
            None,
        );
        blake3_servil::initialize_multithreaded(None);
        roster.train = Train {
            warm_ns: 1_000_000,
            samples: 2,
        };
        let (_, samples, _, _) = measure_all(&roster, &Sizes::Calibrate);
        let (owned, lent) = (
            point("4 KiB", UseCase::ContinuousMessages),
            point("4 KiB", UseCase::LentMessages),
        );
        let ns = |v: &Vec<Measured>| v.iter().map(|m| (m.ns, m.units)).collect::<Vec<_>>();
        assert_eq!(
            ns(&samples.solo[0][owned]),
            ns(&samples.solo[0][lent]),
            "servil st: the owned cell is the lent cell"
        );
        assert_ne!(
            ns(&samples.solo[1][owned]),
            ns(&samples.solo[1][lent]),
            "servil mt's stream is measured on its own"
        );
    }

    /// Every shared sample is taken under contention (#105): over many
    /// duo jobs of short calls, where one copy is often a call or more
    /// ahead, each copy's last sample began before the other's work
    /// ended. Control: with the finishing copy's tail loop removed from
    /// Duo::worker, this failed 10 of 10 runs (November 2).
    #[test]
    fn shared_samples_are_all_contended() {
        blake3_servil::initialize_multithreaded(None);
        let duo = Duo::new();
        // The first placement this machine measures: a pair, or the OS's.
        let cpus = Placement::ALL
            .into_iter()
            .find_map(|p| topology().cpus(p))
            .expect("some placement is measured");
        let point = POINTS[point("64 B", UseCase::ContinuousMessages)];
        let (input, other) = (make_input_seeded(64, 0), make_input_seeded(64, 1));
        let train = Train {
            warm_ns: 100_000,
            samples: 200,
        };
        for _ in 0..30 {
            let [first, second] = duo.run(
                Algorithm::Blake3ServilMt,
                &input,
                &other,
                point,
                1,
                train,
                cpus,
            );
            assert_eq!(first.samples.len(), 200);
            assert!(
                both_contended(&first, &second),
                "copy 0's last sample began at {}, copy 1's at {}; work ended at {} and {}",
                first.samples.last().unwrap().started_ns,
                second.samples.last().unwrap().started_ns,
                first.worked_until_ns,
                second.worked_until_ns
            );
        }
    }

    /// A machine that offers no pair (a Mac's empty table, set here on any
    /// machine) measures the shared cells of its multithreaded contenders
    /// in the OS's layer alone: servil mt's own stream, and official mt's
    /// owned stream as its lent twin. 0.20.2 stopped twice on a Mac here
    /// (runner jobs 1847 and 1848): the second input was sized for the
    /// one-cache layer, and the lent twin's samples copied two layers.
    #[test]
    fn a_machine_with_no_pair_measures_the_os_layer() {
        set_topology_for_tests(Topology::from_tables(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        blake3_servil::initialize_multithreaded(None);
        let algorithms = vec![Algorithm::Blake3ServilMt, Algorithm::Blake3Rayon];
        let mut points = vec![
            point("64 KiB", UseCase::ContinuousMessages),
            point("64 KiB", UseCase::LentMessages),
        ];
        points.sort_unstable();
        let mut roster = Roster::new(
            algorithms,
            true,
            Some(points),
            Some(2),
            &TO_MEASURE_SPEED,
            None,
        );
        roster.train = Train {
            warm_ns: 1_000_000,
            samples: 2,
        };
        let (results, samples, _, _) = measure_all(&roster, &Sizes::Calibrate);
        for a in 0..roster.len() {
            for p in [
                point("64 KiB", UseCase::ContinuousMessages),
                point("64 KiB", UseCase::LentMessages),
            ] {
                assert!(results[a][p].as_ref().unwrap().shared[Placement::Os.index()].is_some());
                assert_eq!(
                    samples.shared[Placement::Os.index()][a][p].len(),
                    2 * 2 * 2,
                    "two copies' trains of two, two rounds"
                );
                for pinned in [Placement::OneCache, Placement::TwoCaches] {
                    assert!(samples.shared[pinned.index()][a][p].is_empty());
                }
            }
        }
    }

    /// The controlled cells are measured where the bench holds the cpuidle
    /// switch (a faked tree here): a run without it leaves them out, and
    /// stops when --points names one; a run with it measures them with the
    /// deep states off during their rounds and every file back after.
    #[test]
    fn controlled_cells_need_the_cpuidle_switch() {
        let root = std::env::temp_dir().join(format!("bench-cpuidle-run-{}", std::process::id()));
        for (i, name) in ["POLL", "C1E", "C6"].iter().enumerate() {
            let dir = root.join(format!("cpu0/cpuidle/state{i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("name"), format!("{name}\n")).unwrap();
            // No newline: the guard's restore writes one, so the file's
            // bytes say whether the switch was thrown and put back.
            fs::write(dir.join("disable"), "0").unwrap();
        }
        let read = |state: usize| {
            fs::read_to_string(root.join(format!("cpu0/cpuidle/state{state}/disable"))).unwrap()
        };
        let plain = point("64 B", UseCase::OneMessage);
        let controlled = point("64 B", UseCase::ControlledIdleOneMessage);

        let without = Roster::new(
            vec![Algorithm::Sha256],
            false,
            None,
            Some(1),
            &TO_MEASURE_SPEED,
            None,
        );
        assert!(without.measures(plain));
        assert!(
            !without.measures(controlled),
            "no switch: no controlled cells"
        );
        let named = std::panic::catch_unwind(|| {
            Roster::new(
                vec![Algorithm::Sha256],
                true,
                Some(vec![controlled]),
                Some(1),
                &TO_MEASURE_SPEED,
                None,
            )
        });
        assert!(
            named.is_err(),
            "a controlled point named without the switch stops the run"
        );

        let control = IdleControl::detect(&root).expect("a writable tree");
        let mut roster = Roster::new(
            vec![Algorithm::Sha256],
            true,
            Some(vec![plain, controlled]),
            Some(1),
            &TO_MEASURE_SPEED,
            Some(control),
        );
        roster.train = Train {
            warm_ns: 1_000_000,
            samples: 2,
        };
        assert!(roster.measures(controlled));
        let (results, samples, _, _) = measure_all(&roster, &Sizes::Calibrate);
        assert!(results[0][controlled].is_some());
        assert_eq!(
            samples.solo[0][controlled].len(),
            1,
            "one sample in its one round"
        );
        assert_eq!(
            [read(0), read(1), read(2)],
            ["0\n", "0\n", "0\n"],
            "the switch was thrown for the controlled rounds and every state put back"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// A nonstop cell's visit is a train (Train): its warm-up, then its
    /// samples back to back, solo and in the two shared copies; a call
    /// after a gap stays one sample a visit.
    #[test]
    fn nonstop_visits_take_trains() {
        blake3_servil::initialize_multithreaded(None);
        let mut roster = Roster::new(
            vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt],
            true,
            Some({
                let mut points = vec![
                    point("64 KiB", UseCase::ContinuousMessages),
                    point("64 B", UseCase::OneMessage),
                ];
                points.sort_unstable();
                points
            }),
            Some(2),
            &TO_MEASURE_SPEED,
            None,
        );
        roster.train = Train {
            warm_ns: 2_000_000,
            samples: 3,
        };
        let (_, samples, _, _) = measure_all(&roster, &Sizes::Calibrate);
        let nonstop = point("64 KiB", UseCase::ContinuousMessages);
        let gap = point("64 B", UseCase::OneMessage);
        for a in 0..roster.len() {
            // Two rounds, a visit each: two trains of three.
            assert_eq!(samples.solo[a][nonstop].len(), 2 * 3);
            // Two copies' trains for the multithreaded contender, none for
            // the single-threaded one.
            let copies = if roster.algorithms[a].multithreaded() {
                2
            } else {
                0
            };
            for pl in Placement::ALL {
                let has = topology().cpus(pl).is_some();
                assert_eq!(
                    samples.shared[pl.index()][a][nonstop].len(),
                    if has { copies * 2 * 3 } else { 0 }
                );
            }
            assert_eq!(samples.solo[a][gap].len(), 2);
            // Within a train, each sample starts after the one before.
            let starts = &samples.solo_started_ns[a][nonstop];
            assert!(
                starts[..3].windows(2).all(|w| w[0] <= w[1])
                    && starts[3..].windows(2).all(|w| w[0] <= w[1])
            );
        }
    }

    /// The samples file reads back into the figures the report prints:
    /// every cell (read_samples, as `compare` reads it).
    #[test]
    fn samples_file_reads_back_into_the_reports_figures() {
        blake3_servil::initialize();
        let roster = Roster::new(
            vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring],
            true,
            Some(vec![
                point("64 B", UseCase::OneMessage),
                point("64 B", UseCase::LentMessages),
                point("16", UseCase::LentBatches),
            ]),
            Some(20),
            &TO_CHECK_FOR_REGRESSIONS,
            None,
        );
        // Contender 0's one-message cell has one slow outlier among 20
        // samples of 1000 ns a unit, which the mean counts in full
        // (independently fixed: (19 * 1000 + 1_000_000) / 20 = 50,950).
        // Contender 1's lent cells have a slower mode: 3 rounds in 10 twice
        // as slow.
        let (_, samples) = run(&roster, 20, |a, p, r| {
            if a == 0 && POINTS[p].use_case == UseCase::OneMessage {
                if r == 19 {
                    1_000_000
                } else {
                    1000
                }
            } else if a == 1 && POINTS[p].use_case != UseCase::OneMessage && r % 10 < 3 {
                20_000 + r as u64
            } else {
                10_000 + 7 * r as u64
            }
        });
        let tsv = generate_samples_tsv(&roster, &samples, &machine_metadata(), "test");
        let path =
            std::env::temp_dir().join(format!("bench-hashes-readback-{}.tsv", std::process::id()));
        fs::write(&path, &tsv).unwrap();
        let read = read_samples(path.to_str().unwrap());
        fs::remove_file(&path).unwrap();
        let figures = |cell: &[Measured]| Mean::of(cell).format_ns(1);
        let mut checked = 0;
        for (a, algorithm) in roster.algorithms.iter().enumerate() {
            for &p in &roster.points {
                let layers =
                    std::iter::once(("solo", &samples.solo[a][p], &samples.solo_cpu[a][p])).chain(
                        Placement::ALL.map(|pl| {
                            (
                                Scenario::Shared(pl).key(),
                                &samples.shared[pl.index()][a][p],
                                &samples.shared_cpu[pl.index()][a][p],
                            )
                        }),
                    );
                for (scenario, cell, cell_cpus) in layers {
                    if cell.is_empty() {
                        continue;
                    }
                    let key = format!(
                        "{}|{scenario}|{:?}|{}",
                        algorithm.key(),
                        POINTS[p].use_case,
                        POINTS[p].label
                    );
                    let (_, back) = read
                        .cells
                        .iter()
                        .find(|(k, _)| *k == key)
                        .unwrap_or_else(|| panic!("{key} read back"));
                    assert_eq!(figures(back), figures(cell), "{key}");
                    if a == 0 && POINTS[p].use_case == UseCase::OneMessage {
                        assert_eq!(
                            figures(back),
                            "50950.000",
                            "{key}'s independent anchor: every sample counted"
                        );
                    }
                    // CPU time: each solo sample's, none for shared ones.
                    let cpu = read
                        .cpu
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, ns)| ns.clone());
                    let wanted = (scenario == "solo").then(|| {
                        samples.solo_cpu_ns[a][p]
                            .iter()
                            .map(|ns| ns.unwrap())
                            .collect::<Vec<u64>>()
                    });
                    assert_eq!(cpu, wanted, "{key}'s CPU time");
                    // Energy alike.
                    let energy = read
                        .energy
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, nj)| nj.clone());
                    let wanted = (scenario == "solo").then(|| {
                        samples.solo_energy_nj[a][p]
                            .iter()
                            .map(|nj| nj.unwrap())
                            .collect::<Vec<u64>>()
                    });
                    assert_eq!(energy, wanted, "{key}'s energy");
                    // Core counts alike (the helper gives solo samples counts, shared none).
                    let cores = read
                        .cores
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, c)| c.clone());
                    let wanted = (scenario == "solo").then(|| {
                        samples.solo_counts[a][p]
                            .iter()
                            .map(|c| {
                                let c = c.unwrap();
                                [
                                    c.p.time_ns,
                                    c.p.cycles,
                                    c.p.instructions,
                                    c.p.cache_references,
                                    c.p.cache_misses,
                                    c.e.time_ns,
                                    c.e.cycles,
                                    c.e.instructions,
                                    c.e.cache_references,
                                    c.e.cache_misses,
                                ]
                            })
                            .collect::<Vec<[u64; LEVEL_FIELDS * 2]>>()
                    });
                    assert_eq!(cores, wanted, "{key}'s core counts");
                    // The CPU each sample ended on, solo and shared alike.
                    let cpus = read
                        .cpus
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, c)| c.clone());
                    let wanted: Vec<u32> = cell_cpus.iter().map(|c| c.unwrap()).collect();
                    assert_eq!(cpus, Some(wanted), "{key}'s CPUs");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, read.cells.len(), "every cell read back, none more");
    }

    #[test]
    fn use_case_axes_are_contiguous_and_cover_every_point() {
        assert_eq!(UseCase::OneMessage.points(), 0..INPUT_COUNT);
        assert_eq!(
            UseCase::ManyMessages.points(),
            INPUT_COUNT..INPUT_COUNT + BATCH_COUNT
        );
        assert_eq!(
            UseCase::ManyLongMessages.points(),
            INPUT_COUNT + BATCH_COUNT..INPUT_COUNT + BATCH_COUNT + LONG_BATCH_COUNT
        );
        let idle = INPUT_COUNT + BATCH_COUNT + LONG_BATCH_COUNT;
        let brief = idle + INPUT_COUNT + BATCH_COUNT;
        let controlled = brief + INPUT_COUNT + BATCH_COUNT;
        for (use_case, offset) in [
            (UseCase::IdleOneMessage, idle),
            (UseCase::IdleManyMessages, idle),
            (UseCase::BriefIdleOneMessage, brief),
            (UseCase::BriefIdleManyMessages, brief),
            (UseCase::ControlledIdleOneMessage, controlled),
            (UseCase::ControlledIdleManyMessages, controlled),
        ] {
            let (twin, mine) = (use_case.call().points(), use_case.points());
            assert_eq!(
                mine,
                // The idle axes follow the axes after other work and the
                // long batches, which have no idle twin; the brief ones
                // follow the idle ones.
                twin.start + offset..twin.end + offset,
                "each idle axis follows its twin's pattern"
            );
            for (a, b) in POINTS[twin].iter().zip(&POINTS[mine]) {
                assert_eq!(
                    (a.label, a.bytes, a.messages),
                    (b.label, b.bytes, b.messages),
                    "an idle axis repeats its twin's points"
                );
            }
        }
        let continuous = controlled + INPUT_COUNT + BATCH_COUNT;
        assert_eq!(
            UseCase::ContinuousMessages.points(),
            continuous..continuous + CONTINUOUS_MESSAGE_COUNT
        );
        assert_eq!(
            UseCase::ContinuousBatches.points(),
            continuous + CONTINUOUS_MESSAGE_COUNT
                ..continuous + CONTINUOUS_MESSAGE_COUNT + CONTINUOUS_BATCH_COUNT
        );
        let lent = continuous + CONTINUOUS_MESSAGE_COUNT + CONTINUOUS_BATCH_COUNT;
        assert_eq!(
            UseCase::LentMessages.points(),
            lent..lent + CONTINUOUS_MESSAGE_COUNT
        );
        let interleaved = lent + CONTINUOUS_MESSAGE_COUNT;
        assert_eq!(
            UseCase::Interleaved.points(),
            interleaved..interleaved + INTERLEAVED_COUNT
        );
        let collection = interleaved + INTERLEAVED_COUNT;
        assert_eq!(
            UseCase::Collection.points(),
            collection..collection + COLLECTION_COUNT
        );
        let lent_batches = collection + COLLECTION_COUNT;
        assert_eq!(
            UseCase::LentBatches.points(),
            lent_batches..lent_batches + CONTINUOUS_BATCH_COUNT
        );
        let continuous_collection = lent_batches + CONTINUOUS_BATCH_COUNT;
        assert_eq!(
            UseCase::ContinuousCollection.points(),
            continuous_collection..continuous_collection + COLLECTION_COUNT
        );
        let merkle = continuous_collection + COLLECTION_COUNT;
        assert_eq!(UseCase::MerkleTree.points(), merkle..merkle + MERKLE_COUNT);
        let turn = merkle + MERKLE_COUNT;
        assert_eq!(
            UseCase::InterleavedTurn.points(),
            turn..turn + INTERLEAVED_COUNT
        );
        assert_eq!(
            UseCase::IdleInterleavedTurn.points(),
            turn + INTERLEAVED_COUNT..turn + 2 * INTERLEAVED_COUNT
        );
        assert_eq!(
            UseCase::BriefIdleInterleavedTurn.points(),
            turn + 2 * INTERLEAVED_COUNT..turn + 3 * INTERLEAVED_COUNT
        );
        assert_eq!(
            UseCase::ControlledIdleInterleavedTurn.points(),
            turn + 3 * INTERLEAVED_COUNT..POINT_COUNT
        );
        for (k, point) in POINTS[UseCase::ContinuousCollection.points()]
            .iter()
            .enumerate()
        {
            assert_eq!(
                (point.label, point.bytes),
                (
                    POINTS[UseCase::Collection.points()][k].label,
                    POINTS[UseCase::Collection.points()][k].bytes
                ),
                "the pipelined collections are the collections"
            );
        }
        for (k, point) in POINTS[UseCase::Collection.points()].iter().enumerate() {
            assert_eq!(
                (point.label, point.messages),
                (COLLECTIONS[k].0, 2048),
                "each collection is 2048 items"
            );
        }
        let covered: Vec<_> = UseCase::ALL.into_iter().flat_map(UseCase::points).collect();
        assert_eq!(covered, (0..POINT_COUNT).collect::<Vec<_>>());
        for (owned, lent) in POINTS[UseCase::ContinuousMessages.points()]
            .iter()
            .zip(&POINTS[UseCase::LentMessages.points()])
        {
            assert_eq!((owned.label, owned.bytes), (lent.label, lent.bytes));
        }
        let many = POINTS[UseCase::Interleaved.points()][0];
        assert!(
            many.bytes == INTERLEAVED.chunk && many.label == "256" && INTERLEAVED.open == 256,
            "many messages at once: 256 messages open, a unit of INTERLEAVED.chunk bytes"
        );
        assert_eq!(
            POINTS[UseCase::ContinuousMessages.points()]
                .iter()
                .map(|p| p.bytes)
                .collect::<Vec<_>>(),
            [64, 128, 256, 512, 1024]
                .into_iter()
                .chain((3..11).map(|k| 64 << (2 * k)))
                .collect::<Vec<_>>(),
            "the continuous messages: every member kernel's length to 1 KiB, then factors of four"
        );
        for (k, point) in POINTS[UseCase::ContinuousBatches.points()]
            .iter()
            .enumerate()
        {
            assert_eq!(
                point.messages,
                16 << (2 * k),
                "the continuous batches go up by factors of four from 16"
            );
        }
        for use_case in [UseCase::ManyMessages, UseCase::ContinuousBatches] {
            assert!(POINTS[use_case.points()]
                .iter()
                .all(|p| p.bytes == p.messages * MESSAGE_LEN));
        }
    }

    #[test]
    fn batch_observes_every_digest_and_matches_empty_vectors() {
        for (algorithm, expected) in [
            (
                Algorithm::Blake3,
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                Algorithm::Sha256,
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                Algorithm::Sha1Dc,
                "da39a3ee5e6b4b0d3255bfef95601890afd80709",
            ),
            (
                Algorithm::Sha3_256,
                "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a",
            ),
        ] {
            let mut calls = 0;
            hash_batch(algorithm, &[], Point::one("", 0), 3, |digest| {
                let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                assert_eq!(hex, expected);
                calls += 1;
            });
            assert_eq!(calls, 3);
        }
    }

    /// Every contender taking part hands `consume` a digest for every
    /// message or batch on the continuous axes, BLAKE3-Servil's queues
    /// included (one buffer per message, pieces, and batches), with fewer,
    /// as many, and more messages or batches than it keeps in flight. What
    /// the digests are is the contenders' tests' business (AGENTS.md,
    /// "Correctness tests").
    /// Every input in blocks (UseCase::in_blocks), batch or message,
    /// reaches a contender at a 64-byte boundary, from a source 16 bytes
    /// into a line as glibc places one: the kept buffers of the after-gap,
    /// continuous, lent and Merkle cells realign it, and the door
    /// (assert_blocks_aligned) would stop the run otherwise.
    #[test]
    fn every_batch_reaches_the_contenders_on_a_64_byte_boundary() {
        blake3_servil::initialize_multithreaded(None);
        let room = make_input(64 * 256 + 128);
        let off = 16 + room.as_ptr().align_offset(64);
        let source = |len: usize| &room[off..][..len];
        assert_eq!(source(1).as_ptr() as usize % 64, 16);
        let cases = [
            Point::many("", 16),
            Point::lent_batch("", 16),
            Point::continuous_batch("", 16),
            Point::merkle("", 64),
            Point::one("", 1000),
            Point::one("", 1000).idle(),
            Point::lent("", 1000),
            Point::continuous("", 1000),
        ];
        for algorithm in Algorithm::ALL
            .into_iter()
            .filter(|algorithm| algorithm.availability().is_ok())
        {
            for point in cases {
                if algorithm.takes_part(point.use_case) {
                    take_sample(algorithm, source(input_len(point)), point, 2);
                }
            }
        }
    }

    /// The door's control: an input in blocks 16 bytes into a line stops
    /// the run, a batch and a message alike.
    #[test]
    #[should_panic(expected = "an input in blocks starts at a 64-byte boundary")]
    fn a_batch_off_its_boundary_stops_the_run() {
        let (copy, at) = block_copy(&make_input(16 * MESSAGE_LEN + 16));
        hash_in_memory(
            Algorithm::Sha256,
            &copy[at + 16..][..16 * MESSAGE_LEN],
            Point::many("", 16),
            1,
            |_| {},
        );
    }

    #[test]
    #[should_panic(expected = "an input in blocks starts at a 64-byte boundary")]
    fn a_message_off_its_boundary_stops_the_run() {
        let (copy, at) = block_copy(&make_input(1000 + 16));
        hash_in_memory(
            Algorithm::Sha256,
            &copy[at + 16..][..1000],
            Point::one("", 1000),
            1,
            |_| {},
        );
    }

    #[test]
    fn continuous_use_cases_observe_every_digest() {
        blake3_servil::initialize_multithreaded(None);
        let long = make_input(PIECE_LEN * 3 + 1000);
        let batch = make_input(16 * MESSAGE_LEN);
        let cases = [
            (Point::lent("", 1000), make_input(1000)),
            (Point::lent("", long.len()), long.clone()),
            (Point::lent_batch("", 16), batch.clone()),
            (Point::continuous("", 1000), make_input(1000)),
            (Point::continuous("", long.len()), long),
            (Point::continuous_batch("", 16), batch),
        ];
        for algorithm in Algorithm::ALL
            .into_iter()
            .filter(|algorithm| algorithm.availability().is_ok())
        {
            for (point, input) in &cases {
                if !algorithm.takes_part(point.use_case) {
                    continue;
                }
                let mut once = 0;
                hash_batch(algorithm, input, *point, 1, |_| once += 1);
                assert!(once >= 1, "{} {:?}", algorithm.key(), point.use_case);
                let count = in_flight(if point.use_case.batch() {
                    input.len()
                } else {
                    input.len().min(PIECE_LEN)
                });
                for iterations in [1, count, 3 * count + 1] {
                    let mut seen = 0;
                    hash_batch(algorithm, input, *point, iterations, |_| seen += 1);
                    assert_eq!(
                        seen,
                        iterations * once,
                        "{} {:?}",
                        algorithm.key(),
                        point.use_case
                    );
                }
            }
        }
    }

    /// A sample of a synchronous use case makes its calls after the gap and
    /// times only the calls: at least a gap per call in wall time, well
    /// under that in the sample.
    #[test]
    fn samples_after_the_gap_leave_the_gaps_out() {
        blake3_servil::initialize();
        let point = Point::one("", 64);
        let input = make_input(64);
        let started = clocks::now();
        let sample = take_sample(Algorithm::Blake3ServilSt, &input, point, 3);
        assert!(clocks::since_ns(started) >= 3 * GAP_NS);
        assert!(sample.elapsed_ns < GAP_NS, "{}", sample.elapsed_ns);
    }
}
