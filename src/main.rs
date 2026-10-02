
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
/// A message in pieces is measured at one length, a long message (FROZEN.md).
const LENT_PIECES_COUNT: usize = 1;
const POINT_COUNT: usize = 2 * (INPUT_COUNT + BATCH_COUNT) + 2 * CONTINUOUS_MESSAGE_COUNT + LENT_PIECES_COUNT + 2 * CONTINUOUS_BATCH_COUNT;
/// A message in pieces reaches each contender's incremental API in pieces
/// of this many bytes (a typical read buffer), the last one shorter.
const PIECE_LEN: usize = 64 * 1024;
/// Every message in the batches is one BLAKE3 block of 64 bytes, the size
/// of a Merkle tree's inner node (two 32-byte children).
const MESSAGE_LEN: usize = 64;

const BENCH_VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SOURCE: &str = env!("BENCH_GIT_SOURCE");
const GIT_COMMIT: &str = env!("BENCH_GIT_COMMIT");
const GIT_TAG: &str = env!("BENCH_GIT_TAG");
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
 * A message in pieces is measured nonstop at one length, 64 MiB: each
 * PIECE_LEN piece copied as a read would copy it and fed to the
 * contender's incremental API (then finalized), so the implementation
 * never sees the total up front. A message up to PIECE_LEN is one piece,
 * the one-message call's work; a long one shows the rate a contender
 * sustains piece after piece, which for a multithreaded incremental API
 * no one-message size predicts (Zooko, October 1, 2026; FROZEN.md).
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
    Point::lent_pieces("64 MiB", 64 * 1024 * 1024),
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
 * queue), and messages, long messages in pieces, and batches through
 * buffers it lends to a synchronous call, each read into a buffer of the
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
    /// One long message after another, each produced in PIECE_LEN pieces,
    /// each copied as a read would, through the incremental API.
    LentPieces,
    LentBatches,
}

impl UseCase {
    const ALL: [UseCase; 9] = [UseCase::OneMessage, UseCase::ManyMessages, UseCase::IdleOneMessage,
        UseCase::IdleManyMessages, UseCase::ContinuousMessages, UseCase::ContinuousBatches,
        UseCase::LentMessages, UseCase::LentPieces, UseCase::LentBatches];

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

