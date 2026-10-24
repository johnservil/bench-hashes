//! `bench-hashes.map.html`: the map of a run's results. Every measurement is
//! a small chart, placed by what the program hashes (row) and how it calls
//! (column); a chart opens in place, with its headers lit and the values
//! under the pointer; each call's name links to its documentation. Drawn
//! from the samples files in a results directory: `bench-hashes.samples.tsv`
//! for the hashing section, `b3sum.samples.tsv` for b3sum's. A full run and
//! a b3sum run each draw it again; `bench-hashes map DIR` draws it from
//! stored files.

use super::{read_samples, Algorithm, UseCase, POINTS, SHOWN_AT_FIRST};
use std::fmt::Write as _;
use std::path::Path;

/// Where each use case sits: its row, its column, the call of BLAKE3
/// servil mt (the servil contender shown when the page opens), and that
/// call's page in the documentation.
fn place(use_case: UseCase) -> (&'static str, &'static str, &'static str, &'static str) {
    match use_case {
        UseCase::OneMessage => (
            "one",
            "busy",
            "hash_multithreaded",
            "fn.hash_multithreaded.html",
        ),
        UseCase::IdleOneMessage => (
            "one",
            "idle",
            "hash_multithreaded",
            "fn.hash_multithreaded.html",
        ),
        UseCase::LentMessages => (
            "one",
            "lent",
            "hash_multithreaded",
            "fn.hash_multithreaded.html",
        ),
        UseCase::ContinuousMessages => (
            "one",
            "piped",
            "Stream::submit_with",
            "struct.Stream.html#method.submit_with",
        ),
        UseCase::ManyMessages => (
            "batch",
            "busy",
            "hash_many_multithreaded",
            "fn.hash_many_multithreaded.html",
        ),
        UseCase::ManyLongMessages => (
            "long",
            "busy",
            "hash_many_multithreaded",
            "fn.hash_many_multithreaded.html",
        ),
        UseCase::IdleManyMessages => (
            "batch",
            "idle",
            "hash_many_multithreaded",
            "fn.hash_many_multithreaded.html",
        ),
        UseCase::LentBatches => (
            "batch",
            "lent",
            "hash_many_multithreaded",
            "fn.hash_many_multithreaded.html",
        ),
        UseCase::ContinuousBatches => (
            "batch",
            "piped",
            "Stream::submit_batch_with",
            "struct.Stream.html#method.submit_batch_with",
        ),
        UseCase::Interleaved => (
            "many",
            "lent",
            "Hasher::update_each_multithreaded",
            "struct.Hasher.html#method.update_each_multithreaded",
        ),
        UseCase::InterleavedTurn => (
            "many",
            "busy",
            "Hasher::update_each_multithreaded",
            "struct.Hasher.html#method.update_each_multithreaded",
        ),
        UseCase::IdleInterleavedTurn => (
            "many",
            "idle",
            "Hasher::update_each_multithreaded",
            "struct.Hasher.html#method.update_each_multithreaded",
        ),
        UseCase::Collection => (
            "coll",
            "lent",
            "hash_each_multithreaded_with",
            "fn.hash_each_multithreaded_with.html",
        ),
        UseCase::ContinuousCollection => (
            "coll",
            "piped",
            "Stream::submit_with",
            "struct.Stream.html#method.submit_with",
        ),
        UseCase::Outboard => (
            "outb",
            "lent",
            "outboard_multithreaded_with",
            "fn.outboard_multithreaded_with.html",
        ),
        UseCase::MerkleTree => (
            "merkle",
            "lent",
            "hash_many_multithreaded",
            "fn.hash_many_multithreaded.html",
        ),
        UseCase::Verify => (
            "recv",
            "lent",
            "Verifier::update",
            "struct.Verifier.html#method.update",
        ),
    }
}

/// The map's address of a use case's chart: `scenario|row|col`, as a link
/// to `bench-hashes.map.html#…` opens it.
pub fn cell_key(scenario: &str, use_case: UseCase) -> String {
    let (row, col, _, _) = place(use_case);
    format!("{scenario}|{row}|{col}")
}

const DOCS: &str = "https://johnservil.github.io/BLAKE3/blake3_servil/";

