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
        UseCase::OneMessage => ("one", "busy", "hash_multithreaded", "fn.hash_multithreaded.html"),
        UseCase::IdleOneMessage => ("one", "idle", "hash_multithreaded", "fn.hash_multithreaded.html"),
        UseCase::LentMessages => ("one", "lent", "hash_multithreaded", "fn.hash_multithreaded.html"),
        UseCase::ContinuousMessages => ("one", "piped", "Queue::messages", "struct.Queue.html#method.messages"),
        UseCase::ManyMessages => ("batch", "busy", "hash_many_multithreaded", "fn.hash_many_multithreaded.html"),
        UseCase::IdleManyMessages => ("batch", "idle", "hash_many_multithreaded", "fn.hash_many_multithreaded.html"),
        UseCase::LentBatches => ("batch", "lent", "hash_many_multithreaded", "fn.hash_many_multithreaded.html"),
        UseCase::ContinuousBatches => ("batch", "piped", "Queue::fixed", "struct.Queue.html#method.fixed"),
        UseCase::Interleaved => ("many", "lent", "Hasher::update_each_multithreaded", "struct.Hasher.html#method.update_each_multithreaded"),
        UseCase::Collection => ("coll", "lent", "hash_each_multithreaded_with", "fn.hash_each_multithreaded_with.html"),
        UseCase::Outboard => ("outb", "lent", "outboard_multithreaded_with", "fn.outboard_multithreaded_with.html"),
        UseCase::Verify => ("recv", "lent", "Verifier::update", "struct.Verifier.html#method.update"),
    }
}

/// The map's address of a use case's chart: `scenario|row|col`, as a link
/// to `bench-hashes.map.html#…` opens it.
pub fn cell_key(scenario: &str, use_case: UseCase) -> String {
    let (row, col, _, _) = place(use_case);
    format!("{scenario}|{row}|{col}")
}

const DOCS: &str = "https://johnservil.github.io/BLAKE3/blake3_servil/";

/// What one unit of a point's time pays for, in the tooltip's words.
fn what(use_case: UseCase) -> &'static str {
    match use_case {
        UseCase::Collection => "the collection",
        UseCase::Interleaved => "a megabyte of pieces",
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

/// `text` as a JSON string, safe inside a script element.
fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); }
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
    series: Vec<Option<Vec<f64>>>,
}

impl Chart {
    fn json(&self) -> String {
        let sizes: Vec<String> = self.sizes.iter().map(|s| json(s)).collect();
        let per: Vec<String> = self.per.iter().map(u64::to_string).collect();
        let series: Vec<String> = self.series.iter().map(|s| match s {
            Some(v) => format!("[{}]", v.iter().map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(",")),
            None => "null".to_owned(),
        }).collect();
        format!("{{\"sizes\":[{}],\"unit\":{},\"call\":{},\"doc\":{},\"cases\":{},\"per\":[{}],\"what\":{},\"series\":[{}]}}",
            sizes.join(","), json(self.unit), json(&self.call), json(&self.doc), self.cases, per.join(","), json(self.what), series.join(","))
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
}

impl Section {
    fn json(&self) -> String {
        let triples = |v: &[(&str, &str, &str)]| v.iter().map(|(a, b, c)| format!("[{},{},{}]", json(a), json(b), json(c))).collect::<Vec<_>>().join(",");
        let names = self.names.iter().map(|n| json(n)).collect::<Vec<_>>().join(",");
        let colors = self.colors.iter().map(|n| json(n)).collect::<Vec<_>>().join(",");
        let shown = self.shown.iter().map(bool::to_string).collect::<Vec<_>>().join(",");
        let cells = self.cells.iter().map(|(k, c)| format!("{}:{}", json(k), c.json())).collect::<Vec<_>>().join(",");
        let details = self.details.iter().map(|(a, b, c)| format!("[{},{},{}]", json(a), json(b), json(c))).collect::<Vec<_>>().join(",");
        format!("{{\"title\":{},\"rows\":[{}],\"cols\":[{}],\"layers\":[{}],\"names\":[{}],\"colors\":[{}],\"shown\":[{}],\"cells\":{{{}}},\"about\":{},\"details\":[{}]}}",
            json(self.title), triples(&self.rows), triples(&self.cols), triples(&self.layers), names, colors, shown, cells, json(&self.about), details)
    }
}

/// The name of the outboards' line the BLAKE3 contender draws with bao-tree.
const BAO_TREE: &str = "bao-tree (iroh-blobs)";

fn header<'a>(headers: &'a [(String, String)], key: &str) -> &'a str {
    headers.iter().find(|(k, _)| k == key).map_or("", |(_, v)| v.as_str())
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
    let os = if os.starts_with("darwin") { "macOS" } else if os.starts_with("linux") { "Linux" } else { os };
    format!("{}, {} CPUs, {os} · {day} · {load}", header(headers, "cpu type"), header(headers, "cpu count"))
}

