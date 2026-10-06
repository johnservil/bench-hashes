
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::io::{IsTerminal, Write as _};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use sysinfo::System;

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
 * alike: at least STEADY_SAMPLES, rounded up to whole Williams cycles.
 * Every participating contender at a point samples the same visits.
 * Thinning long contenders separately aliased their orders and compared
 * different moments. Complete cycles cost more time for long hashes.
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
 * polling (the fork's, 200 us), so each call meets them asleep.
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
const STEADY_SAMPLES: usize = 12;

/// Points on the one-message axis, and on each many-messages axis.
const INPUT_COUNT: usize = 27;
const BATCH_COUNT: usize = 24;
/// Points on the continuous axes: message lengths, and messages per batch.
const CONTINUOUS_MESSAGE_COUNT: usize = 11;
const CONTINUOUS_BATCH_COUNT: usize = 7;
/// Many messages at once is measured at one point (FROZEN.md).
const INTERLEAVED_COUNT: usize = 1;
/// Collections: git's objects, and the files of Nix store paths (FROZEN.md).
const COLLECTION_COUNT: usize = 2;
/// A message's outboard, at two lengths (FROZEN.md).
const OUTBOARD_COUNT: usize = 2;
const VERIFY_COUNT: usize = 2;
const POINT_COUNT: usize = 2 * (INPUT_COUNT + BATCH_COUNT) + 2 * CONTINUOUS_MESSAGE_COUNT + INTERLEAVED_COUNT + COLLECTION_COUNT + OUTBOARD_COUNT + VERIFY_COUNT + 2 * CONTINUOUS_BATCH_COUNT;
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
 * servil fork's update_each and finalize_each) makes one of each per
 * turn, every other one update per piece and one finish per message. The
 * schedule carries over from sample to sample, so samples see the steady
 * state. A sample's unit of work is `chunk` bytes of pieces.
 */
/*
 * A collection of items of different lengths (UseCase::Collection; Zooko,
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
    ("git objects", &[(1, 24), (13, 48), (12, 96), (42, 192), (182, 384), (237, 768), (135, 1536), (141, 3072), (173, 6144), (447, 12288), (321, 24576), (231, 49152), (84, 98304), (22, 196608), (4, 393216), (3, 786432)]),
    ("Nix files", &[(6, 1), (1, 6), (1, 12), (4, 24), (11, 48), (44, 96), (139, 192), (133, 384), (439, 768), (428, 1536), (264, 3072), (179, 6144), (130, 12288), (96, 24576), (70, 49152), (46, 98304), (31, 196608), (17, 393216), (9, 786432)]),
];

/// A collection's items as (offset, length) in one buffer, in the stride's order.
fn collection_items(classes: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let lengths: Vec<usize> = classes.iter().flat_map(|&(n, len)| std::iter::repeat_n(len, n)).collect();
    let count = lengths.len();
    assert!(count % 389 != 0 && count.is_power_of_two(), "a stride sharing no factor with the count takes every item once");
    let mut offset = 0;
    (0..count)
        .map(|j| {
            let len = lengths[j * 389 % count];
            offset += len;
            (offset - len, len)
        })
        .collect()
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
}

const INTERLEAVED: InterleavedSpec = InterleavedSpec {
    open: 256,
    pieces: &[1448, 1448, 4096, 1448, 16 * 1024],
    messages: &[64, 1000, 4470, 16 * 1024, 100_000, 1024 * 1024, 16 * 1024 * 1024],
    chunk: 1024 * 1024,
};
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

const BLAKE3_SOURCE_INFO: &str = env!("BLAKE3_SOURCE_INFO");
const SHA2_SOURCE_INFO: &str = env!("SHA2_SOURCE_INFO");
const RING_SOURCE_INFO: &str = env!("RING_SOURCE_INFO");
const SHA1_CHECKED_SOURCE_INFO: &str = env!("SHA1_CHECKED_SOURCE_INFO");
const SHA3_SOURCE_INFO: &str = env!("SHA3_SOURCE_INFO");
const BLAKE3_SERVIL_SOURCE_INFO: &str = env!("BLAKE3_SERVIL_SOURCE_INFO");

/*
 * Every power of two from 64 B to 128 MiB, plus 3 KiB and 3 MiB. Between
 * 64 B and 1 KiB BLAKE3 is inside one chunk; from 2 KiB to 16 KiB its SIMD
 * paths fill up (4-way NEON at 4 KiB, a sixteen-lane SME2 group at 16
 * KiB); above that the bulk rate settles. 3 KiB is where the fork's integer
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
    Point::one("64 B", 64).idle(),
    Point::one("128 B", 128).idle(),
    Point::one("256 B", 256).idle(),
    Point::one("512 B", 512).idle(),
    Point::one("1 KiB", 1024).idle(),
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
    Point::continuous("64 B", 64),
    Point::continuous("256 B", 256),
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
    Point::lent("256 B", 256),
    Point::lent("1 KiB", 1024),
    Point::lent("4 KiB", 4 * 1024),
    Point::lent("16 KiB", 16 * 1024),
    Point::lent("64 KiB", 64 * 1024),
    Point::lent("256 KiB", 256 * 1024),
    Point::lent("1 MiB", 1024 * 1024),
    Point::lent("4 MiB", 4 * 1024 * 1024),
    Point::lent("16 MiB", 16 * 1024 * 1024),
    Point::lent("64 MiB", 64 * 1024 * 1024),
    Point::interleaved("256 open", INTERLEAVED.chunk),
    Point::collection(0),
    Point::collection(1),
    Point::outboard("1 MiB", 1024 * 1024),
    Point::outboard("64 MiB", 64 * 1024 * 1024),
    Point::verify("1 MiB", 1024 * 1024),
    Point::verify("64 MiB", 64 * 1024 * 1024),
    Point::lent_batch("16", 16),
    Point::lent_batch("64", 64),
    Point::lent_batch("256", 256),
    Point::lent_batch("1024", 1024),
    Point::lent_batch("4096", 4096),
    Point::lent_batch("16384", 16384),
    Point::lent_batch("65536", 65536),
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
    shared: Samples,
    /// Each sample's start on the load windows' scale, by contender and
    /// point. Shared starts belong to the individual copies, after solo.
    solo_started_ns: Vec<Vec<Vec<u64>>>,
    shared_started_ns: Vec<Vec<Vec<u64>>>,
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
    /// The same two calls, each after the program idled (the first
    /// two: each after other work; clocks::Gap).
    IdleOneMessage,
    IdleManyMessages,
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
    /// A collection's items, each hashed once, one after another, in
    /// memory (COLLECTIONS).
    Collection,
    /// Messages one after another, each written into a kept buffer, then
    /// hashed with its outboard (the tree's parent nodes above 16 KiB
    /// groups), as a server adds a file for verified streaming.
    Outboard,
    /// Messages one after another, each received as its encoding (its
    /// groups and parent nodes, as iroh-blobs sends them) in pieces read
    /// into a kept buffer, verified as they arrive, each verified group
    /// written into the message's buffer.
    Verify,
    LentBatches,
}

impl UseCase {
    const ALL: [UseCase; 12] = [UseCase::OneMessage, UseCase::ManyMessages, UseCase::IdleOneMessage,
        UseCase::IdleManyMessages, UseCase::ContinuousMessages, UseCase::ContinuousBatches,
        UseCase::LentMessages, UseCase::Interleaved, UseCase::Collection, UseCase::Outboard, UseCase::Verify, UseCase::LentBatches];

    /// Whether each call comes after a gap (the synchronous use cases:
    /// after other work, or after idling), or one follows another (the
    /// continuous ones).
    fn after_gap(self) -> bool {
        matches!(self.call(), Self::OneMessage | Self::ManyMessages)
    }

    /// How the program calls, as the graph's chips name it.
    fn pattern_key(self) -> &'static str {
        if self.idle() { "idle" } else if self.after_gap() { "busy" } else { "nonstop" }
    }

    /// Whether the program idles before each call.
    fn idle(self) -> bool {
        matches!(self, Self::IdleOneMessage | Self::IdleManyMessages)
    }

    /// The use case whose call this one makes: an idle use case makes its
    /// twin's (the same call, after another gap), every other its own.
    fn call(self) -> UseCase {
        match self {
            Self::IdleOneMessage => Self::OneMessage,
            Self::IdleManyMessages => Self::ManyMessages,
            other => other,
        }
    }

    /// Whether a call hashes a batch of messages.
    fn batch(self) -> bool {
        matches!(self.call(), Self::ManyMessages | Self::ContinuousBatches | Self::LentBatches)
    }

    /// Each message's length in a batch use case.
    fn message_len(self) -> usize {
        match self {
            Self::ManyMessages | Self::IdleManyMessages | Self::ContinuousBatches | Self::LentBatches => MESSAGE_LEN,
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::Interleaved | Self::Collection | Self::Outboard | Self::Verify => panic!("{self:?} hashes messages of their own lengths"),
        }
    }

    /// The contiguous run of POINTS on this use case's axis.
    fn points(self) -> std::ops::Range<usize> {
        let start = POINTS.iter().position(|point| point.use_case == self).expect("each use case has points");
        let end = POINTS.iter().rposition(|point| point.use_case == self).unwrap() + 1;
        assert!(POINTS[start..end].iter().all(|point| point.use_case == self), "a use case's points are contiguous");
        start..end
    }

    fn heading(self) -> &'static str {
        match self {
            Self::OneMessage => "A message in one buffer, after other work",
            Self::ManyMessages => "A batch of 64-byte messages, after other work",
            Self::IdleOneMessage => "A message in one buffer, after idling",
            Self::IdleManyMessages => "A batch of 64-byte messages, after idling",
            Self::ContinuousMessages => "Messages one after another, buffers owned",
            Self::ContinuousBatches => "Batches one after another, buffers owned",
            Self::LentMessages => "Messages one after another, buffers lent",
            Self::Interleaved => "Many messages at once, each arriving in pieces, buffers lent",
            Self::Collection => "A collection's items, each hashed once, in memory",
            Self::Outboard => "Messages with their outboards, for verified streaming, buffers lent",
            Self::Verify => "Messages received in their encodings, verified as they arrive",
            Self::LentBatches => "Batches one after another, buffers lent",
        }
    }

    /// The plot's name in a list of plots.
    fn short(self) -> &'static str {
        match self {
            Self::OneMessage => "one buffer, after other work",
            Self::ManyMessages => "a batch, after other work",
            Self::IdleOneMessage => "one buffer, after idling",
            Self::IdleManyMessages => "a batch, after idling",
            Self::ContinuousMessages => "messages, owned buffers",
            Self::ContinuousBatches => "batches, owned buffers",
            Self::LentMessages => "messages, lent buffers",
            Self::Interleaved => "many at once, lent buffers",
            Self::Collection => "a collection, in memory",
            Self::Outboard => "outboards, lent buffers",
            Self::Verify => "verified as received",
            Self::LentBatches => "batches, lent buffers",
        }
    }

    /// The x column's header in the text report.
    fn column(self) -> &'static str {
        match self {
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::Interleaved => "size",
            Self::Collection => "collection",
            Self::Outboard | Self::Verify => "size",
            Self::ManyMessages | Self::IdleManyMessages | Self::ContinuousBatches | Self::LentBatches => "messages",
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
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::Interleaved | Self::Collection | Self::Outboard | Self::Verify => point.bytes as u64 * iterations as u64,
            Self::ManyMessages | Self::IdleManyMessages | Self::ContinuousBatches | Self::LentBatches => point.messages as u64 * iterations as u64,
        }
    }

    /// The unit a sample is per, in the samples file.
    fn unit_key(self) -> &'static str {
        if self.batch() { "msg" } else { "B" }
    }

    fn time_unit(self) -> &'static str {
        if self.batch() { "ns/msg" } else { "ns/B" }
    }

    /// How the program calls, for a reader of the results.
    /// How the program calls, for the report's opening: the clause after
    /// the tables' names.
    fn pattern(self) -> &'static str {
        if self.idle() {
            "the program hashes, sleeps 1 ms, and hashes again, as a server handles a request, waits, and handles the next; the second call, with the write of its input, is timed"
        } else if self.after_gap() {
            "the program hashes, runs other code and reads 128 MiB of memory (at least 1 ms), and hashes again, as a program hashes between its other tasks; the second call, with the write of its input, is timed"
        } else {
            "the program hashes one input after another, as fast as it can, alone and with a second program doing the same at once"
        }
    }

    /// The prefix that names this use case's points on the command line
    /// ("interleaved 256 open"), empty for the first two.
    fn label_prefix(self) -> &'static str {
        match self {
            Self::OneMessage | Self::ManyMessages => "",
            Self::IdleOneMessage | Self::IdleManyMessages => "idle ",
            Self::ContinuousMessages => "continuous ",
            Self::ContinuousBatches => "continuous batch ",
            Self::LentMessages => "lent ",
            Self::Interleaved => "interleaved ",
            Self::Collection => "collection ",
            Self::Outboard => "outboard ",
            Self::Verify => "verify ",
            Self::LentBatches => "lent batch ",
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
        Self { label, bytes, messages: 1, use_case: UseCase::OneMessage }
    }

    /// The same point, its call made after idling.
    const fn idle(self) -> Self {
        let use_case = match self.use_case {
            UseCase::OneMessage => UseCase::IdleOneMessage,
            UseCase::ManyMessages => UseCase::IdleManyMessages,
            _ => panic!("only the calls after other work have idle twins"),
        };
        Self { use_case, ..self }
    }

    const fn continuous(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::ContinuousMessages }
    }

    const fn many(label: &'static str, messages: usize) -> Self {
        Self { label, bytes: messages * MESSAGE_LEN, messages, use_case: UseCase::ManyMessages }
    }

    const fn continuous_batch(label: &'static str, messages: usize) -> Self {
        Self { label, bytes: messages * MESSAGE_LEN, messages, use_case: UseCase::ContinuousBatches }
    }

    const fn lent(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::LentMessages }
    }

    const fn outboard(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::Outboard }
    }

    const fn verify(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::Verify }
    }

    const fn collection(index: usize) -> Self {
        let (label, classes) = COLLECTIONS[index];
        Self { label, bytes: collection_bytes(classes), messages: collection_count(classes), use_case: UseCase::Collection }
    }

    const fn interleaved(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::Interleaved }
    }

    const fn lent_batch(label: &'static str, messages: usize) -> Self {
        Self { label, bytes: messages * MESSAGE_LEN, messages, use_case: UseCase::LentBatches }
    }

    /// Whether a quick run measures this point: inputs below QUICK_BYTES,
    /// batches below QUICK_MESSAGES.
    fn quick(&self) -> bool {
        if self.use_case.batch() { self.messages < QUICK_MESSAGES } else { self.bytes < QUICK_BYTES }
    }
}

/*
 * A contender is one hash implementation under test: which crate (the
 * crates.io blake3, the servil fork, sha2, ...) in which mode
 * (single-threaded or multithreaded). Which kernel that implementation
 * runs at each input size is chosen at run time and reported by
 * detect_kernels. Adding a contender means a variant here, an entry in
 * ALL, a key, a name, a color, a provenance string, a kernel description,
 * and an arm in hash_batch; the harness handles selection, interleaving,
 * and reporting for any count.
 */
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Algorithm {
    Blake3,
    Sha256,
    Sha1Dc,
    /// The servil fork's single-threaded hash; its kernels are chosen at
    /// run time (SME2 where the CPU has it, integer + NEON hybrids elsewhere).
    Blake3ServilSt,
    /// Apple's CommonCrypto SHA-256 through CC_SHA256_Init/Update/Final.
    Sha256CommonCrypto,
    /// ring's SHA-256: BoringSSL's assembly, with runtime CPU detection.
    Sha256Ring,
    /// crates.io blake3 through Hasher::update_rayon, the crate's own
    /// multithreading, on Rayon's global pool as Rayon sizes it.
    Blake3Rayon,
    /// The fork's multithreaded calls and its queue: the caller's thread
    /// plus the fork's own workers, which sleep between calls.
    Blake3ServilMt,
    /// RustCrypto's SHA3-256 (the sha3 crate), with the ARMv8 SHA-3
    /// instructions where the CPU has them (keccak's run-time detection).
    Sha3_256,
    /// commonware-cryptography's BLAKE3 batches (its pull request #4982):
    /// one message per SIMD lane, in contenders/commonware-blake3.
    Blake3Commonware,
}

impl Algorithm {
    const ALL: [Algorithm; 10] = [
        Algorithm::Blake3,
        Algorithm::Sha256,
        Algorithm::Sha1Dc,
        Algorithm::Blake3ServilSt,
        Algorithm::Sha256CommonCrypto,
        Algorithm::Sha256Ring,
        Algorithm::Blake3Rayon,
        Algorithm::Blake3ServilMt,
        Algorithm::Sha3_256,
        Algorithm::Blake3Commonware,
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
            Self::Blake3Commonware => "blake3-commonware",
        }
    }

    /// Whether this contender runs on the calling thread's core alone:
    /// no SME unit shared with other cores, no helper threads.
    fn core_only(self) -> bool {
        !matches!(self, Self::Blake3ServilSt | Self::Blake3ServilMt | Self::Blake3Rayon)
    }

    /// Whether this contender is measured in a use case. BLAKE3 mt stays
    /// out of the batch use cases: `update_rayon` exists for large inputs,
    /// and a 64-byte message is a call to it that no program would make.
    /// The fork's multithreaded contender takes part through its batch
    /// entry point, `hash_many_multithreaded`, and its queue. BLAKE3
    /// servil st stays out of the continuous use cases: the fork's answer
    /// to a continuous load is its multithreaded queue (FROZEN.md).
    /// BLAKE3 commonware takes part in the batches alone: its other calls
    /// are BLAKE3 official's.
    fn takes_part(self, use_case: UseCase) -> bool {
        let use_case = use_case.call();
        if self == Self::Blake3Commonware {
            return matches!(use_case, UseCase::ManyMessages | UseCase::IdleManyMessages | UseCase::LentBatches | UseCase::ContinuousBatches);
        }
        match use_case {
            UseCase::ManyMessages | UseCase::IdleManyMessages | UseCase::LentBatches => !matches!(self, Self::Blake3Rayon),
            UseCase::ContinuousBatches => !matches!(self, Self::Blake3Rayon | Self::Blake3ServilSt),
            UseCase::ContinuousMessages => !matches!(self, Self::Blake3ServilSt),
            UseCase::OneMessage | UseCase::IdleOneMessage | UseCase::LentMessages | UseCase::Interleaved | UseCase::Collection => true,
            UseCase::Outboard => matches!(self, Self::Blake3 | Self::Blake3ServilSt | Self::Blake3ServilMt),
            UseCase::Verify => matches!(self, Self::Blake3 | Self::Blake3ServilSt | Self::Blake3ServilMt),
        }
    }

    /// Whether this contender can run in this build, or why not. A
    /// property of the target platform alone, so --list reads the same on
    /// every machine of one platform; machine capacity (CPU count, which
    /// kernel a CPU selects) shows up in the results and the kernel
    /// report, never here.
    fn availability(self) -> Result<(), String> {
        match self {
            Self::Blake3
            | Self::Sha256
            | Self::Sha1Dc
            | Self::Sha256Ring
            | Self::Blake3ServilSt
            | Self::Blake3Rayon
            | Self::Blake3ServilMt
            | Self::Sha3_256
            | Self::Blake3Commonware => Ok(()),
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
            Self::Blake3ServilSt => "BLAKE3 servil st",
            Self::Sha256CommonCrypto => "SHA-256 CommonCrypto",
            Self::Sha256Ring => "SHA-256 ring",
            Self::Blake3Rayon => "BLAKE3 official mt",
            Self::Blake3ServilMt => "BLAKE3 servil mt",
            Self::Sha3_256 => "SHA3-256",
            Self::Blake3Commonware => "BLAKE3 commonware",
        }
    }

    /*
     * Contender colours stay off pure green and pure red, which the hover
     * panel reserves for "faster" and "slower". A multithreaded contender
     * wears a darker shade of its single-threaded sibling's hue.
     */
    fn color(self) -> &'static str {
        match self {
            Self::Blake3 => "#3b82f6",
            Self::Sha256 => "#e07a45",
            Self::Sha1Dc => "#8a7a1e",
            Self::Blake3ServilSt => "#7c3aed",
            Self::Sha256CommonCrypto => "#0e9aa7",
            Self::Sha256Ring => "#c2410c",
            Self::Blake3Rayon => "#1e3a8a",
            Self::Blake3ServilMt => "#4c1d95",
            Self::Sha3_256 => "#db2777",
            Self::Blake3Commonware => "#ca8a04",
        }
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
            Self::Blake3Commonware => "commonwarexyz/monorepo pull request #4982, commit 3aa183f0592d65e23b6f9435f938ca6659d64bd1: cryptography/src/blake3/simd, unchanged, in contenders/commonware-blake3",
        }
    }

    /// The contender's mode: how many threads it may use and how, for the
    /// report header. The kernels it runs are a separate matter (see
    /// detect_kernels), chosen at run time.
    fn mode(self) -> &'static str {
        match self {
            Self::Blake3 => "single-threaded; blake3::hash for one message, the crate's hidden batch function blake3::platform::Platform::hash_many::<N> for a batch, sixteen messages per call",
            Self::Sha256
            | Self::Sha1Dc
            | Self::Sha256CommonCrypto
            | Self::Sha256Ring
            | Self::Sha3_256 => "single-threaded",
            Self::Blake3Commonware => "single-threaded; commonware-cryptography's Blake3::hash_many for a batch, one message per SIMD lane (NEON 4 on AArch64; AVX2 8, AVX-512 16 on x86-64), its digests in a new Vec each call",
            Self::Blake3ServilSt => "single-threaded; blake3_servil::hash for one message, blake3_servil::hash_many for a batch, Hasher::update per piece for many messages at once",
            Self::Blake3Rayon => "multithreaded; Hasher::update_rayon (per piece, for a stream) on Rayon's global pool, the crate's own multithreading as a program gets it by default: the tree splits recursively over the pool, and inputs under a few chunks stay on the caller's thread",
            Self::Blake3ServilMt => "multithreaded; blake3_servil::hash_multithreaded for one message, hash_many_multithreaded for a batch, Hasher::update_multithreaded per piece for many messages at once; for continuous loads its queue: Queue::messages for messages of up to 64 KiB, Queue::pieces for longer ones, Queue::fixed for batches: the fork chooses when to wake its worker threads; the kernel tables below show the one-shot calls' thresholds",
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
        assert!(ns > 0 && units > 0, "a sample covers some units in some time");
        assert!(ns < 1 << 40, "a sample of {ns} ns exceeds 2^40 ns (18 minutes)");
        Measured { ns, units }
    }

}