/// What a chart's points are, with their unit: the opened chart's x axis
/// and the tooltip's heading.
fn axis(use_case: UseCase) -> &'static str {
    match use_case {
        UseCase::Interleaved | UseCase::InterleavedTurn | UseCase::IdleInterleavedTurn => {
            "messages open at once"
        }
        UseCase::MerkleTree => "leaves (256 B each)",
        UseCase::Collection | UseCase::ContinuousCollection => "collection",
        _ if use_case.batch() => "messages in the batch (64 B each)",
        _ => "message size",
    }
}

/// What one unit of a point's time pays for, in the tooltip's words.
fn what(use_case: UseCase) -> &'static str {
    match use_case {
        UseCase::Collection | UseCase::ContinuousCollection => "the collection",
        UseCase::Interleaved => "a megabyte of pieces",
        UseCase::InterleavedTurn | UseCase::IdleInterleavedTurn => "a turn",
        UseCase::MerkleTree => "a tree",
        _ if use_case.batch() => "a batch",
        _ => "a message",
    }
}

/// A cell's mean time per unit in nanoseconds, for drawing (the one place
/// it leaves integers: clocks::summary::mean is Q64.64).
fn mean_ns(samples: &[super::Measured]) -> f64 {
    let mean = clocks::summary::mean(samples.iter().map(|m| (m.ns, m.units)));
    mean as f64 / 18_446_744_073_709_551_616.0
}

/// A cell's CPU time (ns) or energy (nJ) per unit, for drawing: all its
/// samples' figures in `column` (the samples file's `cpu` or `energy`) over
/// their units, where the file records them (solo nonstop cells, b3sum's
/// runs), else None.
fn per_unit(column: &[(String, Vec<u64>)], key: &str, samples: &[super::Measured]) -> Option<f64> {
    let (_, v) = column.iter().find(|(k, _)| k == key)?;
    let units: u128 = samples.iter().map(|m| u128::from(m.units)).sum();
    Some(v.iter().map(|&n| u128::from(n)).sum::<u128>() as f64 / units as f64).filter(|&x| x > 0.0)
}

/// A cell's core counts summed over its samples: [E share, P GHz, E GHz].
fn where_ran(column: &[(String, Vec<[u64; 4]>)], key: &str) -> Option<[f64; 3]> {
    let (_, v) = column.iter().find(|(k, _)| k == key)?;
    let [pt, pc, et, ec] = v.iter().fold([0u128; 4], |a, c| {
        [
            a[0] + u128::from(c[0]),
            a[1] + u128::from(c[1]),
            a[2] + u128::from(c[2]),
            a[3] + u128::from(c[3]),
        ]
    });
    let ghz = |cycles: u128, ns: u128| {
        if ns > 0 {
            cycles as f64 / ns as f64
        } else {
            0.0
        }
    };
    (pt + et > 0).then(|| [et as f64 / (pt + et) as f64, ghz(pc, pt), ghz(ec, et)])
}

/// `text` as a JSON string, safe inside a script element.
fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One small chart's data, as the page's script reads it.
struct Chart {
    sizes: Vec<String>,
    unit: &'static str,
    call: String,
    doc: String,
    cases: bool,
    per: Vec<u64>,
    what: &'static str,
    /// What the points are (axis).
    axis: &'static str,
    series: Vec<Option<Vec<f64>>>,
    /// An earlier run's means at the same points, by contender (a map
    /// drawn beside it; empty otherwise): None where it has none.
    before: Vec<Option<Vec<Option<f64>>>>,
    /// CPU time per unit at the same points, by contender (per_unit): None
    /// where the run recorded none.
    cpu: Vec<Option<Vec<Option<f64>>>>,
    /// Energy per unit (nJ), alike.
    energy: Vec<Option<Vec<Option<f64>>>>,
    /// Where the calling thread ran at each point, by contender, from the
    /// samples' core counts: [share of its time on E cores, P GHz, E GHz]
    /// (a clock 0 where it spent no time on that kind); None where none.
    cores: Vec<Option<Vec<Option<[f64; 3]>>>>,
}

