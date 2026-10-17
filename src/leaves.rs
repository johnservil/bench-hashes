/*
 * `bench-hashes leaves`: the leaves of a Merkle tree that a program
 * computes, as a zero-knowledge prover's encoder computes its code words
 * and hashes each as a leaf. N 64-byte messages come from the simplest
 * producer there is, a counter (the buffer's 64-bit words count up from
 * 0, so message i holds 8i .. 8i + 7), the same in every variant; the
 * variants differ in how the program hands them to BLAKE3 servil:
 *
 *   lend-oneshot   produced into the program's buffer, then lent to
 *                  hash_many_multithreaded
 *   lend-stream    produced into the program's buffer, then lent to a
 *                  stream (Stream::submit_batch)
 *   copy-ring      produced into the program's buffer, then copied into
 *                  the stream's ring (Stream::submit_batch_with)
 *   write-ring     produced straight into the stream's ring
 *                  (Stream::submit_batch_with), overlapping the hashing
 *
 * A job is the whole of it, timed as one (the user's decision, October 16,
 * 2026): from producing the first message to having read every digest (each
 * folded into a checksum), nothing subtracted. Each cell reports three
 * rows: wall time per message (Leaves), and, over the cell's whole
 * interval, CPU time of every thread of the process per message
 * (LeavesCpu, clocks::threads) and the CPU package's energy per message in
 * nanojoules (LeavesEnergy, clocks::energy; package-wide, so only a quiet
 * machine's means anything). `solo` runs one program; `two` runs two at
 * once, each on its own thread and buffers, and counts both. The cells
 * rotate in each round; `bench-hashes compare` reads the samples file.
 */

use super::{machine_metadata, output_directory, SAMPLES_COLUMNS, SAMPLES_VERSION};
use std::fs;

const MESSAGE_LEN: usize = 64;
const COUNTS: [usize; 2] = [16 * 1024, 1 << 20];
const VARIANTS: [&str; 4] = ["lend-oneshot", "lend-stream", "copy-ring", "write-ring"];
const SCENARIOS: [&str; 2] = ["solo", "two"];
/// A sample's length: jobs back to back for at least this long.
const SAMPLE_NS: u64 = 5_000_000;
const SAMPLES_PER_ROUND: usize = 3;

/// Message `first` on, in `out` (whole messages): the counter.
fn produce(first: usize, out: &mut [u8]) {
    let mut word = (first * MESSAGE_LEN / 8) as u64;
    for w in out.chunks_exact_mut(8) {
        w.copy_from_slice(&word.to_le_bytes());
        word += 1;
    }
}

/// One program's buffers, kept from job to job as a prover keeps its own.
struct Program {
    buffer: Vec<u8>,
    digests: Vec<[u8; 32]>,
    /// The stream variants' places for the digests (kept, as `digests`).
    places: Vec<blake3_servil::Hash>,
}

impl Program {
    fn new(count: usize) -> Program {
        Program { buffer: vec![0u8; count * MESSAGE_LEN], digests: vec![[0u8; 32]; count], places: vec![blake3_servil::Hash::from_bytes([0; 32]); count] }
    }

    /// One job of `variant` over `count` messages: the digests' checksum.
    fn job(&mut self, variant: &str, count: usize) -> [u8; 32] {
        let mut sum = [0u8; 32];
        let mut fold = |hashes: &[blake3_servil::Hash]| {
            for h in hashes {
                for (s, b) in sum.iter_mut().zip(h.as_bytes()) {
                    *s ^= b;
                }
            }
        };
        let buffer = &mut self.buffer[..count * MESSAGE_LEN];
        match variant {
            "lend-oneshot" => {
                produce(0, buffer);
                let digests = &mut self.digests[..count];
                blake3_servil::hash_many_multithreaded(buffer, MESSAGE_LEN, digests);
                // Sound: a Hash is its 32 bytes.
                fold(unsafe { core::slice::from_raw_parts(digests.as_ptr() as *const blake3_servil::Hash, count) });
            }
            // The stream's variants write the digests into the same kept
            // space as lend-oneshot, then fold them as it does.
            "lend-stream" => {
                produce(0, buffer);
                let buffer = &*buffer;
                let places = &mut self.places[..count];
                blake3_servil::stream(blake3_servil::Mode::Hash, |_| {}, |s| {
                    s.submit_batch(buffer, MESSAGE_LEN, places);
                    s.finish();
                });
                fold(&self.places[..count]);
            }
            "copy-ring" => {
                produce(0, buffer);
                let buffer = &*buffer;
                let places = &mut self.places[..count];
                blake3_servil::stream(blake3_servil::Mode::Hash, |_| {}, |s| {
                    s.submit_batch_with(MESSAGE_LEN, |at, room| room.copy_from_slice(&buffer[at..][..room.len()]), places);
                    s.finish();
                });
                fold(&self.places[..count]);
            }
            "write-ring" => {
                let places = &mut self.places[..count];
                blake3_servil::stream(blake3_servil::Mode::Hash, |_| {}, |s| {
                    s.submit_batch_with(MESSAGE_LEN, |at, room| produce(at / MESSAGE_LEN, room), places);
                    s.finish();
                });
                fold(&self.places[..count]);
            }
            _ => unreachable!("a variant of VARIANTS"),
        }
        sum
    }
}