/// The run's provenance, behind its door, for the readers who want it:
/// when and where it ran, the software at its exact commits (links to
/// them), the build, the power and the load, and the samples file.
fn details(headers: &[(String, String)], load: &str, power: &str, samples: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut add = |label: &str, text: String, link: String| {
        if !text.is_empty() {
            out.push((label.to_owned(), text, link));
        }
    };
    add("Measured", header(headers, "timestamp").to_owned(), String::new());
    let identity = header(headers, "cpu identity");
    let cores = |level: &str| identity.split(" · ").find_map(|f| f.strip_prefix(&format!("hw.perflevel{level}.physicalcpu: ")).map(str::to_owned));
    let machine = match (cores("0"), cores("1")) {
        (Some(p), Some(e)) => format!("{}, {p} performance and {e} efficiency cores, {}", header(headers, "cpu type"), header(headers, "os type")),
        _ => format!("{}, {} CPUs, {}", header(headers, "cpu type"), header(headers, "cpu count"), header(headers, "os type")),
    };
    add("Machine", machine, String::new());
    add("Power", power.to_owned(), String::new());
    add("Other programs", load.to_owned(), String::new());
    let version = header(headers, "bench-hashes version");
    let commit = header(headers, "git commit");
    let clean = header(headers, "git clean status");
    add("bench-hashes", version.split('+').next().unwrap_or("").to_owned(),
        format!("https://github.com/johnservil/bench-hashes/releases/tag/v{}", version.replace('+', "%2B")));
    add("bench-hashes commit", format!("{commit}{}", if clean.is_empty() || clean == "clean" { String::new() } else { format!(" ({clean})") }),
        format!("https://github.com/johnservil/bench-hashes/tree/{commit}"));
    let servil = header(headers, "blake3-servil source");
    if let Some(at) = servil.split("; commit ").nth(1) {
        let fork = at.split([';', ' ']).next().unwrap_or("");
        add("BLAKE3 servil", format!("github.com/johnservil/BLAKE3 at {fork}"), format!("https://github.com/johnservil/BLAKE3/tree/{fork}"));
    }
    for (label, key) in [("BLAKE3 official", "blake3 source"), ("SHA-256", "sha256 source")] {
        add(label, header(headers, key).split(';').next().unwrap_or("").to_owned(), String::new());
    }
    for (label, key) in [("Compiler", "rust compiler"), ("Target", "build target"), ("Files", "files")] {
        add(label, header(headers, key).to_owned(), String::new());
    }
    for (k, v) in headers.iter().filter(|(k, _)| k.starts_with("contender ")) {
        add(&format!("b3sum {}", &k["contender ".len()..]), v.rsplit('/').next().unwrap_or(v).to_owned(), String::new());
    }
    add("Samples", samples.to_owned(), samples.to_owned());
    out
}