/// A cell's mean exactly as measured: total ns over total units. Kept for
/// display: Q64.64 is ample for statistics, yet a half-way decimal can move
/// one display tick if approximated first.
#[derive(Clone, Copy)]
struct ExactMean {
    ns: u128,
    units: u128,
}

impl ExactMean {
    fn of(samples: &[Measured]) -> Self {
        assert!(!samples.is_empty(), "a mean needs samples");
        Self { ns: samples.iter().map(|m| u128::from(m.ns)).sum(), units: samples.iter().map(|m| u128::from(m.units)).sum() }
    }

    fn fixed(self) -> PerUnit {
        Fixed(clocks::summary::mean([(u64::try_from(self.ns).expect("a cell's samples total under 2^64 ns"), u64::try_from(self.units).expect("a cell's units fit in u64"))]))
    }

    /// Scale the measured ratio before its one rounding for the reader.
    fn format_ns(self, factor: u64) -> String {
        let numerator = self.ns.checked_mul(u128::from(factor)).expect("a scaled mean fits in u128");
        let denominator = self.units;
        let mut decimals = 3u32;
        while decimals < 6 && numerator * 10u128.pow(decimals - 2) < denominator {
            decimals += 1;
        }
        let scale = 10u128.pow(decimals);
        let rounded = numerator.checked_mul(scale).and_then(|n| n.checked_add(denominator / 2))
            .expect("a rounded mean fits in u128") / denominator;
        format!("{}.{:0width$}", rounded / scale, rounded % scale, width = decimals as usize)
    }
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
        Fixed(self.0.checked_sub(other.0).expect("a difference is never negative here"))
    }
}

impl std::ops::Mul<u64> for Fixed {
    type Output = Fixed;
    fn mul(self, factor: u64) -> Fixed {
        Fixed(self.0.checked_mul(u128::from(factor)).expect("a scaled value fits in u128"))
    }
}

impl Fixed {
    #[cfg(test)]
    const ONE: Fixed = Fixed(1 << 64);

    /// self / other, as a Fixed, rounded down at the last bit: a long
    /// division, 24 bits at a time so that no step overflows. Both values
    /// are below 2^40 (see Measured) and their ratio below 2^63.
    fn ratio(self, other: Fixed) -> Fixed {
        assert!(other.0 > 0 && self.0 < 1 << 104 && other.0 < 1 << 104, "a ratio of two values below 2^40");
        let (mut quotient, mut remainder) = (self.0 / other.0, self.0 % other.0);
        assert!(quotient < 1 << 63, "a ratio below 2^63");
        for bits in [24, 24, 16] {
            let step = remainder << bits;
            quotient = (quotient << bits) | (step / other.0);
            remainder = step % other.0;
        }
        Fixed(quotient)
    }

    /// How self compares with `permille` / 1000, exactly.
    fn cmp_permille(self, permille: u64) -> std::cmp::Ordering {
        (self.0 * 1000).cmp(&(u128::from(permille) << 64))
    }

    /// self in permille, rounded once.
    fn permille(self) -> u64 {
        u64::try_from((self.0 * 1000 + (1 << 63)) >> 64).expect("a permille figure fits in u64")
    }

    /// Nanoseconds with three decimals, and more below 0.1 ns so that
    /// three significant digits show, rounded once: "0.437", "0.0311",
    /// "121.362".
    fn format_ns(self) -> String {
        let mut decimals = 3u32;
        while decimals < 6 && self.0 < (1u128 << 64) / 10u128.pow(decimals - 2) {
            decimals += 1;
        }
        let whole = 10u128.pow(decimals);
        let rounded = (self.0 * whole + (1u128 << 63)) >> 64;
        format!("{}.{:0width$}", rounded / whole, rounded % whole, width = decimals as usize)
    }

}

/*
 * Summary of one cell's samples: its mean, the total time of its samples
 * over the total work they did (clocks::summary), what a caller pays on
 * average.
 */
#[derive(Clone, Copy)]
struct Statistics {
    mean: PerUnit,
    exact_mean: Option<ExactMean>,
}

impl Statistics {
    fn format_mean(&self, factor: u64) -> String {
        self.exact_mean.map_or_else(|| (self.mean * factor).format_ns(), |m| m.format_ns(factor))
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
    Shared,
}

impl Scenario {
    const ALL: [Scenario; 2] = [Scenario::Solo, Scenario::Shared];

    fn key(self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::Shared => "shared",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Self::Solo => "Solo",
            Self::Shared => "Shared",
        }
    }

    /// What a reader of the results needs to know about the scenario.
    fn description(self) -> &'static str {
        match self {
            Self::Solo => "one copy of each contender, on one thread",
            Self::Shared => "two copies of the contender at once, each hashing its own input on its own thread, inputs one after another; the time of each copy",
        }
    }

    /// Whether this scenario measures `use_case`. Two copies at once
    /// measure the nonstop use cases alone: after a gap, a copy whose twin
    /// had just run the same code met that code in a cache the gap left
    /// warm, and read faster than one program alone (bench-hashes NOTES,
    /// "Shared after a gap"; Zooko, September 30, 2026).
    fn measures(self, use_case: UseCase) -> bool {
        self == Self::Solo || !use_case.after_gap()
    }
}

/// One (contender, point) cell: measured time per unit in each scenario
/// that measures its use case (Scenario::measures).
#[derive(Clone, Copy)]
struct Cell {
    solo: Statistics,
    shared: Option<Statistics>,
}

impl Cell {
    /// Requires `scenario` to measure the cell's use case.
    fn get(&self, scenario: Scenario) -> Statistics {
        match scenario {
            Scenario::Solo => self.solo,
            Scenario::Shared => self.shared.expect("a shared cell exists for the nonstop use cases alone"),
        }
    }
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
 * Contract: two to six contenders, each available on this machine, no
 * duplicates.
 */
struct Roster {
    algorithms: Vec<Algorithm>,
    /// The points measured, as ascending indices into POINTS: every point,
    /// or the subset --points names.
    points: Vec<usize>,
    /// Sample rounds.
    rounds: usize,
    /// Every cell samples in every round (`--rounds`: a fixed design, as
    /// the fork's regression check needs); otherwise each samples in its
    /// share of the rounds (cell_wants_sample).
    every_round: bool,
}

impl Roster {
    /*
     * `points` restricts the run to those POINTS indices; without it a
     * full run measures every point and a quick run the points below
     * QUICK_BYTES and QUICK_MESSAGES. `rounds` fixes the round count
     * (positive), every cell sampled in each, else FULL_ROUNDS or
     * QUICK_ROUNDS with each cell sampled in its share of them.
     */
    fn new(algorithms: Vec<Algorithm>, quick: bool, points: Option<Vec<usize>>, rounds: Option<usize>) -> Self {
        assert!(
            (2..=Algorithm::ALL.len()).contains(&algorithms.len()),
            "a run compares two contenders or more, up to every one there is; {} were selected",
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
        let points = points.unwrap_or_else(|| (0..POINT_COUNT).filter(|&index| !quick || POINTS[index].quick()).collect());
        assert!(!points.is_empty() && points.windows(2).all(|w| w[0] < w[1]), "points ascend, without repeats");
        let every_round = rounds.is_some();
        let rounds = rounds.unwrap_or(if quick { QUICK_ROUNDS } else { FULL_ROUNDS });
        assert!(rounds > 0, "--rounds must be positive");
        Self { algorithms, points, rounds, every_round }
    }

    /// Whether every point of each use case measured runs from the axis's
    /// start to the run's limit for it (a quick run's shorter axes count):
    /// the graph needs axes without holes.
    fn whole_axes(&self) -> bool {
        self.algorithms.iter().all(|algorithm| self.points.iter().any(|&index| algorithm.takes_part(POINTS[index].use_case)))
            && UseCase::ALL.iter().all(|&use_case| {
                let measured: Vec<usize> = use_case.points().filter(|&index| self.measures(index)).collect();
                measured.is_empty() || (measured.len() >= use_case.points().len().min(2) && measured == (use_case.points().start..measured.last().unwrap() + 1).collect::<Vec<_>>())
            })
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
    let participants: Vec<usize> = algorithms.iter().enumerate()
        .filter_map(|(index, algorithm)| algorithm.takes_part(use_case).then_some(index)).collect();
    match participants.len() {
        0 => Vec::new(),
        1 => vec![participants],
        n => williams_orders(n).into_iter()
            .map(|order| order.into_iter().map(|rank| participants[rank]).collect()).collect(),
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
        let reversed: Vec<Vec<usize>> = rows.iter().map(|row| row.iter().rev().copied().collect()).collect();
        rows.extend(reversed);
    }
    assert_orders_balanced(&rows, n);
    rows
}

/*
 * The default contenders: the fastest implementations of BLAKE3 and
 * SHA-256 on the machines this benchmark has measured. For BLAKE3 that is
 * the servil fork, single-threaded and multithreaded. For SHA-256 neither
 * crate wins everywhere: sha2 is faster for one and two blocks (and so for
 * every batch of 64-byte messages), ring from 256 B up, so both run.
 */
const DEFAULT_CONTENDERS: [Algorithm; 4] =
    [Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt, Algorithm::Sha256, Algorithm::Sha256Ring];

/*
 * The contenders the graph shows when it opens; a click on a name shows
 * any other. Three lines, for a first view with little to untangle
 * (Zooko, September 25, 2026): the fastest BLAKE3 at every size (servil
 * mt matches servil below its threading threshold), SHA-256 ring (the
 * faster SHA-256 from 256 B up; sha2, faster below and in 64-byte
 * batches, is one click away), and the crates.io BLAKE3 most programs
 * use. A run with none of them opens with every contender shown.
 */
const SHOWN_AT_FIRST: [Algorithm; 3] = [Algorithm::Blake3ServilMt, Algorithm::Sha256Ring, Algorithm::Blake3];

/*
 * Contenders that run only when --contenders names them, left out of
 * --all: BLAKE3 official mt, which BLAKE3 servil mt beats at every point
 * (Zooko, September 26, 2026: kept so that the crate's maintainers, or
 * anyone, can see it measured on request).
 */
const BY_REQUEST: [Algorithm; 1] = [Algorithm::Blake3Rayon];

/// How the user chose the contenders, for the report header.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    /// DEFAULT_CONTENDERS.
    Default,
    /// `--all`: every contender that can run here, but BY_REQUEST.
    All,
    /// `--contenders a,b,c`.
    Explicit,
}

const USAGE: &str = "\
bench-hashes: hash speed by input size and by messages per batch, alone and
shared with a second copy of the same contender

  bench-hashes                     BLAKE3 servil st and mt
                                   and SHA-256 (sha2 and ring)
  bench-hashes --all               every contender this machine can run
                                   but blake3-official-mt (named only)
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
                                   \"interleaved 256 open\", \"continuous 64 KiB\",
                                   \"continuous batch 1024\"); with
                                   --contenders only
  --rounds N                       exactly N sample rounds, every cell sampled in each
  --trace-clocks PATH              also write one CSV line per sample interval
                                   with its wall time and (on Apple) the
                                   thread's cycles, instructions, and time per
                                   core kind, for the solo sample and each
                                   shared copy

  bench-hashes compare OLD.tsv... -- NEW.tsv...
                                   compare runs' samples files, each side's
                                   pooled, cell by cell, speed with speed
  bench-hashes regress OLD_EXE NEW_EXE [--points NAME,...]
                                   whether NEW is slower than OLD: the two
                                   in alternating runs, a verdict as exit
                                   0, 1 (slower), or 2 (no verdict)
";

struct Options {
    selection: Selection,
    explicit: Vec<Algorithm>,
    points: Option<Vec<usize>>,
    rounds: Option<usize>,
    trace_path: Option<std::path::PathBuf>,
    quick: bool,
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

    /* --trace-clocks PATH may accompany any selection. */
    let trace_path = arguments
        .iter()
        .position(|argument| argument == "--trace-clocks")
        .map(|index| {
            assert!(index + 1 < arguments.len(), "--trace-clocks needs a file path\n\n{USAGE}");
            let path = std::path::PathBuf::from(&arguments[index + 1]);
            arguments.drain(index..=index + 1);
            path
        });

    /* --points LIST and --rounds N narrow a --contenders run. */
    let mut take_value = |flag: &str| {
        arguments.iter().position(|argument| argument == flag).map(|index| {
            assert!(index + 1 < arguments.len(), "{flag} needs a value\n\n{USAGE}");
            let value = arguments[index + 1].clone();
            arguments.drain(index..=index + 1);
            value
        })
    };
    let points = take_value("--points").map(|list| {
        let mut indices: Vec<usize> = list
            .split(',')
            .map(point_named)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices
    });
    let rounds = take_value("--rounds").map(|n| n.parse::<usize>().expect("--rounds takes a whole number"));