    /// What a call hashes, as the graph's chips name it.
    fn what_key(self) -> &'static str {
        match self.call() {
            Self::OneMessage | Self::ContinuousMessages | Self::LentMessages => "messages",
            Self::ManyMessages | Self::ContinuousBatches | Self::LentBatches => "batches",
            Self::LentPieces => "pieces",
            Self::IdleOneMessage | Self::IdleManyMessages => unreachable!("call() names the call after other work"),
        }
    }

    /// Whose buffers a nonstop use case hashes, as the graph's chips name
    /// it: the producer's own, handed over (owned), or lent until each call
    /// returns; None for the calls after a gap.
    fn buffers_key(self) -> Option<&'static str> {
        match self {
            Self::ContinuousMessages | Self::ContinuousBatches => Some("owned"),
            Self::LentMessages | Self::LentPieces | Self::LentBatches => Some("lent"),
            Self::OneMessage | Self::ManyMessages | Self::IdleOneMessage | Self::IdleManyMessages => None,
        }
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
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::LentPieces => panic!("{self:?} hashes one message of the point's size"),
        }
    }

    /// The contiguous run of POINTS on this use case's axis.
    fn points(self) -> std::ops::Range<usize> {
        let start = POINTS.iter().position(|point| point.use_case == self).expect("each use case has points");
        let end = POINTS.iter().rposition(|point| point.use_case == self).unwrap() + 1;
        assert!(POINTS[start..end].iter().all(|point| point.use_case == self), "a use case's points are contiguous");
        start..end
    }

    /// What the x axis counts.
    fn x_axis(self) -> &'static str {
        match self {
            Self::OneMessage | Self::IdleOneMessage | Self::LentMessages | Self::LentPieces => "Message length (logarithmic spacing)",
            Self::ManyMessages | Self::IdleManyMessages | Self::ContinuousBatches | Self::LentBatches => "Messages per batch, 64 B each (logarithmic spacing)",
            Self::ContinuousMessages => "Length of each message (logarithmic spacing)",
        }
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
            Self::LentPieces => "64 MiB messages in 64 KiB pieces, one after another, buffers lent",
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
            Self::LentPieces => "pieces, lent buffers",
            Self::LentBatches => "batches, lent buffers",
        }
    }

    /// The x column's header in the text report.
    fn column(self) -> &'static str {
        match self {
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::LentPieces => "size",
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
            Self::OneMessage | Self::IdleOneMessage | Self::ContinuousMessages | Self::LentMessages | Self::LentPieces => point.bytes as u64 * iterations as u64,
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

    fn rate_unit(self) -> &'static str {
        if self.batch() { "Mmsg/s" } else { "GB/s" }
    }

    fn rate_unit_long(self) -> &'static str {
        if self.batch() { "Million messages per second" } else { "Gigabytes per second" }
    }

    /// rate = rate_scale / (ns per unit): 1 ns/B is 1 GB/s; 1 ns/msg is 1000 Mmsg/s.
    fn rate_scale(self) -> u64 {
        if self.batch() { 1000 } else { 1 }
    }

    /// How the program calls, for a reader of the results.
    /// How the program calls, for the report's opening: the clause after
    /// the tables' names.
    fn pattern(self) -> &'static str {
        if self.idle() {
            "the program hashes, sleeps 1 ms, and hashes again, as a server handles a request, waits, and handles the next; the second call is timed"
        } else if self.after_gap() {
            "the program hashes, runs other code and reads 128 MiB of memory (at least 1 ms), and hashes again, as a program hashes between its other tasks; the second call is timed"
        } else {
            "the program hashes one input after another, as fast as it can, alone and with a second program doing the same at once"
        }
    }

    /// The prefix that names this use case's points on the command line
    /// ("lent pieces 64 MiB"), empty for the first two.
    fn label_prefix(self) -> &'static str {
        match self {
            Self::OneMessage | Self::ManyMessages => "",
            Self::IdleOneMessage | Self::IdleManyMessages => "idle ",
            Self::ContinuousMessages => "continuous ",
            Self::ContinuousBatches => "continuous batch ",
            Self::LentMessages => "lent ",
            Self::LentPieces => "lent pieces ",
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

    const fn lent_pieces(label: &'static str, bytes: usize) -> Self {
        Self { label, bytes, messages: 1, use_case: UseCase::LentPieces }
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
    fn takes_part(self, use_case: UseCase) -> bool {
        let use_case = use_case.call();
        match use_case {
            UseCase::ManyMessages | UseCase::IdleManyMessages | UseCase::LentBatches => !matches!(self, Self::Blake3Rayon),
            UseCase::ContinuousBatches => !matches!(self, Self::Blake3Rayon | Self::Blake3ServilSt),
            UseCase::ContinuousMessages => !matches!(self, Self::Blake3ServilSt),
            UseCase::OneMessage | UseCase::IdleOneMessage | UseCase::LentMessages | UseCase::LentPieces => true,
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
            Self::Blake3ServilSt => "BLAKE3 servil st",
            Self::Sha256CommonCrypto => "SHA-256 CommonCrypto",
            Self::Sha256Ring => "SHA-256 ring",
            Self::Blake3Rayon => "BLAKE3 official mt",
            Self::Blake3ServilMt => "BLAKE3 servil mt",
            Self::Sha3_256 => "SHA3-256",
        }
    }

    /// What the hash is, in a line for a reader who has never heard of it
    /// (the tooltip on its name in the graph's legend).
    fn blurb(self) -> &'static str {
        match self {
            Self::Blake3 => "the official BLAKE3 Rust crate, on one thread",
            Self::Blake3Rayon => "the official BLAKE3 Rust crate, spreading large inputs over its thread pool",
            Self::Blake3ServilSt => "a fork of the official BLAKE3 Rust crate, faster on 64-bit Arm and above all on Apple M4-class chips, on one thread",
            Self::Blake3ServilMt => "a fork of the official BLAKE3 Rust crate, faster on 64-bit Arm and above all on Apple M4-class chips, spreading large inputs over every CPU core",
            Self::Sha256 => "SHA-256 from the sha2 Rust crate, with the CPU's SHA-256 instructions where it has them",
            Self::Sha256Ring => "SHA-256 from the ring Rust crate, with the CPU's SHA-256 instructions where it has them",
            Self::Sha256CommonCrypto => "SHA-256 from Apple's CommonCrypto library",
            Self::Sha1Dc => "SHA-1 with collision detection, from the sha1-checked Rust crate",
            Self::Sha3_256 => "SHA3-256 from the sha3 Rust crate, with the CPU's SHA-3 instructions where it has them",
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
            Self::Blake3ServilSt => "single-threaded; blake3_servil::hash for one message, blake3_servil::hash_many for a batch, Hasher::update per piece for a message in pieces",
            Self::Blake3Rayon => "multithreaded; Hasher::update_rayon (per piece, for a stream) on Rayon's global pool, the crate's own multithreading as a program gets it by default: the tree splits recursively over the pool, and inputs under a few chunks stay on the caller's thread",
            Self::Blake3ServilMt => "multithreaded; blake3_servil::hash_multithreaded for one message, hash_many_multithreaded for a batch, Hasher::update_multithreaded per piece for a message in pieces; for continuous loads its queue: Queue::messages for messages of up to 64 KiB, Queue::pieces for longer ones, Queue::fixed for batches: the fork chooses when to wake its worker threads; the kernel tables below show the one-shot calls' thresholds",
        }
    }

    /// The threads a multithreaded contender runs on, as far as the
    /// benchmarker can say without asking the implementation for machine
    /// capacity: Rayon's global pool is documented to take one thread per
    /// logical CPU by default; the fork sizes its own workers.
    fn thread_resources(self) -> Option<String> {
        match self {
            Self::Blake3Rayon => Some("Rayon's global pool, at its default size (one thread per logical CPU)".to_owned()),
            _ => None,
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

    fn per_unit(self) -> PerUnit {
        Fixed(((u128::from(self.ns) << 64) + u128::from(self.units) / 2) / u128::from(self.units))
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

    /// Nanoseconds, for the SVG's log axis only.
    fn ns_f64(self) -> f64 {
        self.0 as f64 / (1u128 << 64) as f64
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

    /// `numerator` / self, in tenths, rounded once: a rate from a time.
    fn tenths_of(self, numerator: u64) -> u64 {
        self.scaled_of(numerator, 10)
    }

    /// `numerator` / self times `scale`, rounded once.
    fn scaled_of(self, numerator: u64, scale: u64) -> u64 {
        assert!(self.0 > 0);
        u64::try_from((((u128::from(numerator) * u128::from(scale)) << 64) + self.0 / 2) / self.0).expect("a rate fits in u64")
    }
}


/*
 * Summary of one cell's samples: its mean, the total time of its samples
 * over the total work they did (clocks::summary), what a caller pays on
 * average; `minimum` and `maximum`, the extremes seen.
 */
#[derive(Clone, Copy)]
struct Statistics {
    /// Samples behind these figures.
    count: usize,
    minimum: PerUnit,
    mean: PerUnit,
    maximum: PerUnit,
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

    /// The scenario in a plot's subtitle.
    fn subtitle(self) -> &'static str {
        match self {
            Self::Solo => "one program hashing",
            Self::Shared => "two programs hashing at once; the time of either",
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
                                   \"lent pieces 64 MiB\", \"continuous 64 KiB\",
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
 * ("lent pieces 64 MiB", "idle 16", "continuous batch 1024"), the longest
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
    let svg = roster.whole_axes().then(|| generate_svg(&roster, &results, &machine, &selection_note));

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
    let svg_path = directory.join(format!("{stem}.graph.svg"));
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

    if let Some(svg) = &svg {
        fs::write(&svg_path, svg).unwrap_or_else(|error| {
            panic!("failed to write {}: {error}", svg_path.display())
        });
        let guide_path = directory.join(format!("{stem}.guide.html"));
        fs::write(&guide_path, generate_guide(&roster, &results, &machine)).unwrap_or_else(|error| {
            panic!("failed to write {}: {error}", guide_path.display())
        });
        println!("# API guide (HTML) is in \"{}\" .", guide_path.display());
    } else {
        remove_visualizations(&directory);
    }

    println!(
        "# Data results (text) are in \"{}\" .",
        text_path.display(),
    );
    match svg {
        Some(_) => println!("# Graph results (SVG) are in \"{}\" .", svg_path.display()),
        None => println!("# No graph: plots need at least two consecutive points from each axis’s start (its one point, on an axis of one) and a measured cell for every selected contender."),
    }
    println!("# Samples (TSV) are in \"{}\" .", samples_path.display());
}

/// Sparse runs replace the report and samples too, so any older full-run
/// visualization must leave their directory with them.
fn remove_visualizations(directory: &std::path::Path) {
    for name in ["bench-hashes.graph.svg", "bench-hashes.guide.html"] {
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
    hash_batch(algorithm, input, point, iterations, |digest| { black_box(digest); });
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
        DuoCopy { elapsed_ns: measured.calls.wall_ns, counts: measured.calls.counts, preparation: Some(measured.preparation), started_ns }
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
        Algorithm::Blake3 => |input| { black_box(blake3::hash(input)); },
        Algorithm::Blake3ServilSt => |input| {
            #[cfg(test)]
            observe_call("hash");
            black_box(blake3_servil::hash(input));
        },
        Algorithm::Blake3ServilMt => |input| { black_box(blake3_servil::hash_multithreaded(input)); },
        Algorithm::Sha256 => |input| { black_box(Sha256::digest(input)); },
        Algorithm::Sha256Ring => |input| { black_box(ring::digest::digest(&ring::digest::SHA256, input)); },
        Algorithm::Sha256CommonCrypto => |input| { black_box(common_crypto::sha256(input)); },
        Algorithm::Sha3_256 => |input| { black_box(sha3::Sha3_256::digest(input)); },
        Algorithm::Sha1Dc => |input| { black_box(sha1_checked::Sha1::try_digest(input)); },
        Algorithm::Blake3Rayon => |input| { black_box(blake3::Hasher::new().update_rayon(input).finalize()); },
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
    assert!(messages == 1 || input.len() == messages * message_len, "a batch is {messages} messages of {message_len} bytes");
    assert!(algorithm.takes_part(point.use_case), "{} takes no part in {:?}", algorithm.key(), point.use_case);

    match point.use_case {
        UseCase::LentPieces => return hash_stream(algorithm, input, iterations, consume),
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
    }
}

/*
 * A message in pieces (LentPieces): one message produced in PIECE_LEN pieces, each
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
    }
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
const SERVIL_CALLS: [(Algorithm, UseCase, &str); 16] = [
    (Algorithm::Blake3ServilSt, UseCase::OneMessage, "hash(input), each call after other work"),
    (Algorithm::Blake3ServilSt, UseCase::ManyMessages, "hash_many(batch, 64, out), the padded batch contract, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::OneMessage, "hash_multithreaded(input), each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ManyMessages, "hash_many_multithreaded(batch, 64, out), the padded batch contract, each call after other work"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousMessages, "Queue::messages(Mode::Hash) for messages of up to 64 KiB, Queue::pieces(Mode::Hash) in 64 KiB pieces for longer ones, one message after another, each read into free buffers of the program's, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept"),
    (Algorithm::Blake3ServilMt, UseCase::ContinuousBatches, "Queue::fixed(64, Mode::Hash), one batch after another, each read into a free buffer of the program's, submitted with its digests' space, about 1 MiB or 1024 buffers in flight, whichever is fewer, cycled through the handler and a bounded channel with room for all of them (std::sync::mpsc::sync_channel, allocated when made), the queue and the channel made once and kept"),
    (Algorithm::Blake3ServilSt, UseCase::LentMessages, "hash(input), one message after another, each read into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilSt, UseCase::LentPieces, "Hasher::update per 64 KiB piece, then finalize, messages one after another, each piece read into a kept buffer and lent until the update returns"),
    (Algorithm::Blake3ServilSt, UseCase::LentBatches, "hash_many(batch, 64, out), the padded batch contract, batches one after another, each read into a kept buffer and lent with kept digests until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::LentMessages, "hash_multithreaded(input), one message after another, each read into a kept buffer and lent until the call returns"),
    (Algorithm::Blake3ServilMt, UseCase::LentPieces, "Hasher::update_multithreaded per 64 KiB piece, then finalize, messages one after another, each piece read into a kept buffer and lent until the update returns"),
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
        UseCase::OneMessage | UseCase::IdleOneMessage | UseCase::ManyMessages | UseCase::IdleManyMessages | UseCase::LentMessages | UseCase::LentPieces | UseCase::LentBatches => 1,
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

/*
 * Requires a non-empty, ascending slice. For an even count the median is
 * the mean of the two middle values, rounded half up.
 */
/// Each sample's time per unit, in the samples' order.
fn per_units(samples: &[Measured]) -> Vec<PerUnit> {
    samples.iter().map(|m| m.per_unit()).collect()
}

/// A cell's summary from its samples: their mean (clocks::summary), kept
/// exact for display, and their extremes.
fn summarize_measured(samples: &[Measured]) -> Statistics {
    assert!(!samples.is_empty(), "a cell has at least one sample");
    let values = per_units(samples);
    let exact = ExactMean::of(samples);
    Statistics {
        count: samples.len(),
        minimum: *values.iter().min().unwrap(),
        mean: exact.fixed(),
        maximum: *values.iter().max().unwrap(),
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
    /// One sentence on why the path changes here, for the first dot of the kernel.
    why: String,
    /// Mark drawn at every dot in this kernel.
    mark: Mark,
}

/// Dot shapes: the same colour keeps the contender's identity, the shape
/// says which code path produced the point.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Circle,
    Diamond,
    Square,
    /// A fourth kernel.
    Triangle,
    /// A fifth, which only the multithreaded servil contender has.
    DownTriangle,
}

impl Mark {
    /// The shape as a character, for text beside the drawn marks.
    fn glyph(self) -> char {
        match self {
            Mark::Circle => '●',
            Mark::Diamond => '◆',
            Mark::Square => '■',
            Mark::Triangle => '▲',
            Mark::DownTriangle => '▼',
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Diamond => "diamond",
            Self::Square => "square",
            Self::Triangle => "triangle",
            Self::DownTriangle => "downward triangle",
        }
    }
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
            why: "Each chunk (1 KiB) is hashed on its own, one after another.".to_owned(),
            mark: Mark::Circle,
        }];
        if degree > 4 {
            kernels.push(Kernel {
                first: 4 * 1024,
                name: "SSE4.1 vectors, four chunks at a time".to_owned(),
                why: "Four whole chunks fill the narrowest vectors; the wider ones wait for more chunks.".to_owned(),
                mark: Mark::Diamond,
            });
        }
        if degree > 1 {
            kernels.push(Kernel {
                first: degree * 1024,
                name: wide.to_owned(),
                why: "Enough whole chunks to fill this CPU's widest vectors.".to_owned(),
                mark: if degree > 4 { Mark::Square } else { Mark::Diamond },
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
                    why: "Below four whole chunks (4 KiB), each chunk is hashed by the crate's portable code, one after another.".to_owned(),
                    mark: Mark::Circle,
                },
                Kernel {
                    first: 4 * 1024,
                    name: "NEON vectors, four chunks at a time".to_owned(),
                    why: "Four whole chunks fill the NEON vector units; from here most of the input runs four chunks at a time.".to_owned(),
                    mark: Mark::Diamond,
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
            why: "This build hashes every size with the crate's portable code.".to_owned(),
            mark: Mark::Circle,
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
            why: "One method at every size.".to_owned(),
            mark: Mark::Circle,
        }],
    )
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
            why: "One method at every size.".to_owned(),
            mark: Mark::Circle,
        }],
    )
}

fn detect_sha1dc_kernels() -> Kernels {
    Kernels::new(
        "sha1-checked",
        vec![Kernel {
            first: 0,
            name: "portable code with collision detection".to_owned(),
            why: "One method at every size.".to_owned(),
            mark: Mark::Circle,
        }],
    )
}

fn detect_common_crypto_kernels() -> Kernels {
    Kernels::new(
        "CommonCrypto",
        vec![Kernel {
            first: 0,
            name: "Apple's library, with the CPU's SHA-256 instructions".to_owned(),
            why: "One method at every size.".to_owned(),
            mark: Mark::Circle,
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
            why: "One method at every size.".to_owned(),
            mark: Mark::Circle,
        }],
    )
}

/*
 * The servil fork describes its own kernels: kernel_report() and
 * kernel_report_multithreaded() come from the same run-time detection
 * its hash functions use, so the report describes what was measured. The
 * bencher adds only the dot shapes, in order.
 */
fn servil_kernels(report: blake3_servil::KernelReport) -> Kernels {
    const MARKS: [Mark; 5] = [Mark::Circle, Mark::Diamond, Mark::Square, Mark::Triangle, Mark::DownTriangle];
    assert!(
        report.kernels.len() <= MARKS.len(),
        "the graph has {} dot shapes; the fork reports {} kernels",
        MARKS.len(),
        report.kernels.len(),
    );
    let kernels = report
        .kernels
        .iter()
        .zip(MARKS)
        .map(|(kernel, mark)| Kernel { first: kernel.from_len, name: kernel.name.to_owned(), why: kernel.why.to_owned(), mark })
        .collect();
    Kernels::new(report.platform, kernels)
}

/// The code paths a contender runs in a use case, by the point's bytes.
/// The contender must take part in the use case.
fn detect_kernels(algorithm: Algorithm, use_case: UseCase) -> Kernels {
    assert!(algorithm.takes_part(use_case), "{} takes no part in {use_case:?}", algorithm.name());
    // Queue and incremental MT APIs have no kernel-report entry point.
    // Their known API boundaries remain useful; their schedules stay
    // explicitly unreported instead of inheriting one-shot thresholds.
    if algorithm == Algorithm::Blake3ServilMt {
        let why = "The contender leaves this API's kernel schedule unreported.";
        let kernel = |first, api: &str| Kernel { first, name: api.to_owned(), why: why.to_owned(), mark: Mark::Circle };
        let kernels = match use_case {
            UseCase::ContinuousMessages => Some(vec![kernel(0, "Queue::messages"), kernel(PIECE_LEN + 1, "Queue::pieces")]),
            UseCase::ContinuousBatches => Some(vec![kernel(0, "Queue::fixed")]),
            UseCase::LentPieces => Some(vec![kernel(0, "Hasher::update_multithreaded")]),
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
    };
    match use_case {
        /* An idle use case makes its twin's call. */
        UseCase::IdleOneMessage | UseCase::IdleManyMessages => detect_kernels(algorithm, use_case.call()),
        UseCase::OneMessage | UseCase::LentMessages => one_message,
        /* A stream runs the one-message kernels piece by piece, so those that start past PIECE_LEN never run. */
        UseCase::LentPieces => one_message.up_to(PIECE_LEN),
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
                    why: "Each message is its own call; the size of the batch changes nothing.".to_owned(),
                    mark: Mark::Circle,
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
        why: format!("Below {words} messages, the batch function hashes each message on its own, one after another."),
        mark: Mark::Circle,
    }];
    if degree > 1 {
        kernels.push(Kernel {
            first: degree * message_len,
            name: format!("{} vectors, {words} messages at a time", blake3.platform),
            why: format!("From {words} messages, each group of {words} fills this CPU's widest vectors, one message per lane; messages past the last full group are hashed one at a time."),
            mark: Mark::Diamond,
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
                why: "Up to one vector width of chunks, there is nothing to split.".to_owned(),
                mark: Mark::Circle,
            },
            Kernel {
                first: 2 * degree_bytes,
                name: "split over Rayon's threads".to_owned(),
                why: "Above that, the input splits in halves, again and again, and idle threads of the Rayon pool take them.".to_owned(),
                mark: Mark::Diamond,
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
    let mut columns = false;
    let mut cells = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("# load: ") {
            load = Some(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix("# power: ") {
            power = Some(rest.to_owned());
        } else if line.starts_with('#') || line.is_empty() {
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
    SamplesFile { load: load.unwrap_or_else(|| panic!("{path}: a load line")), power: power.unwrap_or_else(|| panic!("{path}: a power line")), cells }
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
/// bulk on one thread; a long message in pieces; batches as members and as
/// tasks of their own.
const REGRESS_POINTS: [&str; 6] = ["lent 64 B", "lent 64 KiB", "lent 1 MiB", "lent pieces 64 MiB", "lent batch 16", "lent batch 4096"];
const REGRESS_ROUNDS: usize = 24;
const REGRESS_PAIRS: usize = 8;

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
    let mut means = |exe: &str| -> std::collections::HashMap<String, u128> {
        let file = regress_run(exe);
        if !file.load.starts_with("quiet") {
            unreliable.push(file.load.clone());
        }
        if !powers.contains(&file.power) {
            powers.push(file.power.clone());
        }
        file.cells.into_iter().map(|(key, samples)| (key, ExactMean::of(&samples).fixed().0)).collect()
    };
    for pair in 0..REGRESS_PAIRS {
        let began = clocks::now();
        let (old, new) = if pair % 2 == 0 { let o = means(old_exe); (o, means(new_exe)) } else { let n = means(new_exe); (means(old_exe), n) };
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
        for label in ["64 B", "256 B", "1 KiB", "4 KiB"] {
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

/// A point's x position on an axis of the points in `range`, as a
/// fraction of the axis width; both axes are logarithmic in bytes. An
/// axis of one point puts it in the middle.
fn x_fraction(point_index: usize, range: std::ops::Range<usize>) -> f64 {
    if range.len() == 1 {
        return 0.5;
    }
    let smallest = (POINTS[range.start].bytes as f64).log2();
    let largest = (POINTS[range.end - 1].bytes as f64).log2();
    ((POINTS[point_index].bytes as f64).log2() - smallest) / (largest - smallest)
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

/*
 * Rate from a time per unit, with the use case's unit: GB/s = 1 / (ns/B),
 * Mmsg/s = 1000 / (ns/msg). Shown to one decimal below 10, whole numbers
 * above.
 */
fn format_rate(time: PerUnit, use_case: UseCase) -> String {
    let tenths = time.tenths_of(use_case.rate_scale());
    if tenths >= 100 {
        format!("{} {}", (tenths + 5) / 10, use_case.rate_unit())
    } else {
        format!("{}.{} {}", tenths / 10, tenths % 10, use_case.rate_unit())
    }
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

/*
 * Layout constants shared by the static geometry (Rust) and the interactive
 * relayout (JavaScript inside the SVG). The script receives them through a
 * JSON block, so a single source of truth drives both.
 *
 * Two plots stack down the canvas, one per use case, sharing the x extent,
 * the unit switch, and the contender toggles at right.
 */
const SVG_WIDTH: f64 = 1300.0;
/// Canvas height: the provenance block ends where the lines end, with the
/// bottom margin that follows.
fn svg_height(provenance_top: f64, provenance_lines: usize) -> f64 {
    provenance_line_y(provenance_top, provenance_lines.saturating_sub(1)) + PROVENANCE_LINE_HEIGHT + 8.0
}
const PLOT_LEFT: f64 = 110.0;
const PLOT_RIGHT: f64 = 1000.0;
/// The first plot's top, below the title, two method lines, and its own
/// heading; each further plot sits PLOT_PITCH lower.
const PLOT_TOP: f64 = 244.0;
const PLOT_HEIGHT: f64 = 340.0;
/// Room under a plot for its x labels, axis title, and shape legend, and
/// above the next for its heading.
const PLOT_PITCH: f64 = 470.0;
const X_INSET: f64 = 40.0;
/// Vertical room per right-hand label (name and detail line). Eight
/// contenders, the most a run takes, stack in 7 × 34 px, inside a plot.
const SERIES_LABEL_GAP: f64 = 34.0;
/// The highest a legend name sits: this far below the plot's top, level
/// with its top grid line, so the stack stays clear of the plot's title
/// and of the header above it. Nine names at SERIES_LABEL_GAP fit between
/// here and the bottom.
const LABEL_TOP_ROOM: f64 = 4.0;
/// Fixed shape slots before each right-hand name, so names align across
/// contenders with different shape counts. Four covers every contender.
const SWATCH_SLOTS: usize = 4;
/// Where provenance starts, below the last of `plots` plots.
fn provenance_top(plots: usize) -> f64 {
    PLOT_TOP + (plots - 1) as f64 * PLOT_PITCH + PLOT_HEIGHT + 85.0
}

const PROVENANCE_LINE_HEIGHT: f64 = 14.0;

fn plot_top(plot_index: usize) -> f64 {
    PLOT_TOP + plot_index as f64 * PLOT_PITCH
}

fn plot_bottom(plot_index: usize) -> f64 {
    plot_top(plot_index) + PLOT_HEIGHT
}

/*
 * The y axis spans [axis_min, axis_max] on a log scale, with room below
 * the smallest minimum and above the largest maximum. The script applies
 * the same rule to whichever contenders are on, so the axis reflects the
 * visible data alone.
 */
fn log_axis_bounds(observed_min: f64, observed_max: f64) -> (f64, f64) {
    assert!(
        observed_min.is_finite() && observed_min > 0.0,
        "log axis requires positive measurements"
    );
    assert!(
        observed_max.is_finite() && observed_max >= observed_min,
        "graph maximum must be finite and at least the minimum"
    );

    (
        nice_log_bound_below(observed_min * 0.92),
        nice_log_bound_above(observed_max * 1.08),
    )
}

/*
 * One plot's geometry: its use case, the points along its x axis, the
 * contenders that take part, its y axis in GB/s, and where each
 * contender's right-hand label sits.
 */
struct Plot {
    index: usize,
    scenario: Scenario,
    use_case: UseCase,
    points: std::ops::Range<usize>,
    /// Roster indices of the contenders measured in this use case.
    contenders: Vec<usize>,
    top: f64,
    bottom: f64,
    log_min: f64,
    log_max: f64,
    axis_min: f64,
    axis_max: f64,
    /// Rate per unit time: rate = scale / (ns per unit). 1 for GB/s from
    /// ns/B, 1000 for million messages per second from ns/message.
    scale: f64,
    /// Pixel x per point of this plot, in axis order.
    x_positions: Vec<f64>,
    /// Pixel y of each contender's label at right; None for a contender
    /// that takes no part here.
    label_y: Vec<Option<f64>>,
    /// Each contender's kernels in this use case; None for a non-participant.
    kernels: Vec<Option<Kernels>>,
    /// Where the provenance block starts, below the last plot.
    provenance_top: f64,
}

impl Plot {
    fn new(index: usize, scenario: Scenario, use_case: UseCase, roster: &Roster, results: &Results) -> Self {
        let measured: Vec<usize> = use_case.points().filter(|&index| roster.measures(index)).collect();
        let points = measured[0]..measured[measured.len() - 1] + 1;
        let contenders: Vec<usize> = (0..roster.len())
            .filter(|&algorithm_index| roster.algorithms[algorithm_index].takes_part(use_case))
            .collect();
        assert!(!contenders.is_empty(), "a plot has at least one contender");

        /*
         * From here down the SVG needs pixel positions on a log axis, which
         * is where floating point earns its place: the values are drawn,
         * never compared or reported. Everything above this point is integer.
         */
        /* The axis spans the contenders shown when the graph opens, as the script's does. */
        let shown = shown_at_first(roster);
        let visible: Vec<usize> = contenders.iter().copied().filter(|&a| shown[a]).collect();
        assert!(!visible.is_empty(), "every contender shown at first takes part in every use case");
        let cells = || visible.iter().flat_map(|&a| points.clone().map(move |s| (a, s))).map(|(a, s)| cell(results, a, s));
        let observed_max = cells().map(|cell| cell.get(scenario).mean).max().expect("there are results");
        let observed_min = cells().map(|cell| cell.get(scenario).mean).min().expect("there are results");

        /*
         * The static render shows gigabytes per second, the default unit:
         * the axis spans the reciprocals of the observed times, so on the
         * log axis the plot mirrors a time-per-byte one, fastest at the
         * top. The script rebuilds all of this when the unit flips.
         */
        let scale = use_case.rate_scale() as f64;
        let observed_lo_rate = scale / observed_max.ns_f64();
        let observed_hi_rate = scale / observed_min.ns_f64();
        let (axis_min, axis_max) = log_axis_bounds(observed_lo_rate, observed_hi_rate);

        let x_positions: Vec<f64> = points
            .clone()
            .map(|point_index| PLOT_LEFT + X_INSET + x_fraction(point_index, points.clone()) * (PLOT_RIGHT - PLOT_LEFT - 2.0 * X_INSET))
            .collect();

        let kernels: Vec<Option<Kernels>> = roster
            .algorithms
            .iter()
            /* Only the methods that start within the plot's inputs (a run's --points may stop early). */
            .map(|&algorithm| algorithm.takes_part(use_case).then(|| detect_kernels(algorithm, use_case).up_to(POINTS[points.end - 1].bytes)))
            .collect();

        let mut plot = Self {
            index,
            scenario,
            use_case,
            points: points.clone(),
            contenders: contenders.clone(),
            top: plot_top(index),
            bottom: plot_bottom(index),
            log_min: axis_min.ln(),
            log_max: axis_max.ln(),
            axis_min,
            axis_max,
            scale,
            x_positions,
            label_y: vec![None; roster.len()],
            kernels,
            provenance_top: 0.0,
        };

        /*
         * Right-edge series labels double as toggles. Each sits level with
         * its line's last point, pushed apart when medians nearly coincide.
         * The script repeats this rule after each toggle; a hidden
         * contender keeps its slot, anchored where its line would end on
         * the current axis and clamped to the plot edge, so its grey label
         * points toward its data.
         */
        let last = points.end - 1;
        let mut label_slots: Vec<(usize, f64)> = contenders
            .iter()
            .map(|&algorithm_index| (algorithm_index, plot.map_y(plot.stats(results, algorithm_index, last).mean)))
            .collect();
        label_slots.sort_by(|a, b| a.1.total_cmp(&b.1));
        for index in 1..label_slots.len() {
            let minimum_y = label_slots[index - 1].1 + SERIES_LABEL_GAP;
            if label_slots[index].1 < minimum_y {
                label_slots[index].1 = minimum_y;
            }
        }
        /*
         * Keep the stack inside the plot: with many contenders the
         * pushed-apart labels can run past the bottom axis, so the whole
         * stack shifts up by the overrun. The script applies the same rule.
         */
        let overrun = (label_slots.last().map(|slot| slot.1).unwrap_or(0.0) + 20.0 - plot.bottom).max(0.0);
        /* ...and never above the plot's top: from there the names space out again downward. */
        let mut previous = f64::NEG_INFINITY;
        for (algorithm_index, label_y) in label_slots {
            let y = (label_y - overrun).max(plot.top + LABEL_TOP_ROOM).max(previous + SERIES_LABEL_GAP);
            plot.label_y[algorithm_index] = Some(y);
            previous = y;
        }
        plot
    }

    /// Pixel y for a rate (GB/s, or million messages per second) on the log axis.
    fn map_rate(&self, value: f64) -> f64 {
        assert!(value > 0.0);
        self.bottom - (value.ln() - self.log_min) / (self.log_max - self.log_min) * (self.bottom - self.top)
    }

    /// Pixel y for a measured value.
    fn map_y(&self, time: PerUnit) -> f64 {
        self.map_rate(self.scale / time.ns_f64())
    }

    /// A contender's statistics at a point, in this plot's scenario.
    fn stats(&self, results: &Results, algorithm_index: usize, point_index: usize) -> Statistics {
        cell(results, algorithm_index, point_index).get(self.scenario)
    }

    fn kernels(&self, algorithm_index: usize) -> &Kernels {
        self.kernels[algorithm_index].as_ref().expect("the contender takes part in this plot")
    }

    /// The point count along this plot's axis.
    fn len(&self) -> usize {
        self.points.len()
    }
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
            write!(data, "{{\"scenario\":\"{}\",\"use\":\"{:?}\",\"batch\":{},\"labels\":[", scenario.key(), use_case, use_case.batch()).unwrap();
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
                /* Per point: the mean (ns per unit), and the exact per-call latency. */
                let per_point = |f: &dyn Fn(Statistics, u64) -> String| -> String {
                    points.iter().map(|&index| f(cell(results, algorithm_index, index).get(scenario), use_case.units(POINTS[index], 1))).collect::<Vec<_>>().join(",")
                };
                write!(data, "\"mean\":[{}],", per_point(&|t, _| t.format_mean(1))).unwrap();
                write!(data, "\"lat\":[{}],", per_point(&|t, units| t.format_mean(units))).unwrap();
                /* The code paths, by the first byte count each serves; the graph's marks name them. */
                let kernels = detect_kernels(*algorithm, use_case).up_to(POINTS[*points.last().unwrap()].bytes);
                data.push_str("\"kernels\":[");
                for (k, kernel) in kernels.kernels.iter().enumerate() {
                    if k > 0 { data.push(','); }
                    write!(data, "{{\"from\":{},\"name\":{},\"mark\":\"{}\"}}", kernel.first, json_string(&kernel.name), kernel.mark.name()).unwrap();
                }
                data.push_str("]}");
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

fn generate_svg(
    roster: &Roster,
    results: &Results,
    machine: &MachineMetadata,
    selection_note: &str,
) -> String {
    assert!(roster.len() >= 2, "a graph compares at least two contenders");

    /* Solo plots first, then shared: one per use case the run measured. */
    let mut plots: Vec<Plot> = Vec::new();
    for scenario in Scenario::ALL {
        for use_case in UseCase::ALL.into_iter().filter(|&use_case| scenario.measures(use_case)) {
            if use_case.points().any(|index| roster.measures(index)) {
                plots.push(Plot::new(plots.len(), scenario, use_case, roster, results));
            }
        }
    }
    let provenance_top = provenance_top(plots.len());
    for plot in &mut plots {
        plot.provenance_top = provenance_top;
    }

    let mut provenance_cats = shared_provenance_cats(machine, selection_note);
    provenance_cats.push(code_path_cat(roster, &plots));
    /* The hashes' own lines follow this header, in the first plot's series groups. */
    provenance_cats.push(ProvCat {
        key: "hashes",
        name: "Hashes",
        summary: "the version and settings of each hash shown".to_owned(),
        lines: Vec::new(),
    });
    /* Each contender's provenance lines go with the first plot it takes part in. */
    let first_plot: Vec<usize> = (0..roster.len())
        .map(|algorithm_index| {
            plots.iter().position(|plot| plot.kernels[algorithm_index].is_some()).expect("every contender takes part in some plot")
        })
        .collect();
    let provenance_total = provenance_cats.len()
        + provenance_cats.iter().map(|cat| cat.lines.len()).sum::<usize>()
        + roster
            .algorithms
            .iter()
            .enumerate()
            .map(|(algorithm_index, &algorithm)| contender_provenance_lines(algorithm, plots[first_plot[algorithm_index]].kernels(algorithm_index)).len())
            .sum::<usize>();
    let svg_height = svg_height(provenance_top, provenance_total);

    let mut svg = String::new();

    writeln!(svg, r##"<?xml version="1.0" encoding="UTF-8"?>"##)
        .unwrap();

    writeln!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SVG_WIDTH:.0} {svg_height:.0}" width="{SVG_WIDTH:.0}" height="{svg_height:.0}" onclick="tapAway()">"##
    )
        .unwrap();

    writeln!(
        svg,
        "  <!-- For maintainers: bench-hashes' src/main.rs (generate_svg) writes this file. The script at the end carries the measurements and \
         layout constants as DATA and redraws from them; <metadata> holds the full provenance and crate checksums; the samples file beside \
         this one holds every timing. The text for readers follows the fork's AGENTS.md, \"Write each page for a reader who holds only the page\". -->"
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <rect id="page" width="{SVG_WIDTH:.0}" height="{svg_height:.0}" fill="#fdfdfc"/>"##
    )
        .unwrap();

    svg.push_str(
        r##"  <style>
    text { font-family: -apple-system, "Segoe UI", "Helvetica Neue", Arial, sans-serif; }
    .title { font-size: 22px; font-weight: 700; fill: #1a1a1a; }
    .method { font-size: 11px; fill: #8a8a8a; }
    .plot-title { font-size: 14px; font-weight: 700; fill: #333333; }
    .plot-sub { font-size: 11px; fill: #8a8a8a; }
    .axis-title { font-size: 12px; fill: #666666; }
    .better-arrow { fill: none; stroke: #8a8a8a; stroke-width: 1.2; stroke-linecap: round; stroke-linejoin: round; }
    .tick-label { font-size: 11px; fill: #777777; }
    .size-label { font-size: 11px; font-weight: 600; fill: #333333; }
    .size-tick { stroke: #bbbbbb; stroke-width: 1; }
    .zoom-btn { cursor: pointer; }
    .zoom-btn rect { fill: #f1f1ee; stroke: #d2d2cd; stroke-width: 1; }
    .zoom-btn text { font-size: 13px; font-weight: 600; fill: #333333; }
    .zoom-btn:hover rect { fill: #e4e4de; }
    .zoom-btn[data-off="true"] { display: none; }
    .zoom-track { fill: #e6e6e1; }
    .zoom-track-hit { fill: transparent; }
    .door { fill: #5b21b6; cursor: pointer; }
    .door:hover { text-decoration: underline; }
    .howto-box { fill: #ffffff; fill-opacity: 0.98; stroke: #c8c8c4; stroke-width: 1; }
    .howto-line { font-size: 12px; fill: #333333; }
    .series-absent { font-size: 12px; fill: #b8b8b2; }
    .series-absent-detail { font-size: 9px; fill: #c4c4be; }
    .header-edge { stroke: #ecece8; stroke-width: 1; }
    .zoom-tick { stroke: #b4b4ae; stroke-width: 1; }
    .zoom-band { fill: #5b21b6; fill-opacity: 0.16; stroke: #5b21b6; stroke-opacity: 0.55; stroke-width: 1; }
    .zoom-grip { cursor: ew-resize; touch-action: none; }
    .zoom-band { cursor: grab; touch-action: none; }
    .zoom-tick[data-at="true"] { stroke: #5b21b6; stroke-width: 2; }
    .chip { cursor: pointer; }
    .chip rect { fill: #ffffff; stroke: #cfcfca; stroke-width: 1; }
    .chip text { font-size: 11px; fill: #8a8a8a; }
    .chip[data-on="true"] rect { fill: #ede9fe; stroke: #a78bfa; }
    .chip[data-on="true"] text { fill: #3b0764; font-weight: 600; }
    .chip-label { font-size: 10px; fill: #9a9a9a; }
    .chip[data-live="false"] { opacity: 0.38; }
    .chip-tie { fill: none; stroke: #cfcfca; stroke-width: 1; }
    .plot-off { visibility: hidden; pointer-events: none; }
    .plot-off .series-prov { visibility: visible; }
    .zoom-grip-hit { fill: transparent; }
    .zoom-grip-bar { fill: #5b21b6; fill-opacity: 0.7; }
    .zoom-grip:hover .zoom-grip-bar { fill-opacity: 1; }
    .value-label { font-size: 10px; font-weight: 700; }
    .series-name { font-size: 13px; font-weight: 700; }
    .series-detail { font-size: 10px; fill: #777777; }
    .series-hint { font-size: 9px; fill: #b0b0b0; }
    .series-note { font-size: 10px; font-weight: 400; fill: #777777; }
    .annotation { font-size: 10px; font-style: italic; fill: #8a8a8a; }
    .prov-head { font-size: 10px; font-weight: 700; fill: #aaaaaa; letter-spacing: 0.1em; }
    .prov-head-row { cursor: pointer; }
    .prov { font-size: 9px; fill: #9a9a9a; }
    .grid { stroke: #e8e8e6; stroke-width: 1; }
    .grid-x { stroke: #f0f0ee; stroke-width: 1; }
    .axis { stroke: #55555a; stroke-width: 1; }
    .divider { stroke: #e0e0de; stroke-width: 1; }
    .series-label { cursor: pointer; transition: transform 0.3s ease; }
    .series-hint { display: none; }
    .marks, .dots { transition: opacity 0.3s ease; }
    .marks { pointer-events: none; }
    /* Hovering a name dims the other contenders' marks alone: every name
       keeps its shown or hidden look, so the names always tell which
       contenders are shown (a dimmed name read as a hidden one). */
    .series[data-dim="true"] .marks, .dots[data-dim="true"] { opacity: 0.25; }
    .series[data-on="false"] .marks, .dots[data-on="false"] { opacity: 0; pointer-events: none; }
    .series[data-on="false"] .series-name { fill: #9a9a9a; }
    .series[data-on="false"] .series-detail { display: none; }
    .series[data-on="false"] .series-hint { display: inline; }
    .series[data-on="false"] .series-prov { display: none; }
    .series[data-on="false"] .series-swatch * { fill: #fdfdfc; stroke: #b0b0b0; }
    .series[data-hl="true"] .series-name { text-decoration: underline; }
    .series-swatch { stroke-width: 2; transition: fill 0.3s ease; }
    #unit-switch { cursor: pointer; }
    .unit-track { fill: #e8e8e6; stroke: #c8c8c4; stroke-width: 1; }
    @media (hover: hover) {
      .prov-head-row:hover .prov-head { text-decoration: underline; }
      .series-label:hover .series-name { text-decoration: underline; }
      #unit-switch:hover .unit-track { stroke: #55555a; }
    }
    .unit-knob { fill: #55555a; transition: cy 0.35s ease; }
    .unit-label { font-size: 10px; font-weight: 600; fill: #b0b0b0; transition: fill 0.35s ease; }
    .unit-label.unit-on { fill: #333333; }
    .dot { cursor: crosshair; }
    .legend { font-size: 10px; fill: #8a8a8a; }
    #hover { pointer-events: none; }
    #hover-guide { stroke: #9a9a9a; stroke-width: 1; stroke-dasharray: 3,3; }
    #hover-box { fill: #ffffff; fill-opacity: 0.97; stroke: #c8c8c4; stroke-width: 1; }
    .hover-head { font-size: 12px; font-weight: 700; fill: #1a1a1a; }
    .hover-sub { font-size: 10px; fill: #777777; }
    .hover-row { font-size: 11px; fill: #333333; }
    .hover-row-focus { font-weight: 700; }
    .hover-ratio { font-size: 11px; font-weight: 600; }
    .hover-note { font-size: 9px; font-style: italic; fill: #9a9a9a; }
  </style>
"##,
    );

    /*
     * The header (title, method lines, unit switch, zoom row, chips) is one
     * group at the top of the page. It stays there as the page scrolls, so
     * a plot and the screen's top never overlap; the chips bring the plot a
     * reader wants up to it (Zooko, September 26, 2026: on a phone a header
     * that followed the page covered the top of every plot).
     */
    writeln!(svg, r##"  <g id="header">"##).unwrap();
    writeln!(svg, r##"  <rect id="header-bg" x="0" y="0" width="{SVG_WIDTH:.0}" height="{HEADER_BOTTOM:.0}" fill="#fdfdfc"/>"##).unwrap();
    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.0}" y="44" class="title">Cryptographic Hash Performance</text>"##
    )
        .unwrap();

    /*
     * Under the title: where and when, then what a reader can do, with a
     * door to how the graph is drawn. Everything a newcomer needs to read
     * the plots is here; the finer points wait behind the door.
     */
    let os_name = match machine.os_type.as_str() {
        os if os.starts_with("darwin") => "macOS",
        os if os.starts_with("linux") => "Linux",
        os => os,
    };
    let date = machine.timestamp.split(' ').next().unwrap_or(&machine.timestamp);
    let busy = machine.load.iter().any(clocks::load::Window::busy);
    let on_battery = machine.power.iter().flatten().any(|power| power.on_battery);
    let caveat = match (busy, machine.power_slowing()) {
        (true, true) => " · other programs were busy and the machine saved power during the run, so some results may read slow",
        (true, false) => " · other programs were busy during the run, so some results may read slow",
        (false, true) if on_battery => " · the machine ran on battery power, which can change results",
        (false, true) => " · the machine ran in a low-power mode, which can change results",
        (false, false) => "",
    };
    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.0}" y="72" class="method">How fast each hash runs on {} ({os_name}), measured {date}{caveat}</text>"##,
        xml_escape(&machine.cpu_type),
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.0}" y="88" class="method">Hover or tap a dot to compare the hashes there · click a name at right to show or hide it · <tspan id="howto-door" class="door" onclick="event.stopPropagation(); toggleHowto()">How to read this graph ▸</tspan></text>"##
    )
    .unwrap();

    /*
     * Unit switch, in the header above the y titles it changes: a vertical track with a knob
     * that slides between GB/s (top) and ns/B (bottom). Clicking anywhere
     * on the switch flips every plot. The knob's position is the state;
     * the label beside it reads darker. Without script the graph stays in
     * GB/s and the switch is inert.
     */
    writeln!(
        svg,
        r##"  <g id="unit-switch" transform="translate({:.1} {:.1})" onclick="event.stopPropagation(); flipUnit()">"##,
        UNIT_SWITCH_LEFT,
        UNIT_SWITCH_TOP,
    )
        .unwrap();
    writeln!(svg, r##"    <title>Switch every plot between rate (GB/s, million messages per second) and time (ns per byte, ns per message)</title>"##).unwrap();
    writeln!(svg, r##"    <rect class="unit-hit" x="-4" y="-4" width="60" height="42" fill="transparent"/>"##).unwrap();
    writeln!(svg, r##"    <rect class="unit-track" x="0" y="0" width="14" height="34" rx="7"/>"##).unwrap();
    writeln!(svg, r##"    <circle id="unit-knob" class="unit-knob" cx="7" cy="7" r="5"/>"##).unwrap();
    writeln!(svg, r##"    <text class="unit-label unit-on" data-unit="gbps" x="20" y="11">rate</text>"##).unwrap();
    writeln!(svg, r##"    <text class="unit-label" data-unit="ns" x="20" y="31">time</text>"##).unwrap();
    writeln!(svg, "  </g>").unwrap();

    /*
     * Zoom, a row above the first plot: the inputs every plot shows. A
     * strip spans the plots' width with a tick for every input the plots
     * have (a batch counts its messages' bytes), on the plots' logarithmic
     * spacing, spanning the x range of the plots' inputs, so at the full
     * range each tick stands over its input below; a band covers the inputs
     * shown, drawn across the whole width in the plots. No numbers: each plot's
     * own axis names its inputs, sizes or message counts. Two arrows at
     * the strip's left end move the first input shown, two at its right
     * end the last, and "all" shows every input; they never move. Without
     * script the graph shows every input and the controls are inert.
     */
    let all_bytes: Vec<usize> = {
        let mut v: Vec<usize> = plots.iter().flat_map(|plot| plot.points.clone().map(|i| POINTS[i].bytes)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (lo, hi) = ((all_bytes[0] as f64).log2(), (*all_bytes.last().expect("a graph has points") as f64).log2());
    let strip_x = |bytes: usize| ZOOM_STRIP_LEFT + ((bytes as f64).log2() - lo) / (hi - lo) * (ZOOM_STRIP_RIGHT - ZOOM_STRIP_LEFT);
    writeln!(svg, r##"  <g id="zoom" transform="translate(0 {:.1})">"##, ZOOM_ROW_TOP).unwrap();
    /* "all" shows only when it can act; the page opens at the full range. */
    let button = |svg: &mut String, id: &str, x: f64, width: f64, glyph: &str, action: &str, title: &str| {
        let off = id == "zoom-all";
        writeln!(
            svg,
            r##"    <g class="zoom-btn" id="{id}" data-off="{off}" transform="translate({x:.1} 0)" onclick="event.stopPropagation(); {action}"><title>{title}</title><rect x="0" y="0" width="{width:.1}" height="18" rx="4"/><text x="{:.1}" y="13" text-anchor="middle">{glyph}</text></g>"##,
            width / 2.0,
        )
        .unwrap();
    };
    writeln!(svg, r##"    <g><title>The inputs every plot shows, from smallest (left) to largest; drag an end of the band, or the band itself</title><rect class="zoom-track-hit" x="{ZOOM_STRIP_LEFT:.1}" y="0" width="{:.1}" height="18"/><rect class="zoom-track" x="{ZOOM_STRIP_LEFT:.1}" y="7" width="{:.1}" height="4" rx="2"/></g>"##, ZOOM_STRIP_RIGHT - ZOOM_STRIP_LEFT, ZOOM_STRIP_RIGHT - ZOOM_STRIP_LEFT).unwrap();
    for (i, &bytes) in all_bytes.iter().enumerate() {
        writeln!(svg, r##"    <line class="zoom-tick" id="zoom-tick-{i}" x1="{0:.1}" y1="4" x2="{0:.1}" y2="14"/>"##, strip_x(bytes)).unwrap();
    }
    let (band_left, band_right) = (ZOOM_STRIP_LEFT, ZOOM_STRIP_RIGHT);
    /* The band itself: drag it to move both ends at once. */
    writeln!(svg, r##"    <rect id="zoom-band" class="zoom-band" x="{:.1}" y="1" width="{:.1}" height="16" rx="3" onpointerdown="gripDown(event, 'both')" onclick="event.stopPropagation()"/>"##, band_left - 3.0, band_right - band_left + 6.0).unwrap();
    // A grip at each end of the band: drag it to move that end.
    for (end, x) in [("from", band_left), ("to", band_right)] {
        writeln!(
            svg,
            r##"    <g class="zoom-grip" id="zoom-grip-{end}" transform="translate({x:.1} 0)" onpointerdown="gripDown(event, '{end}')" onclick="event.stopPropagation()"><title>Drag to move this end</title><rect class="zoom-grip-hit" x="-8" y="-3" width="16" height="24"/><rect class="zoom-grip-bar" x="-1.5" y="3" width="3" height="12" rx="1.5"/></g>"##
        )
        .unwrap();
    }
    button(&mut svg, "zoom-all", PLOT_RIGHT + 14.0, 30.0, "all", "zoomAll()", "Show every input");
    writeln!(svg, "  </g>").unwrap();

    /*
     * Chips that show and hide plots, from the plots this run has, in
     * groups that follow the measurements: what is hashed (messages,
     * batches, pieces); how the program calls (after idling, after other
     * work, nonstop); and under Nonstop, joined to it by a line, the two
     * choices only nonstop plots have (owned or lent buffers; one program
     * or two at once). A plot shows when every chip that applies to it is
     * pressed. A chip whose press would change nothing, as the others
     * stand, is dimmed; a press that would leave no plot is refused (the
     * script's toggleChip).
     */
    type Chip = (&'static str, &'static str, &'static str);
    let has = |test: &dyn Fn(&Plot) -> bool| plots.iter().any(test);
    let mut chip_lines: Vec<(&str, bool, Vec<Chip>)> = Vec::new();
    let mut line_of = |kind: &'static str, nested: bool, chips: Vec<(Chip, bool)>| {
        let chips: Vec<Chip> = chips.into_iter().filter(|(_, present)| *present).map(|(chip, _)| chip).collect();
        if !chips.is_empty() {
            chip_lines.push((kind, nested, chips));
        }
    };
    line_of("what", false, vec![
        (("messages", "Messages", "Show or hide the plots of messages: one in one buffer now and then, or one after another"), has(&|p| p.use_case.what_key() == "messages")),
        (("batches", "Batches", "Show or hide the plots of batches of 64-byte messages"), has(&|p| p.use_case.what_key() == "batches")),
        (("pieces", "Pieces", "Show or hide the plots of long messages arriving in 64 KiB pieces"), has(&|p| p.use_case.what_key() == "pieces")),
    ]);
    line_of("pattern", false, vec![
        (("idle", "After idling", "Show or hide the plots of calls each made after the program slept 1 ms, as a server waiting for its next request"), has(&|p| p.use_case.pattern_key() == "idle")),
        (("busy", "After other work", "Show or hide the plots of calls each made after the program ran other code and read 128 MiB, as a program that hashes between its other tasks"), has(&|p| p.use_case.pattern_key() == "busy")),
    ]);
    line_of("pattern", false, vec![
        (("nonstop", "Nonstop", "Show or hide the plots of inputs hashed one after another, as fast as the program can"), has(&|p| p.use_case.pattern_key() == "nonstop")),
    ]);
    if has(&|p| p.use_case.pattern_key() == "nonstop") {
        line_of("buffers", true, vec![
            (("owned", "Owned", "Show or hide nonstop plots where the program hands each buffer over for good and fills the next while it is hashed"), has(&|p| p.use_case.buffers_key() == Some("owned"))),
            (("lent", "Lent", "Show or hide nonstop plots where the program waits for each call to return before refilling its buffer"), has(&|p| p.use_case.buffers_key() == Some("lent"))),
        ]);
        line_of("scenario", true, vec![
            (("solo", "Solo", "Show or hide the nonstop plots of one program hashing alone"), has(&|p| !p.use_case.after_gap() && p.scenario == Scenario::Solo)),
            (("shared", "Shared", "Show or hide the nonstop plots of two programs hashing at once"), has(&|p| p.scenario == Scenario::Shared)),
        ]);
    }
    /* One line per group, continued on the next where it would pass the right edge; nested lines hang from Nonstop. */
    const NEST: f64 = 18.0;
    let mut line = 0;
    let mut nonstop_line = None;
    for (kind, nested, chips) in &chip_lines {
        let left = PLOT_RIGHT + 14.0 + if *nested { NEST } else { 0.0 };
        let mut x = left;
        if *nested {
            let top = CHIP_ROW_TOP + nonstop_line.expect("nested chips follow Nonstop") as f64 * 24.0 + 18.0;
            let mid = CHIP_ROW_TOP + line as f64 * 24.0 + 9.0;
            writeln!(svg, r##"  <path class="chip-tie" d="M{:.1} {top:.1} L{:.1} {mid:.1} L{:.1} {mid:.1}"/>"##, PLOT_RIGHT + 24.0, PLOT_RIGHT + 24.0, left - 3.0).unwrap();
        }
        for (index, (value, label, tip)) in chips.iter().enumerate() {
            let width = label.chars().count() as f64 * 6.6 + 16.0;
            if index > 0 && x + width > SVG_WIDTH - 4.0 {
                line += 1;
                x = left;
            }
            let y = CHIP_ROW_TOP + line as f64 * 24.0;
            assert!(y + 18.0 <= HEADER_BOTTOM, "the chips fit in the header");
            writeln!(
                svg,
                r##"  <g class="chip" data-kind="{kind}" data-value="{value}" data-on="true" data-live="true" transform="translate({x:.1} {y:.1})" onclick="event.stopPropagation(); toggleChip('{kind}', '{value}')"><title>{}</title><rect x="0" y="0" width="{width:.1}" height="18" rx="9"/><text x="{:.1}" y="13" text-anchor="middle">{}</text></g>"##,
                xml_escape(tip),
                width / 2.0,
                xml_escape(label),
            )
            .unwrap();
            x += width + 6.0;
            if *value == "nonstop" {
                nonstop_line = Some(line);
            }
        }
        line += 1;
    }
    writeln!(svg, r##"  <line class="header-edge" x1="0" y1="{HEADER_BOTTOM:.0}" x2="{SVG_WIDTH:.0}" y2="{HEADER_BOTTOM:.0}"/>"##).unwrap();
    /*
     * Behind the door: how the plots are drawn, for a reader who wants it.
     * A panel under the header, over the plots, shown by the door's click.
     */
    let mut howto = vec![
        format!("Each line is one hash. Each dot is the mean of up to {} timings at that size.", 2 * roster.rounds),
        "A dot's shape marks the method the hash used at that size. The section \"Code paths\" at the bottom names each method.".to_owned(),
        "Rate counts bytes or messages per second, time the nanoseconds per byte or message; the switch at right changes every plot.".to_owned(),
        "The strip at the top narrows every plot to part of its inputs: drag an end of its band, or use the arrows at its ends.".to_owned(),
    ];
    if plots.iter().any(|plot| plot.use_case.after_gap() && !plot.use_case.idle()) {
        howto.push("After other work: the program calls the hash, runs a fixed other program (about 1 MiB of code) and reads 128 MiB of data, at least 1 ms in all, writes the input, and calls again; the second call is timed. So a program works that hashes between its other tasks.".to_owned());
    }
    if plots.iter().any(|plot| plot.use_case.idle()) {
        howto.push("After idling: the program calls the hash, sleeps 1 ms, writes the input, and calls again; the second call is timed. So a server works that waits for its next request.".to_owned());
    }
    if plots.iter().any(|plot| !plot.use_case.after_gap()) {
        howto.push("Nonstop, each input (each 64 KiB piece of a message in pieces) is first read into memory, a memory copy, the cheapest read, inside the time.".to_owned());
        howto.push("Owned buffers: the program hands each buffer over and fills the next while it is hashed. Lent buffers: the program waits for each call to return before refilling its buffer.".to_owned());
    }
    let howto_height = 16.0 + howto.len() as f64 * 16.0;
    writeln!(svg, r##"  <g id="howto" style="display:none" onclick="event.stopPropagation(); toggleHowto()">"##).unwrap();
    writeln!(svg, r##"    <rect class="howto-box" x="{:.0}" y="{:.0}" width="{:.0}" height="{howto_height:.0}" rx="6"/>"##, PLOT_LEFT - 10.0, HEADER_BOTTOM + 4.0, PLOT_RIGHT - PLOT_LEFT + 20.0).unwrap();
    for (i, line) in howto.iter().enumerate() {
        writeln!(svg, r##"    <text class="howto-line" x="{PLOT_LEFT:.0}" y="{:.0}">{}</text>"##, HEADER_BOTTOM + 22.0 + i as f64 * 16.0, xml_escape(line)).unwrap();
    }
    writeln!(svg, "  </g>").unwrap();
    writeln!(svg, "  </g>").unwrap();

    /*
     * Headers occupy the first provenance slots, then every category's
     * detail lines; the per-contender lines emitted inside the first plot's
     * series groups start after them. The script flows detail lines from
     * the header count when categories collapse.
     */
    let shared_count = provenance_cats.len();
    let mut provenance_slot = shared_count
        + provenance_cats.iter().map(|cat| cat.lines.len()).sum::<usize>();

    /* Each plot is one group, which the script moves or hides as the header's chips choose. */
    for plot in &plots {
        writeln!(svg, r##"  <g id="plot-{}" class="plot-group">"##, plot.index).unwrap();
        write_plot(&mut svg, plot, roster, results, &first_plot, &mut provenance_slot);
        writeln!(svg, "  </g>").unwrap();
    }

    /*
     * Hover panel, filled by the script when a dot is hovered. Last among
     * the drawn elements so it paints over every series.
     */
    writeln!(svg, r##"  <g id="hover" style="display:none">"##).unwrap();
    writeln!(
        svg,
        r##"    <line id="hover-guide" x1="0" y1="{:.1}" x2="0" y2="{:.1}"/>"##,
        plots[0].top,
        plots[0].bottom,
    )
        .unwrap();
    writeln!(svg, r##"    <rect id="hover-box" x="0" y="0" width="0" height="0" rx="4"/>"##).unwrap();
    writeln!(svg, r##"    <g id="hover-body"></g>"##).unwrap();
    writeln!(svg, "  </g>").unwrap();

    /* Machine-readable provenance, complete and untruncated. */
    writeln!(svg, "  <metadata>").unwrap();

    for (name, value) in [
        ("timestamp", machine.timestamp.as_str()),
        ("git source", GIT_SOURCE),
        ("git commit", GIT_COMMIT),
        ("git tag", GIT_TAG),
        ("git clean status", GIT_CLEAN_STATUS),
        ("bench-hashes version", BENCH_VERSION),
        ("CPU type", machine.cpu_type.as_str()),
        ("OS type", machine.os_type.as_str()),
        ("Rust compiler", RUSTC_VERSION),
        ("build target", BUILD_TARGET),
        ("target features", TARGET_FEATURES),
        ("sample clock", clocks::WALL_CLOCK),
        ("BLAKE3 source", BLAKE3_SOURCE_INFO),
        ("SHA-256 source", SHA2_SOURCE_INFO),
        ("SHA-1DC source", SHA1_CHECKED_SOURCE_INFO),
        ("SHA3-256 source", SHA3_SOURCE_INFO),
        ("SHA-256 ring source", RING_SOURCE_INFO),
        ("BLAKE3 servil source", BLAKE3_SERVIL_SOURCE_INFO),
    ] {
        writeln!(
            svg,
            "    {}: {}",
            xml_escape(name),
            xml_escape(value),
        )
            .unwrap();
    }
    writeln!(svg, "    power: {}", xml_escape(&machine.describe_power())).unwrap();

    writeln!(svg, "  </metadata>").unwrap();

    /*
     * Human-readable provenance: left-aligned, compact, de-emphasized.
     * Shared lines first; the per-contender lines emitted above follow and
     * close ranks when a contender is hidden.
     */
    writeln!(svg, r##"  <g class="below">"##).unwrap();
    writeln!(
        svg,
        r##"  <line x1="{PLOT_LEFT:.1}" y1="{provenance_top:.1}" x2="{:.1}" y2="{provenance_top:.1}" class="divider"/>"##,
        SVG_WIDTH - PLOT_LEFT,
    )
        .unwrap();

    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.1}" y="{:.1}" class="prov-head">ABOUT THIS RUN</text>"##,
        provenance_top + 20.0,
    )
        .unwrap();

    /* Collapsible categories: a header row per category, then its detail
       lines. Without script everything shows, fully laid out. */
    let mut header_slot = 0;
    for cat in &provenance_cats {
        writeln!(
            svg,
            r##"  <g class="prov-head-row" data-cat="{}" data-name="{}" data-summary="{}" onclick="event.stopPropagation(); toggleProv('{}')">"##,
            cat.key,
            xml_escape(cat.name),
            xml_escape(&cat.summary),
            cat.key,
        )
        .unwrap();
        writeln!(
            svg,
            r##"    <text class="prov-head" x="{PLOT_LEFT:.1}" y="{:.1}">▾ {} — {}</text>"##,
            provenance_line_y(provenance_top, header_slot),
            xml_escape(cat.name),
            xml_escape(&cat.summary),
        )
        .unwrap();
        writeln!(svg, r##"  </g>"##).unwrap();
        header_slot += 1;
        for line in &cat.lines {
            writeln!(
                svg,
                r##"  <text class="prov prov-shared" data-cat="{}" x="{PLOT_LEFT:.1}" y="{:.1}">{}</text>"##,
                cat.key,
                provenance_line_y(provenance_top, header_slot),
                xml_escape(line),
            )
            .unwrap();
            header_slot += 1;
        }
    }

    writeln!(svg, "  </g>").unwrap();
    assert_eq!(
        provenance_slot, provenance_total,
        "the provenance lines emitted must match the count the canvas was sized for"
    );

    /* Data and behaviour for the interactive toggles. */
    write_interaction_script(&mut svg, roster, results, &plots, shared_count);

    svg.push_str("</svg>\n");
    svg
}

/*
 * One plot: heading, axes, every participating contender's band, line,
 * dots, value labels, and clickable label at right, and the shape legend
 * beneath. The first plot's series groups also carry each contender's
 * provenance lines, which hide with the contender.
 */
fn write_plot(svg: &mut String, plot: &Plot, roster: &Roster, results: &Results, first_plot: &[usize], provenance_slot: &mut usize) {
    let p = plot.index;
    let top = plot.top;
    let bottom = plot.bottom;

    /*
     * A plot after a gap has one scenario (one program), so its subtitle
     * says what the program did between calls; a nonstop plot's names its
     * scenario.
     */
    let lead = if plot.use_case.idle() {
        "the program hashes, sleeps 1 ms, and hashes again; the time of the second call"
    } else if plot.use_case.after_gap() {
        "the program hashes, runs other code and reads 128 MiB of memory, and hashes again; the time of the second call"
    } else {
        plot.scenario.subtitle()
    };
    let heading_note = match plot.use_case {
        UseCase::OneMessage | UseCase::IdleOneMessage => lead.to_owned(),
        UseCase::ManyMessages | UseCase::IdleManyMessages => format!("{lead} · each hash takes the whole batch where it can, else one message at a time"),
        UseCase::ContinuousMessages | UseCase::ContinuousBatches => format!("{} · owned: the program hands each buffer over and fills the next while it is hashed", plot.scenario.subtitle()),
        UseCase::LentMessages | UseCase::LentPieces | UseCase::LentBatches => format!("{} · lent: the program waits for each call to return before refilling its buffer", plot.scenario.subtitle()),
    };
    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.0}" y="{:.1}" class="plot-title">{}</text>"##,
        top - 26.0,
        xml_escape(&if plot.use_case.after_gap() { plot.use_case.heading().to_owned() } else { format!("{} · {}", plot.scenario.heading(), plot.use_case.heading()) }),
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <text x="{PLOT_LEFT:.0}" y="{:.1}" class="plot-sub">{}</text>"##,
        top - 11.0,
        xml_escape(&heading_note),
    )
    .unwrap();

    /*
     * The plot area, for marks and dots: nothing is drawn outside it, and
     * when the zoom narrows the inputs shown, points beyond it slide out
     * of view. Room above and below for value labels.
     */
    writeln!(
        svg,
        r##"  <clipPath id="plot-clip-{p}"><rect x="{PLOT_LEFT:.1}" y="{:.1}" width="{:.1}" height="{:.1}"/></clipPath>"##,
        top - 40.0,
        PLOT_RIGHT - PLOT_LEFT,
        bottom - top + 60.0,
    )
    .unwrap();

    /* Horizontal grid and y-axis tick labels; the script rebuilds these. */
    writeln!(svg, r##"  <g id="y-axis-{p}">"##).unwrap();
    for value in log_ticks(plot.axis_min, plot.axis_max) {
        let y = plot.map_rate(value);
        let ns = plot.scale / value;
        writeln!(
            svg,
            r##"    <line x1="{PLOT_LEFT:.1}" y1="{y:.2}" x2="{PLOT_RIGHT:.1}" y2="{y:.2}" class="grid" data-ns="{ns}"/>"##
        )
            .unwrap();

        writeln!(
            svg,
            r##"    <text x="{:.1}" y="{:.2}" class="tick-label" text-anchor="end" data-ns="{ns}">{}</text>"##,
            PLOT_LEFT - 10.0,
            y + 3.5,
            format_gbps_tick(value),
        )
            .unwrap();
    }
    writeln!(svg, "  </g>").unwrap();

    let y_title = format!("{} (log scale) · higher is better", plot.use_case.rate_unit_long());
    writeln!(
        svg,
        r##"  <text id="y-title-{p}" x="{Y_TITLE_X:.0}" y="{:.1}" class="axis-title" text-anchor="middle" transform="rotate(-90 {Y_TITLE_X:.0} {:.1})">{}</text>"##,
        (top + bottom) / 2.0,
        (top + bottom) / 2.0,
        xml_escape(&y_title),
    )
        .unwrap();
    /*
     * An arrow under "higher is better" (or "lower is better"), parallel to
     * the title and on the side below its words as they read, pointing the
     * way that is better. The script's betterArrow draws the same.
     */
    let (y0, y1) = better_arrow_span(&y_title, (top + bottom) / 2.0);
    writeln!(
        svg,
        r##"  <path id="y-better-{p}" class="better-arrow" d="{}"/>"##,
        better_arrow_path(y0, y1, true),
    )
        .unwrap();

    /*
     * Vertical guides and x-axis labels at each tested point, placed by
     * place_size_labels: two rows, a short tick joining a second-row
     * label to its column.
     */
    let labels: Vec<&str> = plot.points.clone().map(|point_index| POINTS[point_index].label).collect();
    let primary: Vec<bool> = plot.points.clone().map(|point_index| POINTS[point_index].bytes.is_power_of_two()).collect();
    let label_rows = place_size_labels(&plot.x_positions, &labels, &primary);
    for (k, point_index) in plot.points.clone().enumerate() {
        let x = plot.x_positions[k];
        let row = label_rows[k].unwrap_or(0);
        let hidden = label_rows[k].is_none();
        let label_y = bottom + 24.0 + 13.0 * f64::from(row);

        writeln!(
            svg,
            r##"  <line x1="{x:.2}" y1="{top:.1}" x2="{x:.2}" y2="{bottom:.1}" class="grid-x" data-plot="{p}" data-size="{k}"/>"##
        )
            .unwrap();

        /* Every column has a tick for the second row; the zoom script shows
           it wherever the label drops there. */
        writeln!(
            svg,
            r##"  <line x1="{x:.2}" y1="{:.1}" x2="{x:.2}" y2="{:.1}" class="size-tick" data-plot="{p}" data-size="{k}"{}/>"##,
            bottom + 4.0,
            bottom + 24.0 + 13.0 - 10.0,
            if row == 1 && !hidden { "" } else { r#" display="none""# },
        )
            .unwrap();

        writeln!(
            svg,
            r##"  <text x="{x:.2}" y="{label_y:.1}" class="size-label" text-anchor="middle" data-plot="{p}" data-size="{k}"{}>{}</text>"##,
            if hidden { r#" opacity="0""# } else { "" },
            xml_escape(POINTS[point_index].label),
        )
            .unwrap();
    }

    writeln!(
        svg,
        r##"  <text x="{:.1}" y="{:.1}" class="axis-title" text-anchor="middle">{}</text>"##,
        (PLOT_LEFT + PLOT_RIGHT) / 2.0,
        bottom + 52.0,
        xml_escape(plot.use_case.x_axis()),
    )
        .unwrap();

    writeln!(
        svg,
        r##"  <line x1="{PLOT_LEFT:.1}" y1="{top:.1}" x2="{PLOT_LEFT:.1}" y2="{bottom:.1}" class="axis"/>"##
    )
        .unwrap();

    writeln!(
        svg,
        r##"  <line x1="{PLOT_LEFT:.1}" y1="{bottom:.1}" x2="{PLOT_RIGHT:.1}" y2="{bottom:.1}" class="axis"/>"##
    )
        .unwrap();

    let value_label_y = place_value_labels(plot, results, &shown_at_first(roster));
    let value_columns = value_label_columns(&plot.x_positions);

    /*
     * Dots are collected here and emitted after every series' line, so no
     * line can sit above another contender's dots and take the hover. Each dot layer carries its plot and series index; the
     * script and stylesheet treat it as part of that series.
     */
    let mut dot_layers: Vec<String> = Vec::new();

    /*
     * One group per contender holds everything that belongs to it: line,
     * dots, value labels, the clickable label at right, and (in the
     * first plot it takes part in) its provenance lines. Toggling flips one attribute on
     * the group.
     */
    for &algorithm_index in &plot.contenders {
        let algorithm = roster.algorithms[algorithm_index];
        let color = algorithm.color();
        let kernels = plot.kernels(algorithm_index);
        let cell_at = |k: usize| cell(results, algorithm_index, plot.points.start + k);
        let last = cell_at(plot.len() - 1);

        let shown = shown_at_first(roster)[algorithm_index];
        writeln!(
            svg,
            r##"  <g class="series" id="series-{p}-{algorithm_index}" data-on="{shown}">"##
        )
            .unwrap();

        writeln!(svg, r##"    <g class="marks" clip-path="url(#plot-clip-{p})">"##).unwrap();

        /* The line through the means, a segment between each pair of points. */
        let point = |k: usize| (plot.x_positions[k], plot.map_y(cell_at(k).get(plot.scenario).mean));
        for k in 0..plot.len().saturating_sub(1) {
            let ((x0, m0), (x1, m1)) = (point(k), point(k + 1));
            writeln!(
                svg,
                r##"      <path class="median" data-k="{k}" d="M {x0:.2} {m0:.2} L {x1:.2} {m1:.2}" fill="none" stroke="{color}" stroke-width="2.5" stroke-linecap="round"/>"##,
            )
            .unwrap();
        }

        let mut dots = format!("  <g class=\"dots\" id=\"dots-{p}-{algorithm_index}\" data-on=\"{shown}\" clip-path=\"url(#plot-clip-{p})\">\n");

        for k in 0..plot.len() {
            let x = plot.x_positions[k];
            let statistics = cell_at(k).get(plot.scenario);

            /*
             * The dot's shape names the code path that produced this point;
             * the shape alone marks a new path, so every dot draws the same
             * size with no ring. Hovering shows the path's explanation.
             */
            let kernel = &kernels.kernels[kernels.kernel_index_for(POINTS[plot.points.start + k].bytes)];
            let mean_y = plot.map_y(statistics.mean);
            writeln!(
                dots,
                r##"    <g class="dot" data-size="{k}" transform="translate({x:.2} {mean_y:.2})" onpointerenter="hoverDot(event,{p},{algorithm_index},{k})" onpointerleave="leaveDot(event)" onclick="tapDot(event,{p},{algorithm_index},{k})">"##,
            )
                .unwrap();
            writeln!(dots, "      {}", mark_shape(kernel.mark, color, 5.0)).unwrap();
            dots.push_str("    </g>\n");

            /* With two dozen columns, a value at every dot would overprint. */
            if !value_columns[k] {
                continue;
            }

            /*
             * Centred over its dot, so a label names one column only (a
             * right-aligned last label, `45 | 36` wide, read as the
             * column before); the last column sits X_INSET from the plot's
             * edge, room for the widest pair. The first column's label
             * starts beside its dot, clear of the y axis.
             */
            let (label_x, anchor) = if k == 0 { (x + 9.0, "start") } else { (x, "middle") };

            let (label_y, display) = match value_label_y[algorithm_index][k] {
                Some(y) => (y, ""),
                None => (0.0, r#" display="none""#),
            };
            writeln!(
                svg,
                r##"      <text class="value-label" data-size="{k}" x="{label_x:.2}" y="{label_y:.2}" fill="{color}" text-anchor="{anchor}"{display}>{}</text>"##,
                format_rate_value(statistics.mean, plot.use_case),
            )
                .unwrap();
        }

        writeln!(svg, "    </g>").unwrap();
        dots.push_str("  </g>\n");
        dot_layers.push(dots);

        /* Clickable label at right: swatch, name, detail, hint. */
        let statistics = last.get(plot.scenario);
        let label_x = PLOT_RIGHT + 14.0;
        let label_y = plot.label_y[algorithm_index].expect("a participant has a label slot");

        writeln!(
            svg,
            r##"    <g class="series-label" transform="translate(0 {label_y:.2})" onclick="event.stopPropagation(); toggleSeries({algorithm_index})" onpointerenter="hoverLabel(event,{algorithm_index},true)" onpointerleave="hoverLabel(event,{algorithm_index},false)">"##
        )
            .unwrap();
        /* The batch plots' note beside the crates.io crate's name, in full. */
        let batch_note = if plot.use_case.batch() && algorithm == Algorithm::Blake3 {
            "; in this plot, from two messages, through the crate's hidden batch function blake3::platform::Platform::hash_many, sixteen messages per call"
        } else {
            ""
        };
        writeln!(
            svg,
            r##"      <title>{}: {}{}. Click to hide or show it.</title>"##,
            xml_escape(algorithm.name()),
            algorithm.blurb(),
            xml_escape(batch_note),
        )
            .unwrap();
        writeln!(
            svg,
            r##"      <rect x="{:.1}" y="-14" width="{:.1}" height="{:.0}" fill="transparent"/>"##,
            label_x - 4.0,
            SVG_WIDTH - label_x - 6.0,
            SERIES_LABEL_GAP - 4.0,
        )
            .unwrap();
        /* Swatch: every dot shape this contender uses, in its colour, so the
           right-hand names match the marks in the plot. Names share one x
           across contenders; shapes fill fixed slots, so rows align. Each
           shape carries a tooltip naming its code path. */
        let mut swatch_marks: Vec<(Mark, &str)> = Vec::new();
        for kernel in &kernels.kernels {
            if !swatch_marks.iter().any(|slot| slot.0 == kernel.mark) {
                swatch_marks.push((kernel.mark, &kernel.name));
            }
        }
        let name_x = label_x + 14.0 + (SWATCH_SLOTS as f64) * 13.0;
        writeln!(svg, r##"      <g class="series-swatch" transform="translate(0 0)">"##).unwrap();
        for (mark_index, (mark, name)) in swatch_marks.iter().enumerate() {
            writeln!(
                svg,
                r##"        <g transform="translate({:.1} 0)"><title>{}</title>{}</g>"##,
                label_x + 4.5 + mark_index as f64 * 13.0,
                xml_escape(name),
                mark_shape(*mark, color, 4.5),
            )
            .unwrap();
        }
        writeln!(svg, r##"      </g>"##).unwrap();
        /* In the batch plots the crates.io crate runs a function its docs hide; the name says so beside it. */
        let note = if plot.use_case.batch() && algorithm == Algorithm::Blake3 {
            r##"<tspan class="series-note" dx="5">hidden batch API</tspan>"##
        } else {
            ""
        };
        writeln!(
            svg,
            r##"      <text class="series-name" x="{:.1}" y="4" fill="{color}">{}{note}</text>"##,
            name_x,
            xml_escape(algorithm.name()),
        )
            .unwrap();
        writeln!(
            svg,
            r##"      <text class="series-detail" x="{:.1}" y="18">{} · {} {} at {}</text>"##,
            name_x,
            format_rate(statistics.mean, plot.use_case),
            statistics.mean.format_ns(),
            plot.use_case.time_unit(),
            xml_escape(POINTS[plot.points.end - 1].label),
        )
            .unwrap();
        writeln!(
            svg,
            r##"      <text class="series-hint" x="{:.1}" y="18">hidden · click to show</text>"##,
            name_x,
        )
            .unwrap();
        writeln!(svg, "    </g>").unwrap();

        /* This contender's provenance lines, hidden along with it. */
        if first_plot[algorithm_index] == p {
            for line in contender_provenance_lines(algorithm, kernels) {
                writeln!(
                    svg,
                    r##"    <text class="prov series-prov" x="{PLOT_LEFT:.1}" y="{:.1}">{}</text>"##,
                    provenance_line_y(plot.provenance_top, *provenance_slot),
                    xml_escape(&line),
                )
                    .unwrap();
                *provenance_slot += 1;
            }
        }

        writeln!(svg, "  </g>").unwrap();
    }

    for layer in &dot_layers {
        svg.push_str(layer);
    }

    /*
     * The hashes of this run that take no part in this plot: listed under
     * the legend in a pale, still style, apart from the hidden ones (which
     * keep their place and a click), with the reason as a tooltip.
     */
    let absent: Vec<Algorithm> = roster.algorithms.iter().copied().filter(|algorithm| !algorithm.takes_part(plot.use_case)).collect();
    for (i, algorithm) in absent.iter().enumerate() {
        let y = bottom + 30.0 + i as f64 * 30.0;
        let label_x = PLOT_RIGHT + 14.0 + 14.0 + (SWATCH_SLOTS as f64) * 13.0;
        writeln!(svg, r##"  <g class="series-absent-row"><title>{}</title>"##, xml_escape(&absent_reason(*algorithm, plot.use_case))).unwrap();
        writeln!(svg, r##"    <text class="series-absent" x="{label_x:.1}" y="{y:.1}">{}</text>"##, xml_escape(algorithm.name())).unwrap();
        writeln!(svg, r##"    <text class="series-absent-detail" x="{label_x:.1}" y="{:.1}">not measured here</text>"##, y + 14.0).unwrap();
        writeln!(svg, "  </g>").unwrap();
    }
}

/// Screen y from and to of the phrase after the title's last " · ", for
/// the y title `title` centred at `mid` and rotated to read upward, at the
/// axis title's 12 px font (about 6.2 px a character; the script's
/// betterArrow uses the same figure).
fn better_arrow_span(title: &str, mid: f64) -> (f64, f64) {
    let width = |t: &str| t.chars().count() as f64 * 6.2;
    let phrase = title.rsplit(" · ").next().unwrap_or(title);
    let end = mid - width(title) / 2.0;
    (end + width(phrase), end)
}

/// The arrow beside the y title from `y0` (the phrase's start) to `y1` (its
/// end, the top): its head at the top when higher is better, else at the
/// bottom.
fn better_arrow_path(y0: f64, y1: f64, up: bool) -> String {
    let x = BETTER_ARROW_X;
    let (tail, head, dir) = if up { (y0, y1, 1.0) } else { (y1, y0, -1.0) };
    format!("M{x:.1} {tail:.1} L{x:.1} {head:.1} M{:.1} {:.1} L{x:.1} {head:.1} L{:.1} {:.1}", x - 3.5, head + 5.0 * dir, x + 3.5, head + 5.0 * dir)
}

/// The y title's x: close to the tick numbers it names (right-aligned 10 px
/// left of the plot, up to five characters), with the arrow between.
const Y_TITLE_X: f64 = 54.0;
/// The arrow's x: beside the rotated y title, on the side below its words.
const BETTER_ARROW_X: f64 = Y_TITLE_X + 7.0;

/// Why a hash of the run takes no part in a plot, for the tooltip on its
/// pale name there.
fn absent_reason(algorithm: Algorithm, use_case: UseCase) -> String {
    let how = match (algorithm, use_case) {
        (Algorithm::Blake3ServilSt, _) => "for messages one after another, BLAKE3 servil offers its multithreaded queue",
        (_, UseCase::ManyMessages | UseCase::ContinuousBatches | UseCase::LentBatches) => "it has no way to hash a batch of messages over threads",
        _ => "it has no way to hash these inputs",
    };
    format!("{} is measured in the other plots; {how}.", algorithm.name())
}

/*
 * X-axis labels in two rows. About 7.2 px per character at the bold label
 * font, plus a 12 px gutter. The powers of two go first, then the sizes between
 * them, each pass left to right. A label takes the first row if it
 * overlaps no label there and covers no second-row tick, else the second
 * row if it overlaps no label there and its tick crosses no first-row
 * label, else it stays hidden (2304 B beside 2 KiB, 7935 B beside 8 KiB)
 * until the zoom spreads the points. The script's placeSizeLabels applies
 * the same rule. Returns each column's row, None for hidden.
 */
fn place_size_labels(x: &[f64], labels: &[&str], primary: &[bool]) -> Vec<Option<u8>> {
    let half = |k: usize| (labels[k].chars().count() as f64 * 7.2 + 12.0) / 2.0;
    let mut rows: Vec<Option<u8>> = vec![None; x.len()];
    for pass in [true, false] {
        for k in (0..x.len()).filter(|&k| primary[k] == pass) {
            let overlaps = |row: u8| {
                (0..x.len()).any(|j| rows[j] == Some(row) && (x[j] - x[k]).abs() < half(j) + half(k))
            };
            let covers_tick = (0..x.len()).any(|j| rows[j] == Some(1) && (x[j] - x[k]).abs() < half(k));
            let tick_crosses = (0..x.len()).any(|j| rows[j] == Some(0) && (x[j] - x[k]).abs() < half(j));
            rows[k] = if !overlaps(0) && !covers_tick {
                Some(0)
            } else if !overlaps(1) && !tick_crosses {
                Some(1)
            } else {
                None
            };
        }
    }
    rows
}

/*
 * The columns that carry value labels: the first and the last, and,
 * walking left from the last, each column at least VALUE_COLUMN_SPACING
 * from the one labelled before and from the first, with both neighbours
 * at least VALUE_COLUMN_ROOM away. Crowded stretches (2-8 KiB) carry no
 * values until the zoom spreads them; hovering a dot shows every value.
 * The script's valueColumns applies the same rule to its window.
 */
const VALUE_COLUMN_SPACING: f64 = 110.0;
const VALUE_COLUMN_ROOM: f64 = 32.0;

fn value_label_columns(x: &[f64]) -> Vec<bool> {
    let n = x.len();
    let mut labeled = vec![false; n];
    labeled[0] = true;
    labeled[n - 1] = true;
    let mut previous = x[n - 1];
    for k in (1..n - 1).rev() {
        if previous - x[k] >= VALUE_COLUMN_SPACING
            && x[k] - x[0] >= VALUE_COLUMN_SPACING
            && x[k] - x[k - 1] >= VALUE_COLUMN_ROOM
            && x[k + 1] - x[k] >= VALUE_COLUMN_ROOM
        {
            labeled[k] = true;
            previous = x[k];
        }
    }
    labeled
}

/*
 * Value labels sit above their dot by default. Within a column, labels
 * are processed top to bottom; one that would land within a label height
 * of a label already placed, or on a dot of the column, moves below its
 * dot instead, and if that also collides it steps down once more; a
 * label with no clear place within VALUE_LABEL_REACH below its dot stays
 * hidden (hovering the dot shows the value). A label's baseline at y
 * clears a dot at d when y <= d - 6 or y >= d + 14 (the text spans y - 9
 * to y + 1, the dot d - 5 to d + 5), and the plot's edges when it lies
 * within [top + 10, bottom - 3]. The script repeats this rule.
 */
const VALUE_LABEL_ABOVE: f64 = -11.0;
const VALUE_LABEL_BELOW: f64 = 17.0;
const VALUE_LABEL_HEIGHT: f64 = 11.0;
const VALUE_LABEL_REACH: f64 = VALUE_LABEL_BELOW + VALUE_LABEL_HEIGHT;

fn value_label_clear(y: f64, labels: &[f64], dots: &[f64], plot: &Plot) -> bool {
    y >= plot.top + 10.0
        && y <= plot.bottom - 3.0
        && labels.iter().all(|t| (t - y).abs() >= VALUE_LABEL_HEIGHT)
        && dots.iter().all(|&d| y <= d - 6.0 || y >= d + 14.0)
}

fn place_value_labels(plot: &Plot, results: &Results, shown: &[bool]) -> Vec<Vec<Option<f64>>> {
    let mut placed = vec![vec![None; plot.len()]; results.len()];
    for k in 0..plot.len() {
        let point_index = plot.points.start + k;
        let mut order: Vec<usize> = plot.contenders.iter().copied().filter(|&a| shown[a]).collect();
        order.sort_by(|&a, &b| {
            plot.stats(results, b, point_index).mean.cmp(&plot.stats(results, a, point_index).mean)
        });
        /* Smallest y (fastest, highest on the plot) first. */
        order.reverse();
        let dots: Vec<f64> = order.iter().map(|&a| plot.map_y(plot.stats(results, a, point_index).mean)).collect();
        let mut taken: Vec<f64> = Vec::new();
        for algorithm_index in order {
            let dot_y = plot.map_y(plot.stats(results, algorithm_index, point_index).mean);
            let y = [VALUE_LABEL_ABOVE, VALUE_LABEL_BELOW, VALUE_LABEL_REACH]
                .into_iter()
                .map(|offset| dot_y + offset)
                .find(|&y| value_label_clear(y, &taken, &dots, plot));
            if let Some(y) = y {
                taken.push(y);
            }
            placed[algorithm_index][k] = y;
        }
    }
    placed
}

/// The zoom strip's ends: where the plots' first and last inputs sit, so
/// at the full range each tick stands over its input in the plots; the
/// arrow pairs sit between these and the plots' edges.
const ZOOM_STRIP_LEFT: f64 = PLOT_LEFT + X_INSET;
const ZOOM_STRIP_RIGHT: f64 = PLOT_RIGHT - X_INSET;
/// Where the zoom guides end, below the zoom row's top, above the first
/// plot's title.
const ZOOM_GUIDE_BOTTOM: f64 = 26.0;
/// The header's bottom: the header group's background reaches here.
const HEADER_BOTTOM: f64 = ZOOM_ROW_TOP + ZOOM_GUIDE_BOTTOM + 2.0;
/// The chips' first row's top, in the header above "all".
const CHIP_ROW_TOP: f64 = 44.0;

/// The unit switch, in the header straight above the y titles it changes:
/// its track centred on their column.
const UNIT_SWITCH_LEFT: f64 = Y_TITLE_X - 7.0;
const UNIT_SWITCH_TOP: f64 = 86.0;

/// The zoom row's top, between the method lines and the first plot's title.
const ZOOM_ROW_TOP: f64 = 172.0;
/// An input size: whole MiB or KiB where it is one, else exact bytes
/// ("16 MiB", "3 KiB", "2304 B", "1025 B").
fn format_bytes(bytes: usize) -> String {
    if bytes >= 1 << 20 && bytes % (1 << 20) == 0 {
        format!("{} MiB", bytes >> 20)
    } else if bytes >= 1 << 10 && bytes % (1 << 10) == 0 {
        format!("{} KiB", bytes >> 10)
    } else {
        format!("{bytes} B")
    }
}

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

/*
 * A mark centred on the origin. Diamonds and squares are sized to match a
 * circle's visual weight at the same radius.
 */
fn mark_shape(mark: Mark, color: &str, radius: f64) -> String {
    let stroke = r##"stroke="#fdfdfc" stroke-width="1.5""##;
    match mark {
        Mark::Circle => format!(r##"<circle r="{radius:.1}" fill="{color}" {stroke}/>"##),
        Mark::Diamond => {
            let r = radius * 1.25;
            format!(
                r##"<path d="M 0 {a:.2} L {r:.2} 0 L 0 {r:.2} L {a:.2} 0 Z" fill="{color}" {stroke}/>"##,
                a = -r,
            )
        }
        Mark::Square => {
            let h = radius * 0.9;
            format!(
                r##"<rect x="{a:.2}" y="{a:.2}" width="{w:.2}" height="{w:.2}" fill="{color}" {stroke}/>"##,
                a = -h,
                w = 2.0 * h,
            )
        }
        Mark::Triangle => {
            /* Point up; the centroid sits at the origin. */
            let r = radius * 1.3;
            format!(
                r##"<path d="M 0 {top:.2} L {r:.2} {base:.2} L {left:.2} {base:.2} Z" fill="{color}" {stroke}/>"##,
                top = -r,
                base = r * 0.5,
                left = -r,
            )
        }
        Mark::DownTriangle => {
            /* Point down; the centroid sits at the origin. */
            let r = radius * 1.3;
            format!(
                r##"<path d="M 0 {bottom:.2} L {r:.2} {base:.2} L {left:.2} {base:.2} Z" fill="{color}" {stroke}/>"##,
                bottom = r,
                base = -r * 0.5,
                left = -r,
            )
        }
    }
}

fn provenance_line_y(provenance_top: f64, slot: usize) -> f64 {
    provenance_top + 40.0 + slot as f64 * PROVENANCE_LINE_HEIGHT
}

/* Provenance that describes the run as a whole. */
/// One collapsible provenance category: a header row plus its detail lines.
struct ProvCat {
    key: &'static str,
    name: &'static str,
    summary: String,
    lines: Vec<String>,
}

fn shared_provenance_cats(machine: &MachineMetadata, selection_note: &str) -> Vec<ProvCat> {
    vec![
        ProvCat {
            key: "machine",
            name: "Machine",
            summary: format!(
                "{} · {} CPUs · {}{}{}",
                machine.cpu_type,
                machine.cpu_count,
                machine.os_type,
                match &machine.load {
                    load if load.iter().any(clocks::load::Window::busy) => " · busy during the run",
                    load if !load.is_empty() => " · quiet during the run",
                    _ => "",
                },
                if machine.power.iter().flatten().any(|power| power.on_battery) {
                    " · on battery"
                } else if machine.power_slowing() {
                    " · low-power mode"
                } else {
                    ""
                },
            ),
            lines: vec![
                format!(
                    "Machine: {} · {} logical CPUs · {}",
                    machine.cpu_type, machine.cpu_count, machine.os_type,
                ),
                format!("Load during the run: {}", clocks::load::describe(&machine.load)),
                format!("Power: {}", machine.describe_power()),
                format!("Toolchain: {RUSTC_VERSION} · {BUILD_TARGET}"),
                format!("Sample clock: {}", clocks::WALL_CLOCK),
            ],
        },
        ProvCat {
            key: "run",
            name: "Run",
            summary: format!("{} · bench-hashes {}", machine.timestamp, BENCH_VERSION.split('+').next().unwrap_or(BENCH_VERSION)),
            lines: vec![
                format!(
                    "Run: {} · bench-hashes {BENCH_VERSION}",
                    machine.timestamp,
                ),
                format!("Contenders: {selection_note}"),
                format!("Tag: {GIT_TAG} · Working tree: {GIT_CLEAN_STATUS}"),
            ],
        },
        ProvCat {
            key: "sources",
            name: "Sources",
            summary: format!("bench-hashes @ {}", &GIT_COMMIT[..12.min(GIT_COMMIT.len())]),
            lines: vec![
                format!("Source: {GIT_SOURCE} @ {GIT_COMMIT}"),
                "Full crate checksums are in this file's metadata element".to_owned(),
            ],
        },
    ]
}

/*
 * What each code path is, folded away at the bottom: the dots' shapes and
 * the hover panel name a contender's code path; this section says what
 * each name means and from which size it runs, contender by contender,
 * for one message and for batches.
 */
fn code_path_cat(roster: &Roster, plots: &[Plot]) -> ProvCat {
    let mut lines = Vec::new();
    for (algorithm_index, algorithm) in roster.algorithms.iter().enumerate() {
        /*
         * Each method once per hash, with the plots it runs in when that is
         * not all of them. Where it starts is told in the plot's own x:
         * bytes for an input, messages for a batch.
         */
        let mut methods: Vec<(&Kernel, String, Vec<UseCase>)> = Vec::new();
        let mut takes_part: Vec<UseCase> = Vec::new();
        for use_case in UseCase::ALL {
            let Some(plot) = plots.iter().find(|plot| plot.use_case == use_case) else { continue };
            let Some(kernels) = &plot.kernels[algorithm_index] else { continue };
            takes_part.push(use_case);
            for kernel in &kernels.kernels {
                let from = match kernel.first {
                    0 => "from the start".to_owned(),
                    first if use_case.batch() => format!("from {} messages", first.div_ceil(use_case.message_len())),
                    first => format!("from {}", format_bytes(first)),
                };
                match methods.iter_mut().find(|(k, f, _)| k.name == kernel.name && k.why == kernel.why && *f == from) {
                    Some((_, _, use_cases)) => use_cases.push(use_case),
                    None => methods.push((kernel, from, vec![use_case])),
                }
            }
        }
        for (kernel, from, use_cases) in methods {
            let plots_named = if use_cases.len() == takes_part.len() {
                String::new()
            } else {
                format!(" ({})", use_cases.iter().map(|u| u.short()).collect::<Vec<_>>().join(", "))
            };
            let text = format!("{} · {} {}, {from}{plots_named}: {}", algorithm.name(), kernel.mark.glyph(), kernel.name, kernel.why);
            /* One line to about 180 characters; longer ones continue indented. */
            let mut line = String::new();
            for word in text.split(' ') {
                if !line.is_empty() && line.len() + word.len() > 180 {
                    lines.push(std::mem::take(&mut line));
                    line.push_str("    ");
                }
                if !line.is_empty() && !line.ends_with("    ") {
                    line.push(' ');
                }
                line.push_str(word);
            }
            lines.push(line);
        }
    }
    ProvCat { key: "paths", name: "Code paths", summary: "the method behind each dot shape, hash by hash".to_owned(), lines }
}

/* Provenance that belongs to one contender and hides with it: the
   implementation, its mode, and the platform its kernels ran on. */
fn contender_provenance_lines(
    algorithm: Algorithm,
    kernels: &Kernels,
) -> Vec<String> {
    let name = algorithm.name();
    let platform = kernels.platform;
    match algorithm {
        Algorithm::Blake3 => vec![format!(
            "{name}: {} · {} · platform {platform}",
            package_name_and_version(BLAKE3_SOURCE_INFO),
            algorithm.mode(),
        )],
        Algorithm::Sha256 => vec![format!(
            "{name}: {} · {} · {}",
            package_name_and_version(SHA2_SOURCE_INFO),
            algorithm.mode(),
            kernels.kernels[0].name,
        )],
        Algorithm::Sha1Dc => vec![format!(
            "{name}: {} · {}",
            package_name_and_version(SHA1_CHECKED_SOURCE_INFO),
            algorithm.mode(),
        )],
        Algorithm::Sha3_256 => vec![format!(
            "{name}: {} · {} · {}",
            package_name_and_version(SHA3_SOURCE_INFO),
            algorithm.mode(),
            kernels.kernels[0].name,
        )],
        Algorithm::Blake3ServilSt => vec![
            format!("{name}: {} · hash, hash_many for a batch, Hasher::update for a message in pieces", short_git_source(BLAKE3_SERVIL_SOURCE_INFO)),
            format!("{name}: single-threaded · platform {platform}"),
        ],
        Algorithm::Sha256CommonCrypto => vec![format!("{name}: {} · {}", algorithm.mode(), kernels.kernels[0].name)],
        Algorithm::Sha256Ring => vec![format!(
            "{name}: {} · {} · {}",
            package_name_and_version(RING_SOURCE_INFO),
            algorithm.mode(),
            kernels.kernels[0].name,
        )],
        Algorithm::Blake3Rayon => vec![
            format!(
                "{name}: {} · Hasher::update_rayon · platform {platform}",
                package_name_and_version(BLAKE3_SOURCE_INFO),
            ),
            format!("{name}: {}", algorithm.thread_resources().expect("BLAKE3 mt runs on Rayon's pool")),
        ],
        Algorithm::Blake3ServilMt => vec![
            format!("{name}: {} · hash_multithreaded, hash_many_multithreaded for a batch, Hasher::update_multithreaded for a message in pieces; Queue::messages, Queue::pieces, and Queue::fixed for inputs one after another", short_git_source(BLAKE3_SERVIL_SOURCE_INFO)),
            format!("{name}: multithreaded on the fork's own threads · platform {platform}"),
        ],
    }
}

/*
 * The script re-derives every y position from the visible contenders'
 * data, using the same rules as the Rust layout: nice log bounds with 8%
 * headroom, the same tick mantissas, the same label stacking gap. The
 * measurements and layout constants travel as JSON so the two stay in
 * lockstep. Each plot carries its own points, series, and units; the
 * contender names, colours, and on/off state are shared.
 */
/// Which contenders the graph shows when it opens: those in SHOWN_AT_FIRST,
/// or every contender when the run has none of them.
fn shown_at_first(roster: &Roster) -> Vec<bool> {
    let shown: Vec<bool> = roster.algorithms.iter().map(|algorithm| SHOWN_AT_FIRST.contains(algorithm)).collect();
    if shown.contains(&true) { shown } else { vec![true; roster.len()] }
}

fn write_interaction_script(
    svg: &mut String,
    roster: &Roster,
    results: &Results,
    plots: &[Plot],
    shared_count: usize,
) {
    let mut data = String::from("{\"names\":[");
    for (index, algorithm) in roster.algorithms.iter().enumerate() {
        if index > 0 { data.push(','); }
        data.push_str(&json_string(algorithm.name()));
    }
    data.push_str("],\"shown\":[");
    for (index, shown) in shown_at_first(roster).iter().enumerate() {
        if index > 0 { data.push(','); }
        data.push_str(if *shown { "true" } else { "false" });
    }
    data.push_str("],\"colors\":[");
    for (index, algorithm) in roster.algorithms.iter().enumerate() {
        if index > 0 { data.push(','); }
        write!(data, "\"{}\"", algorithm.color()).unwrap();
    }
    data.push_str("],\"plots\":[");
    for (plot_index, plot) in plots.iter().enumerate() {
        if plot_index > 0 { data.push(','); }
        write!(
            data,
            "{{\"scenario\":\"{}\",\"use\":\"{:?}\",\"call\":\"{:?}\",\"pattern\":\"{}\",\"what\":\"{}\",\"buffers\":{},\"top\":{:.1},\"bottom\":{:.1},\"scale\":{},\"timeUnit\":{},\"rateUnit\":{},\"rateLong\":{},\"timeLong\":{},\"x\":[",
            plot.scenario.key(),
            plot.use_case,
            plot.use_case.call(),
            plot.use_case.pattern_key(),
            plot.use_case.what_key(),
            plot.use_case.buffers_key().map_or("null".to_owned(), |key| format!("\"{key}\"")),
            plot.top,
            plot.bottom,
            plot.use_case.rate_scale(),
            json_string(plot.use_case.time_unit()),
            json_string(plot.use_case.rate_unit()),
            json_string(plot.use_case.rate_unit_long()),
            json_string(match plot.use_case {
                _ if plot.use_case.batch() => "Nanoseconds per message",
                _ => "Nanoseconds per byte",
            }),
        )
        .unwrap();
        for (index, x) in plot.x_positions.iter().enumerate() {
            if index > 0 { data.push(','); }
            write!(data, "{x:.2}").unwrap();
        }
        data.push_str("],\"bytes\":[");
        for (index, point_index) in plot.points.clone().enumerate() {
            if index > 0 { data.push(','); }
            write!(data, "{}", POINTS[point_index].bytes).unwrap();
        }
        data.push_str("],\"sizes\":[");
        for (index, point_index) in plot.points.clone().enumerate() {
            if index > 0 { data.push(','); }
            data.push_str(&json_string(POINTS[point_index].label));
        }
        data.push_str("],\"labelY\":[");
        for (index, label_y) in plot.label_y.iter().enumerate() {
            if index > 0 { data.push(','); }
            match label_y {
                Some(y) => write!(data, "{y:.2}").unwrap(),
                None => data.push_str("null"),
            }
        }
        data.push_str("],\"series\":[");
        for algorithm_index in 0..roster.len() {
            if algorithm_index > 0 { data.push(','); }
            let Some(kernels) = &plot.kernels[algorithm_index] else {
                data.push_str("null");
                continue;
            };
            data.push_str("{\"kernels\":[");
            for (kernel_index, kernel) in kernels.kernels.iter().enumerate() {
                if kernel_index > 0 { data.push(','); }
                let first = plot
                    .points
                    .clone()
                    .position(|point_index| POINTS[point_index].bytes >= kernel.first)
                    .expect("every kernel starts at or below the axis's largest point");
                write!(
                    data,
                    "{{\"from\":{first},\"name\":{},\"mark\":\"{}\"}}",
                    json_string(&kernel.name),
                    kernel.mark.name(),
                )
                .unwrap();
            }
            let cell_at = |k: usize| cell(results, algorithm_index, plot.points.start + k);
            /* Each point's mean, extremes, and sample count. */
            for (key, pick) in [
                ("med", (|t: Statistics| t.format_mean(1)) as fn(Statistics) -> String),
                ("min", |t| t.minimum.format_ns()),
                ("max", |t| t.maximum.format_ns()),
                ("n", |t| t.count.to_string()),
            ] {
                write!(data, "],\"{key}\":[").unwrap();
                for k in 0..plot.len() {
                    if k > 0 { data.push(','); }
                    data.push_str(&pick(cell_at(k).get(plot.scenario)));
                }
            }
            data.push(']');
            data.push('}');
        }
        data.push_str("]}");
    }
    write!(
        data,
        "],\"sharedProv\":{shared_count},\"svgWidth\":{SVG_WIDTH:.0},\"plotLeft\":{PLOT_LEFT},\"plotRight\":{PLOT_RIGHT},\"xInset\":{X_INSET},\"labelGap\":{SERIES_LABEL_GAP},\"labelTopRoom\":{LABEL_TOP_ROOM},\"rounds\":{},\"labelAbove\":{VALUE_LABEL_ABOVE},\"labelBelow\":{VALUE_LABEL_BELOW},\"labelHeight\":{VALUE_LABEL_HEIGHT},\"valueSpacing\":{VALUE_COLUMN_SPACING},\"valueRoom\":{VALUE_COLUMN_ROOM},\"provTop\":{:.1},\"provLine\":{PROVENANCE_LINE_HEIGHT},\"stripLeft\":{ZOOM_STRIP_LEFT},\"betterX\":{BETTER_ARROW_X},\"stripRight\":{ZOOM_STRIP_RIGHT}}}",
        roster.rounds,
        plots[0].provenance_top,
    )
        .unwrap();

    svg.push_str("  <script><![CDATA[\n");
    writeln!(svg, "const DATA = {data};").unwrap();
    svg.push_str(INTERACTION_SCRIPT);
    svg.push_str("  ]]></script>\n");
}

const INTERACTION_SCRIPT: &str = r##"
/* One on/off state per contender, shared by every plot. */
const on = DATA.shown.slice();

/*
 * Zoom: the inputs every plot shows, as a range of input sizes, a batch
 * counting its messages' bytes, so all four plots move in lock step. The
 * range runs from one data point to another of ALLB, every input size the
 * plots have. Each plot shows its points inside the range, or, when fewer
 * than two fall inside, the two nearest it. A plot's x axis maps log2 of
 * bytes across its window, as the static render maps its whole axis.
 */
const ALLB = [...new Set(DATA.plots.flatMap(pl => pl.bytes))].sort((a, b) => a - b);
let zFrom = 0, zTo = ALLB.length - 1;
function windowFor(p, lo, hi) {
  const b = DATA.plots[p].bytes;
  let ks = b.map((_, k) => k).filter(k => b[k] >= lo && b[k] <= hi);
  if (ks.length < 2) {
    const d = k => Math.max(0, Math.log2(lo) - Math.log2(b[k]), Math.log2(b[k]) - Math.log2(hi));
    ks = b.map((_, k) => k).sort((x, y) => d(x) - d(y) || x - y).slice(0, 2).sort((x, y) => x - y);
  }
  const k0 = ks[0], k1 = ks[ks.length - 1];
  /* A plot of one point keeps it in the middle, whatever the zoom. */
  if (b.length === 1) return { k0, k1, w0: Math.log2(b[0]) - 1, w1: Math.log2(b[0]) + 1 };
  return { k0, k1, w0: Math.log2(b[k0]), w1: Math.log2(b[k1]) };
}
/* The window each plot moves to, the one it moves from, and how far along (eased, 0 to 1). */
let win = DATA.plots.map((_, p) => windowFor(p, ALLB[zFrom], ALLB[zTo]));
let winFrom = win, zoomE = 1, zoomAnimation = null;
/* Pixel x of each point, as last laid out. */
const currentX = DATA.plots.map(() => null);
function xsFor(p) {
  const a = winFrom[p], c = win[p];
  const w0 = (1 - zoomE) * a.w0 + zoomE * c.w0, w1 = (1 - zoomE) * a.w1 + zoomE * c.w1;
  const left = DATA.plotLeft + DATA.xInset, width = DATA.plotRight - DATA.plotLeft - 2 * DATA.xInset;
  return DATA.plots[p].bytes.map(v => left + (Math.log2(v) - w0) / (w1 - w0) * width);
}
function setZoom(from, to, duration = 600) {
  from = Math.max(0, from); to = Math.min(ALLB.length - 1, to);
  if (to <= from || (from === zFrom && to === zTo)) return;
  /* A click during a transition starts from where that one was headed. */
  if (zoomAnimation) { cancelAnimationFrame(zoomAnimation); zoomAnimation = null; }
  zFrom = from; zTo = to;
  winFrom = win;
  win = DATA.plots.map((_, p) => windowFor(p, ALLB[zFrom], ALLB[zTo]));
  updateZoomControls();
  const startTime = performance.now();
  const ease = x => x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
  const step = now => {
    const raw = Math.min(1, (now - startTime) / duration);
    zoomE = ease(raw);
    relayout();
    if (hovered) showHover(hovered[0], hovered[1], hovered[2]);
    if (raw < 1) {
      zoomAnimation = requestAnimationFrame(step);
    } else {
      winFrom = win;
      zoomAnimation = null;
    }
  };
  zoomE = 0;
  zoomAnimation = requestAnimationFrame(step);
}
function zoomAll() { setZoom(0, ALLB.length - 1); }
/* The arrow beside a y title, under its "better" phrase, as the Rust side's better_arrow_path draws it. */
function betterArrow(p, title, up) {
  const phrase = title.split(" · ").pop();
  /* The title's and the phrase's drawn lengths where the browser measures them; else the estimate the Rust side uses. */
  let total = [...title].length * 6.2, phraseWidth = [...phrase].length * 6.2;
  const el = document.getElementById("y-title-" + p);
  if (el && el.getComputedTextLength && el.getSubStringLength && el.textContent === title) {
    try {
      const measured = el.getComputedTextLength();
      if (measured > 0) { total = measured; phraseWidth = el.getSubStringLength(title.length - phrase.length, phrase.length); }
    } catch (e) { /* keep the estimate */ }
  }
  const mid = (DATA.plots[p].top + DATA.plots[p].bottom) / 2, end = mid - total / 2;
  const y0 = end + phraseWidth, y1 = end, x = DATA.betterX;
  const [tail, head, dir] = up ? [y0, y1, 1] : [y1, y0, -1];
  const f = v => v.toFixed(1);
  document.getElementById("y-better-" + p).setAttribute("d",
    `M${f(x)} ${f(tail)} L${f(x)} ${f(head)} M${f(x - 3.5)} ${f(head + 5 * dir)} L${f(x)} ${f(head)} L${f(x + 3.5)} ${f(head + 5 * dir)}`);
}
/*
 * The chips show and hide plots. A plot shows when every chip that
 * applies to it is pressed: what it hashes and how the program calls,
 * and for a nonstop plot whose buffers and how many programs. The plots
 * shown close ranks from the first plot's place, and what lies below them
 * follows. A press that would leave no plot is refused; a chip whose
 * press would change nothing, as the others stand, is dimmed.
 */
const chipOn = { what: {}, pattern: {}, buffers: {}, scenario: {} };
document.querySelectorAll(".chip").forEach(c => { chipOn[c.getAttribute("data-kind")][c.getAttribute("data-value")] = true; });
const plotShown = (plot, on) => !!(on.what[plot.what] && on.pattern[plot.pattern]
  && (plot.pattern !== "nonstop" || (on.buffers[plot.buffers] && on.scenario[plot.scenario])));
const shownWith = on => DATA.plots.map(plot => plotShown(plot, on));
const flipped = (kind, value) => ({ ...chipOn, [kind]: { ...chipOn[kind], [value]: !chipOn[kind][value] } });
const plotShift = DATA.plots.map(() => 0);
let belowShift = 0;
function toggleChip(kind, value) {
  const next = flipped(kind, value);
  if (!shownWith(next).some(v => v)) return;
  chipOn[kind] = next[kind];
  document.querySelector(`.chip[data-kind="${kind}"][data-value="${value}"]`).setAttribute("data-on", chipOn[kind][value] ? "true" : "false");
  layoutPlots();
}
function dimChips() {
  const now = shownWith(chipOn).join();
  document.querySelectorAll(".chip").forEach(c => {
    const kind = c.getAttribute("data-kind"), value = c.getAttribute("data-value");
    c.setAttribute("data-live", shownWith(flipped(kind, value)).join() !== now ? "true" : "false");
  });
}
function layoutPlots() {
  const pitch = DATA.plots.length > 1 ? DATA.plots[1].top - DATA.plots[0].top : 0;
  let shown = 0;
  DATA.plots.forEach((plot, p) => {
    const visible = plotShown(plot, chipOn);
    const group = document.getElementById("plot-" + p);
    group.classList.toggle("plot-off", !visible);
    plotShift[p] = visible ? (shown - p) * pitch : 0;
    group.setAttribute("transform", `translate(0 ${plotShift[p]})`);
    if (visible) shown++;
  });
  belowShift = (shown - DATA.plots.length) * pitch;
  document.querySelectorAll(".below").forEach(g => g.setAttribute("transform", `translate(0 ${belowShift})`));
  if (hovered && !plotShown(DATA.plots[hovered[0]], chipOn)) hideHover();
  dimChips();
  layoutProv();
}
/* The door under the title opens and closes the panel on how to read the graph. */
function toggleHowto() {
  const panel = document.getElementById("howto"), open = panel.style.display === "none";
  panel.style.display = open ? "" : "none";
  document.getElementById("howto-door").textContent = open ? "How to read this graph ▾" : "How to read this graph ▸";
}
/*
 * Dragging: a grip moves its end of the range, the band both ends at once,
 * each to an input's tick, never past the other end. The strip follows the
 * pointer's travel at full speed while the hand moves fast, and at a third
 * of it while it moves slowly (under 0.3 px per ms), so a slow hand can
 * settle on one of several close ticks; and an end leaves its tick only
 * once the point aimed at is 3 px nearer another, so it stays where the
 * hand stops. The ticks under the ends light up while dragging.
 */
let dragging = null;
function nearestIndex(x) {
  let best = 0;
  ALLB.forEach((v, i) => { if (Math.abs(stripX(v) - x) < Math.abs(stripX(ALLB[best]) - x)) best = i; });
  return best;
}
function aimIndex(x, current) {
  const j = nearestIndex(x);
  return j !== current && Math.abs(stripX(ALLB[j]) - x) + 3 < Math.abs(stripX(ALLB[current]) - x) ? j : current;
}
function svgXOf(ev) {
  const root = document.getElementById("zoom").ownerSVGElement;
  if (!root || !root.getScreenCTM || !root.getScreenCTM()) return ev.clientX;
  const pt = root.createSVGPoint();
  pt.x = ev.clientX; pt.y = ev.clientY;
  return pt.matrixTransform(root.getScreenCTM().inverse()).x;
}
function markTicks(active) {
  ALLB.forEach((_, i) => document.getElementById("zoom-tick-" + i).setAttribute("data-at", active && (i === zFrom || i === zTo) ? "true" : "false"));
}
function gripDown(ev, end) {
  ev.stopPropagation(); ev.preventDefault();
  const at = end === "to" ? zTo : zFrom;
  dragging = { end, lastX: svgXOf(ev), lastT: performance.now(), aim: stripX(ALLB[at]), span: zTo - zFrom };
  markTicks(true);
}
function gripMove(ev) {
  if (!dragging) return;
  const x = svgXOf(ev), t = performance.now();
  const dx = x - dragging.lastX, dt = Math.max(1, t - dragging.lastT);
  dragging.aim += dx * (Math.abs(dx) / dt < 0.3 ? 0.35 : 1);
  dragging.lastX = x; dragging.lastT = t;
  const last = ALLB.length - 1;
  if (dragging.end === "from") {
    setZoom(Math.min(aimIndex(dragging.aim, zFrom), zTo - 1), zTo, 150);
  } else if (dragging.end === "to") {
    setZoom(zFrom, Math.max(aimIndex(dragging.aim, zTo), zFrom + 1), 150);
  } else {
    const from = Math.max(0, Math.min(last - dragging.span, aimIndex(dragging.aim, zFrom)));
    setZoom(from, from + dragging.span, 150);
  }
  markTicks(true);
}
function gripUp() {
  if (!dragging) return;
  dragging = null;
  markTicks(false);
}
window.addEventListener("pointermove", gripMove);
window.addEventListener("pointerup", gripUp);
window.addEventListener("pointercancel", gripUp);
/* The zoom strip's x of an input, on the log spacing the Rust side draws its ticks with. */
const stripX = bytes => DATA.stripLeft + (Math.log2(bytes) - Math.log2(ALLB[0])) / (Math.log2(ALLB[ALLB.length - 1]) - Math.log2(ALLB[0])) * (DATA.stripRight - DATA.stripLeft);
function updateZoomControls() {
  const last = ALLB.length - 1;
  /* The band over the inputs shown. */
  const x0 = stripX(ALLB[zFrom]), x1 = stripX(ALLB[zTo]);
  const band = document.getElementById("zoom-band");
  band.setAttribute("x", (x0 - 3).toFixed(1));
  band.setAttribute("width", (x1 - x0 + 6).toFixed(1));
  document.getElementById("zoom-grip-from").setAttribute("transform", `translate(${x0.toFixed(1)} 0)`);
  document.getElementById("zoom-grip-to").setAttribute("transform", `translate(${x1.toFixed(1)} 0)`);
  document.getElementById("zoom-all").setAttribute("data-off", zFrom === 0 && zTo === last ? "true" : "false");
}

/*
 * Display unit. Data is stored as ns per unit (byte or message, by plot);
 * the rate is the plot's scale over it, so on the log axis switching units
 * mirrors each plot: the fastest contender moves from the bottom to the
 * top. Every drawn or printed value goes through val() and fmt(); ratios
 * between contenders are unitless and stay put.
 */
let unit = "gbps";

/*
 * Blend between the units. `blend` runs from 0 (time) to 1 (rate), and the
 * plotted value is the log-space interpolation of the two readings, so
 * during a switch every point travels a straight line on the log axis and
 * each plot mirrors through its middle. Text follows the unit from the
 * midpoint; the two axes cross-fade.
 */
let blend = 1;
const valAt = (ns, b, scale) => Math.exp((1 - b) * Math.log(ns) + b * Math.log(scale / ns));
const val = (ns, p) => valAt(ns, blend, DATA.plots[p].scale);
/* Values in the settled unit, for text. */
const settled = (ns, p) => unit === "ns" ? ns : DATA.plots[p].scale / ns;
function fmt(ns, p, digits) {
  const v = settled(ns, p);
  if (unit === "ns") return v.toFixed(digits === undefined ? 3 : digits);
  if (v < 1 && digits !== undefined) return v.toFixed(Math.min(9, Math.max(2, Math.ceil(-Math.log10(v)) + 1)));
  return v >= 10 ? v.toFixed(digits === undefined ? 0 : Math.max(0, digits - 2)) : v.toFixed(digits === undefined ? 1 : Math.max(1, digits - 1));
}
const unitLabel = p => unit === "ns" ? DATA.plots[p].timeUnit : DATA.plots[p].rateUnit;
const otherUnitLabel = p => unit === "ns" ? DATA.plots[p].rateUnit : DATA.plots[p].timeUnit;
const fmtOther = (ns, p) => unit === "ns" ? rate(ns, p) : ns.toFixed(3) + " " + DATA.plots[p].timeUnit;

let animation = null;
let chosen = "gbps";
function flipUnit() { setUnit(chosen === "ns" ? "gbps" : "ns"); }
function setUnit(u) {
  chosen = u;
  const target = u === "ns" ? 0 : 1;
  if (animation) cancelAnimationFrame(animation);
  document.getElementById("unit-knob").setAttribute("cy", u === "ns" ? 27 : 7);
  document.getElementById("unit-switch").querySelectorAll(".unit-label")
    .forEach(l => l.setAttribute("class", "unit-label" + (l.getAttribute("data-unit") === u ? " unit-on" : "")));

  const start = blend, startTime = performance.now(), DURATION = 700;
  const ease = x => x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
  /* Prepare each incoming axis so it can fade in while the old one fades out. */
  const outgoing = [], incoming = [];
  DATA.plots.forEach((_, p) => {
    const out = document.getElementById("y-axis-" + p);
    out.setAttribute("id", "y-axis-old-" + p);
    const inc = document.createElementNS(NS, "g");
    inc.setAttribute("id", "y-axis-" + p);
    inc.setAttribute("opacity", "0");
    out.parentNode.insertBefore(inc, out.nextSibling);
    outgoing.push(out); incoming.push(inc);
  });

  const step = now => {
    const raw = Math.min(1, (now - startTime) / DURATION);
    const e = ease(raw);
    blend = start + (target - start) * e;
    /* Text follows the unit once the plot is past halfway. */
    unit = blend >= 0.5 ? "gbps" : "ns";
    DATA.plots.forEach((plot, p) => {
      const title = unit === "ns" ? plot.timeLong + " (log scale) · lower is better" : plot.rateLong + " (log scale) · higher is better";
      document.getElementById("y-title-" + p).textContent = title;
      betterArrow(p, title, unit !== "ns");
      outgoing[p].setAttribute("opacity", (1 - e).toFixed(3));
      incoming[p].setAttribute("opacity", e.toFixed(3));
    });
    relayout();
    if (hovered) showHover(hovered[0], hovered[1], hovered[2]);
    if (raw < 1) {
      animation = requestAnimationFrame(step);
    } else {
      outgoing.forEach(out => out.parentNode.removeChild(out));
      animation = null;
    }
  };
  animation = requestAnimationFrame(step);
}
const NS = "http://www.w3.org/2000/svg";

function niceBelow(v) {
  const mag = Math.pow(10, Math.floor(Math.log10(v)));
  const n = v / mag;
  return (n >= 5 ? 5 : n >= 2 ? 2 : 1) * mag;
}
function niceAbove(v) {
  const mag = Math.pow(10, Math.floor(Math.log10(v)));
  const n = v / mag;
  return (n <= 1 ? 1 : n <= 2 ? 2 : n <= 5 ? 5 : 10) * mag;
}
/* Below 1, two significant digits, trailing zeros dropped (0.15, 0.2, 0.05); format_gbps_tick in Rust writes the same. */
function fmtTick(v) {
  if (v >= 10) return v.toFixed(0);
  if (v >= 1) return v.toFixed(1);
  return v.toFixed(Math.ceil(-Math.log10(v)) + 1).replace(/0+$/, "").replace(/\.$/, "");
}

function ticks(lo, hi) {
  const out = [];
  const eLo = Math.floor(Math.log10(lo)) - 1, eHi = Math.ceil(Math.log10(hi)) + 1;
  for (let e = eLo; e <= eHi; e++) {
    for (const m of [1, 1.5, 2, 3, 5, 7]) {
      const v = m * Math.pow(10, e);
      if (v >= lo * 0.999999 && v <= hi * 1.000001) out.push(v);
    }
  }
  return out;
}

/* Current y mapping per plot, kept by relayout() so the hover panel places itself. */
const currentMapY = DATA.plots.map(() => null);

function relayout() {
  DATA.plots.forEach((_, p) => relayoutPlot(p));
  layoutProv();
}


function relayoutPlot(p) {
  const plot = DATA.plots[p];
  const visible = plot.series.map((s, i) => i).filter(i => on[i] && plot.series[i]);
  const X = xsFor(p);
  currentX[p] = X;
  const w = win[p];
  /* The data's extent over a window's points. */
  const rangeOf = wnd => {
    let lo = Infinity, hi = 0;
    for (const i of visible) {
      const s = plot.series[i];
      for (let k = wnd.k0; k <= wnd.k1; k++) {
        lo = Math.min(lo, s.med[k]);
        hi = Math.max(hi, s.med[k]);
      }
    }
    return visible.length === 0 ? [0.1, 1] : [lo, hi];
  };
  /*
   * Axis bounds at both ends of the unit blend, then interpolated in log
   * space alongside the data, so the axis and the points move together;
   * the same between the zoom's start and end windows.
   */
  const boundsAt = (b, lo, hi) => {
    const a = valAt(lo, b, plot.scale), c = valAt(hi, b, plot.scale);
    const dLo = Math.min(a, c), dHi = Math.max(a, c);
    return [Math.log(niceBelow(dLo * 0.92)), Math.log(niceAbove(dHi * 1.08))];
  };
  const startRange = rangeOf(winFrom[p]), endRange = rangeOf(w);
  const zoomed = b => {
    const [a0, a1] = boundsAt(b, ...startRange), [c0, c1] = boundsAt(b, ...endRange);
    return [(1 - zoomE) * a0 + zoomE * c0, (1 - zoomE) * a1 + zoomE * c1];
  };
  const [n0, n1] = zoomed(0), [g0, g1] = zoomed(1);
  const lMin = (1 - blend) * n0 + blend * g0, lMax = (1 - blend) * n1 + blend * g1;
  /* mapY takes ns per unit, as stored; the unit transform happens inside. */
  const mapY = ns => plot.bottom - (Math.log(val(ns, p)) - lMin) / (lMax - lMin) * (plot.bottom - plot.top);
  currentMapY[p] = mapY;
  /*
   * Y axis for the settled unit. Each tick is a value in that unit; its
   * stored equivalent is placed with the blended mapY, so mid-animation
   * the ticks ride the same mirroring motion as the data, while the
   * outgoing axis (still in the old unit, in its own group) fades out and
   * this one fades in.
   */
  const axis = document.getElementById("y-axis-" + p);
  while (axis.firstChild) axis.removeChild(axis.firstChild);
  const [tMin, tMax] = unit === "ns" ? [n0, n1] : [g0, g1];
  const tickToNs = v => unit === "ns" ? v : plot.scale / v;
  for (const v of ticks(Math.exp(tMin), Math.exp(tMax))) {
    const y = mapY(tickToNs(v));
    const line = document.createElementNS(NS, "line");
    line.setAttribute("x1", DATA.plotLeft); line.setAttribute("x2", DATA.plotRight);
    line.setAttribute("y1", y.toFixed(2)); line.setAttribute("y2", y.toFixed(2));
    line.setAttribute("class", "grid");
    axis.appendChild(line);
    line.setAttribute("data-ns", tickToNs(v));
    const t = document.createElementNS(NS, "text");
    t.setAttribute("x", (DATA.plotLeft - 10).toFixed(1)); t.setAttribute("y", (y + 3.5).toFixed(2));
    t.setAttribute("class", "tick-label"); t.setAttribute("text-anchor", "end");
    t.setAttribute("data-ns", tickToNs(v));
    t.textContent = fmtTick(v);
    axis.appendChild(t);
  }
  /* The outgoing axis, if one is fading, rides the same motion. */
  const old = document.getElementById("y-axis-old-" + p);
  if (old) {
    old.querySelectorAll("line").forEach(l => { const y = mapY(+l.getAttribute("data-ns")); l.setAttribute("y1", y.toFixed(2)); l.setAttribute("y2", y.toFixed(2)); });
    old.querySelectorAll("text").forEach(t => { t.setAttribute("y", (mapY(+t.getAttribute("data-ns")) + 3.5).toFixed(2)); });
  }

  /* Each series: its line through the means, dots, value labels. */
  plot.series.forEach((s, i) => {
    if (!s) return;
    const g = document.getElementById("series-" + p + "-" + i);
    const dots = document.getElementById("dots-" + p + "-" + i);
    g.setAttribute("data-on", on[i] ? "true" : "false");
    dots.setAttribute("data-on", on[i] ? "true" : "false");
    if (!on[i]) return;
    const pt = (k, v) => X[k].toFixed(2) + " " + mapY(v).toFixed(2);
    g.querySelectorAll(".median").forEach(el => {
      const k = +el.getAttribute("data-k");
      el.setAttribute("d", `M ${pt(k, s.med[k])} L ${pt(k + 1, s.med[k + 1])}`);
    });
    dots.querySelectorAll(".dot").forEach(dot => {
      const k = +dot.getAttribute("data-size");
      dot.setAttribute("transform", `translate(${X[k].toFixed(2)} ${mapY(s.med[k]).toFixed(2)})`);
    });
  });

  /*
   * Value labels: above the dot unless that collides within the column,
   * at the columns valueColumns picks in the window.
   */
  const columns = valueColumns(X, w.k0, w.k1);
  for (let k = 0; k < plot.x.length; k++) {
    const shown = columns[k];
    const order = visible.slice().sort((a, b) => mapY(plot.series[a].med[k]) - mapY(plot.series[b].med[k]));
    const taken = [], dotYs = order.map(i => mapY(plot.series[i].med[k]));
    const clear = y => y >= plot.top + 10 && y <= plot.bottom - 3
      && taken.every(t => Math.abs(t - y) >= DATA.labelHeight) && dotYs.every(d => y <= d - 6 || y >= d + 14);
    for (const i of order) {
      const dotY = mapY(plot.series[i].med[k]);
      const y = [DATA.labelAbove, DATA.labelBelow, DATA.labelBelow + DATA.labelHeight].map(o => dotY + o).find(clear);
      if (y !== undefined) taken.push(y);
      document.getElementById("series-" + p + "-" + i).querySelectorAll(".value-label").forEach(t => {
        if (+t.getAttribute("data-size") === k) {
          t.setAttribute("y", (y === undefined ? 0 : y).toFixed(2));
          t.setAttribute("x", (k === w.k0 ? X[k] + 9 : X[k]).toFixed(2));
          t.setAttribute("text-anchor", k === w.k0 ? "start" : "middle");
          t.setAttribute("display", shown && y !== undefined ? "inline" : "none");
          t.textContent = fmt(plot.series[i].med[k], p, 2);
        }
      });
    }
  }
  /*
   * Right-edge labels, every participating contender in its slot. Each
   * anchors level with its line's last point on the current axis; a hidden
   * contender's anchor is clamped to the plot edge, so its grey label
   * points toward where its data lies. Then push overlapping labels apart
   * and keep the stack inside the plot.
   */
  /* The window's last point; mid-zoom the anchor moves between the two windows' last points. */
  const last = w.k1, lastFrom = winFrom[p].k1;
  const clamp = y => Math.min(plot.bottom - 8, Math.max(plot.top + 8, y));
  const slots = plot.series
    .map((s, i) => s ? [i, clamp((1 - zoomE) * mapY(s.med[lastFrom]) + zoomE * mapY(s.med[last]))] : null)
    .filter(slot => slot)
    .sort((a, b) => a[1] - b[1]);
  for (let k = 1; k < slots.length; k++) {
    slots[k][1] = Math.max(slots[k][1], slots[k - 1][1] + DATA.labelGap);
  }
  const overrun = Math.max(0, slots[slots.length - 1][1] + 20 - plot.bottom);
  let previous = -Infinity;
  for (const [i, y0] of slots) {
    /* Never above the plot's top: from there the names space out again downward. */
    const y = Math.max(y0 - overrun, plot.top + DATA.labelTopRoom, previous + DATA.labelGap);
    previous = y;
    const lab = document.getElementById("series-" + p + "-" + i).querySelector(".series-label");
    lab.setAttribute("transform", `translate(0 ${y.toFixed(2)})`);
    const detail = lab.querySelector(".series-detail");
    const s = plot.series[i];
    detail.textContent = `${fmt(s.med[last], p, 2)} ${unitLabel(p)} · ${fmtOther(s.med[last], p)} at ${plot.sizes[last]}`;
  }

  /*
   * The x axis: each column's guide and label follow its point, fading
   * over 12 px past the plot's edges. A label that would run into its
   * shown left neighbour drops to a second row with a tick to its column,
   * the static render's rule.
   */
  const labelY = row => plot.bottom + 24 + 13 * row;
  const opacities = X.map(x => Math.max(0, Math.min(1, (Math.min(x - DATA.plotLeft, DATA.plotRight - x) + 12) / 12)));
  const rows = placeSizeLabels(X, plot.sizes, plot.bytes, opacities);
  for (let k = 0; k < X.length; k++) {
    const x = X[k], opacity = opacities[k], row = rows[k];
    const [grid, tick, label] = xAxis[p][k];
    grid.setAttribute("x1", x.toFixed(2)); grid.setAttribute("x2", x.toFixed(2));
    grid.setAttribute("opacity", opacity.toFixed(3));
    label.setAttribute("x", x.toFixed(2)); label.setAttribute("y", labelY(Math.max(row, 0)).toFixed(1));
    label.setAttribute("opacity", (row >= 0 ? opacity : 0).toFixed(3));
    tick.setAttribute("x1", x.toFixed(2)); tick.setAttribute("x2", x.toFixed(2));
    tick.setAttribute("opacity", opacity.toFixed(3));
    tick.setAttribute("display", row === 1 ? "inline" : "none");
  }
}

/*
 * X-axis label rows, the static render's place_size_labels: powers of two
 * first, then the sizes between, each left to right; a label takes the
 * first row if it overlaps no label there and covers no second-row tick,
 * else the second if it overlaps none there and its tick crosses no
 * first-row label, else it hides (-1). Columns outside the plot hide.
 */
function placeSizeLabels(X, sizes, bytes, opacities) {
  const half = k => (sizes[k].length * 7.2 + 12) / 2;
  const rows = X.map(() => -1);
  const isPow2 = b => Number.isInteger(Math.log2(b));
  for (const pass of [true, false]) {
    for (let k = 0; k < X.length; k++) {
      if (isPow2(bytes[k]) !== pass || opacities[k] <= 0) continue;
      const overlaps = row => rows.some((r, j) => r === row && Math.abs(X[j] - X[k]) < half(j) + half(k));
      const coversTick = rows.some((r, j) => r === 1 && Math.abs(X[j] - X[k]) < half(k));
      const tickCrosses = rows.some((r, j) => r === 0 && Math.abs(X[j] - X[k]) < half(j));
      rows[k] = !overlaps(0) && !coversTick ? 0 : !overlaps(1) && !tickCrosses ? 1 : -1;
    }
  }
  return rows;
}

/*
 * Columns that carry value labels, the static render's value_label_columns
 * over the window k0..k1: its ends, and walking left from its last, each
 * column far enough from the one labelled before and from the first, with
 * room on both sides.
 */
function valueColumns(X, k0, k1) {
  const labeled = X.map((_, k) => k === k0 || k === k1);
  let previous = X[k1];
  for (let k = k1 - 1; k > k0; k--) {
    if (previous - X[k] >= DATA.valueSpacing && X[k] - X[k0] >= DATA.valueSpacing
        && X[k] - X[k - 1] >= DATA.valueRoom && X[k + 1] - X[k] >= DATA.valueRoom) {
      labeled[k] = true;
      previous = X[k];
    }
  }
  return labeled;
}

/* Each plot's x-axis elements by column: [guide, tick, label]. */
const xAxis = DATA.plots.map((plot, p) => plot.x.map((_, k) => ["grid-x", "size-tick", "size-label"].map(cls =>
  document.querySelector(`.${cls}[data-plot="${p}"][data-size="${k}"]`))));

/*
 * The static render labels values at a subset of columns; a zoomed window
 * labels others, so every series gets a (hidden) label at every column.
 */
DATA.plots.forEach((plot, p) => plot.series.forEach((s, i) => {
  if (!s) return;
  const marks = document.getElementById("series-" + p + "-" + i).querySelector(".marks");
  const have = new Set([...marks.querySelectorAll(".value-label")].map(t => +t.getAttribute("data-size")));
  const color = DATA.colors[i];
  plot.x.forEach((_, k) => {
    if (have.has(k)) return;
    const t = document.createElementNS(NS, "text");
    t.setAttribute("class", "value-label"); t.setAttribute("data-size", k);
    t.setAttribute("fill", color); t.setAttribute("display", "none");
    marks.appendChild(t);
  });
}));

const provOpen = {run: false, machine: false, sources: false, paths: false, hashes: false};

function toggleProv(cat) {
  provOpen[cat] = !provOpen[cat];
  layoutProv();
}

function layoutProv() {
  /* Headers and details flow in document order: each header stays
     visible at its slot, details show only when their category is open. */
  let slot = 0;
  const y = s => (DATA.provTop + 40 + s * DATA.provLine).toFixed(1);
  document.querySelectorAll(".prov-head-row, .prov-shared").forEach(el => {
    if (el.classList.contains("prov-head-row")) {
      const cat = el.getAttribute("data-cat");
      el.querySelector("text").textContent =
        (provOpen[cat] ? "▾ " : "▸ ") + el.getAttribute("data-name") + " — " + el.getAttribute("data-summary");
      el.querySelector("text").setAttribute("y", y(slot++));
    } else if (provOpen[el.getAttribute("data-cat")]) {
      el.style.display = "";
      el.setAttribute("y", y(slot++));
    } else {
      el.style.display = "none";
    }
  });
  /* A contender's lines live in the series group of the first plot it
     takes part in; they follow the section
     below, so the group's own shift comes off. */
  DATA.names.forEach((_, i) => {
    const p = DATA.plots.findIndex((_, q) => document.querySelector(`#series-${q}-${i} .series-prov`));
    document.querySelectorAll(`#series-${p}-${i} .series-prov`).forEach(t => {
      const shown = on[i] && provOpen.hashes;
      t.style.display = shown ? "" : "none";
      if (shown) t.setAttribute("y", (DATA.provTop + 40 + slot++ * DATA.provLine + belowShift - plotShift[p]).toFixed(1));
    });
  });
  /* The page never gets shorter than it loaded: mobile WebKit zooms a
     page whose content shrinks until it fills the screen's height, which
     left a phone zoomed past the plots' left edge with no way back out
     (Zooko, September 26, 2026). Hidden plots leave room below; opening
     "About this run" may still lengthen the page. */
  const svgEl = document.querySelector("svg");
  layoutProv.floor ??= +svgEl.getAttribute("height");
  const h = Math.max(DATA.provTop + 40 + slot * DATA.provLine + 8 + belowShift, layoutProv.floor);
  svgEl.setAttribute("height", h.toFixed(0));
  svgEl.setAttribute("viewBox", `0 0 ${DATA.svgWidth} ${h.toFixed(0)}`);
  document.getElementById("page").setAttribute("height", h.toFixed(0));
}

function highlightSeries(i, active) {
  DATA.plots.forEach((plot, p) => {
    for (let j = 0; j < DATA.names.length; j++) {
      if (!plot.series[j]) continue;
      const series = document.getElementById("series-" + p + "-" + j);
      const dots = document.getElementById("dots-" + p + "-" + j);
      /* A hidden contender under the pointer dims nothing: it has no marks to single out. */
      const dim = active && on[i] && j !== i && on[j];
      series.setAttribute("data-dim", dim ? "true" : "false");
      series.setAttribute("data-hl", active && j === i ? "true" : "false");
      if (dots) dots.setAttribute("data-dim", dim ? "true" : "false");
    }
  });
}

function toggleSeries(i) {
  on[i] = !on[i];
  relayout();
  if (labelUnderMouse !== null) highlightSeries(labelUnderMouse, true);
  if (hovered) showHover(hovered[0], hovered[1], hovered[2]);
}

/* The dot the panel describes ([plot, series, point]), so a toggle can rebuild the panel in place. */
let hovered = null;
/* The dot a tap pinned the panel to; a mouse leaving a dot then leaves the panel up. */
let pinned = null;

function rate(ns, p) {
  const t = DATA.plots[p].scale / ns;
  return (t >= 10 ? t.toFixed(0) : t.toFixed(1)) + " " + DATA.plots[p].rateUnit;
}

function markGlyph(mark, color) {
  const stroke = ["stroke", "#fdfdfc"], sw = ["stroke-width", "1.5"];
  let el;
  if (mark === "diamond") { el = document.createElementNS(NS, "path"); el.setAttribute("d", "M 0 -6.25 L 6.25 0 L 0 6.25 L -6.25 0 Z"); }
  else if (mark === "square") { el = document.createElementNS(NS, "rect"); el.setAttribute("x", -4.5); el.setAttribute("y", -4.5); el.setAttribute("width", 9); el.setAttribute("height", 9); }
  else if (mark === "triangle") { el = document.createElementNS(NS, "path"); el.setAttribute("d", "M 0 -6.5 L 6.5 3.25 L -6.5 3.25 Z"); }
  else if (mark === "downward triangle") { el = document.createElementNS(NS, "path"); el.setAttribute("d", "M 0 6.5 L 6.5 -3.25 L -6.5 -3.25 Z"); }
  else { el = document.createElementNS(NS, "circle"); el.setAttribute("r", 5); }
  el.setAttribute("fill", color); el.setAttribute(...stroke); el.setAttribute(...sw);
  return el;
}


function textEl(x, y, cls, content, extra) {
  const t = document.createElementNS(NS, "text");
  t.setAttribute("x", x); t.setAttribute("y", y); t.setAttribute("class", cls);
  if (extra) for (const k in extra) t.setAttribute(k, extra[k]);
  t.textContent = content;
  return t;
}

/*
 * Hovering a dot: the hovered contender's mean and range at that point,
 * then every visible contender of that plot ranked fastest first, each
 * with its speed relative to the hovered one. Hidden contenders stay out
 * of the ranking.
 */
function showHover(p, focus, k) {
  hovered = [p, focus, k];
  document.getElementById("hover").setAttribute("transform", `translate(0 ${plotShift[p]})`);
  highlightSeries(focus, true);
  const plot = DATA.plots[p];
  const mapY = currentMapY[p];
  if (!on[focus] || !mapY || !plot.series[focus]) { document.getElementById("hover").style.display = "none"; return; }
  const body = document.getElementById("hover-body");
  while (body.firstChild) body.removeChild(body.firstChild);

  const f = plot.series[focus];
  const name = i => DATA.names[i];
  const rows = plot.series.map((s, i) => s ? [i, s.med[k]] : null).filter(r => r && on[r[0]]).sort((a, b) => a[1] - b[1]);

  const PAD = 10, LINE = 16;
  /*
   * Text widths, estimated from character counts at each class's font
   * size (a browser measures text only once it is displayed): wide enough
   * for the system sans fonts the style names.
   */
  const widthOf = (text, cls) => text.length * ({ "hover-head": 7.2, "hover-row": 6.4, "hover-ratio": 6.8, "hover-sub": 5.8, "hover-note": 5.0 }[cls] || 6.4);
  let wide = 350;
  const note = (el, extra) => { wide = Math.max(wide, (+el.getAttribute("x") || 0) + widthOf(el.textContent, el.getAttribute("class").split(" ")[0]) + (extra || 0) + PAD); return el; };
  let y = PAD + 12;
  body.appendChild(note(textEl(PAD, y, "hover-head", `${name(focus)} at ${plot.sizes[k]}`)));
  y += 14;
  /* In the rate unit the fastest sample (min time) is the top of the range. */
  const asc = (a, b) => unit === "ns" ? [a, b] : [b, a];
  const [rLo, rHi] = asc(f.min[k], f.max[k]);
  body.appendChild(note(textEl(PAD, y, "hover-sub", `mean ${fmt(f.med[k], p)} ${unitLabel(p)} (${fmtOther(f.med[k], p)})`)));
  y += 13;
  body.appendChild(note(textEl(PAD, y, "hover-sub", `fastest and slowest of ${f.n[k]} timings: ${fmt(rLo, p)}–${fmt(rHi, p)} ${unitLabel(p)}`)));

  /* Code path at this point, by name; the Code paths section at the bottom says what it is. */
  let ri = 0;
  f.kernels.forEach((r, j) => { if (k >= r.from) ri = j; });
  const kernel = f.kernels[ri];
  y += 14;
  const pathRow = textEl(PAD + 14, y, "hover-sub", "method: " + kernel.name);
  const shape = markGlyph(kernel.mark, DATA.colors[focus]);
  shape.setAttribute("transform", `translate(${PAD + 5} ${y - 3.5}) scale(0.8)`);
  body.appendChild(shape);
  body.appendChild(note(pathRow));
  y += 10;

  if (rows.length > 1) {
    /* Each row's cells, then columns as wide as their widest entry. */
    const other = v => fmtOther(v, p).replace(" " + otherUnitLabel(p), "");
    const table = rows.map(([i, med]) => {
      const s = plot.series[i];
      let rel, color;
      if (i === focus) { rel = "—"; color = "#9a9a9a"; }
      else {
        /* The focus's mean over this row's: the row's time ratio. */
        const r = f.med[k] / s.med[k];
        if (r > 0.95 && r < 1.05) { rel = "about the same"; color = "#777777"; }
        else if (r >= 1.05) { rel = "\u25b2 " + r.toFixed(2) + "\u00d7 faster"; color = "#15803d"; }
        else { rel = "\u25bc " + (1 / r).toFixed(2) + "\u00d7 slower"; color = "#b91c1c"; }
      }
      return { i, s, name: name(i), value: fmt(s.med[k], p), other: other(med), rel, color };
    });
    const GAP = 14;
    const colW = (key, cls, head) => Math.max(widthOf(head, "hover-sub"), ...table.map(r => widthOf(r[key], cls)));
    const nameX = PAD + 15;
    const valueEnd = nameX + colW("name", "hover-row", "contender") + GAP + colW("value", "hover-row", unitLabel(p));
    const otherEnd = valueEnd + GAP + colW("other", "hover-row", otherUnitLabel(p));
    const relW = colW("rel", "hover-ratio", `relative to ${name(focus)}`);
    wide = Math.max(wide, otherEnd + GAP + relW + PAD);
    y += LINE;
    body.appendChild(textEl(PAD, y, "hover-sub", "hash"));
    body.appendChild(textEl(valueEnd, y, "hover-sub", unitLabel(p), { "text-anchor": "end" }));
    body.appendChild(textEl(otherEnd, y, "hover-sub", otherUnitLabel(p), { "text-anchor": "end" }));
    const relHead = textEl(0, y, "hover-sub", `relative to ${name(focus)}`, { "text-anchor": "end" });
    body.appendChild(relHead);
    y += 4;
    const relCells = [relHead];
    for (const r of table) {
      y += LINE;
      /* Swatch: this contender's mark at this point, in its own colour. */
      let rj = 0;
      r.s.kernels.forEach((kr, j) => { if (k >= kr.from) rj = j; });
      const sw = markGlyph(r.s.kernels[rj].mark, DATA.colors[r.i]);
      sw.setAttribute("class", "hover-swatch");
      sw.setAttribute("style", `fill: ${DATA.colors[r.i]}`);
      sw.setAttribute("transform", `translate(${PAD + 5} ${y - 4}) scale(0.85)`);
      body.appendChild(sw);
      const cls = "hover-row" + (r.i === focus ? " hover-row-focus" : "");
      body.appendChild(textEl(nameX, y, cls, r.name));
      body.appendChild(textEl(valueEnd, y, cls, r.value, { "text-anchor": "end" }));
      body.appendChild(textEl(otherEnd, y, cls, r.other, { "text-anchor": "end" }));
      const rel = textEl(0, y, "hover-ratio", r.rel, { "text-anchor": "end", fill: r.color });
      body.appendChild(rel);
      relCells.push(rel);
    }
    y += 12;
    body.appendChild(note(textEl(PAD, y, "hover-note", `each row's speed compared with ${name(focus)}; means, ranked fastest first`)));
    y += 4;
    /* The comparison column ends at the panel's right edge, known once every line is measured. */
    relCells.forEach(el => el.setAttribute("x", wide - PAD));
  }
  const H = y + PAD - 6, W = Math.min(wide, 640);

  /* Place beside the column, flipping left near the right edge. */
  const x = currentX[p][k];
  if (x < DATA.plotLeft || x > DATA.plotRight) { document.getElementById("hover").style.display = "none"; return; }
  const dotY = mapY(f.med[k]);
  let bx = x + 14;
  if (bx + W > DATA.plotRight + 10) bx = x - 14 - W;
  let by = Math.min(Math.max(dotY - H / 2, plot.top - 30), plot.bottom + 30 - H);

  const box = document.getElementById("hover-box");
  box.setAttribute("x", bx); box.setAttribute("y", by);
  box.setAttribute("width", W); box.setAttribute("height", H);
  body.setAttribute("transform", `translate(${bx} ${by})`);
  const guide = document.getElementById("hover-guide");
  guide.setAttribute("x1", x); guide.setAttribute("x2", x);
  guide.setAttribute("y1", plot.top); guide.setAttribute("y2", plot.bottom);
  document.getElementById("hover").style.display = "";
}

function hideHover() {
  hovered = null;
  highlightSeries(0, false);
  document.getElementById("hover").style.display = "none";
}

/*
 * Two inputs, one panel. A mouse hovers: entering a dot shows the panel,
 * leaving hides it, unless a click pinned it. A finger taps: pointerenter
 * fires too, without a matching leave, so touch is handled by tap alone.
 * Tapping a dot pins the panel to it; tapping it again, or the background,
 * clears it. Name highlighting follows the mouse only, since a finger
 * has no way to leave.
 */
function hoverDot(event, p, i, k) { if (event.pointerType === "mouse") showHover(p, i, k); }
function leaveDot(event) { if (event.pointerType === "mouse" && !pinned) hideHover(); }
function tapDot(event, p, i, k) {
  event.stopPropagation();
  if (pinned && pinned[0] === p && pinned[1] === i && pinned[2] === k) { pinned = null; hideHover(); return; }
  pinned = [p, i, k];
  showHover(p, i, k);
}
function tapAway() { pinned = null; hideHover(); }
/* The name under the mouse, so a click that shows or hides it redraws the dimming. */
let labelUnderMouse = null;
function hoverLabel(event, i, active) {
  if (event.pointerType !== "mouse") return;
  labelUnderMouse = active ? i : null;
  highlightSeries(i, active);
}

window.toggleSeries = toggleSeries;
window.toggleProv = toggleProv;
window.hoverDot = hoverDot;
window.leaveDot = leaveDot;
window.tapDot = tapDot;
window.tapAway = tapAway;
window.hoverLabel = hoverLabel;
window.setUnit = setUnit;
window.flipUnit = flipUnit;
window.zoomAll = zoomAll;
window.toggleHowto = toggleHowto;
window.toggleChip = toggleChip;
window.gripDown = gripDown;
window.gripMove = gripMove;
window.gripUp = gripUp;
updateZoomControls();
relayout();
/* The better arrows, measured against the titles as drawn, once now and again when the fonts arrive. */
const measureArrows = () => DATA.plots.forEach((_, p) => { const t = document.getElementById("y-title-" + p); betterArrow(p, t.textContent, !t.textContent.endsWith("lower is better")); });
measureArrows();
if (document.fonts && document.fonts.ready) document.fonts.ready.then(measureArrows);

"##;

/// "source URL · branch B · commit C" for a git-dependency provenance line.
fn short_git_source(description: &str) -> String {
    let mut fields = description.split("; ");
    let _name = fields.next();
    let rest: Vec<&str> = fields.collect();
    rest.iter()
        .map(|f| if let Some(c) = f.strip_prefix("commit ") { format!("commit {}", &c[..12.min(c.len())]) } else { f.to_string() })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn package_name_and_version(source_info: &str) -> &str {
    source_info
        .split(';')
        .next()
        .expect("package source information must not be empty")
}

fn nice_log_bound_below(value: f64) -> f64 {
    assert!(value.is_finite() && value > 0.0);

    let magnitude = 10.0_f64.powf(value.log10().floor());
    let normalized = value / magnitude;

    let nice = if normalized >= 5.0 {
        5.0
    } else if normalized >= 2.0 {
        2.0
    } else {
        1.0
    };

    nice * magnitude
}

fn nice_log_bound_above(value: f64) -> f64 {
    assert!(value.is_finite() && value > 0.0);

    let magnitude = 10.0_f64.powf(value.log10().floor());
    let normalized = value / magnitude;

    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}

fn log_ticks(axis_min: f64, axis_max: f64) -> Vec<f64> {
    let lowest_exponent = axis_min.log10().floor() as i32 - 1;
    let highest_exponent = axis_max.log10().ceil() as i32 + 1;

    let mut ticks = Vec::new();

    for exponent in lowest_exponent..=highest_exponent {
        for mantissa in [1.0, 1.5, 2.0, 3.0, 5.0, 7.0] {
            let value = mantissa * 10.0_f64.powi(exponent);

            /*
             * Tolerate one part in a million of floating-point error at
             * the axis bounds themselves.
             */
            if value >= axis_min * 0.999_999
                && value <= axis_max * 1.000_001
            {
                ticks.push(value);
            }
        }
    }

    assert!(
        ticks.len() >= 2,
        "a log axis must have at least two ticks"
    );

    ticks
}

fn format_gbps_tick(value: f64) -> String {
    assert!(
        value > 0.0,
        "log-axis ticks must be positive"
    );

    /*
     * Below 1, two significant digits with trailing zeros dropped, so the
     * ticks 0.15 and 0.2 (or 0.015 and 0.02) read apart; the script's
     * fmtTick writes the same.
     */
    if value >= 10.0 {
        format!("{value:.0}")
    } else if value >= 1.0 {
        format!("{value:.1}")
    } else {
        let decimals = (-value.log10()).ceil() as usize + 1;
        let text = format!("{value:.decimals$}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

/// A measured value as a bare rate number, as the graph's value labels
/// show it in the default unit: whole numbers at 10 and above, one
/// decimal from 1, and two significant digits below 1 (0.21, 0.012), as
/// the axis ticks read; the script's fmt writes the same.
fn format_rate_value(time: PerUnit, use_case: UseCase) -> String {
    let scale = use_case.rate_scale();
    let tenths = time.tenths_of(scale);
    if tenths >= 100 {
        return format!("{}", (tenths + 5) / 10);
    }
    if time.scaled_of(scale, 100) >= 100 {
        return format!("{}.{}", tenths / 10, tenths % 10);
    }
    /* Below 1: the fewest decimals (two to nine) that give two significant digits. */
    let mut decimals = 2;
    while decimals < 9 && time.scaled_of(scale, 10u64.pow(decimals)) < 10 {
        decimals += 1;
    }
    let scaled = time.scaled_of(scale, 10u64.pow(decimals));
    format!("0.{scaled:0width$}", width = decimals as usize)
}

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
        let t = |ns, units| Measured::new(ns, units).per_unit();
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
        assert_eq!(t(6, 1).tenths_of(1), 2); // 1 / 6 GB/s: 0.1667, two tenths
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

    /// Axis ticks below 1 keep two significant digits, so neighbouring
    /// ticks read apart; zoom labels name sizes the way the script does.
    #[test]
    fn tick_and_size_labels() {
        let ticks: Vec<String> = [70.0, 10.0, 7.0, 1.5, 1.0, 0.7, 0.2, 0.15, 0.1, 0.05, 0.015].iter().map(|&v| format_gbps_tick(v)).collect();
        assert_eq!(ticks, ["70", "10", "7.0", "1.5", "1.0", "0.7", "0.2", "0.15", "0.1", "0.05", "0.015"]);
        let sizes: Vec<String> = [64, 192, 1024, 1025, 1536, 2304, 3072, 1 << 20, 3 << 20, 16 << 20].iter().map(|&b| format_bytes(b)).collect();
        assert_eq!(sizes, ["64 B", "192 B", "1 KiB", "1025 B", "1536 B", "2304 B", "3 KiB", "1 MiB", "3 MiB", "16 MiB"]);
        /* Value labels: ns per byte (ns, bytes) to GB/s, two significant digits below 1. */
        let values: Vec<String> = [(1, 10), (10, 100), (1, 1), (476, 100), (80, 1), (2155, 100), (1000, 1)]
            .iter()
            .map(|&(ns, bytes)| format_rate_value(Measured::new(ns, bytes).per_unit(), UseCase::OneMessage))
            .collect();
        assert_eq!(values, ["10", "10", "1.0", "0.21", "0.013", "0.046", "0.0010"]);
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
        assert!(guide.contains("\"lat\":["), "each series carries per-call latency");
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
        let pieces = lent + CONTINUOUS_MESSAGE_COUNT;
        assert_eq!(UseCase::LentPieces.points(), pieces..pieces + LENT_PIECES_COUNT);
        assert_eq!(UseCase::LentBatches.points(), pieces + LENT_PIECES_COUNT..POINT_COUNT);
        let covered: Vec<_> = UseCase::ALL.into_iter().flat_map(UseCase::points).collect();
        assert_eq!(covered, (0..POINT_COUNT).collect::<Vec<_>>());
        for (owned, lent) in POINTS[UseCase::ContinuousMessages.points()].iter().zip(&POINTS[UseCase::LentMessages.points()]) {
            assert_eq!((owned.label, owned.bytes), (lent.label, lent.bytes));
        }
        let long = POINTS[UseCase::LentPieces.points()][0];
        assert!(long.bytes == 64 << 20 && long.label == "64 MiB", "a message in pieces is one long message, 64 MiB");
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

    /// Every contender taking part hands `consume` the digests of every
    /// message or batch on the continuous axes, the fork's queues
    /// included (one buffer per message, pieces, and batches), with fewer,
    /// as many, and more messages or batches than it keeps in flight; and
    /// every digest is the message's hash (servil's queues and lent calls against
    /// servil's hash, the others against their own one-shot call).
    #[test]
    fn continuous_use_cases_observe_every_digest() {
        let long = make_input(PIECE_LEN * 3 + 1000);
        let batch = make_input(16 * MESSAGE_LEN);
        let cases = [
            (Point::lent("", 1000), make_input(1000)),
            (Point::lent("", long.len()), long.clone()),
            (Point::lent_pieces("", long.len()), long.clone()),
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
                /* The digests the same input gives in memory. */
                let in_memory_point = if point.use_case.batch() { Point::many("", 16) } else { Point::one("", input.len()) };
                let mut expected = Vec::new();
                hash_in_memory(algorithm, input, in_memory_point, 1, |digest| expected.extend_from_slice(digest));
                let count = in_flight(if point.use_case.batch() { input.len() } else { input.len().min(PIECE_LEN) });
                for iterations in [1, count, 3 * count + 1] {
                    let mut seen = Vec::new();
                    hash_batch(algorithm, input, *point, iterations, |digest| seen.extend_from_slice(digest));
                    assert_eq!(seen.len(), iterations * expected.len(), "{} {:?}", algorithm.key(), point.use_case);
                    assert!(seen.chunks(expected.len()).all(|each| each == expected), "{} {:?}", algorithm.key(), point.use_case);
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