/// The checksum every variant must give: each message hashed alone.
fn expected(count: usize) -> [u8; 32] {
    let mut buffer = vec![0u8; count * MESSAGE_LEN];
    produce(0, &mut buffer);
    let mut sum = [0u8; 32];
    for m in buffer.chunks_exact(MESSAGE_LEN) {
        for (s, b) in sum.iter_mut().zip(blake3_servil::hash(m).as_bytes()) {
            *s ^= b;
        }
    }
    sum
}

/// One cell's samples this round: wall samples (ns, messages, start ms)
/// from every program, and the interval's CPU ns and package nJ.
struct Round {
    wall: Vec<(u64, u64, u64)>,
    cpu_ns: Option<u64>,
    energy_nj: Option<u64>,
    messages: u64,
}

fn measure_cell(programs: &mut [Program], variant: &str, count: usize, want: [u8; 32]) -> Round {
    let threads_before = clocks::threads::snapshot();
    let energy_before = clocks::energy::package_nj();
    let per_program: Vec<Vec<clocks::Batch>> = std::thread::scope(|scope| {
        let handles: Vec<_> = programs
            .iter_mut()
            .map(|p| {
                scope.spawn(move || {
                    clocks::measure(SAMPLES_PER_ROUND, SAMPLE_NS, || assert_eq!(p.job(variant, count), want, "{variant}: every leaf's digest"))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("a program's thread")).collect()
    });
    let energy_nj = energy_before.zip(clocks::energy::package_nj()).map(|(b, a)| a - b);
    let cpu_ns = threads_before.zip(clocks::threads::snapshot()).map(|(b, a)| clocks::threads::since(&b, &a).iter().map(|t| t.cpu_ns).sum());
    let wall: Vec<(u64, u64, u64)> =
        per_program.iter().flatten().map(|b| (b.wall_ns, b.calls * count as u64, b.started_ns / 1_000_000)).collect();
    // Every job of the cell, the untimed sizing runs' included, counts for
    // CPU time and energy: they ran inside the interval too.
    let messages = per_program.iter().flatten().map(|b| b.calls).sum::<u64>() * count as u64;
    Round { wall, cpu_ns, energy_nj, messages }
}

pub fn command(arguments: &[String]) {
    let usage = "bench-hashes leaves [--rounds N] [--variants V,...] [--counts N,...] [--scenarios solo,two]";
    let mut rounds = 12usize;
    let (mut variants, mut counts, mut scenarios) = (VARIANTS.to_vec(), COUNTS.to_vec(), SCENARIOS.to_vec());
    let mut it = arguments.iter();
    let list = |v: Option<&String>| -> Vec<String> { v.expect(usage).split(',').map(str::to_owned).collect() };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rounds" => rounds = it.next().expect(usage).parse().expect("--rounds takes a whole number"),
            "--variants" => variants = list(it.next()).iter().map(|v| *VARIANTS.iter().find(|&&k| k == v).unwrap_or_else(|| panic!("no variant {v}: {VARIANTS:?}"))).collect(),
            "--counts" => counts = list(it.next()).iter().map(|c| c.parse().expect("a count")).collect(),
            "--scenarios" => scenarios = list(it.next()).iter().map(|v| *SCENARIOS.iter().find(|&&k| k == v).unwrap_or_else(|| panic!("no scenario {v}"))).collect(),
            other => panic!("{usage}: unknown argument {other}"),
        }
    }
    assert!(rounds >= 2, "two rounds at least");
    blake3_servil::initialize_multithreaded();
    let mut machine = machine_metadata();
    let mut cells: Vec<(&str, usize, &str)> = Vec::new();
    for &sc in &scenarios {
        for &n in &counts {
            for &v in &variants {
                cells.push((sc, n, v));
            }
        }
    }
    let wants: Vec<[u8; 32]> = counts.iter().map(|&n| expected(n)).collect();
    let mut programs: Vec<Program> = (0..2).map(|_| Program::new(*counts.iter().max().unwrap())).collect();
    // Each cell's rounds; first one untimed round of every cell (one-time
    // costs: the pool, the stream's storage, the buffers' pages).
    let mut results: Vec<Vec<Round>> = cells.iter().map(|_| Vec::new()).collect();
    for round in 0..=rounds {
        for k in 0..cells.len() {
            let c = (k + round) % cells.len();
            let (scenario, count, variant) = cells[c];
            let n = if scenario == "solo" { 1 } else { 2 };
            let want = wants[counts.iter().position(|&x| x == count).unwrap()];
            let r = measure_cell(&mut programs[..n], variant, count, want);
            if round > 0 {
                results[c].push(r);
            }
        }
        eprintln!("bench-hashes leaves: round {round} of {rounds} done");
    }
    machine.load = clocks::load::windows();
    machine.power[1] = super::Power::read();

    let mut tsv = format!(
        "{SAMPLES_VERSION}\n# timestamp: {}\n# bench-hashes version: {}\n# git commit: {}\n# blake3-servil source: {}\n# cpu type: {}\n# cpu count: {}\n# os type: {}\n# rounds: {rounds}\n# leaves: {MESSAGE_LEN}-byte messages from a counter; wall from producing the first to reading the last digest\n# power: {}\n# load: {}\n",
        machine.timestamp,
        super::BENCH_VERSION,
        super::GIT_COMMIT,
        super::BLAKE3_SERVIL_SOURCE_INFO,
        machine.cpu_type,
        machine.cpu_count,
        machine.os_type,
        machine.describe_power(),
        clocks::load::describe(&machine.load)
    );
    tsv += SAMPLES_COLUMNS;
    tsv += "\n";
    let mut report = String::new();
    for (c, &(scenario, count, variant)) in cells.iter().enumerate() {
        let rs = &results[c];
        let point = format!("{count} messages");
        let row = |use_case: &str, unit: &str, samples: Vec<(u64, u64, u64)>| -> String {
            let values: Vec<String> = samples.iter().map(|(v, n, _)| format!("{v}/{n}")).collect();
            let starts: Vec<String> = samples.iter().map(|(_, _, s)| s.to_string()).collect();
            format!("{variant}\t{scenario}\t{use_case}\t{point}\t{unit}\t{}\t{}\n", values.join(","), starts.join(","))
        };
        let wall: Vec<(u64, u64, u64)> = rs.iter().flat_map(|r| r.wall.iter().copied()).collect();
        tsv += &row("Leaves", "msg", wall.clone());
        let firsts: Vec<u64> = rs.iter().map(|r| r.wall[0].2).collect();
        if rs.iter().all(|r| r.cpu_ns.is_some()) {
            tsv += &row("LeavesCpu", "msg", rs.iter().zip(&firsts).map(|(r, &s)| (r.cpu_ns.unwrap(), r.messages, s)).collect());
        }
        if rs.iter().all(|r| r.energy_nj.is_some()) {
            tsv += &row("LeavesEnergy", "msg", rs.iter().zip(&firsts).map(|(r, &s)| (r.energy_nj.unwrap(), r.messages, s)).collect());
        }
        // The report: means over the cell (ns, CPU ns, nJ per message, as
        // exact totals over totals, shown to two decimals).
        let per = |num: u64, den: u64| format!("{}.{:02}", num / den, num % den * 100 / den);
        let (w, n) = wall.iter().fold((0, 0), |(a, b), (v, m, _)| (a + v, b + m));
        let m: u64 = rs.iter().map(|r| r.messages).sum();
        let cpu = rs.iter().map(|r| r.cpu_ns).sum::<Option<u64>>().map_or("-".into(), |c| per(c, m));
        let nj = rs.iter().map(|r| r.energy_nj).sum::<Option<u64>>().map_or("-".into(), |e| per(e, m));
        report += &format!("{scenario:5} {count:>8} {variant:13} {:>8} ns {:>8} CPU ns {:>8} nJ  per message\n", per(w, n), cpu, nj);
    }
    let directory = output_directory(&machine);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("leaves.samples.tsv");
    fs::write(&path, tsv).unwrap();
    println!("bench-hashes leaves: {}\nload: {}\n\n{report}\n# Samples (TSV) are in {:?} .", super::BLAKE3_SERVIL_SOURCE_INFO, clocks::load::describe(&machine.load), path);
}