    let (selection, explicit) = parse_selection(&arguments);
    assert!(
        (points.is_none() && rounds.is_none()) || selection == Selection::Explicit,
        "--points and --rounds narrow a --contenders run\n\n{USAGE}"
    );
    Options { selection, explicit, points, rounds, trace_path, quick }
}

/*
 * The point a `--points` name names: a label's prefix names its use cases
 * ("interleaved 256 open", "idle 16", "continuous batch 1024"), the longest
 * prefix that matches, the plain label the calls after other work.
 */
fn point_named(name: &str) -> usize {
    let name = name.trim();
    let (prefix, label) = UseCase::ALL
        .into_iter()
        .map(UseCase::label_prefix)
        .filter(|prefix| name.starts_with(prefix))
        .max_by_key(|prefix| prefix.len())
        .map(|prefix| (prefix, &name[prefix.len()..]))
        .expect("the plain prefix matches every label");
    let wanted = |point: &Point| point.use_case.label_prefix() == prefix;
    POINTS.iter().position(|point| point.label == label && wanted(point)).unwrap_or_else(|| {
        let labels: Vec<String> = POINTS.iter().filter(|point| wanted(point)).map(|point| format!("{prefix}{}", point.label)).collect();
        panic!("--points: no point {name:?}; the points named so are {}", labels.join(", "))
    })
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
                println!("  {:<20} {:<22} {status}", algorithm.key(), algorithm.name());
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

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.first().map(String::as_str) == Some("compare") {
        return compare_command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("regress") {
        std::process::exit(regress_command(&arguments[1..]));
    }
    if arguments.first().map(String::as_str) == Some("b3sum") {
        return b3sum::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("chart") {
        return chart::command(&arguments[1..]);
    }
    if arguments.first().map(String::as_str) == Some("map") {
        return map::command(&arguments[1..]);
    }
    let Options { selection, explicit, points, rounds, trace_path, quick } = parse_arguments();
    let mut trace = trace_path.map(ClockTrace::new);
    let mut machine = machine_metadata();

    let (algorithms, selection_note) = match selection {
        Selection::Default => (DEFAULT_CONTENDERS.to_vec(), String::from("the default contenders")),
        Selection::All => (
            /* SHA-1DC, the slowest by far, runs in quick runs only when named. */
            Algorithm::ALL
                .into_iter()
                .filter(|algorithm| algorithm.availability().is_ok() && !BY_REQUEST.contains(algorithm))
                .filter(|&algorithm| !quick || algorithm != Algorithm::Sha1Dc)
                .collect(),
            String::from("every contender available on this machine but those run on request"),
        ),
        Selection::Explicit => {
            let keys = explicit.iter().map(|algorithm| algorithm.key()).collect::<Vec<_>>().join(",");
            (explicit, format!("--contenders {keys}"))
        }
    };
    let roster = Roster::new(algorithms, quick, points, rounds);
    let (results, samples, load) = measure_all(&roster, trace.as_mut());
    machine.load = load;
    machine.power[1] = Power::read();

    if let Some(trace) = &trace {
        trace.write();
    }

    let text = generate_text(&roster, &results, &machine, &selection_note);
    /* The graph draws whole axes: every point of each use case it shows. */
    let whole = roster.whole_axes();

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
    fs::write(&samples_path, &samples).unwrap_or_else(|error| {
        panic!("failed to write {}: {error}", samples_path.display())
    });

    fs::write(&text_path, &text).unwrap_or_else(|error| {
        panic!("failed to write {}: {error}", text_path.display())
    });
    let checks_path = directory.join(format!("{stem}.checks.txt"));
    fs::write(&checks_path, consistency(&roster, &results)).unwrap_or_else(|error| {
        panic!("failed to write {}: {error}", checks_path.display())
    });

    if whole {
        let guide_path = directory.join(format!("{stem}.guide.html"));
        fs::write(&guide_path, generate_guide(&roster, &results, &machine)).unwrap_or_else(|error| {
            panic!("failed to write {}: {error}", guide_path.display())
        });
        println!("# API guide (HTML) is in \"{}\" .", guide_path.display());
    } else {
        remove_guide(&directory);
    }

    println!(
        "# Data results (text) are in \"{}\" .",
        text_path.display(),
    );
    println!("# Samples (TSV) are in \"{}\" .", samples_path.display());
    map::write(&directory, None);
    let chart_path = directory.join(format!("{stem}.chart.svg"));
    match chart::from_samples(samples_path.to_str().expect("a path in UTF-8")) {
        Some(chart) => {
            fs::write(&chart_path, chart).unwrap_or_else(|error| panic!("failed to write {}: {error}", chart_path.display()));
            println!("# Speed chart (SVG) is in \"{}\" .", chart_path.display());
        }
        None => {
            // A quick run has no 1 MiB cells: no chart, and none left from an older run.
            if chart_path.exists() {
                fs::remove_file(&chart_path).unwrap_or_else(|error| panic!("failed to remove stale {}: {error}", chart_path.display()));
            }
        }
    }
}

/// Sparse runs replace the report and samples too, so a guide from an
/// older full run leaves their directory with them.
fn remove_guide(directory: &std::path::Path) {
    for name in ["bench-hashes.guide.html"] {
        let path = directory.join(name);
        if path.exists() {
            fs::remove_file(&path).unwrap_or_else(|error| panic!("failed to remove stale {}: {error}", path.display()));
        }
    }
}

/*
 * The orders must place every contender in every position equally often
 * and realise every ordered adjacency equally often. This is what lets a
 * handful of orders stand in for all permutations.
 */
fn assert_orders_balanced(rows: &[Vec<usize>], n: usize) {
    assert_eq!(rows.len() % n, 0, "the order count must be a multiple of the contender count");
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
            let expected = if follower == algorithm { 0 } else { per_adjacency };
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

fn measure_all(roster: &Roster, mut trace: Option<&mut ClockTrace>) -> (Results, RunSamples, Vec<clocks::load::Window>) {
    /* Inputs for the points measured; an empty buffer stands in for the rest. */
    /*
     * The bytes are irrelevant to every contender's speed, so each point
     * hashes a prefix of one buffer, written once (every page backed by
     * memory of its own); the second copy in a duo sample hashes a prefix
     * of a second buffer, so two copies share no cache lines, for the use
     * cases measured with two copies alone. (A buffer per point held 3.3 GB
     * in a full run.)
     */
    let largest = |wanted: &dyn Fn(usize) -> bool| (0..POINT_COUNT).filter(|&index| roster.measures(index) && wanted(index)).map(|index| POINTS[index].bytes).max().unwrap_or(0);
    let own = make_input_seeded(largest(&|_| true), 0);
    let other = make_input_seeded(largest(&|index| Scenario::Shared.measures(POINTS[index].use_case)), 1);
    let prefixes = |buffer: &[u8]| -> Vec<std::ops::Range<usize>> {
        (0..POINT_COUNT).map(|index| if roster.measures(index) && POINTS[index].bytes <= buffer.len() { 0..POINTS[index].bytes } else { 0..0 }).collect()
    };
    let inputs: Vec<&[u8]> = prefixes(&own).into_iter().map(|range| &own[range]).collect();
    let duo_inputs: Vec<&[u8]> = prefixes(&other).into_iter().map(|range| &other[range]).collect();
    let mut progress = Progress::new(roster);
    let duo = Duo::new();

    /*
     * Each algorithm/point combination gets its own calibrated iteration
     * count so that timed blocks have approximately equal durations.
     * The digest checks have already called each contender at every point.
     * Startup and calibration both happen before the timed samples.
     */
    let empty = || -> Samples {
        (0..roster.len()).map(|_| (0..POINT_COUNT).map(|_| Vec::with_capacity(2 * roster.rounds)).collect()).collect()
    };
    let mut samples = RunSamples {
        solo: empty(),
        shared: empty(),
        solo_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
        shared_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
    };
    let mut batch_iterations: Vec<Vec<usize>> = vec![vec![1usize; POINT_COUNT]; roster.len()];
    let point_orders: Vec<_> = POINTS.iter()
        .map(|point| participating_orders(&roster.algorithms, point.use_case)).collect();
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
    for pattern in ["nonstop", "busy", "idle"] {
        progress.phase("calibrating");

        for (point_index, point) in POINTS.iter().enumerate() {
            for algorithm_index in 0..roster.len() {
                let algorithm = roster.algorithms[algorithm_index];
                if algorithm.takes_part(point.use_case) && roster.measures(point_index) && point.use_case.pattern_key() == pattern {
                    let (iterations, per_iteration_ns) = calibrate_batch(algorithm, inputs[point_index], *point);
                    /*
                     * A synchronous cell's sample: calls after the gap summing
                     * about GAP_SAMPLE_NS (one at least), from calls timed
                     * after the gap (sized from their time back to back, a
                     * 64-byte call's sample summed about 50 calls, each after
                     * its own 1 ms gap, where 7 fill it).
                     */
                    batch_iterations[algorithm_index][point_index] = if point.use_case.after_gap() {
                        let input = inputs[point_index];
                        let timed = take_sample(algorithm, input, *point, CALIBRATION_GAPS as usize);
                        let per_call_ns = u128::from(timed.elapsed_ns) / u128::from(CALIBRATION_GAPS);
                        GAP_SAMPLE_NS.div_ceil(per_call_ns.max(per_iteration_ns).max(1)) as usize
                    } else {
                        iterations
                    };
                }
            }
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
        progress.phase(match pattern { "nonstop" => "measuring one after another", "busy" => "measuring after other work", _ => "measuring after idling" });

        for round in 0..roster.rounds {
            progress.round(round, &samples.solo);
            clocks::load::tick();

            for point_offset in 0..roster.points.len() {
                let size_index = roster.points[(point_offset + round) % roster.points.len()];
                let point = POINTS[size_index];
                let orders = &point_orders[size_index];
                if point.use_case.pattern_key() != pattern || orders.is_empty()
                    || (!roster.every_round && !cell_wants_sample(round + size_index, roster.rounds, orders.len())) {
                    continue;
                }
                let algorithm_order = &orders[visits[size_index] % orders.len()];
                visits[size_index] += 1;

                let input = inputs[size_index];

                for (position, &algorithm_index) in algorithm_order.iter().enumerate() {
                    let algorithm = roster.algorithms[algorithm_index];
                    let iterations =
                        batch_iterations[algorithm_index][size_index];

                    /* The solo sample: this thread runs the batch, alone. */
                    clocks::load::tick();
                    let DuoCopy { elapsed_ns, counts, preparation, started_ns } = take_sample(algorithm, input, point, iterations);

                    /*
                     * The shared sample, under the same conditions, for the
                     * nonstop use cases: two copies run a batch each at once,
                     * on two threads, and each copy's own time is a sample.
                     */
                    let copies = Scenario::Shared.measures(point.use_case)
                        .then(|| duo.run(algorithm, input, duo_inputs[size_index], point, iterations));

                    let total_units = point.use_case.units(point, iterations);
                    let per_unit = |ns: u64| Measured::new(ns, total_units);
                    samples.solo[algorithm_index][size_index].push(per_unit(elapsed_ns));
                    samples.solo_started_ns[algorithm_index][size_index].push(started_ns);
                    for copy in copies.iter().flatten() {
                        samples.shared[algorithm_index][size_index].push(per_unit(copy.elapsed_ns));
                        samples.shared_started_ns[algorithm_index][size_index].push(copy.started_ns);
                    }

                    if let Some(trace) = trace.as_deref_mut() {
                        /* A solo row leaves the shared columns empty. */
                        let (later, copy_fields) = match &copies {
                            Some(copies) => (copies.iter().map(|copy| copy.elapsed_ns).max().unwrap().to_string(),
                                format!("{},{},{},{}", copies[0].elapsed_ns, counts_csv(copies[0].counts), copies[1].elapsed_ns, counts_csv(copies[1].counts))),
                            None => (String::new(), format!(",{NO_COUNTS},,{NO_COUNTS}")),
                        };
                        let scenario = if copies.is_some() { "solo and shared" } else { "solo" };
                        trace.lines.push(format!(
                            "{round},{},{},{},{iterations},{elapsed_ns},{},{:?},{later},{copy_fields},{scenario}",
                            point_offset * algorithm_order.len() + position,
                            algorithm.key(),
                            input.len(),
                            counts_csv(counts),
                            point.use_case,
                        ));
                        if let Some(preparation) = preparation {
                            assert!(copies.is_none(), "a use case with a preparation runs after a gap, alone");
                            trace.lines.push(format!(
                                "{round},{},{},{},{iterations},{},{},{:?},,,{NO_COUNTS},,{NO_COUNTS},preparation solo",
                                point_offset * algorithm_order.len() + position, algorithm.key(), input.len(),
                                preparation.wall_ns, counts_csv(preparation.counts), point.use_case,
                            ));
                        }
                    }
                }
            }
        }
    }

    progress.finish(&samples.solo);
    let load: Vec<clocks::load::Window> =
        clocks::load::windows().into_iter().filter(|window| window.end_ns > measuring_from_ns).collect();

    let mut results: Results = vec![vec![None; POINT_COUNT]; roster.len()];

    for algorithm_index in 0..roster.len() {
        for (size_index, point) in POINTS.iter().enumerate() {
            if !roster.algorithms[algorithm_index].takes_part(point.use_case) || !roster.measures(size_index) {
                continue;
            }
            let solo = &samples.solo[algorithm_index][size_index];
            let shared = &samples.shared[algorithm_index][size_index];
            let copies = if Scenario::Shared.measures(point.use_case) { 2 } else { 0 };
            assert!(!solo.is_empty() && solo.len() <= roster.rounds, "one solo sample per round at most, and one at least");
            assert_eq!(shared.len(), copies * solo.len(), "two shared samples, one per copy, beside every solo sample of a nonstop cell");
            results[algorithm_index][size_index] = Some(Cell {
                solo: summarize_measured(solo),
                shared: (copies > 0).then(|| summarize_measured(shared)),
            });
        }
    }

    (results, samples, load)
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
    started: Instant,
    measuring_started: Option<Instant>,
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
        self.draw(&format!("[{:>5}s] {name}…", tenths_of_seconds(self.started.elapsed())));
    }

    /* Called at the start of each round; `samples` holds every round so far. */
    fn round(&mut self, round: usize, samples: &Samples) {
        self.round_inner(round, &running_medians(self.roster, samples, self.roster.progress_point()));
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
            let elapsed_ns = measuring_started.elapsed().as_nanos();
            let left_ns = elapsed_ns * (rounds - round) as u128 / round as u128;
            format!("{:>3}s left", (left_ns + 500_000_000) / 1_000_000_000)
        } else {
            " estimating".to_owned()
        };

        self.draw(&format!(
            "[{:>5}s] measuring {bar} {:>3}/{rounds} rounds · {remaining} · {medians}",
            tenths_of_seconds(self.started.elapsed()),
            round,
        ));
    }

    fn finish(&mut self, samples: &Samples) {
        self.finish_inner(&running_medians(self.roster, samples, self.roster.progress_point()));
    }

    fn finish_inner(&mut self, medians: &str) {
        let bar = "█".repeat(Self::BAR_WIDTH);
        let rounds = self.roster.rounds;
        self.draw(&format!(
            "[{:>5}s] measured  {bar} {rounds}/{rounds} rounds · {medians}",
            tenths_of_seconds(self.started.elapsed()),
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

fn running_medians(roster: &Roster, samples: &Samples, size_index: usize) -> String {
    if samples.iter().all(|contender| contender[size_index].is_empty()) {
        return format!("means at {} pending", POINTS[size_index].label);
    }

    /* Contenders that take no part in this point's use case have no samples there. */
    let parts: Vec<String> = (0..roster.len())
        .filter(|&algorithm_index| !samples[algorithm_index][size_index].is_empty())
        .map(|algorithm_index| {
            format!("{} {}", roster.algorithms[algorithm_index].name(), ExactMean::of(&samples[algorithm_index][size_index]).format_ns(1))
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
    let mut input: Vec<u8> = (0..size.div_ceil(8) as u64).flat_map(|index| (seed << 48 | index).to_le_bytes()).collect();
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

fn use_digest(digest: &[u8]) {
    DIGESTS.with_borrow_mut(|(slots, next)| {
        let slot = &mut slots[*next % DIGEST_SLOTS];
        slot[..digest.len().min(32)].copy_from_slice(&digest[..digest.len().min(32)]);
        *next += 1;
    });
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
        let idle = point.use_case.idle();
        if point.use_case.call() == UseCase::OneMessage {
            // Select the API before the gap and clocks. A cold call should
            // pay for its API, rather than the benchmark's dispatch tree.
            return take_prepared_sample(input, iterations, idle, one_message_call(algorithm));
        }
        if point.use_case.call() == UseCase::ManyMessages {
            let hash_many: Option<fn(&[u8], usize, &mut [[u8; 32]])> = match algorithm {
                Algorithm::Blake3ServilSt => Some(blake3_servil::hash_many),
                Algorithm::Blake3ServilMt => Some(blake3_servil::hash_many_multithreaded),
                _ => None,
            };
            if let Some(hash_many) = hash_many {
                let mut digests = take_batch_digests(point.messages);
                let measured = take_prepared_sample(input, iterations, idle, |input| {
                    #[cfg(test)]
                    observe_call("hash_many");
                    hash_many(black_box(input), MESSAGE_LEN, &mut digests[..point.messages]);
                    black_box(digests[..point.messages].as_flattened());
                });
                keep_batch_digests(digests);
                return measured;
            }
        }
        return take_prepared_sample(input, iterations, idle, |input| run_batch(algorithm, input, point, 1));
    }
    let started_ns = clocks::load::now_ns();
    let counts0 = clocks::Counts::read();
    let started = clocks::now();
    run_batch(algorithm, input, point, iterations);
    let elapsed_ns = clocks::since_ns(started);
    let counts = counts0.zip(clocks::Counts::read()).map(|(before, after)| after.since(before));
    DuoCopy { elapsed_ns, counts, preparation: None, started_ns }
}

/// Prepare and time an already-selected call, each after the program
/// idled (`idle`) or after its other work. The producer and work buffers
/// stay outside the API's interval, as does dispatch selection.
fn take_prepared_sample(input: &[u8], iterations: usize, idle: bool, mut call: impl FnMut(&[u8])) -> DuoCopy {
    GAP_BUFFERS.with(|kept| {
        let mut buffers = kept.borrow_mut();
        let (work, produced) = &mut *buffers;
        produced.resize(input.len(), 0);
        let gap = if idle { clocks::Gap::Idle } else { clocks::Gap::Busy(work) };
        let started_ns = clocks::load::now_ns();
        let measured = clocks::measure_after_gaps_prepared(iterations as u64, gap, GAP_NS,
            produced.as_mut_slice(),
            |produced| produced.copy_from_slice(black_box(input)),
            |produced| call(black_box(produced)));
        // The write and the call, both the program's work (FROZEN.md, Zooko,
        // October 3, 2026); the trace keeps the write's own share.
        let counts = measured.calls.counts.zip(measured.preparation.counts).map(|(call, write)| call.plus(write));
        DuoCopy { elapsed_ns: measured.calls.wall_ns + measured.preparation.wall_ns, counts, preparation: Some(measured.preparation), started_ns }
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

/// One whole message through the contender's plain API. Selection happens
/// once, before the gap; every result remains observable to the optimizer.
fn one_message_call(algorithm: Algorithm) -> fn(&[u8]) {
    match algorithm {
        Algorithm::Blake3 => |input| use_digest(blake3::hash(input).as_bytes()),
        Algorithm::Blake3ServilSt => |input| {
            #[cfg(test)]
            observe_call("hash");
            use_digest(blake3_servil::hash(input).as_bytes());
        },
        Algorithm::Blake3ServilMt => |input| use_digest(blake3_servil::hash_multithreaded(input).as_bytes()),
        Algorithm::Sha256 => |input| use_digest(&Sha256::digest(input)),
        Algorithm::Sha256Ring => |input| use_digest(ring::digest::digest(&ring::digest::SHA256, input).as_ref()),
        Algorithm::Sha256CommonCrypto => |input| use_digest(&common_crypto::sha256(input)),
        Algorithm::Sha3_256 => |input| use_digest(&sha3::Sha3_256::digest(input)),
        Algorithm::Sha1Dc => |input| use_digest(sha1_checked::Sha1::try_digest(input).hash()),
        Algorithm::Blake3Rayon => |input| use_digest(blake3::Hasher::new().update_rayon(input).finalize().as_bytes()),
        Algorithm::Blake3Commonware => unreachable!("BLAKE3 commonware hashes batches alone"),
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
 * it every batch of two messages or more: the fork's hash_many, the
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
    let message_len = if point.use_case.batch() { point.use_case.message_len() } else { input.len() };
    assert!(messages == 1 || point.use_case == UseCase::Collection || input.len() == messages * message_len, "a batch is {messages} messages of {message_len} bytes");
    assert!(algorithm.takes_part(point.use_case), "{} takes no part in {:?}", algorithm.key(), point.use_case);

    match point.use_case {
        UseCase::Interleaved => return hash_interleaved(algorithm, input, iterations, consume),
        UseCase::Collection => return hash_collection(algorithm, input, point, iterations, consume),
        UseCase::Outboard => return hash_outboard(algorithm, input, iterations, consume),
        UseCase::Verify => return hash_verify(algorithm, input, iterations, consume),
        UseCase::ContinuousMessages => return hash_continuous_messages(algorithm, input, iterations, consume),
        UseCase::ContinuousBatches => return hash_continuous_batches(algorithm, input, point, iterations, consume),
        UseCase::LentMessages | UseCase::LentBatches => return hash_lent(algorithm, input, point, iterations, consume),
        UseCase::OneMessage | UseCase::IdleOneMessage | UseCase::ManyMessages | UseCase::IdleManyMessages => hash_in_memory(algorithm, input, point, iterations, consume),
    }
}

/// hash_batch for the use cases whose input is in memory: one message, or
/// a batch of them.
fn hash_in_memory(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize, consume: impl FnMut(&[u8])) {
    let messages = point.messages;
    let message_len = if point.use_case.batch() { point.use_case.message_len() } else { input.len() };
    match algorithm {
        Algorithm::Blake3 => {
            if !point.use_case.batch() {
                each_message(input, message_len, iterations, |m| *blake3::hash(m).as_bytes(), consume)
            } else {
                blake3_batch(input, message_len, iterations, consume)
            }
        }
        Algorithm::Sha256 => each_message(input, message_len, iterations, |m| Sha256::digest(m), consume),
        Algorithm::Sha3_256 => each_message(input, message_len, iterations, |m| {
            let digest: [u8; 32] = sha3::Sha3_256::digest(m).into();
            digest
        }, consume),
        Algorithm::Sha1Dc => each_message(input, message_len, iterations, |m| {
            let result = sha1_checked::Sha1::try_digest(m);
            let mut digest = [0u8; 20];
            digest.copy_from_slice(result.hash());
            digest
        }, consume),
        Algorithm::Blake3ServilSt => {
            if !point.use_case.batch() {
                each_message(input, message_len, iterations, |m| *blake3_servil::hash(m).as_bytes(), consume)
            } else {
                servil_batch(input, messages, message_len, iterations, blake3_servil::hash_many, consume)
            }
        }
        Algorithm::Sha256CommonCrypto => each_message(input, message_len, iterations, |m| common_crypto::sha256(m), consume),
        Algorithm::Sha256Ring => each_message(input, message_len, iterations, |m| ring::digest::digest(&ring::digest::SHA256, m), consume),
        Algorithm::Blake3Rayon => {
            assert_eq!(messages, 1, "BLAKE3 mt takes no part in the many-messages use case");
            each_message(input, message_len, iterations, |m| *blake3::Hasher::new().update_rayon(m).finalize().as_bytes(), consume)
        }
        Algorithm::Blake3ServilMt => {
            if !point.use_case.batch() {
                each_message(input, message_len, iterations, |m| *blake3_servil::hash_multithreaded(m).as_bytes(), consume)
            } else {
                servil_batch(input, messages, message_len, iterations, blake3_servil::hash_many_multithreaded, consume)
            }
        }
        Algorithm::Blake3Commonware => commonware_batch(input, messages, message_len, iterations, consume),
    }
}

/// A batch through commonware's `hash_many`, which takes the messages as
/// slices (here the batch's 64-byte arrays, in place) and returns their
/// digests in a new Vec.
fn commonware_batch(input: &[u8], messages: usize, message_len: usize, iterations: usize, mut consume: impl FnMut(&[u8])) {
    assert_eq!((message_len, input.len()), (64, messages * 64), "a batch of 64-byte messages");
    let (batch, _) = input.as_chunks::<64>();
    for _ in 0..iterations {
        let digests = commonware_blake3::blake3::hash_many(black_box(batch));
        for digest in &digests {
            consume(&digest.0);
        }
    }
}

/*
 * Messages with their outboards: `iterations` messages, each a copy of
 * `input` written into a kept buffer, then hashed with its outboard (the
 * parent nodes above 16 KiB groups, pre-order, as iroh-blobs stores them);
 * each hash to `consume`. BLAKE3 builds it with bao-tree, the servil fork
 * with outboard_with (one thread) or outboard_multithreaded_with; all give
 * the same bytes.
 */
fn hash_outboard(algorithm: Algorithm, input: &[u8], iterations: usize, mut consume: impl FnMut(&[u8])) {
    let mut buffers = take_buffers(1, input.len());
    for _ in 0..iterations {
        let buffer = &mut buffers[0];
        buffer.clear();
        buffer.extend_from_slice(black_box(input));
        match algorithm {
            Algorithm::Blake3 => {
                let outboard = bao_tree::io::outboard::PreOrderMemOutboard::create(&buffer[..], bao_tree::BlockSize::from_chunk_log(4));
                consume(outboard.root.as_bytes());
                black_box(&outboard.data);
            }
            Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt => {
                let build = if algorithm == Algorithm::Blake3ServilSt { blake3_servil::outboard_with } else { blake3_servil::outboard_multithreaded_with };
                let (hash, outboard) = build(blake3_servil::Mode::Hash, &buffer[..]);
                consume(hash.as_bytes());
                black_box(&outboard);
            }
            other => unreachable!("{} takes no part in outboards", other.key()),
        }
    }
    keep_buffers(1, input.len(), buffers);
}

/*
 * Messages received in their encodings (UseCase::Verify): `iterations`
 * messages, each `input`'s encoding as iroh-blobs sends a whole blob (its
 * 16 KiB groups and the parent nodes above them, in pre-order; bao-tree's
 * encode_ranges, made once per input outside the timed calls, the
 * sender's work), received in pieces of up to PIECE_LEN read into a kept
 * buffer, verified as they arrive against the message's hash, each
 * verified group written into the message's kept buffer; then the hash
 * to `consume`. BLAKE3 decodes with bao-tree's decode_ranges (its reads
 * from the received bytes, its writes into the message's buffer), the
 * servil fork with a Verifier fed each piece. Both copy each received
 * byte in and each verified byte out once.
 */
fn hash_verify(algorithm: Algorithm, input: &[u8], iterations: usize, mut consume: impl FnMut(&[u8])) {
    let (hash, encoded) = encoding(input);
    let mut buffers = take_buffers(1, input.len());
    let mut piece = STREAM_BUFFER.with(|kept| std::mem::take(&mut *kept.borrow_mut()));
    if piece.len() < PIECE_LEN {
        piece = written(PIECE_LEN, 1u8);
    }
    for _ in 0..iterations {
        let message = &mut buffers[0];
        message.resize(input.len(), 0);
        let received = black_box(&encoded[..]);
        match algorithm {
            Algorithm::Blake3 => {
                // The received bytes, read PIECE_LEN at most at a time through the kept buffer.
                struct Received<'a> { rest: &'a [u8], piece: &'a mut [u8] }
                impl std::io::Read for Received<'_> {
                    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                        let n = out.len().min(self.rest.len()).min(self.piece.len());
                        self.piece[..n].copy_from_slice(&self.rest[..n]);
                        out[..n].copy_from_slice(&self.piece[..n]);
                        self.rest = &self.rest[n..];
                        Ok(n)
                    }
                }
                let tree = bao_tree::BaoTree::new(input.len() as u64, bao_tree::BlockSize::from_chunk_log(4));
                let root = bao_tree::blake3::Hash::from_bytes(hash);
                let outboard = bao_tree::io::outboard::EmptyOutboard { tree, root };
                let target: &mut [u8] = &mut message[..];
                bao_tree::io::sync::decode_ranges(Received { rest: received, piece: &mut piece }, &bao_tree::ChunkRanges::all(), target, outboard)
                    .unwrap_or_else(|e| panic!("bao-tree refused its own encoding: {e}"));
            }
            // A Verifier has no multithreaded form: both servil contenders use it.
            Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt => {
                let mut verifier = blake3_servil::Verifier::new(blake3_servil::Mode::Hash, &blake3_servil::Hash::from_bytes(hash), input.len() as u64);
                let mut at = 0;
                for part in received.chunks(PIECE_LEN) {
                    piece[..part.len()].copy_from_slice(part);
                    let ok = verifier.update(&piece[..part.len()], |group| {
                        message[at..at + group.len()].copy_from_slice(group);
                        at += group.len();
                    });
                    black_box(ok);
                }
            }
            other => unreachable!("{} takes no part in verifying", other.key()),
        }
        consume(black_box(&hash));
        black_box(&message[..]);
    }
    keep_buffers(1, input.len(), buffers);
    STREAM_BUFFER.with(|kept| *kept.borrow_mut() = piece);
}

/// `input`'s hash and whole encoding (bao-tree's pre-order, 16 KiB
/// groups), made once for each input, for every thread.
fn encoding(input: &[u8]) -> ([u8; 32], std::sync::Arc<Vec<u8>>) {
    static MADE: std::sync::Mutex<Vec<((usize, [u8; 8]), [u8; 32], std::sync::Arc<Vec<u8>>)>> = std::sync::Mutex::new(Vec::new());
    let id = (input.len(), input[..8].try_into().expect("a message of 8 bytes or more"));
    let mut made = MADE.lock().unwrap();
    if let Some((_, hash, encoded)) = made.iter().find(|(k, _, _)| *k == id) {
        return (*hash, encoded.clone());
    }
    let outboard = bao_tree::io::outboard::PreOrderMemOutboard::create(input, bao_tree::BlockSize::from_chunk_log(4));
    let mut encoded = Vec::with_capacity(input.len() + outboard.data.len());
    bao_tree::io::sync::encode_ranges_validated(input, &outboard, &bao_tree::ChunkRanges::all(), &mut encoded).expect("an encoding");
    let hash = *outboard.root.as_bytes();
    made.push((id, hash, std::sync::Arc::new(encoded)));
    (hash, made.last().unwrap().2.clone())
}

/*
 * A collection (COLLECTIONS): `iterations` passes over its items, laid out
 * one after another in `input`, each item hashed by the contender's
 * one-message call (hash_in_memory), its hash to `consume`.
 */
fn hash_collection(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize, mut consume: impl FnMut(&[u8])) {
    static ITEMS: [std::sync::OnceLock<Vec<(usize, usize)>>; COLLECTION_COUNT] = [const { std::sync::OnceLock::new() }; COLLECTION_COUNT];
    let index = COLLECTIONS.iter().position(|(label, _)| *label == point.label).expect("a collection's point");
    let items = ITEMS[index].get_or_init(|| collection_items(COLLECTIONS[index].1));
    assert_eq!(input.len(), point.bytes, "the collection's items fill its input");
    if matches!(algorithm, Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt) {
        // The fork's call for a collection: every item in one call.
        let each = if algorithm == Algorithm::Blake3ServilSt { blake3_servil::hash_each_with } else { blake3_servil::hash_each_multithreaded_with };
        let slices: Vec<&[u8]> = items.iter().map(|&(offset, len)| &input[offset..offset + len]).collect();
        let mut digests = vec![[0u8; 32]; slices.len()];
        for _ in 0..iterations {
            each(blake3_servil::Mode::Hash, black_box(&slices), &mut digests);
            for digest in &digests {
                consume(digest);
            }
        }
        return;
    }
    for _ in 0..iterations {
        for &(offset, len) in items {
            hash_in_memory(algorithm, &input[offset..offset + len], Point::one("", len), 1, &mut consume);
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
    use sha2::Digest as _;
    use sha1_checked::digest::Update as _;
    match algorithm {
        Algorithm::Blake3 => each_stream(input, iterations, |pieces| {
            let mut hasher = blake3::Hasher::new();
            pieces(&mut |piece| { hasher.update(piece); });
            *hasher.finalize().as_bytes()
        }, consume),
        Algorithm::Blake3Rayon => each_stream(input, iterations, |pieces| {
            let mut hasher = blake3::Hasher::new();
            pieces(&mut |piece| { hasher.update_rayon(piece); });
            *hasher.finalize().as_bytes()
        }, consume),
        Algorithm::Blake3ServilSt => each_stream(input, iterations, |pieces| {
            let mut hasher = blake3_servil::Hasher::new();
            pieces(&mut |piece| { hasher.update(piece); });
            *hasher.finalize().as_bytes()
        }, consume),
        Algorithm::Blake3ServilMt => each_stream(input, iterations, |pieces| {
            let mut hasher = blake3_servil::Hasher::new();
            pieces(&mut |piece| { hasher.update_multithreaded(piece); });
            *hasher.finalize().as_bytes()
        }, consume),
        Algorithm::Sha256 => each_stream(input, iterations, |pieces| {
            let mut hasher = Sha256::new();
            pieces(&mut |piece| hasher.update(piece));
            let digest: [u8; 32] = hasher.finalize().into();
            digest
        }, consume),
        Algorithm::Sha256Ring => each_stream(input, iterations, |pieces| {
            let mut context = ring::digest::Context::new(&ring::digest::SHA256);
            pieces(&mut |piece| context.update(piece));
            let mut digest = [0u8; 32];
            digest.copy_from_slice(context.finish().as_ref());
            digest
        }, consume),
        Algorithm::Sha256CommonCrypto => each_stream(input, iterations, |pieces| common_crypto::sha256_pieces(pieces), consume),
        Algorithm::Sha1Dc => each_stream(input, iterations, |pieces| {
            let mut hasher = sha1_checked::Sha1::new();
            pieces(&mut |piece| hasher.update(piece));
            let mut digest = [0u8; 20];
            digest.copy_from_slice(hasher.try_finalize().hash());
            digest
        }, consume),
        Algorithm::Sha3_256 => each_stream(input, iterations, |pieces| {
            let mut hasher = sha3::Sha3_256::new();
            pieces(&mut |piece| sha3::Digest::update(&mut hasher, piece));
            let digest: [u8; 32] = hasher.finalize().into();
            digest
        }, consume),
        Algorithm::Blake3Commonware => unreachable!("BLAKE3 commonware hashes batches alone"),
    }
}

/*
 * Many messages at once (INTERLEAVED): `iterations` units of
 * INTERLEAVED.chunk bytes of pieces, each piece copied from `input` into
 * the program's piece buffer (as a receive writes it), then fed to its
 * message's incremental API; each message's hash, when it ends, goes to
 * `consume`.
 */
fn hash_interleaved(algorithm: Algorithm, input: &[u8], iterations: usize, consume: impl FnMut(&[u8])) {
    use sha2::Digest as _;
    use sha1_checked::digest::Update as _;
    let key = algorithm.key();
    match algorithm {
        Algorithm::Blake3 => each_interleaved(key, input, iterations, blake3::Hasher::new, per_piece(|h: &mut blake3::Hasher, p| { h.update(p); }), per_message(blake3::Hasher::new, |h| *h.finalize().as_bytes()), consume),
        Algorithm::Blake3Rayon => each_interleaved(key, input, iterations, blake3::Hasher::new, per_piece(|h: &mut blake3::Hasher, p| { h.update_rayon(p); }), per_message(blake3::Hasher::new, |h| *h.finalize().as_bytes()), consume),
        Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt => {
            let update = if algorithm == Algorithm::Blake3ServilSt { blake3_servil::Hasher::update_each } else { blake3_servil::Hasher::update_each_multithreaded };
            let mut digests = Vec::new();
            each_interleaved(key, input, iterations, blake3_servil::Hasher::new, update, move |hashers: &mut [blake3_servil::Hasher], ended: &[usize], consume: &mut dyn FnMut(&[u8])| {
                digests.resize(ended.len(), [0u8; 32]);
                blake3_servil::Hasher::finalize_each(hashers, ended, &mut digests);
                for (&i, digest) in ended.iter().zip(&digests) {
                    consume(digest);
                    hashers[i].reset();
                }
            }, consume)
        }
        Algorithm::Sha256 => each_interleaved(key, input, iterations, Sha256::new, per_piece(|h: &mut Sha256, p| sha2::Digest::update(h, p)), per_message(Sha256::new, |h| -> [u8; 32] { h.finalize().into() }), consume),
        Algorithm::Sha256Ring => {
            let new = || ring::digest::Context::new(&ring::digest::SHA256);
            each_interleaved(key, input, iterations, new, per_piece(|h: &mut ring::digest::Context, p| h.update(p)), per_message(new, |h| {
                let mut digest = [0u8; 32];
                digest.copy_from_slice(h.finish().as_ref());
                digest
            }), consume)
        }
        Algorithm::Sha256CommonCrypto => each_interleaved(key, input, iterations, common_crypto::Sha256State::new, per_piece(|h: &mut common_crypto::Sha256State, p| h.update(p)), per_message(common_crypto::Sha256State::new, |h| h.finish()), consume),
        Algorithm::Sha1Dc => each_interleaved(key, input, iterations, sha1_checked::Sha1::new, per_piece(|h: &mut sha1_checked::Sha1, p| h.update(p)), per_message(sha1_checked::Sha1::new, |h| {
            let mut digest = [0u8; 20];
            digest.copy_from_slice(h.try_finalize().hash());
            digest
        }), consume),
        Algorithm::Sha3_256 => each_interleaved(key, input, iterations, sha3::Sha3_256::new, per_piece(|h: &mut sha3::Sha3_256, p| sha3::Digest::update(h, p)), per_message(sha3::Sha3_256::new, |h| -> [u8; 32] { h.finalize().into() }), consume),
        Algorithm::Blake3Commonware => unreachable!("BLAKE3 commonware hashes batches alone"),
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
fn per_message<S, D: AsRef<[u8]>>(new: impl Fn() -> S, finish: impl Fn(S) -> D) -> impl FnMut(&mut [S], &[usize], &mut dyn FnMut(&[u8])) {
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
    iterations: usize,
    new: impl Fn() -> S,
    update: impl Fn(&mut [S], &[(usize, &[u8])]),
    mut finish: impl FnMut(&mut [S], &[usize], &mut dyn FnMut(&[u8])),
    mut consume: impl FnMut(&[u8]),
) {
    let spec = &INTERLEAVED;
    let longest = spec.pieces.iter().copied().max().unwrap();
    assert!(input.len() >= spec.chunk && input.len() > longest, "the pieces come from a source longer than a piece");
    assert!(TURN <= spec.open, "a turn holds at most one piece of each open message");
    let kept = SCHEDULES.with(|all| {
        let mut all = all.borrow_mut();
        all.iter().position(|(k, _)| *k == key).map(|i| all.swap_remove(i).1)
    });
    let mut schedule: Schedule<S> = match kept {
        Some(any) => *any.downcast().expect("one schedule type per contender"),
        None => Schedule {
            states: (0..spec.open).map(|_| new()).collect(),
            left: (0..spec.open).map(|n| spec.messages[n % spec.messages.len()]).collect(),
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
        let mut budget = spec.chunk;
        while budget > 0 {
            // A turn: up to TURN pieces, each read into its own buffer.
            turn.clear();
            ended.clear();
            let mut at = 0;
            while turn.len() < TURN && budget > 0 {
                let slot = schedule.piece % spec.open;
                let len = spec.pieces[schedule.piece % spec.pieces.len()].min(schedule.left[slot]).min(budget);
                schedule.piece += 1;
                if schedule.source + len > input.len() {
                    schedule.source = 0;
                }
                buffer[at..at + len].copy_from_slice(&black_box(input)[schedule.source..schedule.source + len]);
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
            let pieces: Vec<(usize, &[u8])> = turn.iter().map(|&(slot, at, len)| (slot, &buffer[at..at + len])).collect();
            update(&mut schedule.states, black_box(&pieces));
            finish(&mut schedule.states, &ended, &mut consume);
        }
    }
    STREAM_BUFFER.with(|kept| *kept.borrow_mut() = buffer);
    SCHEDULES.with(|all| all.borrow_mut().push((key, Box::new(schedule))));
}

/*
 * The continuous messages use case: `iterations` messages, each a copy of
 * `input` read into a buffer of the program's (a memory copy, the
 * cheapest read) in pieces of up to PIECE_LEN, then hashed, its digest
 * into `consume`. Every contender but the servil fork's multithreaded one
 * hashes each message after reading it, one call for a message of up to
 * PIECE_LEN, its incremental API per piece for a longer one, so reading
 * and hashing take turns; the fork's queue takes the program's buffers
 * and hashes while the next are read.
 */
fn hash_continuous_messages(algorithm: Algorithm, input: &[u8], iterations: usize, mut consume: impl FnMut(&[u8])) {
    if algorithm == Algorithm::Blake3ServilMt {
        return queue_messages(input, iterations, consume);
    }
    if input.len() > PIECE_LEN {
        return hash_stream(algorithm, input, iterations, consume);
    }
    let one = Point { label: "", bytes: input.len(), messages: 1, use_case: UseCase::OneMessage };
    let mut buffers = take_buffers(1, input.len());
    for _ in 0..iterations {
        let buffer = &mut buffers[0];
        buffer.clear();
        buffer.extend_from_slice(black_box(input));
        hash_in_memory(algorithm, buffer, one, 1, &mut consume);
    }
    keep_buffers(1, input.len(), buffers);
}

/*
 * The continuous batches use case: `iterations` batches, each a copy of
 * `input` read into a buffer of the program's, then hashed, the batch's
 * digests into `consume`. Every contender but the servil fork's
 * multithreaded one hashes each batch after reading it, as it hashes a
 * batch in memory (hash_batch); the fork's queue of fixed-length messages
 * takes the program's buffers and hashes while the next are read.
 */
fn hash_continuous_batches(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize, mut consume: impl FnMut(&[u8])) {
    if algorithm == Algorithm::Blake3ServilMt {
        return queue_batches(input, point.messages, iterations, consume);
    }
    let batch = Point { use_case: UseCase::ManyMessages, ..point };
    let mut buffers = take_buffers(1, input.len());
    for _ in 0..iterations {
        let buffer = &mut buffers[0];
        buffer.clear();
        buffer.extend_from_slice(black_box(input));
        hash_in_memory(algorithm, buffer, batch, 1, &mut consume);
    }
    keep_buffers(1, input.len(), buffers);
}

/// The producer lends a whole message or batch for one synchronous call.
/// Each read is timed; the kept buffer and digest space avoid allocation
/// after calibration, just as the queue's producer does.
fn hash_lent(algorithm: Algorithm, input: &[u8], point: Point, iterations: usize, mut consume: impl FnMut(&[u8])) {
    let in_memory = Point { use_case: if point.use_case.batch() { UseCase::ManyMessages } else { UseCase::OneMessage }, ..point };
    let mut buffers = take_buffers(1, input.len());
    for _ in 0..iterations {
        let buffer = &mut buffers[0];
        buffer.clear();
        buffer.extend_from_slice(black_box(input));
        hash_in_memory(algorithm, buffer, in_memory, 1, &mut consume);
    }
    keep_buffers(1, input.len(), buffers);
}

thread_local! {
    /*
     * The continuous use cases' read buffers, kept from one sample to the
     * next on each thread, a set per count and length: a fresh allocation
     * of a large buffer would put its page faults inside the timed sample,
     * and one set shared by every cell had each cell free or grow the
     * last one's inside its own sample (SHA-256's 1 KiB messages took 2-5%
     * more cycles per byte after the fork's queue's 1024 buffers,
     * September 28, 2026).
     */
    static INPUT_BUFFERS: std::cell::RefCell<std::collections::HashMap<(usize, usize), Vec<Vec<u8>>>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `count` empty buffers with room for `len` bytes, this thread's kept set
/// for the pair (made on first use); give them back with keep_buffers.
fn take_buffers(count: usize, len: usize) -> Vec<Vec<u8>> {
    let mut kept = INPUT_BUFFERS.with(|kept| kept.borrow_mut().remove(&(count, len))).unwrap_or_default();
    assert!(kept.len() <= count, "a kept set holds the buffers taken for it");
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

fn keep_buffers(count: usize, len: usize, buffers: Vec<Vec<u8>>) {
    INPUT_BUFFERS.with(|kept| kept.borrow_mut().insert((count, len), buffers));
}

/*
 * How many buffers a program keeps in flight through the fork's queue
 * (FROZEN.md): enough to cover the queue's round trip, so that the queue's
 * throughput is the hashing's (Little's law: in flight = rate x round
 * trip): about IN_FLIGHT_BYTES in all, or IN_FLIGHT_BUFFERS buffers,
 * whichever is fewer, and two at least, so one is read while another is
 * hashed.
 */
const IN_FLIGHT_BYTES: usize = 1 << 20;
const IN_FLIGHT_BUFFERS: usize = 1024;

fn in_flight(buffer_len: usize) -> usize {
    (IN_FLIGHT_BYTES / buffer_len.max(1)).clamp(2, IN_FLIGHT_BUFFERS)
}

/*
 * The program's side of the queue in the continuous use cases, made once
 * per cell on each thread and kept from sample to sample, as a program
 * makes one queue and uses it for its life: the queue, and a bounded
 * channel (std::sync::mpsc::sync_channel, a ring allocated when it is
 * made) through which the handler hands the buffers and digests back to
 * the program's thread, with room for everything that can be waiting in
 * it, so a send never waits. After the cell's first sample nothing in the
 * program or the queue allocates.
 */
struct Returns<Q, T> {
    queue: Q,
    returned: std::sync::mpsc::Receiver<T>,
}

/// Room for every return in flight: `count` buffers, and for pieces a
/// digest per message, a message holding two pieces or more.
fn returns_room(count: usize) -> usize {
    2 * count + 1
}

/// What the continuous messages' handler hands back: a free buffer, a
/// digest, or both.
enum Back {
    Buffer(Vec<u8>),
    Digest(blake3_servil::Hash),
    Both(Vec<u8>, blake3_servil::Hash),
}

struct MessagesBack(std::sync::mpsc::SyncSender<Back>);

impl blake3_servil::MessageHandler for MessagesBack {
    type Buffer = Vec<u8>;
    fn hashed(&mut self, buffer: Vec<u8>, hash: blake3_servil::Hash) {
        self.0.try_send(Back::Both(buffer, hash)).expect("the returns have room for every buffer in flight");
    }
}

impl blake3_servil::PieceHandler for MessagesBack {
    type Buffer = Vec<u8>;
    fn piece_done(&mut self, buffer: Vec<u8>) {
        self.0.try_send(Back::Buffer(buffer)).expect("the returns have room for every buffer in flight");
    }
    fn finished(&mut self, hash: blake3_servil::Hash) {
        self.0.try_send(Back::Digest(hash)).expect("the returns have room for every digest in flight");
    }
}

struct BatchesBack(std::sync::mpsc::SyncSender<(Vec<u8>, Vec<[u8; 32]>)>);

impl blake3_servil::FixedHandler for BatchesBack {
    type Buffer = Vec<u8>;
    type Digests = Vec<[u8; 32]>;
    fn hashed(&mut self, buffer: Vec<u8>, digests: Vec<[u8; 32]>) {
        self.0.try_send((buffer, digests)).expect("the returns have room for every buffer in flight");
    }
}

type MessageQueue = Returns<blake3_servil::Queue<MessagesBack, blake3_servil::shape::Messages>, Back>;
type PieceQueue = Returns<blake3_servil::Queue<MessagesBack, blake3_servil::shape::Pieces>, Back>;
type BatchQueue = Returns<blake3_servil::Queue<BatchesBack, blake3_servil::shape::Fixed>, (Vec<u8>, Vec<[u8; 32]>)>;

thread_local! {
    /// Each continuous cell's queue and returns on this thread, by buffers
    /// in flight and buffer length (see Returns).
    static MESSAGE_QUEUES: std::cell::RefCell<std::collections::HashMap<(usize, usize), MessageQueue>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static PIECE_QUEUES: std::cell::RefCell<std::collections::HashMap<(usize, usize), PieceQueue>> = std::cell::RefCell::new(std::collections::HashMap::new());
    static BATCH_QUEUES: std::cell::RefCell<std::collections::HashMap<(usize, usize), BatchQueue>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// This thread's queue and returns for the cell, made on first use (with
/// `make`, given the returns' sending end); put it back with `keep`.
fn take_returns<Q, T>(
    kept: &'static std::thread::LocalKey<std::cell::RefCell<std::collections::HashMap<(usize, usize), Returns<Q, T>>>>,
    key: (usize, usize),
    make: impl FnOnce(std::sync::mpsc::SyncSender<T>) -> Q,
) -> Returns<Q, T> {
    kept.with(|kept| kept.borrow_mut().remove(&key)).unwrap_or_else(|| {
        let (sender, returned) = std::sync::mpsc::sync_channel(returns_room(key.0));
        Returns { queue: make(sender), returned }
    })
}

/*
 * `iterations` messages of `input` through the fork's queue: the program keeps in_flight buffers of up to
 * PIECE_LEN, reads each message into free ones (a message of up to
 * PIECE_LEN into one, through Queue::messages; a longer one piece by
 * piece, through Queue::pieces, which starts the next message after each
 * finish), submits them, and gets them back through the handler and its
 * returns (see Returns). It waits on the returns when it has no free
 * buffer (the program's choice; the queue never blocks), and at the end
 * for every digest and buffer still in flight.
 */
fn queue_messages(input: &[u8], iterations: usize, mut consume: impl FnMut(&[u8])) {
    let piece_len = input.len().min(PIECE_LEN);
    let count = in_flight(piece_len);
    let key = (count, piece_len);
    let mut free = take_buffers(count, piece_len);
    let mut digests = 0;
    /* One return taken: a free buffer, a digest, or both. */
    let mut back = |returned: &std::sync::mpsc::Receiver<Back>, free: &mut Vec<Vec<u8>>, digests: &mut usize| match returned.recv().expect("the queue returns everything") {
        Back::Buffer(buffer) => free.push(buffer),
        Back::Digest(hash) => {
            consume(hash.as_bytes());
            *digests += 1;
        }
        Back::Both(buffer, hash) => {
            consume(hash.as_bytes());
            *digests += 1;
            free.push(buffer);
        }
    };
    let mut fill = |returned: &std::sync::mpsc::Receiver<Back>, free: &mut Vec<Vec<u8>>, digests: &mut usize, piece: &[u8]| -> Vec<u8> {
        while free.is_empty() {
            back(returned, free, digests);
        }
        let mut buffer = free.pop().unwrap();
        buffer.clear();
        buffer.extend_from_slice(piece);
        buffer
    };
    if input.len() <= PIECE_LEN {
        let returns = take_returns(&MESSAGE_QUEUES, key, |sender| {
            blake3_servil::Queue::messages(blake3_servil::Mode::Hash, MessagesBack(sender))
        });
        for _ in 0..iterations {
            returns.queue.submit(fill(&returns.returned, &mut free, &mut digests, black_box(input)));
        }
        /* Every digest delivered, every buffer free. */
        while digests < iterations || free.len() < count {
            back(&returns.returned, &mut free, &mut digests);
        }
        MESSAGE_QUEUES.with(|kept| kept.borrow_mut().insert(key, returns));
    } else {
        let returns = take_returns(&PIECE_QUEUES, key, |sender| {
            blake3_servil::Queue::pieces(blake3_servil::Mode::Hash, MessagesBack(sender))
        });
        for _ in 0..iterations {
            for piece in black_box(input).chunks(PIECE_LEN) {
                returns.queue.submit(fill(&returns.returned, &mut free, &mut digests, piece));
            }
            returns.queue.finish();
        }
        while digests < iterations || free.len() < count {
            back(&returns.returned, &mut free, &mut digests);
        }
        PIECE_QUEUES.with(|kept| kept.borrow_mut().insert(key, returns));
    }
    keep_buffers(count, piece_len, free);
}

/*
 * `iterations` batches of `input`'s `messages` 64-byte messages through
 * the fork's queue of fixed-length messages: the program keeps in_flight buffers, each with its digests' space,
 * reads each batch into a free one, submits it, and gets both back
 * through the handler and its returns (see Returns); it waits on the
 * returns when it has no free buffer, and at the end for every buffer
 * still in flight.
 */
fn queue_batches(input: &[u8], messages: usize, iterations: usize, mut consume: impl FnMut(&[u8])) {
    assert_eq!(input.len(), messages * MESSAGE_LEN, "a batch is whole 64-byte messages");
    let count = in_flight(input.len());
    let key = (count, input.len());
    let returns = take_returns(&BATCH_QUEUES, key, |sender| {
        blake3_servil::Queue::fixed(MESSAGE_LEN, blake3_servil::Mode::Hash, BatchesBack(sender))
    });
    let mut free = BATCH_PAIRS.with(|kept| kept.borrow_mut().remove(&key)).unwrap_or_else(|| {
        (0..count).map(|_| (written(input.len(), 1u8), written(messages, [1u8; 32]))).collect()
    });
    for _ in 0..iterations {
        let (mut buffer, digests) = match free.pop() {
            Some(pair) => pair,
            None => {
                let (buffer, digests) = returns.returned.recv().expect("a buffer comes back");
                consume(digests.as_flattened());
                (buffer, digests)
            }
        };
        buffer.clear();
        buffer.extend_from_slice(black_box(input));
        returns.queue.submit(buffer, digests);
    }
    while free.len() < count {
        let (buffer, digests) = returns.returned.recv().expect("every buffer comes back");
        consume(digests.as_flattened());
        free.push((buffer, digests));
    }
    BATCH_QUEUES.with(|kept| kept.borrow_mut().insert(key, returns));
    BATCH_PAIRS.with(|kept| kept.borrow_mut().insert(key, free));
}

type BatchPairs = Vec<(Vec<u8>, Vec<[u8; 32]>)>;
thread_local! {
    /// Keep the paired buffers and their descriptor vector together. Pairing
    /// and unzipping anew would allocate three vectors in every sample.
    static BATCH_PAIRS: std::cell::RefCell<std::collections::HashMap<(usize, usize), BatchPairs>> = std::cell::RefCell::new(std::collections::HashMap::new());
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
 * What the benchmark asks of the servil fork, by contender and use case:
 * the fork's call and its usage pattern. FROZEN.md lists the same, with the
 * reasons; a test compares them, so a change here needs FROZEN.md changed,
 * with Zooko's decision. Keep the dispatch (hash_batch) calling exactly
 * these.
 */
#[cfg(test)]
const SERVIL_CALLS: [(Algorithm, UseCase, &str); 22] = [
    (Algorithm::Blake3ServilSt, UseCase::OneMessage, "hash(input), each call after other work"),
    (Algorithm::Blake3ServilSt, UseCase::ManyMessages, "hash_many(batch, 64, out), the padded batch contract, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::OneMessage, "hash_multithreaded(input), each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ManyMessages, "hash_many_multithreaded(batch, 64, out), the padded batch contract, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousMessages, "Queue::messages(Mode::Hash) for messages of up to 64 KiB, Queue::pieces(Mode::Hash) in 64 KiB pieces for longer ones, one message after another, each read into free buffers of the program's, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousBatches, "Queue::fixed(64, Mode::Hash), one batch after another, each read into a free buffer of the program's, submitted with its digests' space, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept"),
    (Algorithm::Blake3ServilSt, UseCase::LentMessages, "hash(input), one message after another, each read into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::Interleaved, "a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; Hasher::update_each per turn, then Hasher::finalize_each for the messages that ended in it, each buffer lent until the calls return"),
    (Algorithm::Blake3ServilSt, UseCase::Collection, "hash_each_with(Mode::Hash, items, out), every item of the collection in one call, in memory"),
    (Algorithm::Blake3ServilSt, UseCase::Outboard, "outboard_with(Mode::Hash, message), messages one after another, each written into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::Verify, "Verifier::new(Mode::Hash, hash, len), then Verifier::update per piece, messages one after another, each received as its encoding (bao-tree's pre-order, 16 KiB groups) in pieces of up to 64 KiB read into a kept buffer, each verified group copied into the message's kept buffer"),
    (Algorithm::Blake3ServilSt, UseCase::LentBatches, "hash_many(batch, 64, out), the padded batch contract, batches one after another, each read into a kept buffer and lent with kept digests until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::LentMessages, "hash_multithreaded(input), one message after another, each read into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::Interleaved, "a Hasher per open message, 256 messages open at once, the pieces arriving in turns of 64, each read into its own kept buffer; Hasher::update_each_multithreaded per turn, then Hasher::finalize_each for the messages that ended in it, each buffer lent until the calls return"),
    (Algorithm::Blake3ServilMt, UseCase::Collection, "hash_each_multithreaded_with(Mode::Hash, items, out), every item of the collection in one call, in memory"),
    (Algorithm::Blake3ServilMt, UseCase::Outboard, "outboard_multithreaded_with(Mode::Hash, message), messages one after another, each written into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::Verify, "Verifier::new(Mode::Hash, hash, len), then Verifier::update per piece (it has no multithreaded form), messages one after another, each received as its encoding (bao-tree's pre-order, 16 KiB groups) in pieces of up to 64 KiB read into a kept buffer, each verified group copied into the message's kept buffer"),
    (Algorithm::Blake3ServilMt, UseCase::LentBatches, "hash_many_multithreaded(batch, 64, out), the padded batch contract, batches one after another, each read into a kept buffer and lent with kept digests until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::IdleOneMessage, "hash(input), each call after idling"),
    (Algorithm::Blake3ServilSt, UseCase::IdleManyMessages, "hash_many(batch, 64, out), the padded batch contract, each call after idling"),
    (Algorithm::Blake3ServilMt, UseCase::IdleOneMessage, "hash_multithreaded(input), each call after idling"),
    (Algorithm::Blake3ServilMt, UseCase::IdleManyMessages, "hash_many_multithreaded(batch, 64, out), the padded batch contract, each call after idling"),
];

/// The frozen contract as text, from the code's own tables: FROZEN.md's
/// block must read the same (the test frozen_contract_matches_frozen_md).
#[cfg(test)]
fn frozen_contract() -> String {
    let mut text = String::new();
    for use_case in UseCase::ALL {
        let labels: Vec<&str> = POINTS.iter().filter(|point| point.use_case == use_case).map(|point| point.label).collect();
        text += &format!("use case {use_case:?}: {}\n", labels.join(", "));
    }
    let keys = |scenarios: &[Scenario]| scenarios.iter().map(|scenario| scenario.key()).collect::<Vec<_>>().join(", ");
    text += &format!("scenarios: {}\n", keys(&Scenario::ALL));
    let shared: Vec<String> = UseCase::ALL.into_iter().filter(|&use_case| Scenario::Shared.measures(use_case)).map(|use_case| format!("{use_case:?}")).collect();
    text += &format!("shared measures: {}\n", shared.join(", "));
    for (algorithm, use_case, call) in SERVIL_CALLS {
        assert!(algorithm.takes_part(use_case), "{} takes part in {use_case:?}", algorithm.key());
        text += &format!("{} {use_case:?}: {call}\n", algorithm.key());
    }
    text
}

/// `iterations` passes over the batch through one of the fork's batch entry
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
fn blake3_batch(input: &[u8], message_len: usize, iterations: usize, consume: impl FnMut(&[u8])) {
    match message_len {
        MESSAGE_LEN => blake3_batch_of::<MESSAGE_LEN>(input, iterations, consume),
        other => panic!("no blake3 batch of {other}-byte messages"),
    }
}

#[inline(always)]
fn blake3_batch_of<const N: usize>(input: &[u8], iterations: usize, mut consume: impl FnMut(&[u8])) {
    /* BLAKE3's IV and flags, private in the crate. */
    const IV: [u32; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];
    const CHUNK_START: u8 = 1 << 0;
    const CHUNK_END: u8 = 1 << 1;
    const ROOT: u8 = 1 << 3;
    let (messages, rest) = input.as_chunks::<N>();
    assert!(rest.is_empty(), "a batch is whole {N}-byte messages");
    let platform = blake3::platform::Platform::detect();
    let mut digests = take_batch_digests(messages.len());
    for _ in 0..iterations {
        for (group, out) in black_box(messages).chunks(16).zip(digests[..messages.len()].chunks_mut(16)) {
            let mut table = [&group[0]; 16];
            for (slot, message) in table.iter_mut().zip(group) {
                *slot = message;
            }
            platform.hash_many::<N>(&table[..group.len()], &IV, 0, blake3::IncrementCounter::No, 0, CHUNK_START, CHUNK_END | ROOT, out.as_flattened_mut());
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
    /// Each copy's elapsed time and counts; None while a copy is still running.
    finished: std::sync::Mutex<[Option<DuoCopy>; 2]>,
    done: std::sync::Condvar,
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
    preparation: Option<clocks::Batch>,
}

#[derive(Clone, Copy)]
struct DuoJob {
    algorithm: Algorithm,
    inputs: [*const [u8]; 2],
    point: Point,
    iterations: usize,
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

    /// Run `iterations` of `algorithm` on both threads at once, copy 0 over
    /// `input` and copy 1 over `other`; returns each copy's time from its
    /// own start and its counts. The sample is the later finish.
    fn run(&self, algorithm: Algorithm, input: &[u8], other: &[u8], point: Point, iterations: usize) -> [DuoCopy; 2] {
        use std::sync::atomic::Ordering;
        assert_eq!(input.len(), other.len(), "the two copies hash inputs of one size");
        {
            let mut job = self.job.lock().unwrap();
            assert!(job.is_none(), "one duo job at a time");
            *job = Some(DuoJob { algorithm, inputs: [input, other], point, iterations, taken: 0 });
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
        let copies = finished.map(|copy| copy.unwrap());
        *finished = [None, None];
        copies
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
            self.ready.fetch_add(1, Ordering::AcqRel);
            while self.generation.load(Ordering::Acquire) == seen {
                std::thread::yield_now();
            }
            seen = self.generation.load(Ordering::Acquire);
            // Sound: run() holds the borrows until both finishes are read.
            let sample = take_sample(job.algorithm, unsafe { &*job.inputs[copy] }, job.point, job.iterations);
            let mut finished = self.finished.lock().unwrap();
            finished[copy] = Some(sample);
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
            let mut context = Context { count: [0; 2], hash: [0; 8], wbuf: [0; 16] };
            // Safe: `context` is a valid CC_SHA256_CTX.
            assert_eq!(unsafe { CC_SHA256_Init(&mut context) }, 1, "CC_SHA256_Init failed");
            Sha256State(context)
        }

        pub fn update(&mut self, piece: &[u8]) {
            let len = u32::try_from(piece.len()).expect("CC_SHA256_Update takes a 32-bit length");
            // Safe: the context is valid and `piece` holds `len` bytes.
            assert_eq!(unsafe { CC_SHA256_Update(&mut self.0, piece.as_ptr(), len) }, 1, "CC_SHA256_Update failed");
        }

        pub fn finish(mut self) -> [u8; DIGEST_LEN] {
            let mut digest = [0u8; DIGEST_LEN];
            // Safe: Final writes exactly 32 bytes to `digest`.
            assert_eq!(unsafe { CC_SHA256_Final(digest.as_mut_ptr(), &mut self.0) }, 1, "CC_SHA256_Final failed");
            digest
        }
    }

    /// One Update per piece, in order.
    pub fn sha256_pieces(pieces: super::Pieces) -> [u8; DIGEST_LEN] {
        let mut context = Context { count: [0; 2], hash: [0; 8], wbuf: [0; 16] };
        let mut digest = [0u8; DIGEST_LEN];
        // Safe: `context` is a valid CC_SHA256_CTX for every call, each
        // piece is valid for its `len` bytes, and Final writes exactly 32
        // bytes to `digest`. Each call returns 1 on success.
        unsafe {
            assert_eq!(CC_SHA256_Init(&mut context), 1, "CC_SHA256_Init failed");
            pieces(&mut |piece| {
                let len = u32::try_from(piece.len()).expect("CC_SHA256_Update takes a 32-bit length");
                assert_eq!(CC_SHA256_Update(&mut context, piece.as_ptr(), len), 1, "CC_SHA256_Update failed");
            });
            assert_eq!(CC_SHA256_Final(digest.as_mut_ptr(), &mut context), 1, "CC_SHA256_Final failed");
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
 * Optional per-sample trace for clock diagnosis: every solo sample's wall
 * nanoseconds and (on Apple) the thread's counts per core kind (the clocks
 * crate), with the round, its position in the round, the
 * contender, size, and use case; then the duo sample's later finish and
 * each copy's own time and P/E counts. One CSV line per sample interval.
 * Off unless --trace-clocks PATH is given; every clock read sits outside
 * the timed intervals.
 */
struct ClockTrace {
    lines: Vec<String>,
    path: std::path::PathBuf,
}

impl ClockTrace {
    fn new(path: std::path::PathBuf) -> Self {
        let mut lines = Vec::with_capacity(8192);
        let mut header = "round,position,contender,size_bytes,iterations,wall_ns,p_cycles,p_instructions,p_time_ns,e_cycles,e_instructions,e_time_ns,use_case,duo_ns".to_owned();
        for copy in 0..2 {
            for field in ["ns", "p_cycles", "p_instructions", "p_time_ns", "e_cycles", "e_instructions", "e_time_ns"] {
                header += &format!(",copy{copy}_{field}");
            }
        }
        /* Rows from the rounds carry both scenarios; after-idle rows one call burst, no copies. */
        header += ",scenario";
        lines.push(header);
        Self { lines, path }
    }

    fn write(&self) {
        let body = self.lines.join("\n") + "\n";
        fs::write(&self.path, body).unwrap_or_else(|error| {
            panic!("failed to write {}: {error}", self.path.display())
        });
        eprintln!("clock trace: {} samples in {}", self.lines.len() - 1, self.path.display());
    }
}

/// A trace's six count fields (P cycles, instructions, time; then E),
/// empty for a copy that took no sample and where the platform counts
/// none (Linux: no per-thread cycle counts): never zeros that read as counts.
const NO_COUNTS: &str = ",,,,,";

fn counts_csv(counts: Option<clocks::Counts>) -> String {
    let Some(c) = counts else { return NO_COUNTS.to_owned() };
    format!("{},{},{},{},{},{}", c.p.cycles, c.p.instructions, c.p.time_ns, c.e.cycles, c.e.instructions, c.e.time_ns)
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
        if cfg!(target_os = "macos") { Self::read_macos() } else { Self::read_linux() }
    }

    fn read_macos() -> Option<Power> {
        let pmset = |args: &[&str]| {
            let out = std::process::Command::new("pmset").args(args).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        Self::parse_pmset(&pmset(&["-g", "batt"])?, &pmset(&["-g"]).unwrap_or_default())
    }

    /// The state from `pmset -g batt` and `pmset -g` (the settings in use).
    fn parse_pmset(batt: &str, settings: &str) -> Option<Power> {
        let source = batt.lines().next()?.split('\'').nth(1)?.to_owned();
        let battery_percent = batt
            .split_whitespace()
            .find_map(|word| word.strip_suffix("%;").or_else(|| word.strip_suffix('%'))?.parse().ok());
        /* The settings in use: "lowpowermode 1", or on Macs with a High Power mode "powermode 0|1|2". */
        let setting = |key: &str| settings.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some(key)).then(|| words.next()?.parse::<u64>().ok()).flatten()
        });
        let (mode, low_power) = match (setting("powermode"), setting("lowpowermode")) {
            (Some(1), _) | (None, Some(1)) => (Some("Low Power Mode".to_owned()), true),
            (Some(2), _) => (Some("High Power mode".to_owned()), false),
            (Some(0), _) => (Some("automatic power mode".to_owned()), false),
            _ => (None, false),
        };
        Some(Power { on_battery: source == "Battery Power", battery_percent, mode, low_power })
    }

    fn read_linux() -> Option<Power> {
        let read = |path: std::path::PathBuf| fs::read_to_string(path).ok().map(|text| text.trim().to_owned());
        let (mut on_battery, mut battery_percent, mut any) = (false, None, false);
        for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
            let path = entry.path();
            if read(path.join("type")).as_deref() == Some("Battery") {
                any = true;
                on_battery |= read(path.join("status")).as_deref() == Some("Discharging");
                battery_percent = battery_percent.or(read(path.join("capacity")).and_then(|c| c.parse().ok()));
            } else if read(path.join("type")).as_deref() == Some("Mains") {
                any = true;
            }
        }
        let profile = read("/sys/firmware/acpi/platform_profile".into());
        if !any && profile.is_none() {
            return None;
        }
        let low_power = profile.as_deref().is_some_and(|p| p.starts_with("low-power") || p == "quiet" || p == "cool");
        Some(Power { on_battery, battery_percent, mode: profile.map(|p| format!("platform profile {p}")), low_power })
    }

    /// Whether this state can make results read slower than the machine
    /// runs on mains power.
    fn slowing(&self) -> bool {
        self.on_battery || self.low_power
    }

    /// "mains power, High Power mode", "battery power (47% charged), Low Power Mode".
    fn describe(&self) -> String {
        let mut line = if self.on_battery { "battery power".to_owned() } else { "mains power".to_owned() };
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
                format!("{} at the start, {} at the end", start.describe(), end.describe())
            }
            (Some(one), _) | (None, Some(one)) => one.describe(),
        };
        if self.power_slowing() { line + "; results may differ from a run on mains power in the normal mode" } else { line }
    }
}

/// Spread complete cycles of `orders` over `rounds`, at a point's offset.
/// Requires positive counts. Defaults have room for the whole design;
/// explicit --rounds bypasses thinning and accepts partial designs.
fn cell_wants_sample(slot: usize, rounds: usize, orders: usize) -> bool {
    assert!(rounds > 0 && orders > 0);
    let target = STEADY_SAMPLES.next_multiple_of(orders).min(rounds);
    (slot % rounds) * target % rounds < target
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
        UseCase::OneMessage | UseCase::IdleOneMessage | UseCase::ManyMessages | UseCase::IdleManyMessages | UseCase::LentMessages | UseCase::Interleaved | UseCase::Collection | UseCase::Outboard | UseCase::Verify | UseCase::LentBatches => 1,
    }
}

fn calibrate_batch(
    algorithm: Algorithm,
    input: &[u8],
    point: Point,
) -> (usize, u128) {
    let fewest = continuous_min_inputs(input, point);
    let mut iterations = fewest;
    /*
     * A cell's first calls carry one-time costs (a pool or a queue's
     * delivery thread starting, a self-test, first-touched buffers), which
     * would size its samples short: one untimed batch first (Devon Jonte,
     * bench-hashes#4: the queue's 64 B samples 154-199 us against the 1 ms
     * target; the Mac's 176 us).
     */
    run_batch(algorithm, input, point, fewest);

    loop {
        let started = clocks::now();
        run_batch(algorithm, input, point, iterations);
        let elapsed_ns = u128::from(clocks::since_ns(started));

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
            let scaled = (
                iterations as u128 * TARGET_SAMPLE_NS
                    + elapsed_ns / 2
            ) / elapsed_ns;

            let scaled = scaled.max(fewest as u128);

            assert!(
                scaled <= usize::MAX as u128,
                "calibrated iteration count must fit in usize"
            );

            return (scaled as usize, elapsed_ns / iterations as u128);
        }

        let growth =
            (CALIBRATION_PROBE_NS + elapsed_ns - 1) / elapsed_ns;

        assert!(
            growth >= 2,
            "a sub-probe measurement must require a larger batch"
        );

        iterations = iterations
            .checked_mul(
                usize::try_from(growth)
                    .expect("calibration growth factor must fit in usize"),
            )
            .expect("calibration iteration count overflowed");
    }
}


/// A cell's summary from its samples: their mean (clocks::summary), kept
/// exact for display.
fn summarize_measured(samples: &[Measured]) -> Statistics {
    assert!(!samples.is_empty(), "a cell has at least one sample");
    let exact = ExactMean::of(samples);
    Statistics {
        mean: exact.fixed(),
        exact_mean: Some(exact),
    }
}

/*
 * One code path a contender uses for a range of input sizes. `first` is the
 * smallest input in bytes that takes this path; a contender's kernels are
 * listed in ascending order of `first`, the first starting at 0.
 */
#[derive(Clone)]
struct Kernel {
    first: usize,
    /// Short name for the report and the hover panel, e.g. "NEON hash_many".
    name: String,
}

/*
 * A contender's code paths by input size, with the platform name for the
 * report header. `kernels` is non-empty, ascending in `first`, and starts
 * at 0.
 */
struct Kernels {
    platform: &'static str,
    kernels: Vec<Kernel>,
}

impl Kernels {
    fn new(platform: &'static str, kernels: Vec<Kernel>) -> Self {
        assert!(!kernels.is_empty(), "a contender has at least one kernel");
        assert_eq!(kernels[0].first, 0, "the first kernel covers the smallest inputs");
        assert!(
            kernels.windows(2).all(|pair| pair[0].first < pair[1].first),
            "kernels ascend in their first input size"
        );
        Self { platform, kernels }
    }

    /// The kernels that start at `bytes` or below: those an input of at
    /// most `bytes` can reach.
    fn up_to(mut self, bytes: usize) -> Self {
        self.kernels.retain(|kernel| kernel.first <= bytes);
        self
    }

    /// Index of the kernel for an input of `bytes` (a batch's bytes in all).
    fn kernel_index_for(&self, bytes: usize) -> usize {
        self.kernels
            .iter()
            .rposition(|kernel| bytes >= kernel.first)
            .expect("the first kernel starts at 0")
    }
}

/*
 * Code paths of the crates.io blake3 crate, from BLAKE3 v1.8.7's
 * src/platform.rs and the SIMD hash_many fallback chains. One chunk is
 * 1024 bytes; hash_many batches whole chunks and hands leftovers below the
 * SIMD degree to the single-chunk path.
 */
fn detect_blake3_kernels() -> Kernels {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let (platform, one, wide, degree) = if std::arch::is_x86_feature_detected!("avx512f")
            && std::arch::is_x86_feature_detected!("avx512vl")
        {
            ("AVX-512", "AVX-512 vectors, one chunk at a time", "AVX-512 vectors, sixteen chunks at a time", 16)
        } else if std::arch::is_x86_feature_detected!("avx2") {
            ("AVX2", "SSE4.1 vectors, one chunk at a time", "AVX2 vectors, eight chunks at a time", 8)
        } else if std::arch::is_x86_feature_detected!("sse4.1") {
            ("SSE4.1", "SSE4.1 vectors, one chunk at a time", "SSE4.1 vectors, four chunks at a time", 4)
        } else if std::arch::is_x86_feature_detected!("sse2") {
            ("SSE2", "SSE2 vectors, one chunk at a time", "SSE2 vectors, four chunks at a time", 4)
        } else {
            ("portable", "portable code, one chunk at a time", "portable code", 1)
        };
        let mut kernels = vec![Kernel {
            first: 0,
            name: one.to_owned(),
        }];
        if degree > 4 {
            kernels.push(Kernel {
                first: 4 * 1024,
                name: "SSE4.1 vectors, four chunks at a time".to_owned(),
            });
        }
        if degree > 1 {
            kernels.push(Kernel {
                first: degree * 1024,
                name: wide.to_owned(),
            });
        }
        return Kernels::new(platform, kernels);
    }

    #[cfg(target_arch = "aarch64")]
    {
        return Kernels::new(
            "NEON",
            vec![
                Kernel {
                    first: 0,
                    name: "portable code, one chunk at a time".to_owned(),
                },
                Kernel {
                    first: 4 * 1024,
                    name: "NEON vectors, four chunks at a time".to_owned(),
                },
            ],
        );
    }

    #[allow(unreachable_code)]
    Kernels::new(
        "portable",
        vec![Kernel {
            first: 0,
            name: "portable code".to_owned(),
        }],
    )
}

/*
 * SHA-256 and SHA-1DC each run one code path at every size. Their kernels
 * still carry a name so the hover panel can say what produced the point.
 */
fn detect_sha256_kernels() -> Kernels {
    let name = if cfg!(target_arch = "aarch64") {
        "the CPU's SHA-256 instructions"
    } else if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
        "the CPU's SHA-256 instructions, where it has them"
    } else {
        "portable code"
    };
    Kernels::new(
        "sha2",
        vec![Kernel {
            first: 0,
            name: name.to_owned(),
        }],
    )
}

/// commonware's batch kernel: its own choice, by the same detection.
fn detect_commonware_kernels() -> Kernels {
    #[cfg(target_arch = "x86_64")]
    let name = if std::arch::is_x86_feature_detected!("avx512f") { "AVX-512, 16 messages a call" } else if std::arch::is_x86_feature_detected!("avx2") { "AVX2, 8 messages a call" } else { "blake3::hash, one message a call" };
    #[cfg(target_arch = "aarch64")]
    let name = "NEON, 4 messages a call";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let name = "blake3::hash, one message a call";
    Kernels::new("commonware", vec![Kernel { first: 0, name: name.to_owned() }])
}

fn detect_sha3_kernels() -> Kernels {
    #[cfg(target_arch = "aarch64")]
    let instructions = std::arch::is_aarch64_feature_detected!("sha3");
    #[cfg(not(target_arch = "aarch64"))]
    let instructions = false;
    Kernels::new(
        "sha3",
        vec![Kernel {
            first: 0,
            name: if instructions { "ARMv8 SHA-3 instructions".to_owned() } else { "portable code".to_owned() },
        }],
    )
}

fn detect_sha1dc_kernels() -> Kernels {
    Kernels::new(
        "sha1-checked",
        vec![Kernel {
            first: 0,
            name: "portable code with collision detection".to_owned(),
        }],
    )
}

fn detect_common_crypto_kernels() -> Kernels {
    Kernels::new(
        "CommonCrypto",
        vec![Kernel {
            first: 0,
            name: "Apple's library, with the CPU's SHA-256 instructions".to_owned(),
        }],
    )
}

fn detect_ring_kernels() -> Kernels {
    let name = if cfg!(target_arch = "aarch64") {
        "the CPU's SHA-256 instructions, interleaved"
    } else if cfg!(any(target_arch = "x86", target_arch = "x86_64")) {
        "the CPU's SHA-256 instructions, or vector code"
    } else {
        "portable code"
    };
    Kernels::new(
        "ring",
        vec![Kernel {
            first: 0,
            name: name.to_owned(),
        }],
    )
}

/*
 * The servil fork describes its own kernels: kernel_report() and
 * kernel_report_multithreaded() come from the same run-time detection
 * its hash functions use, so the report describes what was measured.
 */
fn servil_kernels(report: blake3_servil::KernelReport) -> Kernels {
    let kernels = report.kernels.iter().map(|kernel| Kernel { first: kernel.from_len, name: kernel.name.to_owned() }).collect();
    Kernels::new(report.platform, kernels)
}

/// The code paths a contender runs in a use case, by the point's bytes.
/// The contender must take part in the use case.
fn detect_kernels(algorithm: Algorithm, use_case: UseCase) -> Kernels {
    assert!(algorithm.takes_part(use_case), "{} takes no part in {use_case:?}", algorithm.name());
    // Queue and incremental MT APIs have no kernel-report entry point.
    // Their known API boundaries remain useful; their schedules stay
    // explicitly unreported instead of inheriting one-shot thresholds.
    if use_case == UseCase::Verify {
        let api = if algorithm == Algorithm::Blake3 { "bao-tree decode_ranges" } else { "Verifier::update" };
        return Kernels::new("API (kernel unreported)", vec![Kernel { first: 0, name: api.to_owned() }]);
    }
    if use_case == UseCase::Outboard {
        let api = match algorithm {
            Algorithm::Blake3 => "bao-tree PreOrderMemOutboard::create",
            Algorithm::Blake3ServilMt => "outboard_multithreaded_with",
            _ => "outboard_with",
        };
        return Kernels::new("API (kernel unreported)", vec![Kernel { first: 0, name: api.to_owned() }]);
    }
    if algorithm == Algorithm::Blake3ServilSt && use_case == UseCase::Collection {
        return Kernels::new("API (kernel unreported)", vec![Kernel { first: 0, name: "hash_each_with".to_owned() }]);
    }
    if algorithm == Algorithm::Blake3ServilSt && use_case == UseCase::Interleaved {
        return Kernels::new("API (kernel unreported)", vec![Kernel { first: 0, name: "Hasher::update_each".to_owned() }]);
    }
    if algorithm == Algorithm::Blake3ServilMt {
        let kernel = |first, api: &str| Kernel { first, name: api.to_owned() };
        let kernels = match use_case {
            UseCase::ContinuousMessages => Some(vec![kernel(0, "Queue::messages"), kernel(PIECE_LEN + 1, "Queue::pieces")]),
            UseCase::ContinuousBatches => Some(vec![kernel(0, "Queue::fixed")]),
            UseCase::Interleaved => Some(vec![kernel(0, "Hasher::update_each_multithreaded")]),
            UseCase::Collection => Some(vec![kernel(0, "hash_each_multithreaded_with")]),
            _ => None,
        };
        if let Some(kernels) = kernels {
            return Kernels::new("API (kernel unreported)", kernels);
        }
    }
    let one_message = match algorithm {
        Algorithm::Blake3 => detect_blake3_kernels(),
        Algorithm::Sha256 => detect_sha256_kernels(),
        Algorithm::Sha1Dc => detect_sha1dc_kernels(),
        Algorithm::Sha3_256 => detect_sha3_kernels(),
        Algorithm::Blake3ServilSt => servil_kernels(blake3_servil::kernel_report()),
        Algorithm::Sha256CommonCrypto => detect_common_crypto_kernels(),
        Algorithm::Sha256Ring => detect_ring_kernels(),
        Algorithm::Blake3Rayon => detect_blake3_rayon_kernels(),
        Algorithm::Blake3ServilMt => servil_kernels(blake3_servil::kernel_report_multithreaded()),
        Algorithm::Blake3Commonware => detect_commonware_kernels(),
    };
    match use_case {
        /* An idle use case makes its twin's call. */
        UseCase::IdleOneMessage | UseCase::IdleManyMessages => detect_kernels(algorithm, use_case.call()),
        UseCase::OneMessage | UseCase::LentMessages | UseCase::Collection | UseCase::Outboard | UseCase::Verify => one_message,
        /* Pieces run the one-message kernels, so those that start past the longest piece never run. */
        UseCase::Interleaved => one_message.up_to(16 * 1024),
        /* A message of up to PIECE_LEN is one call's input; a longer one arrives in pieces. */
        UseCase::ContinuousMessages => one_message.up_to(PIECE_LEN),
        UseCase::ContinuousBatches | UseCase::LentBatches => detect_kernels(algorithm, UseCase::ManyMessages),
        UseCase::ManyMessages if algorithm == Algorithm::Blake3 => {
            detect_blake3_many_kernels(use_case.message_len())
        }
        UseCase::ManyMessages if algorithm == Algorithm::Blake3ServilSt => {
            servil_kernels(blake3_servil::kernel_report_many(use_case.message_len()))
        }
        UseCase::ManyMessages if algorithm == Algorithm::Blake3ServilMt => {
            servil_kernels(blake3_servil::kernel_report_many_multithreaded(use_case.message_len()))
        }
        UseCase::ManyMessages => {
            /* One call per message: the kernel for the message's length, whatever the batch size. */
            let kernel = &one_message.kernels[one_message.kernel_index_for(use_case.message_len())];
            Kernels::new(
                one_message.platform,
                vec![Kernel {
                    first: 0,
                    name: format!("{}, one message per call", kernel.name),
                }],
            )
        }
    }
}

/*
 * The blake3 crate's hidden batch function as blake3_batch calls it,
 * sixteen messages per call: each call hashes its messages in groups as
 * wide as the platform's widest vectors (the SIMD degree the one-message
 * table's last kernel names) and the rest one at a time.
 */
fn detect_blake3_many_kernels(message_len: usize) -> Kernels {
    let blake3 = detect_blake3_kernels();
    let degree = (blake3.kernels[blake3.kernels.len() - 1].first / 1024).max(1);
    let words = match degree {
        4 => "four",
        8 => "eight",
        16 => "sixteen",
        _ => "several",
    };
    let mut kernels = vec![Kernel {
        first: 0,
        name: "one message at a time".to_owned(),
    }];
    if degree > 1 {
        kernels.push(Kernel {
            first: degree * message_len,
            name: format!("{} vectors, {words} messages at a time", blake3.platform),
        });
    }
    Kernels::new(blake3.platform, kernels)
}

/*
 * update_rayon splits the tree with rayon::join down to the SIMD degree,
 * so any input above one SIMD width of chunks may cross threads; the
 * kernel boundary is where the crate's serial path ends.
 */
fn detect_blake3_rayon_kernels() -> Kernels {
    let single = detect_blake3_kernels();
    let degree_bytes = single.kernels.last().map(|r| r.first).unwrap_or(0).max(2 * 1024);
    Kernels::new(
        single.platform,
        vec![
            Kernel {
                first: 0,
                name: "on the calling thread".to_owned(),
            },
            Kernel {
                first: 2 * degree_bytes,
                name: "split over Rayon's threads".to_owned(),
            },
        ],
    )
}

/// The kernels a contender ran in a use case, by point: one line for a
/// contender with a single kernel, a table for one that changes kernel
/// along the axis.
fn append_kernel_report(output: &mut String, algorithm: Algorithm, use_case: UseCase) {
    let kernels = detect_kernels(algorithm, use_case);
    let mut line = format!("    {}:", algorithm.name());
    let mut previous = None;
    for point in &POINTS[use_case.points()] {
        let kernel_index = kernels.kernel_index_for(point.bytes);
        if previous != Some(kernel_index) {
            let separator = if previous.is_some() { ";" } else { "" };
            write!(line, "{separator} {} {}", point.label, kernels.kernels[kernel_index].name).unwrap();
            previous = Some(kernel_index);
        }
    }
    writeln!(output, "{line}").unwrap();
}

/*
 * Every sample, for tools that do their own statistics (the fork's
 * performance-regression check reads this file). Lines starting with '#'
 * carry the provenance and machine identity as `key: value`; then one row
 * per measured cell and scenario: contender key, scenario (`solo` or
 * `shared`), use case, point label, the unit a sample is per (`B` or
 * `msg`), and the samples as measured, comma-separated, each `ns/units`
 * (the nanoseconds the clock gave over the units they covered), in the
 * order taken (shared: the two copies of each interval in turn).
 */
/// The samples file's first line and its column row, which the reader
/// (read_samples) requires: a file in another format is read with the
/// tools of the commit that wrote it.
const SAMPLES_VERSION: &str = "# bench-hashes samples v4";
const SAMPLES_COLUMNS: &str = "contender\tscenario\tuse_case\tpoint\tunit\tns/units\tstart ms";

/// A samples file read back: its `# load:` line (the run's own verdict,
/// "quiet: ..." or "busy: ..."), and each cell's samples as measured,
/// keyed "contender|scenario|use_case|point", in the file's order.
struct SamplesFile {
    /// Its `# key: value` lines, in order.
    headers: Vec<(String, String)>,
    load: String,
    /// Its `# power:` line.
    power: String,
    cells: Vec<(String, Vec<Measured>)>,
}

fn read_samples(path: &str) -> SamplesFile {
    let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("failed to read {path}: {error}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(SAMPLES_VERSION), "{path}: a samples file of this version begins with {SAMPLES_VERSION:?}");
    let mut load = None;
    let mut power = None;
    let mut headers = Vec::new();
    let mut columns = false;
    let mut cells = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("# load: ") {
            load = Some(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix("# power: ") {
            power = Some(rest.to_owned());
        } else if line.starts_with('#') || line.is_empty() {
            if let Some((key, value)) = line.strip_prefix("# ").and_then(|rest| rest.split_once(": ")) {
                headers.push((key.to_owned(), value.to_owned()));
            }
        } else if !columns {
            assert_eq!(line, SAMPLES_COLUMNS, "{path}: the column row");
            columns = true;
        } else {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 7, "{path}: a row of seven fields: {line}");
            let samples = fields[5].split(',').map(|sample| {
                let (ns, units) = sample.split_once('/').expect("ns/units");
                Measured::new(ns.parse().expect("ns"), units.parse().expect("units"))
            }).collect();
            let key = fields[..4].join("|");
            assert!(cells.iter().all(|(k, _)| *k != key), "{path}: a samples file holds each cell once: {key} twice");
            cells.push((key, samples));
        }
    }
    SamplesFile { headers, load: load.unwrap_or_else(|| panic!("{path}: a load line")), power: power.unwrap_or_else(|| panic!("{path}: a power line")), cells }
}

/*
 * `bench-hashes compare OLD... -- NEW...`: each side's samples files
 * pooled, every cell both sides measured compared speed with speed and
 * share with share (clocks::speeds::compare): old -> new medians, new
 * over old of the fast and of the slow speed, the slow speed's share on
 * each side. For an A/B (old new new old), compare old with new, and each
 * side with itself (its first run against its second) to see what
 * repetition alone moves.
 */
fn compare_command(arguments: &[String]) {
    let usage = "usage: bench-hashes compare OLD.tsv... -- NEW.tsv...";
    let split = arguments.iter().position(|argument| argument == "--").expect(usage);
    let (old, new) = (&arguments[..split], &arguments[split + 1..]);
    assert!(!old.is_empty() && !new.is_empty(), "{usage}");
    // Each side: every cell's mean in every run (one samples file a run).
    let side = |paths: &[String]| {
        let mut order: Vec<String> = Vec::new();
        let mut cells: std::collections::HashMap<String, Vec<u128>> = std::collections::HashMap::new();
        for path in paths {
            let file = read_samples(path);
            if !file.load.starts_with("quiet") {
                println!("{path}: {}: its samples are no evidence of speed", file.load);
            }
            for (key, samples) in file.cells {
                if !cells.contains_key(&key) {
                    order.push(key.clone());
                }
                cells.entry(key).or_default().push(ExactMean::of(&samples).fixed().0);
            }
        }
        (order, cells)
    };
    let ((order, old), (_, new)) = (side(old), side(new));
    for key in order {
        if let Some(new_means) = new.get(&key) {
            let (a, b) = (median_u128(&old[&key]), median_u128(new_means));
            let r = clocks::summary::ratio_permille(b, a);
            println!("{key}: {} -> {} ns/unit  x{}.{:03}  ({} -> {} runs)", Fixed(a).format_ns(), Fixed(b).format_ns(), r / 1000, r % 1000, old[&key].len(), new_means.len());
        }
    }
}

/// The median of some values (for an even count, the mean of the middle two).
fn median_u128(values: &[u128]) -> u128 {
    assert!(!values.is_empty(), "a median needs a value");
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]).div_ceil(2) }
}

/*
 * `bench-hashes regress OLD NEW`: whether the executable NEW is slower than
 * OLD, measured on this machine (the fork's tools/perf_regress.py builds
 * the two). REGRESS_PAIRS pairs of runs, one of each side, back to back in
 * alternating order (A B, B A, ...), each over REGRESS_POINTS, the lent
 * cells, where a program hands its buffer to a call and waits (the queue's
 * cells move 6-60% between processes of identical code, beyond what a 3%
 * check can judge), solo alone (the shared copies' 64 B cells switch
 * between states 20-30% apart per process). Each pair gives each cell one
 * ratio, new mean over old; a cell is slower or faster by
 * clocks::summary::verdict at a 3% margin. Calibrated on the Mac (fork
 * NOTES, "The regression check, calibrated"). Exit 0: no cell slower; 1: a
 * cell slower; 2: no verdict, when a run's load was busy or unobserved
 * (clocks::load).
 */
const REGRESS_SUBJECTS: [Algorithm; 2] = [Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt];
/// The lent cells' code paths: one message short, in the pool's pieces,
/// bulk on one thread; batches as members and as tasks of their own.
const REGRESS_POINTS: [&str; 5] = ["lent 64 B", "lent 64 KiB", "lent 1 MiB", "lent batch 16", "lent batch 4096"];
const REGRESS_ROUNDS: usize = 24;
const REGRESS_PAIRS: usize = 8;
/// How many times a pair runs again when one of its runs is no evidence.
const REGRESS_REPEATS: usize = 3;

/// The margin a solo cell's verdict uses.
const REGRESS_MARGIN_PERMILLE: u64 = 30;

/// One run of `exe` over the regress points: its samples file.
fn regress_run(exe: &str) -> SamplesFile {
    let directory = std::env::temp_dir().join(format!("bench-hashes-regress-{}-{}", std::process::id(), clocks::load::now_ns()));
    fs::create_dir_all(&directory).unwrap();
    let contenders: Vec<&str> = REGRESS_SUBJECTS.iter().map(|a| a.key()).collect();
    let status = std::process::Command::new(exe)
        .args(["--contenders", &contenders.join(","), "--points", &REGRESS_POINTS.join(","), "--rounds", &REGRESS_ROUNDS.to_string()])
        .current_dir(&directory)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap_or_else(|error| panic!("failed to run {exe}: {error}"));
    assert!(status.success(), "{exe} failed: {status}");
    let found: Vec<_> = fs::read_dir(directory.join("benchmark-results")).unwrap()
        .map(|entry| entry.unwrap().path().join("bench-hashes.samples.tsv")).collect();
    assert_eq!(found.len(), 1, "one samples file");
    let file = read_samples(found[0].to_str().unwrap());
    fs::remove_dir_all(&directory).unwrap();
    file
}

fn regress_command(arguments: &[String]) -> i32 {
    let [old_exe, new_exe] = arguments else { panic!("usage: bench-hashes regress OLD_EXE NEW_EXE") };
    eprintln!("regress: {new_exe} against {old_exe}, {REGRESS_PAIRS} pairs over {} points", REGRESS_POINTS.len());
    let mut unreliable = Vec::new();
    let mut powers: Vec<String> = Vec::new();
    let mut ratios: std::collections::BTreeMap<String, Vec<u64>> = std::collections::BTreeMap::new();
    // A run's means, and its load line when clocks found it no evidence
    // (other programs busy, or no load window).
    let mut means = |exe: &str| -> (std::collections::HashMap<String, u128>, Option<String>) {
        let file = regress_run(exe);
        if !powers.contains(&file.power) {
            powers.push(file.power.clone());
        }
        let busy = (!file.load.starts_with("quiet")).then(|| file.load.clone());
        (file.cells.into_iter().map(|(key, samples)| (key, ExactMean::of(&samples).fixed().0)).collect(), busy)
    };
    for pair in 0..REGRESS_PAIRS {
        let began = clocks::now();
        // A pair with a run that is no evidence runs again, in the same
        // order, up to REGRESS_REPEATS times (a process launch on macOS
        // now and then costs other programs about 1.7 CPU-seconds inside
        // a run's window: fork runner jobs 1304-1305, October 5, 2026).
        let mut attempt = 0;
        let (old, new) = loop {
            let ((old, a), (new, b)) = if pair % 2 == 0 { let o = means(old_exe); (o, means(new_exe)) } else { let n = means(new_exe); (means(old_exe), n) };
            let busy: Vec<String> = a.into_iter().chain(b).collect();
            if busy.is_empty() {
                break (old, new);
            }
            attempt += 1;
            if attempt > REGRESS_REPEATS {
                unreliable.extend(busy);
                break (old, new);
            }
            eprintln!("regress: pair {} again: {}", pair + 1, busy.join("; "));
        };
        for (key, old_mean) in &old {
            ratios.entry(key.clone()).or_default().push(clocks::summary::ratio_permille(new[key], *old_mean));
        }
        let tenths = (clocks::since_ns(began) + 50_000_000) / 100_000_000;
        eprintln!("regress: pair {} of {REGRESS_PAIRS} in {}.{} s", pair + 1, tenths / 10, tenths % 10);
    }
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
        format!("{}{}.{}%", if d < 0 { "-" } else { "+" }, d.abs() / 10, d.abs() % 10)
    };
    let mut slower = 0;
    for (key, values) in ratios.iter().filter(|(key, _)| key.split('|').nth(1) == Some("solo")) {
        let verdict = clocks::summary::verdict(values, REGRESS_MARGIN_PERMILLE);
        if verdict == clocks::summary::Verdict::Level {
            continue;
        }
        let word = if verdict == clocks::summary::Verdict::Slower { slower += 1; "SLOWER" } else { "faster" };
        let pairs: Vec<String> = values.iter().map(|&r| percent(r)).collect();
        println!("  {word} {key}: {} (median of {} pairs: {})", percent(clocks::summary::median_permille(values)), values.len(), pairs.join(" "));
    }
    if slower > 0 {
        println!("regress: REGRESSION: {slower} cells slower");
    } else {
        println!("regress: no cell slower");
    }
    println!("{power}");
    i32::from(slower > 0)
}

fn generate_samples_tsv(roster: &Roster, samples: &RunSamples, machine: &MachineMetadata, selection_note: &str) -> String {
    let mut out = String::new();
    writeln!(out, "{SAMPLES_VERSION}").unwrap();
    for (key, value) in [
        ("timestamp", machine.timestamp.as_str()),
        ("bench-hashes version", BENCH_VERSION),
        ("git commit", GIT_COMMIT),
        ("git clean status", GIT_CLEAN_STATUS),
        ("blake3-servil source", BLAKE3_SERVIL_SOURCE_INFO),
        ("blake3 source", BLAKE3_SOURCE_INFO),
        ("sha256 source", SHA2_SOURCE_INFO),
        ("cpu type", machine.cpu_type.as_str()),
        ("cpu count", machine.cpu_count.to_string().as_str()),
        ("os type", machine.os_type.as_str()),
        ("cpu identity", machine.cpu_identity.as_str()),
        ("rust compiler", RUSTC_VERSION),
        ("build target", BUILD_TARGET),
        ("target features", TARGET_FEATURES),
        ("sample clock", clocks::WALL_CLOCK),
        ("contenders", selection_note),
        ("rounds", roster.rounds.to_string().as_str()),
        ("points", roster.points.iter().map(|&index| POINTS[index].label).collect::<Vec<_>>().join(",").as_str()),
    ] {
        writeln!(out, "# {key}: {value}").unwrap();
    }
    writeln!(out, "# power: {}", machine.describe_power()).unwrap();
    writeln!(out, "# load: {}", clocks::load::describe(&machine.load)).unwrap();
    let windows: Vec<String> = machine.load.iter()
        .map(|w| format!("{}-{}:{}:{}", w.start_ns / 1_000_000, w.end_ns / 1_000_000, w.other_milli_cpus, w.steal_milli_cpus))
        .collect();
    writeln!(out, "# load windows (start ms-end ms:other milli-CPUs:steal milli-CPUs): {}", windows.join(",")).unwrap();
    for &algorithm in &roster.algorithms {
        for use_case in UseCase::ALL.iter().filter(|&&use_case| algorithm.takes_part(use_case)) {
            writeln!(out, "# kernel platform {} {:?}: {}", algorithm.key(), use_case, detect_kernels(algorithm, *use_case).platform).unwrap();
        }
    }
    writeln!(out, "{SAMPLES_COLUMNS}").unwrap();
    for (algorithm_index, &algorithm) in roster.algorithms.iter().enumerate() {
        for scenario in Scenario::ALL {
            let rows = match scenario {
                Scenario::Solo => &samples.solo,
                Scenario::Shared => &samples.shared,
            };
            for (point_index, point) in POINTS.iter().enumerate() {
                let cell_samples = &rows[algorithm_index][point_index];
                if cell_samples.is_empty() {
                    continue;
                }
                let values: Vec<String> = cell_samples.iter().map(|m| format!("{}/{}", m.ns, m.units)).collect();
                let timestamps = match scenario {
                    Scenario::Solo => &samples.solo_started_ns,
                    Scenario::Shared => &samples.shared_started_ns,
                };
                let cell_starts = &timestamps[algorithm_index][point_index];
                assert_eq!(cell_starts.len(), cell_samples.len(), "each sample has its own load timestamp");
                let starts: Vec<String> = cell_starts.iter()
                    .map(|ns| (ns / 1_000_000).to_string())
                    .collect();
                writeln!(
                    out,
                    "{}\t{}\t{:?}\t{}\t{}\t{}\t{}",
                    algorithm.key(),
                    scenario.key(),
                    point.use_case,
                    point.label,
                    point.use_case.unit_key(),
                    values.join(","),
                    starts.join(","),
                )
                .unwrap();
            }
        }
    }
    out
}

/*
 * The text report: results first, one table per scenario and use case,
 * each cell's mean;
 * then which code path each contender ran, and where the numbers came
 * from, for whoever needs to trust or reproduce them. The consistency
 * checks go to a file of their own (consistency).
 */
fn generate_text(roster: &Roster, results: &Results, machine: &MachineMetadata, selection_note: &str) -> String {
    let mut output = String::new();

    writeln!(output, "Hash speed on {} ({}, {} CPUs), {}", machine.cpu_type, machine.os_type, machine.cpu_count, machine.timestamp).unwrap();
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
    for key in ["busy", "idle", "nonstop"] {
        let names: Vec<String> = UseCase::ALL
            .into_iter()
            .filter(|&use_case| use_case.pattern_key() == key && roster.points.iter().any(|&index| POINTS[index].use_case == use_case))
            .map(|use_case| format!("\u{201c}{}\u{201d}", use_case.heading()))
            .collect();
        let Some(pattern) = UseCase::ALL.into_iter().find(|use_case| use_case.pattern_key() == key).map(UseCase::pattern) else { continue };
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
        writeln!(output, "{}: {}.", scenario.heading().to_uppercase(), scenario.description()).unwrap();
        writeln!(output).unwrap();
        for use_case in UseCase::ALL.into_iter().filter(|&use_case| scenario.measures(use_case)) {
            append_table(&mut output, roster, results, scenario, use_case);
        }
    }

    writeln!(output, "KERNELS: the code path each contender ran, from the point named on (a call after idling runs the same as after other work).").unwrap();
    for use_case in UseCase::ALL.into_iter().filter(|&use_case| use_case.call() == use_case) {
        writeln!(output, "  {}:", use_case.heading()).unwrap();
        for &algorithm in roster.algorithms.iter().filter(|algorithm| algorithm.takes_part(use_case)) {
            append_kernel_report(&mut output, algorithm, use_case);
        }
    }
    writeln!(output).unwrap();

    writeln!(output, "PROVENANCE").unwrap();
    writeln!(output, "  bench-hashes {BENCH_VERSION}, {GIT_SOURCE}, commit {GIT_COMMIT} ({GIT_CLEAN_STATUS})").unwrap();
    writeln!(output, "  contenders: {selection_note}").unwrap();
    for algorithm in &roster.algorithms {
        writeln!(output, "  {}: {}; {}", algorithm.name(), algorithm.source(), algorithm.mode()).unwrap();
    }
    writeln!(output, "  CPU: {}", machine.cpu_identity).unwrap();
    writeln!(output, "  {RUSTC_VERSION}; target {BUILD_TARGET}; features {TARGET_FEATURES}").unwrap();
    writeln!(output, "  clock: {}", clocks::WALL_CLOCK).unwrap();
    writeln!(output, "  load during the run: {}", clocks::load::describe(&machine.load)).unwrap();
    writeln!(output, "  power: {}", machine.describe_power()).unwrap();

    output
}

/*
 * Consistency checks: relations that hold for every contender alike when
 * the benchmark measures what it means to, each judged on cells' fast
 * speeds, CONSISTENCY_PERMILLE apart or more (bench-hashes NOTES,
 * "Consistency checks"). A broken one is a bug
 * in the benchmark or in the code under test, or a finding to explain.
 * They go to a file of their own, for maintainers.
 */
const CONSISTENCY_PERMILLE: u64 = 100;
/// The largest work check 4 compares: inputs this size stay in the
/// first-level data cache of the machines this benchmark targets (32-128
/// KiB). Beyond it each level of the memory hierarchy costs more per
/// byte, for every contender (SHA-256's batches: 32 ns a message up to
/// 16 KiB, 36 from 256 KiB, VM), which is no bug.
const CACHED_BYTES: usize = 32 * 1024;

/// Where `slow` is slower than `fast` by more than the margin: slow over
/// fast, in permille.
fn slower_by(slow: Statistics, fast: Statistics) -> Option<u64> {
    (fast.mean.cmp_permille(0).is_gt()
        && slow.mean.ratio(fast.mean).cmp_permille(1000 + CONSISTENCY_PERMILLE).is_gt())
        .then(|| slow.mean.ratio(fast.mean).permille())
}

/// The consistency checks' report: one line per broken relation.
fn consistency(roster: &Roster, results: &Results) -> String {
    let stats = |a: usize, p: usize, scenario: Scenario| cell(results, a, p).get(scenario);
    let point = |use_case: UseCase, label: &str| use_case.points().find(|&p| POINTS[p].label == label && roster.measures(p));
    let mut broken: Vec<String> = Vec::new();
    let mut note = |check: &str, a: usize, what: String, permille: u64| {
        broken.push(format!("{check}: {}, {what}: x{}.{:03}", roster.algorithms[a].name(), permille / 1000, permille % 1000));
    };
    for (a, &algorithm) in roster.algorithms.iter().enumerate() {
        /* 1. Nonstop is no slower than after other work for small messages (its read of the input included). */
        for label in ["64 B", "256 B", "1 KiB", "4 KiB"].into_iter().filter(|_| algorithm.takes_part(UseCase::LentMessages) && algorithm.takes_part(UseCase::OneMessage)) {
            if let (Some(n), Some(b)) = (point(UseCase::LentMessages, label), point(UseCase::OneMessage, label)) {
                let (fnon, fb) = (stats(a, n, Scenario::Solo), stats(a, b, Scenario::Solo));
                if let Some(r) = slower_by(fnon, fb) {
                    note("nonstop slower than after other work", a, format!("{label}, {} against {} ns/B", fnon.mean.format_ns(), fb.mean.format_ns()), r);
                }
            }
        }
        for use_case in UseCase::ALL.into_iter().filter(|&u| algorithm.takes_part(u)) {
            let points: Vec<usize> = use_case.points().filter(|&p| roster.measures(p)).collect();
            /* 2 and 3: two programs at once, for the use cases measured both ways. */
            if Scenario::Shared.measures(use_case) {
                for &p in &points {
                    let (solo, shared) = (stats(a, p, Scenario::Solo), stats(a, p, Scenario::Shared));
                    if let Some(r) = slower_by(solo, shared) {
                        note("shared faster than solo", a, format!("{}, {}: solo {} against shared {} {}", use_case.short(), POINTS[p].label, solo.mean.format_ns(), shared.mean.format_ns(), use_case.time_unit()), r);
                    }
                    if algorithm.core_only() {
                        if let Some(r) = slower_by(shared, solo) {
                            note("a hash on the cores alone slowed by a second copy", a, format!("{}, {}: shared {} against solo {} {}", use_case.short(), POINTS[p].label, shared.mean.format_ns(), solo.mean.format_ns(), use_case.time_unit()), r);
                        }
                    }
                }
            }
            /*
             * 4. Twice the work takes at most twice the time: no slower per
             * unit than a size that divides it, for work that stays in the
             * first-level cache (up to CACHED_BYTES) and outside the idle
             * use cases (whose cells may meet different clock states).
             */
            let size = |p: usize| if use_case.batch() { POINTS[p].messages } else { POINTS[p].bytes };
            let cached: Vec<usize> = points.iter().copied().filter(|&p| POINTS[p].bytes <= CACHED_BYTES && !use_case.idle()).collect();
            let points = cached;
            for &large in &points {
                for &small in points.iter().filter(|&&small| size(small) < size(large) && size(large) % size(small) == 0) {
                    for scenario in Scenario::ALL.into_iter().filter(|&scenario| scenario.measures(use_case)) {
                        let (l, m) = (stats(a, large, scenario), stats(a, small, scenario));
                        if let Some(r) = slower_by(l, m) {
                            note("more work, slower per unit", a, format!("{} ({}), {} against {}: {} against {} {}", use_case.short(), scenario.key(), POINTS[large].label, POINTS[small].label, l.mean.format_ns(), m.mean.format_ns(), use_case.time_unit()), r);
                        }
                    }
                }
            }
        }
    }
    let mut out = format!(
        "# bench-hashes consistency checks (for maintainers): relations that hold for every contender when the benchmark measures what it means to, judged on means, over {}% apart.\n\
         # 1. Nonstop no slower than after other work, 64 B-4 KiB. 2. Shared no faster than solo. 3. A hash on the cores alone no slower shared. 4. No slower per unit than a size that divides the work, up to 32 KiB, outside the idle use cases.\n",
        CONSISTENCY_PERMILLE / 10,
    );
    if broken.is_empty() {
        out += "all hold\n";
    }
    for line in broken {
        out += &line;
        out.push('\n');
    }
    out
}

/// One results table: a row per point, a column per contender taking part.
fn append_table(output: &mut String, roster: &Roster, results: &Results, scenario: Scenario, use_case: UseCase) {
    let contenders: Vec<usize> = (0..roster.len()).filter(|&index| roster.algorithms[index].takes_part(use_case)).collect();
    let points: Vec<usize> = use_case.points().filter(|&index| roster.measures(index)).collect();
    if points.is_empty() {
        return;
    }
    writeln!(output, "  {} ({})", use_case.heading(), use_case.time_unit()).unwrap();
    write!(output, "  {:<8}", use_case.column()).unwrap();
    for &algorithm_index in &contenders {
        write!(output, "  {:>14}", column_heading(roster.algorithms[algorithm_index])).unwrap();
    }
    writeln!(output).unwrap();
    for &point_index in &points {
        write!(output, "  {:<8}", POINTS[point_index].label).unwrap();
        for &algorithm_index in &contenders {
            let statistics = cell(results, algorithm_index, point_index).get(scenario);
            write!(output, "  {:>13} ", statistics.format_mean(1)).unwrap();
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
    let mut system = System::new_all();
    system.refresh_all();

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

    let kernel = System::kernel_version()
        .unwrap_or_else(|| "unreported".to_owned());

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
fn cpu_identity() -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        let first = cpuinfo.split("\n\n").next().unwrap_or("");
        for key in ["CPU implementer", "CPU variant", "CPU part", "CPU revision", "model name", "Features", "flags"] {
            if let Some(line) = first.lines().find(|line| line.split(':').next().is_some_and(|k| k.trim() == key)) {
                parts.push(format!("{key}: {}", line.split_once(':').unwrap().1.trim()));
            }
        }
        for path in ["/sys/firmware/devicetree/base/compatible", "/sys/class/dmi/id/product_name"] {
            if let Ok(model) = fs::read(path) {
                let model = String::from_utf8_lossy(&model).replace('\0', " ").trim().to_owned();
                if !model.is_empty() {
                    parts.push(format!("machine: {model}"));
                }
            }
        }
    }
    if cfg!(target_os = "macos") {
        for key in ["machdep.cpu.brand_string", "hw.perflevel0.physicalcpu", "hw.perflevel1.physicalcpu"] {
            if let Ok(out) = std::process::Command::new("sysctl").args(["-n", key]).output() {
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

    let (year, month, day) =
        civil_date_from_unix_days(days);

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

    let year_of_era = (
        day_of_era
            - day_of_era / 1460
            + day_of_era / 36_524
            - day_of_era / 146_096
    ) / 365;

    let mut year = year_of_era + era * 400;

    let day_of_year = day_of_era
        - (
            365 * year_of_era
                + year_of_era / 4
                - year_of_era / 100
        );

    let month_prime = (5 * day_of_year + 2) / 153;
    let day =
        day_of_year - (153 * month_prime + 2) / 5 + 1;

    let month =
        month_prime + if month_prime < 10 { 3 } else { -9 };

    if month <= 2 {
        year += 1;
    }

    (year, month, day)
}

fn sanitize_alphanumeric(input: &str) -> String {
    let sanitized: String = input
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();

    assert!(
        !sanitized.is_empty(),
        "sanitized identifier must not be empty; input was {input:?}"
    );

    sanitized
}

fn output_directory(machine: &MachineMetadata) -> std::path::PathBuf {
    let cpu = sanitize_alphanumeric(&machine.cpu_type);
    let os = sanitize_alphanumeric(&machine.os_type);

    std::path::PathBuf::from("benchmark-results")
        .join(format!("{cpu}.{os}"))
}

/// A self-contained decision guide: the questions, a complete example per
/// call (compiled as this crate's examples), and this run's medians for
/// every plot, drawn by the page's own small chart. Escape '<' inside
/// script data so graph text cannot close the page's script.
fn generate_guide(roster: &Roster, results: &Results, machine: &MachineMetadata) -> String {
    let mut data = String::from("{\"contenders\":[");
    for (index, algorithm) in roster.algorithms.iter().enumerate() {
        if index > 0 { data.push(','); }
        write!(data, "{{\"key\":{},\"name\":{},\"color\":\"{}\"}}", json_string(algorithm.key()), json_string(algorithm.name()), algorithm.color()).unwrap();
    }
    write!(data, "],\"machine\":{},\"date\":{},\"plots\":[", json_string(&machine.cpu_type), json_string(machine.timestamp.split(' ').next().unwrap_or(""))).unwrap();
    let mut first = true;
    for scenario in Scenario::ALL {
        for use_case in UseCase::ALL.into_iter().filter(|&use_case| scenario.measures(use_case)) {
            let points: Vec<usize> = use_case.points().filter(|&index| roster.measures(index)).collect();
            if points.is_empty() { continue; }
            if !first { data.push(','); }
            first = false;
            write!(data, "{{\"scenario\":\"{}\",\"use\":\"{:?}\",\"batch\":{},\"map\":{},\"labels\":[", scenario.key(), use_case, use_case.batch(), json_string(&map::cell_key(scenario.key(), use_case))).unwrap();
            data.push_str(&points.iter().map(|&index| json_string(POINTS[index].label)).collect::<Vec<_>>().join(","));
            data.push_str("],\"units\":[");
            data.push_str(&points.iter().map(|&index| use_case.units(POINTS[index], 1).to_string()).collect::<Vec<_>>().join(","));
            data.push_str("],\"bytes\":[");
            data.push_str(&points.iter().map(|&index| POINTS[index].bytes.to_string()).collect::<Vec<_>>().join(","));
            data.push_str("],\"series\":{");
            let mut first_series = true;
            for (algorithm_index, algorithm) in roster.algorithms.iter().enumerate() {
                if !algorithm.takes_part(use_case) { continue; }
                if !first_series { data.push(','); }
                first_series = false;
                write!(data, "{}:{{", json_string(algorithm.key())).unwrap();
                /* Per point: the mean (ns per unit). */
                let means: Vec<String> = points.iter().map(|&index| cell(results, algorithm_index, index).get(scenario).format_mean(1)).collect();
                write!(data, "\"mean\":[{}]", means.join(",")).unwrap();
                data.push('}');
            }
            data.push_str("}}");
        }
    }
    data.push_str("]}");
    let mut examples = String::from("{");
    for (index, (call, source)) in GUIDE_EXAMPLES.iter().enumerate() {
        if index > 0 { examples.push(','); }
        write!(examples, "{}:{}", json_string(call), json_string(source)).unwrap();
    }
    examples.push('}');
    let escape = |text: String| text.replace('<', "\\u003c");
    include_str!("guide.html").replace("@DATA@", &escape(data)).replace("@EXAMPLES@", &escape(examples))
}

/// Each recommended call's complete program, compiled as an example of
/// this crate (`cargo build --examples`), so what the guide shows builds.
const GUIDE_EXAMPLES: [(&str, &str); 9] = [
    ("hash", include_str!("../examples/guide_hash.rs")),
    ("hash_multithreaded", include_str!("../examples/guide_hash_multithreaded.rs")),
    ("hash_many", include_str!("../examples/guide_hash_many.rs")),
    ("hash_many_multithreaded", include_str!("../examples/guide_hash_many_multithreaded.rs")),
    ("update", include_str!("../examples/guide_update.rs")),
    ("update_multithreaded", include_str!("../examples/guide_update_multithreaded.rs")),
    ("Queue::messages", include_str!("../examples/guide_queue_messages.rs")),
    ("Queue::pieces", include_str!("../examples/guide_queue_pieces.rs")),
    ("Queue::fixed", include_str!("../examples/guide_queue_fixed.rs")),
];

fn json_string(text: &str) -> String {
    let mut quoted = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '\"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            c if c < ' ' || matches!(c, '\u{2028}' | '\u{2029}') => write!(quoted, "\\u{:04x}", c as u32).unwrap(),
            c => quoted.push(c),
        }
    }
    quoted.push('\"');
    quoted
}

/* Provenance that describes the run as a whole. */

/* Provenance that belongs to one contender and hides with it: the
   implementation, its mode, and the platform its kernels ran on. */

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

#[cfg(test)]
mod harness_tests;
mod b3sum;
mod chart;
mod map;

#[cfg(test)]
mod correctness_tests {
    use super::*;

    /// pmset's outputs on an M4 Max (fork runner jobs 350 and 351, September 27, 2026).
    #[test]
    fn power_from_pmset() {
        let settings = "System-wide power settings:\nCurrently in use:\n standby              1\n sleep                1 (sleep prevented by powerd)\n powermode            2\n womp                 0\n";
        let battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=35389539)\t47%; discharging; 2:36 remaining present: true\n";
        let power = Power::parse_pmset(battery, settings).unwrap();
        assert!(power.on_battery && power.slowing());
        assert_eq!(power.describe(), "battery power (47% charged), High Power mode");
        let mains = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=35389539)\t48%; charging; 1:10 remaining present: true\n";
        let power = Power::parse_pmset(mains, &settings.replace("powermode            2", "lowpowermode         1")).unwrap();
        assert!(!power.on_battery && power.low_power);
        assert_eq!(power.describe(), "mains power, Low Power Mode");
        let desktop = Power::parse_pmset("Now drawing from 'AC Power'\n", "").unwrap();
        assert!(!desktop.slowing());
        assert_eq!(desktop.describe(), "mains power");
    }

    /// Fixed and Measured: the one division rounds half up at 2^-64, the
    /// ratio is exact to its last bit, and the display rounds once, with
    /// three significant digits at every size. Expected answers worked by
    /// hand (and the ratio against exact integer division).
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
        let (a, b) = (t(31, 1000), t(7, 3));
        let exact = (a.0 << 20) / b.0; // the first 20 fractional bits, exactly
        assert_eq!(a.ratio(b).0 >> 44, exact);
        for x in [1u128, 3, 7, 1 << 40, (1 << 103) + 12345] {
            for y in [1u128, 2, 5, 1 << 50, (1 << 103) - 1] {
                if x / y >= 1 << 63 {
                    continue; // outside ratio's contract
                }
                let q = Fixed(x).ratio(Fixed(y)).0;
                // q = floor(x * 2^64 / y): q * y <= x * 2^64 < (q + 1) * y, checked in 256 bits as two halves.
                let wide = |a: u128, b: u128| -> (u128, u128) {
                    let (a1, a0, b1, b0) = (a >> 64, a & u64::MAX as u128, b >> 64, b & u64::MAX as u128);
                    let (lo, mid1, mid2, hi) = (a0 * b0, a1 * b0, a0 * b1, a1 * b1);
                    let (mid, c1) = mid1.overflowing_add(mid2);
                    let (low, c2) = lo.overflowing_add(mid << 64);
                    (hi + (mid >> 64) + ((c1 as u128) << 64) + c2 as u128, low)
                };
                let target = (x >> 64, x << 64);
                assert!(wide(q, y) <= target && wide(q + 1, y) > target, "{x} / {y}");
            }
        }
        assert_eq!(Fixed::ONE.cmp_permille(1000), std::cmp::Ordering::Equal);
        assert!(t(1049, 1000).cmp_permille(1050).is_lt() && t(1051, 1000).cmp_permille(1050).is_gt());
        assert_eq!(t(1, 3).permille(), 333);
    }

    fn point(label: &str, use_case: UseCase) -> usize {
        POINTS.iter().position(|point| point.label == label && point.use_case == use_case).unwrap()
    }

    /// Results and samples for two contenders over `rounds` rounds: each
    /// cell's sample in round r is `value(contender, point, r)` ns over one
    /// unit, solo and both shared copies alike (the shared ones for the
    /// nonstop use cases alone), summarised as measure_all does.
    fn run(roster: &Roster, rounds: usize, value: impl Fn(usize, usize, usize) -> u64) -> (Results, RunSamples) {
        let empty = || -> Samples { vec![vec![Vec::new(); POINT_COUNT]; roster.len()] };
        let mut samples = RunSamples { solo: empty(), shared: empty(),
            solo_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()],
            shared_started_ns: vec![vec![Vec::new(); POINT_COUNT]; roster.len()] };
        let mut results: Results = vec![vec![None; POINT_COUNT]; roster.len()];
        for a in 0..roster.len() {
            for &p in &roster.points {
                for r in 0..rounds {
                    let v = Measured::new(value(a, p, r), 1);
                    samples.solo[a][p].push(v);
                    if Scenario::Shared.measures(POINTS[p].use_case) {
                        samples.shared[a][p].extend([v, v]);
                        // Separate starts cross load-window boundaries, and
                        // distinguish the two copies from one another.
                        samples.shared_started_ns[a][p].extend([10_000_000 + r as u64 * 1_000_000, 11_000_000 + r as u64 * 1_000_000]);
                    }
                    samples.solo_started_ns[a][p].push(r as u64 * 1_000_000);
                }
                results[a][p] = Some(Cell {
                    solo: summarize_measured(&samples.solo[a][p]),
                    shared: Scenario::Shared.measures(POINTS[p].use_case).then(|| summarize_measured(&samples.shared[a][p])),
                });
            }
        }
        (results, samples)
    }

    /// Each consistency check fires on the relation it holds, and a run
    /// that keeps every relation reads "all hold".
    #[test]
    fn consistency_checks_name_each_broken_relation() {
        let points = vec![point("8 MiB", UseCase::OneMessage), point("8 MiB", UseCase::IdleOneMessage), point("64 B", UseCase::OneMessage),
            point("64 B", UseCase::LentMessages), point("16", UseCase::LentBatches), point("64", UseCase::LentBatches)];
        let mut points = points;
        points.sort_unstable();
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Sha256], true, Some(points), Some(24));
        let jitter = |r: usize| (r % 5) as u64 * 20;
        let (results, _) = run(&roster, 24, |_, _, r| 10_000 + jitter(r));
        assert!(consistency(&roster, &results).ends_with("all hold\n"), "{}", consistency(&roster, &results));

        /* 1: nonstop slower at 64 B; 4: 64 messages slower per message than 16. */
        let (mut results, mut samples) = run(&roster, 24, |_, p, r| jitter(r) + match (POINTS[p].use_case, POINTS[p].label) {
            (UseCase::IdleOneMessage, _) => 15_000,
            (UseCase::LentMessages, _) => 20_000,
            (UseCase::LentBatches, "64") => 12_000,
            _ => 10_000,
        });
        /* 2: servil's shared copies twice as fast at 16 messages; 3: SHA-256's (on the cores alone) half as fast. */
        let p16 = POINTS.iter().position(|p| p.use_case == UseCase::LentBatches && p.label == "16").unwrap();
        for (a, factor) in [(0usize, (1u64, 2u64)), (1, (3, 2))] {
            for sample in samples.shared[a][p16].iter_mut() {
                *sample = Measured::new(sample.ns * factor.0 / factor.1, sample.units);
            }
            results[a][p16].as_mut().unwrap().shared = Some(summarize_measured(&samples.shared[a][p16]));
        }
        let report = consistency(&roster, &results);
        for (check, who) in [("nonstop slower than after other work", "SHA-256"),
            ("more work, slower per unit", "SHA-256"), ("shared faster than solo", "BLAKE3 servil st"),
            ("a hash on the cores alone slowed by a second copy", "SHA-256")] {
            assert!(report.lines().any(|line| line.starts_with(&format!("{check}: {who}"))), "{check} for {who}:\n{report}");
        }
        assert!(!report.contains("a hash on the cores alone slowed by a second copy: BLAKE3 servil st"), "servil shares its SME unit: exempt\n{report}");
    }

    /// Every point hashes a prefix of one buffer: the input of each size is
    /// the first bytes of every larger one.
    #[test]
    fn inputs_are_prefixes_of_the_largest() {
        let largest = make_input_seeded(1 << 20, 1);
        for bytes in [0, 1, 63, 64, 4096, 4470, 1 << 20] {
            assert_eq!(make_input_seeded(bytes, 1), largest[..bytes], "{bytes} bytes");
        }
    }

    /// The benchmark asks of the fork exactly what FROZEN.md says it does.
    #[test]
    fn frozen_contract_matches_frozen_md() {
        let manifest = include_str!("../FROZEN.md");
        let start = manifest.find("```frozen\n").expect("FROZEN.md has a frozen block") + "```frozen\n".len();
        let end = start + manifest[start..].find("```").expect("the frozen block ends");
        assert_eq!(
            &manifest[start..end],
            frozen_contract(),
            "the benchmark's contract with the fork changed: change FROZEN.md with it, with Zooko's decision and its reason"
        );
    }

    #[test]
    fn means_at_decimal_halfways_round_the_original_measurement_once() {
        // Independently established rational values: 2135/400 = 5.3375,
        // 20555/400 = 51.3875; half-up decimal rounding gives these anchors.
        for (ns, expected) in [(2135, "5.338"), (20555, "51.388")] {
            let measured = [Measured::new(ns, 400), Measured::new(ns, 400)];
            let statistics = summarize_measured(&measured);
            assert_eq!(statistics.format_mean(1), expected);
            assert_eq!(statistics.format_mean(400), format!("{ns}.000"));
        }
        // A mean is total time over total work: 3000 ns over 400 and 1000
        // over 400 make 4000 over 800, 5 ns a unit.
        assert_eq!(summarize_measured(&[Measured::new(3000, 400), Measured::new(1000, 400)]).format_mean(1), "5.000");
    }

    #[test]
    fn prepared_samples_select_the_promised_api_and_keep_digest_space() {
        CALLS.with(|calls| calls.borrow_mut().clear());
        let input = make_input(MESSAGE_LEN);
        take_sample(Algorithm::Blake3ServilSt, &input, Point::one("", MESSAGE_LEN), 2);
        take_sample(Algorithm::Blake3ServilSt, &input, Point::many("", 1), 3);
        /* One untimed call before each sample's timed ones (clocks: each timed call follows the same call). */
        assert_eq!(CALLS.with(|calls| calls.borrow().clone()), ["hash", "hash", "hash", "hash_many", "hash_many", "hash_many", "hash_many"]);
    }

    #[test]
    fn sparse_regression_runs_write_samples_without_a_graph() {
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt, Algorithm::Sha256],
            true, Some(vec![point("64 B", UseCase::ContinuousMessages)]), Some(24));
        assert!(!roster.whole_axes());
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Blake3ServilMt, Algorithm::Sha256],
            true, Some(vec![point("64 B", UseCase::ContinuousMessages), point("256 B", UseCase::ContinuousMessages)]), Some(24));
        assert!(!roster.whole_axes(), "one selected contender has no cells here");
    }

    #[test]
    fn samples_file_places_each_sample_in_the_load_windows() {
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring], true,
            Some(vec![point("64 B", UseCase::OneMessage), point("128 B", UseCase::OneMessage), point("64 B", UseCase::LentMessages), point("256 B", UseCase::LentMessages)]), Some(3));
        let (_, samples) = run(&roster, 3, |_, _, r| 10_000 + r as u64);
        let mut machine = machine_metadata();
        machine.load = vec![
            clocks::load::Window { start_ns: 0, end_ns: 1_500_000, other_milli_cpus: 1200, steal_milli_cpus: 0 },
            clocks::load::Window { start_ns: 1_500_000, end_ns: 3_000_000, other_milli_cpus: 100, steal_milli_cpus: 7 },
        ];
        let tsv = generate_samples_tsv(&roster, &samples, &machine, "test");
        assert!(tsv.starts_with("# bench-hashes samples v4\n"));
        assert!(tsv.contains("\n# load: busy: "), "{tsv}");
        assert!(tsv.contains("\n# load windows (start ms-end ms:other milli-CPUs:steal milli-CPUs): 0-1:1200:0,1-3:100:7\n"), "{tsv}");
        assert!(tsv.contains("\ncontender\tscenario\tuse_case\tpoint\tunit\tns/units\tstart ms\n"));
        assert!(tsv.contains("\nblake3-servil-st\tsolo\tOneMessage\t64 B\tB\t10000/1,10001/1,10002/1\t0,1,2\n"), "{tsv}");
        assert!(tsv.contains("\nblake3-servil-st\tshared\tLentMessages\t64 B\tB\t10000/1,10000/1,10001/1,10001/1,10002/1,10002/1\t10,11,11,12,12,13\n"), "{tsv}");
        assert!(!tsv.contains("\tshared\tOneMessage\t"), "calls after a gap run alone: {tsv}");
    }

    /// The samples file reads back into the figures the report prints:
    /// every cell (read_samples, as `compare` reads it).
    #[test]
    fn samples_file_reads_back_into_the_reports_figures() {
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring], true,
            Some(vec![point("64 B", UseCase::OneMessage), point("64 B", UseCase::LentMessages), point("16", UseCase::LentBatches)]), Some(20));
        // Contender 1's lent cells: 3 rounds in 10 twice as slow.
        let (_, samples) = run(&roster, 20, |a, p, r| if a == 1 && POINTS[p].use_case != UseCase::OneMessage && r % 10 < 3 { 20_000 + r as u64 } else { 10_000 + 7 * r as u64 });
        let tsv = generate_samples_tsv(&roster, &samples, &machine_metadata(), "test");
        let path = std::env::temp_dir().join(format!("bench-hashes-readback-{}.tsv", std::process::id()));
        fs::write(&path, &tsv).unwrap();
        let read = read_samples(path.to_str().unwrap());
        fs::remove_file(&path).unwrap();
        let figures = |cell: &[Measured]| summarize_measured(cell).format_mean(1);
        let mut checked = 0;
        for (a, algorithm) in roster.algorithms.iter().enumerate() {
            for &p in &roster.points {
                for (scenario, cell) in [("solo", &samples.solo[a][p]), ("shared", &samples.shared[a][p])] {
                    if cell.is_empty() {
                        continue;
                    }
                    let key = format!("{}|{scenario}|{:?}|{}", algorithm.key(), POINTS[p].use_case, POINTS[p].label);
                    let (_, back) = read.cells.iter().find(|(k, _)| *k == key).unwrap_or_else(|| panic!("{key} read back"));
                    assert_eq!(figures(back), figures(cell), "{key}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, read.cells.len(), "every cell read back, none more");
    }

    #[test]
    fn graph_strings_escape_controls_and_guide_keeps_script_data_inside() {
        assert_eq!(json_string("a\n\r\t\0\"\\"), "\"a\\n\\r\\t\\u0000\\\"\\\\\"");
        let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring], true, None, Some(24));
        let (results, _) = run(&roster, 24, |_, _, r| 10_000 + r as u64);
        let mut machine = machine_metadata();
        machine.cpu_type = "</script><b>".to_owned();
        let guide = generate_guide(&roster, &results, &machine);
        assert!(guide.contains("\\u003c/script>\\u003cb>"));
        assert_eq!(guide.matches("</script>").count(), 1, "only the template closes the outer script");
        assert!(!guide.contains("@DATA@") && !guide.contains("@EXAMPLES@"));
        assert!(guide.contains("\"map\":\"solo|"), "each plot names its chart on the map");
    }

    #[test]
    fn use_case_axes_are_contiguous_and_cover_every_point() {
        assert_eq!(UseCase::OneMessage.points(), 0..INPUT_COUNT);
        assert_eq!(UseCase::ManyMessages.points(), INPUT_COUNT..INPUT_COUNT + BATCH_COUNT);
        let idle = INPUT_COUNT + BATCH_COUNT;
        for use_case in [UseCase::IdleOneMessage, UseCase::IdleManyMessages] {
            let (twin, mine) = (use_case.call().points(), use_case.points());
            assert_eq!(mine, twin.start + idle..twin.end + idle, "each idle axis follows its twin's pattern");
            for (a, b) in POINTS[twin].iter().zip(&POINTS[mine]) {
                assert_eq!((a.label, a.bytes, a.messages), (b.label, b.bytes, b.messages), "an idle axis repeats its twin's points");
            }
        }
        let continuous = 2 * idle;
        assert_eq!(UseCase::ContinuousMessages.points(), continuous..continuous + CONTINUOUS_MESSAGE_COUNT);
        assert_eq!(UseCase::ContinuousBatches.points(), continuous + CONTINUOUS_MESSAGE_COUNT..continuous + CONTINUOUS_MESSAGE_COUNT + CONTINUOUS_BATCH_COUNT);
        let lent = continuous + CONTINUOUS_MESSAGE_COUNT + CONTINUOUS_BATCH_COUNT;
        assert_eq!(UseCase::LentMessages.points(), lent..lent + CONTINUOUS_MESSAGE_COUNT);
        let interleaved = lent + CONTINUOUS_MESSAGE_COUNT;
        assert_eq!(UseCase::Interleaved.points(), interleaved..interleaved + INTERLEAVED_COUNT);
        let collection = interleaved + INTERLEAVED_COUNT;
        assert_eq!(UseCase::Collection.points(), collection..collection + COLLECTION_COUNT);
        let outboard = collection + COLLECTION_COUNT;
        assert_eq!(UseCase::Outboard.points(), outboard..outboard + OUTBOARD_COUNT);
        let verify = outboard + OUTBOARD_COUNT;
        assert_eq!(UseCase::Verify.points(), verify..verify + VERIFY_COUNT);
        assert_eq!(UseCase::LentBatches.points(), verify + VERIFY_COUNT..POINT_COUNT);
        for (k, point) in POINTS[UseCase::Collection.points()].iter().enumerate() {
            assert_eq!((point.label, point.messages), (COLLECTIONS[k].0, 2048), "each collection is 2048 items");
        }
        let covered: Vec<_> = UseCase::ALL.into_iter().flat_map(UseCase::points).collect();
        assert_eq!(covered, (0..POINT_COUNT).collect::<Vec<_>>());
        for (owned, lent) in POINTS[UseCase::ContinuousMessages.points()].iter().zip(&POINTS[UseCase::LentMessages.points()]) {
            assert_eq!((owned.label, owned.bytes), (lent.label, lent.bytes));
        }
        let many = POINTS[UseCase::Interleaved.points()][0];
        assert!(many.bytes == INTERLEAVED.chunk && many.label == "256 open" && INTERLEAVED.open == 256, "many messages at once: 256 open, a unit of INTERLEAVED.chunk bytes");
        for (k, point) in POINTS[UseCase::ContinuousMessages.points()].iter().enumerate() {
            assert_eq!(point.bytes, 64 << (2 * k), "the continuous messages go up by factors of four from 64 B");
        }
        for (k, point) in POINTS[UseCase::ContinuousBatches.points()].iter().enumerate() {
            assert_eq!(point.messages, 16 << (2 * k), "the continuous batches go up by factors of four from 16");
        }
        for use_case in [UseCase::ManyMessages, UseCase::ContinuousBatches] {
            assert!(POINTS[use_case.points()].iter().all(|p| p.bytes == p.messages * MESSAGE_LEN));
        }
    }

    #[test]
    fn batch_observes_every_digest_and_matches_empty_vectors() {
        for (algorithm, expected) in [
            (Algorithm::Blake3, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
            (Algorithm::Sha256, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            (Algorithm::Sha1Dc, "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            (Algorithm::Sha3_256, "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"),
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
    /// message or batch on the continuous axes, the fork's queues
    /// included (one buffer per message, pieces, and batches), with fewer,
    /// as many, and more messages or batches than it keeps in flight. What
    /// the digests are is the contenders' tests' business (AGENTS.md,
    /// "Correctness tests").
    #[test]
    fn continuous_use_cases_observe_every_digest() {
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
        for algorithm in Algorithm::ALL.into_iter().filter(|algorithm| algorithm.availability().is_ok()) {
            for (point, input) in &cases {
                if !algorithm.takes_part(point.use_case) {
                    continue;
                }
                let mut once = 0;
                hash_batch(algorithm, input, *point, 1, |_| once += 1);
                assert!(once >= 1, "{} {:?}", algorithm.key(), point.use_case);
                let count = in_flight(if point.use_case.batch() { input.len() } else { input.len().min(PIECE_LEN) });
                for iterations in [1, count, 3 * count + 1] {
                    let mut seen = 0;
                    hash_batch(algorithm, input, *point, iterations, |_| seen += 1);
                    assert_eq!(seen, iterations * once, "{} {:?}", algorithm.key(), point.use_case);
                }
            }
        }
    }

    /// A sample of a synchronous use case makes its calls after the gap and
    /// times only the calls: at least a gap per call in wall time, well
    /// under that in the sample.
    #[test]
    fn samples_after_the_gap_leave_the_gaps_out() {
        let point = Point::one("", 64);
        let input = make_input(64);
        let started = clocks::now();
        let sample = take_sample(Algorithm::Blake3ServilSt, &input, point, 3);
        assert!(clocks::since_ns(started) >= 3 * GAP_NS);
        assert!(sample.elapsed_ns < GAP_NS, "{}", sample.elapsed_ns);
    }
}