impl Chart {
    fn json(&self) -> String {
        let sizes: Vec<String> = self.sizes.iter().map(|s| json(s)).collect();
        let per: Vec<String> = self.per.iter().map(u64::to_string).collect();
        let series: Vec<String> = self
            .series
            .iter()
            .map(|s| match s {
                Some(v) => format!(
                    "[{}]",
                    v.iter()
                        .map(|x| format!("{x:.6}"))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                None => "null".to_owned(),
            })
            .collect();
        let optional = |lines: &[Option<Vec<Option<f64>>>]| {
            lines
                .iter()
                .map(|s| match s {
                    Some(v) => format!(
                        "[{}]",
                        v.iter()
                            .map(|x| x.map_or("null".to_owned(), |x| format!("{x:.6}")))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    None => "null".to_owned(),
                })
                .collect::<Vec<String>>()
                .join(",")
        };
        format!("{{\"sizes\":[{}],\"unit\":{},\"call\":{},\"doc\":{},\"cases\":{},\"per\":[{}],\"what\":{},\"axis\":{},\"series\":[{}],\"before\":[{}],\"cpu\":[{}],\"energy\":[{}],\"cores\":[{}]}}",
            sizes.join(","), json(self.unit), json(&self.call), json(&self.doc), self.cases, per.join(","), json(self.what), json(self.axis), series.join(","), optional(&self.before), optional(&self.cpu), optional(&self.energy),
            self.cores.iter().map(|c| match c {
                Some(v) => format!("[{}]", v.iter().map(|x| x.map_or("null".to_owned(), |[e, p, q]| format!("[{e:.4},{p:.3},{q:.3}]"))).collect::<Vec<_>>().join(",")),
                None => "null".to_owned(),
            }).collect::<Vec<_>>().join(","))
    }
}

struct Section {
    title: &'static str,
    /// Rows, columns, and layers: (key, name, what it means).
    rows: Vec<(&'static str, &'static str, &'static str)>,
    cols: Vec<(&'static str, &'static str, &'static str)>,
    layers: Vec<(&'static str, &'static str, &'static str)>,
    names: Vec<String>,
    colors: Vec<String>,
    shown: Vec<bool>,
    cells: Vec<(String, Chart)>,
    about: String,
    /// The run's provenance, behind its door: (label, text, link or "").
    details: Vec<(String, String, String)>,
    /// The run's name, as an earlier run beside a later one is named.
    run: String,
    /// The earlier run drawn beside this one, dashed: its name and its
    /// line (empty when there is none).
    before: (String, String),
}

impl Section {
    fn json(&self) -> String {
        let triples = |v: &[(&str, &str, &str)]| {
            v.iter()
                .map(|(a, b, c)| format!("[{},{},{}]", json(a), json(b), json(c)))
                .collect::<Vec<_>>()
                .join(",")
        };
        let names = self
            .names
            .iter()
            .map(|n| json(n))
            .collect::<Vec<_>>()
            .join(",");
        let colors = self
            .colors
            .iter()
            .map(|n| json(n))
            .collect::<Vec<_>>()
            .join(",");
        let shown = self
            .shown
            .iter()
            .map(bool::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let cells = self
            .cells
            .iter()
            .map(|(k, c)| format!("{}:{}", json(k), c.json()))
            .collect::<Vec<_>>()
            .join(",");
        let details = self
            .details
            .iter()
            .map(|(a, b, c)| format!("[{},{},{}]", json(a), json(b), json(c)))
            .collect::<Vec<_>>()
            .join(",");
        format!("{{\"title\":{},\"rows\":[{}],\"cols\":[{}],\"layers\":[{}],\"names\":[{}],\"colors\":[{}],\"shown\":[{}],\"cells\":{{{}}},\"about\":{},\"details\":[{}],\"run\":{},\"before\":[{},{}]}}",
            json(self.title), triples(&self.rows), triples(&self.cols), triples(&self.layers), names, colors, shown, cells, json(&self.about), details, json(&self.run), json(&self.before.0), json(&self.before.1))
    }
}

/// The name of the outboards' line the BLAKE3 contender draws with bao-tree.
const BAO_TREE: &str = "bao-tree (iroh-blobs)";

fn header<'a>(headers: &'a [(String, String)], key: &str) -> &'a str {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map_or("", |(_, v)| v.as_str())
}

/// The run a samples file holds, in a line for every reader: the machine,
/// the day, and whether other programs kept it busy (the details are
/// behind the run's door).
fn about(headers: &[(String, String)], load: &str) -> String {
    let day = header(headers, "timestamp").split(' ').next().unwrap_or("");
    let busy = load.split("busy in ").nth(1).and_then(|rest| {
        let mut words = rest.split_whitespace();
        Some((words.next()?.to_owned(), words.nth(1)?.to_owned()))
    });
    let load = match busy {
        Some((n, of)) => format!("other programs busy in {n} of {of} seconds"),
        None if load.starts_with("quiet") => "the machine quiet".to_owned(),
        None => "load not measured".to_owned(),
    };
    let os = header(headers, "os type");
    let os = if os.starts_with("darwin") {
        "macOS"
    } else if os.starts_with("linux") {
        "Linux"
    } else {
        os
    };
    format!(
        "{}, {} CPUs, {os} · {day} · {load}",
        header(headers, "cpu type"),
        header(headers, "cpu count")
    )
}

/// The run's provenance, behind its door, for the readers who want it:
/// when and where it ran, the software at its exact commits (links to
/// them), the build, the power and the load, and the samples file.
fn details(
    headers: &[(String, String)],
    load: &str,
    power: &str,
    samples: &str,
) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut add = |label: &str, text: String, link: String| {
        if !text.is_empty() {
            out.push((label.to_owned(), text, link));
        }
    };
    add(
        "Measured",
        header(headers, "timestamp").to_owned(),
        String::new(),
    );
    let identity = header(headers, "cpu identity");
    let cores = |level: &str| {
        identity.split(" · ").find_map(|f| {
            f.strip_prefix(&format!("hw.perflevel{level}.physicalcpu: "))
                .map(str::to_owned)
        })
    };
    let machine = match (cores("0"), cores("1")) {
        (Some(p), Some(e)) => format!(
            "{}, {p} performance and {e} efficiency cores, {}",
            header(headers, "cpu type"),
            header(headers, "os type")
        ),
        _ => format!(
            "{}, {} CPUs, {}",
            header(headers, "cpu type"),
            header(headers, "cpu count"),
            header(headers, "os type")
        ),
    };
    add("Machine", machine, String::new());
    add("Power", power.to_owned(), String::new());
    add("Other programs", load.to_owned(), String::new());
    let version = header(headers, "bench-hashes version");
    let commit = header(headers, "git commit");
    let clean = header(headers, "git clean status");
    // The repository the benchmark was built from, as its header records it.
    let bench = header(headers, "bench-hashes source");
    let bench = bench.strip_prefix("https://").unwrap_or(bench);
    add(
        "bench-hashes",
        version.split('+').next().unwrap_or("").to_owned(),
        String::new(),
    );
    add(
        "bench-hashes commit",
        format!(
            "{bench} at {commit}{}",
            if clean.is_empty() || clean == "clean" {
                String::new()
            } else {
                format!(" ({clean})")
            }
        ),
        format!("https://{bench}/tree/{commit}"),
    );
    let servil = servil_source(headers);
    if let Some(at) = servil.split("; commit ").nth(1) {
        let fork = at.split([';', ' ']).next().unwrap_or("");
        // The repository the run's code came from, as its header records it.
        let repo = servil
            .split("; source https://")
            .nth(1)
            .and_then(|r| r.split(';').next())
            .expect("the blake3-servil source names its repository");
        add(
            "BLAKE3 servil",
            format!("{repo} at {fork}"),
            format!("https://{repo}/tree/{fork}"),
        );
    }
    // Every other contender's source, as the run recorded it.
    for (k, v) in headers
        .iter()
        .filter(|(k, _)| k.starts_with("source ") && !k.starts_with("source blake3-servil"))
    {
        let key = &k["source ".len()..];
        let name = Algorithm::ALL
            .into_iter()
            .find(|a| a.key() == key)
            .map_or(key.to_owned(), |a| a.name().to_owned());
        add(
            &name,
            v.split(';').next().unwrap_or("").to_owned(),
            String::new(),
        );
    }
    for (label, key) in [
        ("Rust compiler", "rust compiler"),
        ("C compiler", "c compiler"),
        ("RUSTFLAGS", "rustflags"),
        ("Dependencies", "dependency lock"),
        ("Target", "build target"),
        ("Files", "files"),
    ] {
        add(label, header(headers, key).to_owned(), String::new());
    }
    for (_, v) in headers.iter().filter(|(k, _)| k == "contender") {
        let (name, how) = v
            .split_once(": ")
            .expect("a contender line names its contender first");
        add(
            &format!("b3sum {name}"),
            how.rsplit('/').next().unwrap_or(how).to_owned(),
            String::new(),
        );
    }
    add("Samples", samples.to_owned(), samples.to_owned());
    out
}

fn hashing(path: &Path) -> Section {
    let file = read_samples(path.to_str().expect("a path in UTF-8"));
    let mut algorithms: Vec<Algorithm> = Algorithm::ALL
        .into_iter()
        .filter(|a| {
            file.cells
                .iter()
                .any(|(k, _)| k.split('|').next() == Some(a.key()))
        })
        .collect();
    algorithms.sort_by_key(|a| Algorithm::ALL.iter().position(|b| b == a));
    let mut cells = Vec::new();
    for use_case in UseCase::ALL {
        let (_, _, call, doc) = place(use_case);
        for scenario in ["solo", "shared"] {
            let points = &POINTS[use_case.points()];
            // Every chart in bytes per second, so one axis serves them all:
            // a batch's time per message over its message's bytes.
            let per_byte = if use_case.batch() {
                use_case.message_len() as f64
            } else {
                1.0
            };
            let mean = |a: &Algorithm, label: &str| {
                let key = format!("{}|{scenario}|{use_case:?}|{label}", a.key());
                file.cells
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, s)| mean_ns(s) / per_byte)
            };
            let column = |col: &[(String, Vec<u64>)], a: &Algorithm, label: &str| {
                let key = format!("{}|{scenario}|{use_case:?}|{label}", a.key());
                file.cells
                    .iter()
                    .find(|(k, _)| *k == key)
                    .and_then(|(_, s)| per_unit(col, &key, s))
                    .map(|v| v / per_byte)
            };
            // The points this run measured for some contender, in axis order.
            let measured: Vec<_> = points
                .iter()
                .filter(|p| algorithms.iter().any(|a| mean(a, p.label).is_some()))
                .collect();
            if measured.is_empty() {
                continue;
            }
            let mut series: Vec<Option<Vec<f64>>> = algorithms
                .iter()
                .map(|a| measured.iter().map(|p| mean(a, p.label)).collect())
                .collect();
            // The BLAKE3 contender builds and verifies outboards with bao-tree
            // (iroh-blobs' crate), so its line there carries bao-tree's name, as its own contender.
            let official = algorithms.iter().position(|a| *a == Algorithm::Blake3);
            let bao_tree = if matches!(use_case, UseCase::Outboard | UseCase::Verify) {
                official.and_then(|i| series[i].take())
            } else {
                None
            };
            series.push(bao_tree);
            // A line of `col` for each contender (and bao-tree's, as above).
            let lines = |col: &[(String, Vec<u64>)]| -> Vec<Option<Vec<Option<f64>>>> {
                let mut lines: Vec<Option<Vec<Option<f64>>>> = algorithms
                    .iter()
                    .map(|a| {
                        let line: Vec<Option<f64>> =
                            measured.iter().map(|p| column(col, a, p.label)).collect();
                        line.iter().any(Option::is_some).then_some(line)
                    })
                    .collect();
                let bao = if matches!(use_case, UseCase::Outboard | UseCase::Verify) {
                    official.and_then(|i| lines[i].take())
                } else {
                    None
                };
                lines.push(bao);
                lines
            };
            let (cpu_lines, energy_lines) = (lines(&file.cpu), lines(&file.energy));
            let mut cores: Vec<Option<Vec<Option<[f64; 3]>>>> = algorithms
                .iter()
                .map(|a| {
                    let line: Vec<Option<[f64; 3]>> = measured
                        .iter()
                        .map(|p| {
                            where_ran(
                                &file.cores,
                                &format!("{}|{scenario}|{use_case:?}|{}", a.key(), p.label),
                            )
                        })
                        .collect();
                    line.iter().any(Option::is_some).then_some(line)
                })
                .collect();
            let bao_cores = if matches!(use_case, UseCase::Outboard | UseCase::Verify) {
                official.and_then(|i| cores[i].take())
            } else {
                None
            };
            cores.push(bao_cores);
            cells.push((
                cell_key(scenario, use_case),
                Chart {
                    sizes: measured.iter().map(|p| p.label.to_owned()).collect(),
                    unit: "GB/s",
                    call: call.to_owned(),
                    doc: format!("{DOCS}{doc}"),
                    cases: matches!(
                        use_case,
                        UseCase::Interleaved
                            | UseCase::InterleavedTurn
                            | UseCase::IdleInterleavedTurn
                            | UseCase::Collection
                            | UseCase::ContinuousCollection
                    ),
                    per: measured.iter().map(|p| p.bytes as u64).collect(),
                    what: what(use_case),
                    axis: axis(use_case),
                    series,
                    before: Vec::new(),
                    cpu: cpu_lines,
                    energy: energy_lines,
                    cores,
                },
            ));
        }
    }
    Section {
        title: "How fast each hash runs, by what a program hashes and how it calls",
        rows: vec![
            ("one", "one message", "One message in memory, of the length along the chart's axis."),
            ("batch", "a batch of short messages", "Many 64-byte messages hashed together, such as a Merkle tree's nodes; the axis counts the messages."),
            ("long", "a batch of longer messages", "Many 4 KiB messages hashed together, such as a store's blocks or a backup's chunks; the axis counts the messages."),
            ("merkle", "a Merkle tree", "A Merkle tree over 256-byte leaves the program has just written into place, leaves and parents hashed apart, every layer kept, as a zero-knowledge prover commits to its Reed-Solomon codeword (WHIR); the axis counts the leaves."),
            ("many", "many messages at once, in pieces", "256 messages in progress at once, their pieces arriving 64 at a time, as a server's uploads arrive."),
            ("coll", "a collection of messages", "A collection of messages of every size, such as a repository's objects or a store's files, hashed together."),
            ("outb", "a message with its outboard, for verified streaming", "A message's hash and its outboard: the hash tree's nodes, kept beside the message so that any part of it can be verified when it is sent (Bao, iroh-blobs)."),
            ("recv", "a message received, verified as it arrives", "A message received with its hash tree's nodes, each part verified as it arrives (iroh-blobs)."),
        ],
        cols: vec![
            ("busy", "now and then, after other work", "The program hashes now and then and does other work in between, which pushes the hash's code and data out of the CPU's caches."),
            ("idle", "now and then, after a pause", "The program hashes now and then and waits in between (for the network, say), so the core rests and starts the next hash slowly."),
            ("lent", "nonstop, waiting for each call", "The program hashes input after input as fast as it can, reading each input once the hash before it has returned."),
            ("piped", "nonstop, pipelined", "The program hashes input after input as fast as it can, and reads the next inputs while earlier ones hash: it hands its inputs to BLAKE3 servil's stream, which hashes them on other threads. The other hashes have no such call and hash on the program's thread."),
        ],
        layers: vec![
            ("solo", "one program", "One program hashes; the machine is otherwise quiet."),
            ("shared", "two programs at once", "Two programs hash at the same moment, nonstop, sharing the machine."),
        ],
        // A run with --threads N names servil mt's count (its pool is the run's).
        names: {
            let threads = header(&file.headers, "contenders").split("; --threads ").nth(1).map(str::to_owned);
            algorithms.iter().map(|a| match (&threads, a) {
                (Some(n), Algorithm::Blake3ServilMt) => format!("{}, {n} threads", a.name()),
                _ => a.name().to_owned(),
            }).chain([BAO_TREE.to_owned()]).collect()
        },
        colors: algorithms.iter().map(|a| a.color()).chain(["#15803d"]).map(str::to_owned).collect(),
        shown: algorithms.iter().map(|a| SHOWN_AT_FIRST.contains(a)).chain([true]).collect(),
        cells,
        about: about(&file.headers, &file.load),
        details: details(&file.headers, &file.load, &file.power, "bench-hashes.samples.tsv"),
        run: run_name(&file.headers),
        before: (String::new(), String::new()),
    }
}

fn b3sum(path: &Path) -> Section {
    let file = read_samples(path.to_str().expect("a path in UTF-8"));
    let mut names: Vec<String> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    for (key, _) in &file.cells {
        let mut parts = key.split('|');
        let (name, _cache, _, input) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
        if !inputs.iter().any(|i| i == input) {
            inputs.push(input.to_owned());
        }
    }
    // Single files are "4 KiB" and the like; anything else is a tree of files.
    let single = |i: &str| {
        i.split_once(' ').is_some_and(|(n, unit)| {
            n.parse::<u64>().is_ok() && ["KiB", "MiB", "GiB"].contains(&unit)
        })
    };
    let palette = [
        "#3b82f6", "#4c1d95", "#c2410c", "#0e9aa7", "#db2777", "#8a7a1e",
    ];
    let mut cells = Vec::new();
    for (cache, col) in [("warm", "cached"), ("cold", "storage")] {
        for (row, points) in [
            (
                "file",
                inputs.iter().filter(|i| single(i)).collect::<Vec<_>>(),
            ),
            ("tree", inputs.iter().filter(|i| !single(i)).collect()),
        ] {
            let cell = |name: &str, input: &str| {
                file.cells
                    .iter()
                    .find(|(k, _)| *k == format!("{name}|{cache}|b3sum|{input}"))
                    .map(|(_, s)| s)
            };
            if points.is_empty()
                || names
                    .iter()
                    .all(|n| points.iter().all(|p| cell(n, p).is_none()))
            {
                continue;
            }
            let lines = |col: &[(String, Vec<u64>)]| -> Vec<Option<Vec<Option<f64>>>> {
                names
                    .iter()
                    .map(|n| {
                        let line: Vec<Option<f64>> = points
                            .iter()
                            .map(|p| {
                                cell(n, p).and_then(|s| {
                                    per_unit(col, &format!("{n}|{cache}|b3sum|{p}"), s)
                                })
                            })
                            .collect();
                        line.iter().any(Option::is_some).then_some(line)
                    })
                    .collect()
            };
            cells.push((
                format!("solo|{row}|{col}"),
                Chart {
                    sizes: points.iter().map(|p| p.to_string()).collect(),
                    unit: "GB/s",
                    call: "b3sum".to_owned(),
                    doc: format!("{}/tree/servil/b3sum", env!("BENCH_SERVIL_GIT")),
                    cases: row == "tree",
                    per: points
                        .iter()
                        .map(|p| {
                            names
                                .iter()
                                .find_map(|n| cell(n, p))
                                .map_or(0, |s| s[0].units)
                        })
                        .collect(),
                    what: "a run",
                    axis: if row == "tree" { "tree" } else { "file size" },
                    series: names
                        .iter()
                        .map(|n| {
                            points
                                .iter()
                                .map(|p| cell(n, p).map(|s| mean_ns(s)))
                                .collect()
                        })
                        .collect(),
                    before: Vec::new(),
                    cpu: lines(&file.cpu),
                    energy: lines(&file.energy),
                    cores: names.iter().map(|_| None).collect(),
                },
            ));
        }
    }
    Section {
        title: "How fast b3sum hashes files, from its start to its exit",
        rows: vec![
            ("file", "a file", "One file, of the size along the chart's axis."),
            ("tree", "a tree of files", "Many files, given to b3sum in one run."),
        ],
        cols: vec![
            ("cached", "in the page cache", "Each file was read moments before, so the operating system holds it in memory."),
            ("storage", "read from storage", "Each file was dropped from the operating system's memory before the run, so b3sum reads it from the disk."),
        ],
        layers: vec![("solo", "one program", "One program hashes; the machine is otherwise quiet.")],
        colors: (0..names.len()).map(|i| palette[i % palette.len()].to_owned()).collect(),
        shown: names.iter().map(|_| true).collect(),
        names,
        cells,
        about: about(&file.headers, &file.load),
        details: details(&file.headers, &file.load, &file.power, "b3sum.samples.tsv"),
        run: run_name(&file.headers),
        before: (String::new(), String::new()),
    }
}

/// Draw `directory`'s map from the samples files in it; true if it held any.
/// BLAKE3 servil's source as the run recorded it (either servil contender's
/// line: one crate), or "" where the run measured neither.
fn servil_source(headers: &[(String, String)]) -> &str {
    headers
        .iter()
        .find(|(k, _)| k.starts_with("source blake3-servil"))
        .map_or("", |(_, v)| v.as_str())
}

/// A run's name: its bench-hashes release and BLAKE3 servil's commit.
fn run_name(headers: &[(String, String)]) -> String {
    let version = header(headers, "bench-hashes version")
        .split('+')
        .next()
        .unwrap_or("");
    let fork = servil_source(headers)
        .split("commit ")
        .nth(1)
        .map_or("", |c| &c[..c.len().min(7)]);
    if fork.is_empty() {
        format!("bench-hashes {version}")
    } else {
        format!("bench-hashes {version}, BLAKE3 servil {fork}")
    }
}

/// Draw `old`'s runs beside `new`'s: each chart's earlier means, for the
/// contenders and points both runs hold.
fn beside(new: &mut Section, old: &Section) {
    // Its day and load; the machine is the run's own.
    let when = old
        .about
        .split_once(" · ")
        .map_or(old.about.as_str(), |(_, rest)| rest);
    new.before = (old.run.clone(), when.to_owned());
    for (key, chart) in &mut new.cells {
        let Some((_, was)) = old.cells.iter().find(|(k, _)| k == key) else {
            continue;
        };
        chart.before = new
            .names
            .iter()
            .map(|name| {
                let j = old.names.iter().position(|n| n == name)?;
                let series = was.series[j].as_ref()?;
                Some(
                    chart
                        .sizes
                        .iter()
                        .map(|size| was.sizes.iter().position(|s| s == size).map(|k| series[k]))
                        .collect(),
                )
            })
            .collect();
    }
}

/// Whether a samples file is this release's to read (its first line);
/// another release's is left to its own tools, with a line saying so.
fn this_release(path: &Path) -> bool {
    let first = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let first = first.lines().next().unwrap_or("");
    if first != super::SAMPLES_VERSION {
        println!(
            "# {} is another release's ({first}): read with the tools of the commit that wrote it; its section is left out of the map until a run under this release.",
            path.display()
        );
    }
    first == super::SAMPLES_VERSION
}

fn sections(directory: &Path) -> Vec<Section> {
    let mut sections = Vec::new();
    let hashing_path = directory.join("bench-hashes.samples.tsv");
    if hashing_path.exists() && this_release(&hashing_path) {
        sections.push(hashing(&hashing_path));
    }
    let b3sum_path = directory.join("b3sum.samples.tsv");
    if b3sum_path.exists() && this_release(&b3sum_path) {
        sections.push(b3sum(&b3sum_path));
    }
    sections
}

/// Write `directory`'s map (`bench-hashes.map.html`), from its samples
/// files; with `old`, a results directory of an earlier run, its map
/// beside that run (`bench-hashes.beside.map.html`), the earlier lines
/// dashed. Whether there was anything to draw.
pub fn write(directory: &Path, old: Option<&Path>) -> bool {
    let mut sections = sections(directory);
    if sections.iter().all(|s| s.cells.is_empty()) {
        return false;
    }
    if let Some(old) = old {
        let old = sections_of(old);
        for section in &mut sections {
            if let Some(was) = old.iter().find(|o| o.title == section.title) {
                beside(section, was);
            }
        }
    }
    let data = format!(
        "[{}]",
        sections
            .iter()
            .map(Section::json)
            .collect::<Vec<_>>()
            .join(",")
    );
    let page = include_str!("map.html").replace("@SECTIONS@", &data);
    let out = directory.join(if old.is_some() {
        "bench-hashes.beside.map.html"
    } else {
        "bench-hashes.map.html"
    });
    std::fs::write(&out, page).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    println!("# Map (HTML) is in \"{}\" .", out.display());
    true
}

fn sections_of(directory: &Path) -> Vec<Section> {
    let sections = sections(directory);
    assert!(
        sections.iter().any(|s| !s.cells.is_empty()),
        "{} holds no samples file with cells",
        directory.display()
    );
    sections
}

/// `bench-hashes map DIR [--beside OLD_DIR]`: draw DIR's map from its
/// samples files, or (with --beside) beside an earlier run's.
pub fn command(args: &[String]) {
    let usage = "usage: bench-hashes map DIR [--beside OLD_DIR] (results directories)";
    let (dir, old) = match args {
        [dir] => (dir, None),
        [dir, flag, old] if flag == "--beside" => (dir, Some(Path::new(old))),
        _ => panic!("{usage}"),
    };
    assert!(
        write(Path::new(dir), old),
        "{dir} holds no samples file with cells (bench-hashes.samples.tsv, b3sum.samples.tsv)"
    );
}