fn hashing(path: &Path) -> Section {
    let file = read_samples(path.to_str().expect("a path in UTF-8"));
    let mut algorithms: Vec<Algorithm> = Algorithm::ALL.into_iter().filter(|a| file.cells.iter().any(|(k, _)| k.split('|').next() == Some(a.key()))).collect();
    algorithms.sort_by_key(|a| Algorithm::ALL.iter().position(|b| b == a));
    let mut cells = Vec::new();
    for use_case in UseCase::ALL {
        let (_, _, call, doc) = place(use_case);
        for scenario in ["solo", "shared"] {
            let points = &POINTS[use_case.points()];
            // Every chart in bytes per second, so one axis serves them all:
            // a batch's time per message over its message's bytes.
            let per_byte = if use_case.batch() { use_case.message_len() as f64 } else { 1.0 };
            let mean = |a: &Algorithm, label: &str| {
                let key = format!("{}|{scenario}|{use_case:?}|{label}", a.key());
                file.cells.iter().find(|(k, _)| *k == key).map(|(_, s)| mean_ns(s) / per_byte)
            };
            // The points this run measured for some contender, in axis order.
            let measured: Vec<_> = points.iter().filter(|p| algorithms.iter().any(|a| mean(a, p.label).is_some())).collect();
            if measured.is_empty() {
                continue;
            }
            let mut series: Vec<Option<Vec<f64>>> = algorithms.iter().map(|a| measured.iter().map(|p| mean(a, p.label)).collect()).collect();
            // The BLAKE3 contender builds and verifies outboards with bao-tree
            // (iroh-blobs' crate), so its line there carries bao-tree's name, as its own contender.
            let official = algorithms.iter().position(|a| *a == Algorithm::Blake3);
            let bao_tree = if matches!(use_case, UseCase::Outboard | UseCase::Verify) { official.and_then(|i| series[i].take()) } else { None };
            series.push(bao_tree);
            cells.push((cell_key(scenario, use_case), Chart {
                sizes: measured.iter().map(|p| p.label.to_owned()).collect(),
                unit: "GB/s",
                call: call.to_owned(),
                doc: format!("{DOCS}{doc}"),
                cases: matches!(use_case, UseCase::Interleaved | UseCase::Collection),
                per: measured.iter().map(|p| p.bytes as u64).collect(),
                what: what(use_case),
                series,
            }));
        }
    }
    Section {
        title: "How fast each hash runs, by what a program hashes and how it calls",
        rows: vec![
            ("one", "one message", "One message in memory, of the length along the chart's axis."),
            ("batch", "a batch of short messages", "Many 64-byte messages hashed together, such as a Merkle tree's nodes; the axis counts the messages."),
            ("many", "many messages at once, in pieces", "64 messages in progress at once, each arriving in pieces, as a server's uploads arrive."),
            ("coll", "a collection of items", "A collection of items of every size, such as a repository's objects or a store's files, hashed together."),
            ("outb", "a message with its outboard, for verified streaming", "A message's hash and its outboard: the hash tree's nodes, kept beside the message so that any part of it can be verified when it is sent (Bao, iroh-blobs)."),
            ("recv", "a message received, verified as it arrives", "A message received with its hash tree's nodes, each part verified as it arrives (iroh-blobs)."),
        ],
        cols: vec![
            ("busy", "now and then, after other work", "The program hashes now and then and does other work in between, which pushes the hash's code and data out of the CPU's caches."),
            ("idle", "now and then, after a pause", "The program hashes now and then and waits in between (for the network, say), so the core rests and starts the next hash slowly."),
            ("lent", "nonstop, waiting for each call", "The program hashes input after input as fast as it can, reading each input once the hash before it has returned."),
            ("piped", "nonstop, pipelined", "The program hashes input after input as fast as it can, and reads the next inputs while earlier ones hash: it hands its buffers to BLAKE3 servil's Queue, which hashes them on other threads. The other hashes have no such call and hash on the program's thread."),
        ],
        layers: vec![
            ("solo", "one program", "One program hashes; the machine is otherwise quiet."),
            ("shared", "two programs at once", "Two programs hash at the same moment, nonstop, sharing the machine."),
        ],
        names: algorithms.iter().map(|a| a.name()).chain([BAO_TREE]).map(str::to_owned).collect(),
        colors: algorithms.iter().map(|a| a.color()).chain(["#15803d"]).map(str::to_owned).collect(),
        shown: algorithms.iter().map(|a| SHOWN_AT_FIRST.contains(a)).chain([true]).collect(),
        cells,
        about: about(&file.headers, &file.load),
        details: details(&file.headers, &file.load, &file.power, "bench-hashes.samples.tsv"),
    }
}

fn b3sum(path: &Path) -> Section {
    let file = read_samples(path.to_str().expect("a path in UTF-8"));
    let mut names: Vec<String> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    for (key, _) in &file.cells {
        let mut parts = key.split('|');
        let (name, _cache, _, input) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
        if !inputs.iter().any(|i| i == input) {
            inputs.push(input.to_owned());
        }
    }
    // Single files are "4 KiB" and the like; anything else is a tree of files.
    let single = |i: &str| i.split_once(' ').is_some_and(|(n, unit)| n.parse::<u64>().is_ok() && ["KiB", "MiB", "GiB"].contains(&unit));
    let palette = ["#3b82f6", "#4c1d95", "#c2410c", "#0e9aa7", "#db2777", "#8a7a1e"];
    let mut cells = Vec::new();
    for (cache, col) in [("warm", "cached"), ("cold", "storage")] {
        for (row, points) in [("file", inputs.iter().filter(|i| single(i)).collect::<Vec<_>>()), ("tree", inputs.iter().filter(|i| !single(i)).collect())] {
            let cell = |name: &str, input: &str| file.cells.iter().find(|(k, _)| *k == format!("{name}|{cache}|b3sum|{input}")).map(|(_, s)| s);
            if points.is_empty() || names.iter().all(|n| points.iter().all(|p| cell(n, p).is_none())) {
                continue;
            }
            cells.push((format!("solo|{row}|{col}"), Chart {
                sizes: points.iter().map(|p| p.to_string()).collect(),
                unit: "GB/s",
                call: "b3sum".to_owned(),
                doc: "https://github.com/johnservil/BLAKE3/tree/servil/b3sum".to_owned(),
                cases: row == "tree",
                per: points.iter().map(|p| names.iter().find_map(|n| cell(n, p)).map_or(0, |s| s[0].units)).collect(),
                what: "a run",
                series: names.iter().map(|n| points.iter().map(|p| cell(n, p).map(|s| mean_ns(s))).collect()).collect(),
            }));
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
    }
}

/// Draw `directory`'s map from the samples files in it; true if it held any.
pub fn write(directory: &Path) -> bool {
    let mut sections = Vec::new();
    let hashing_path = directory.join("bench-hashes.samples.tsv");
    if hashing_path.exists() {
        sections.push(hashing(&hashing_path));
    }
    let b3sum_path = directory.join("b3sum.samples.tsv");
    if b3sum_path.exists() {
        sections.push(b3sum(&b3sum_path));
    }
    if sections.iter().all(|s| s.cells.is_empty()) {
        return false;
    }
    let data = format!("[{}]", sections.iter().map(Section::json).collect::<Vec<_>>().join(","));
    let page = include_str!("map.html").replace("@SECTIONS@", &data);
    let out = directory.join("bench-hashes.map.html");
    std::fs::write(&out, page).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    println!("# Map (HTML) is in \"{}\" .", out.display());
    true
}

/// `bench-hashes map DIR`: draw DIR's map from its samples files.
pub fn command(args: &[String]) {
    let [dir] = args else { panic!("usage: bench-hashes map DIR (a results directory)") };
    assert!(write(Path::new(dir)), "{dir} holds no samples file with cells (bench-hashes.samples.tsv, b3sum.samples.tsv)");
}
