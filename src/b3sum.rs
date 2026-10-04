//! `bench-hashes b3sum NAME=COMMAND...`: how long b3sum takes to hash files,
//! from its start to its exit, as the person who runs it waits (README,
//! "b3sum"). Builds of b3sum, and its ways of reading files, side by side on
//! the same files. Every run through `clocks::child::run`; each cell's
//! summary its mean, and each comparison the pairs' verdict
//! (`clocks::summary`), as the rest of bench-hashes; its samples file in
//! bench-hashes' format, so `bench-hashes compare` reads it.

use super::{output_directory, machine_metadata, ExactMean, Measured, SAMPLES_COLUMNS, SAMPLES_VERSION};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
const GIB: u64 = 1024 * MIB;

/// The margin a cell's verdict uses against the first contender, as the
/// regression check's solo cells.
const MARGIN_PERMILLE: u64 = 30;

// ---------- Inputs ----------

/// One input: the files one b3sum run hashes, all its arguments.
struct Input {
    label: String,
    files: Vec<PathBuf>,
    bytes: u64,
    /// One b3sum process per file, one after another, as `for F in ...; do
    /// b3sum $F; done` runs them; else one process for all the files.
    each: bool,
}

/// The inputs: single files from a page to a gigabyte (b3sum maps files of
/// 16 KiB and more; the fork's pool takes 1 MiB and more); a tree of small
/// files of one size, as `b3sum $(find src -type f)` hashes one; and a
/// tree of mixed sizes, a source checkout's range, one file after another
/// in a single run as `find . -type f -print0 | xargs -0 b3sum` runs it.
fn inputs(dir: &Path, quick: bool) -> Vec<Input> {
    let sizes: &[u64] = if quick { &[4 * KIB, MIB, 16 * MIB] } else { &[4 * KIB, 64 * KIB, MIB, 16 * MIB, 256 * MIB, GIB] };
    fs::create_dir_all(dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    let mut inputs: Vec<Input> = sizes
        .iter()
        .map(|&len| {
            let name = format!("file-{len}");
            ensure_file(dir, &name, len);
            Input { label: size_label(len), files: vec![dir.join(name)], bytes: len, each: false }
        })
        .collect();
    let uniform: &[(u64, u64)] = if quick { &[(100, 16 * KIB)] } else { &[(1000, 16 * KIB)] };
    let uniform_tree = tree(dir, &format!("tree-{}x{}", uniform[0].0, uniform[0].1), uniform, format!("{} x {}", uniform[0].0, size_label(uniform[0].1)));
    let each = Input { label: format!("{} x {}, a process each", uniform[0].0, size_label(uniform[0].1)), files: uniform_tree.files.clone(), bytes: uniform_tree.bytes, each: true };
    inputs.push(uniform_tree);
    inputs.push(each);
    let mixed: &[(u64, u64)] = if quick { MIXED_QUICK } else { MIXED };
    let count: u64 = mixed.iter().map(|&(n, _)| n).sum();
    let bytes: u64 = mixed.iter().map(|&(n, len)| n * len).sum();
    inputs.push(tree(dir, &format!("mixed-{count}"), mixed, format!("mixed tree, {count} files, {}", size_label_approx(bytes))));
    inputs
}

/// The mixed tree, (files, bytes each): most files small, most bytes in a
/// few large ones, as in a source checkout.
const MIXED: &[(u64, u64)] = &[(300, KIB), (300, 4 * KIB), (200, 16 * KIB), (120, 64 * KIB), (60, 256 * KIB), (15, MIB), (4, 4 * MIB), (1, 16 * MIB)];
const MIXED_QUICK: &[(u64, u64)] = &[(30, KIB), (30, 4 * KIB), (20, 16 * KIB), (12, 64 * KIB), (6, 256 * KIB), (2, MIB)];

/// A directory of files with the given (count, length) classes, named in
/// the order b3sum gets them: the sizes interleaved (file j takes the
/// class of position j x 389 mod count in the classes listed out, 389 and
/// the counts sharing no factor), as a directory listing mixes them.
fn tree(dir: &Path, name: &str, classes: &[(u64, u64)], label: String) -> Input {
    let lengths: Vec<u64> = classes.iter().flat_map(|&(n, len)| std::iter::repeat_n(len, n as usize)).collect();
    let count = lengths.len() as u64;
    assert_eq!(gcd(count, 389), 1, "a stride sharing no factor with the count takes every file once");
    fs::create_dir_all(dir.join(name)).unwrap();
    let mut files = Vec::new();
    for j in 0..count {
        let len = lengths[(j * 389 % count) as usize];
        let file = format!("{name}/{j:04}");
        ensure_file(dir, &file, len);
        files.push(dir.join(file));
    }
    Input { label, files, bytes: lengths.iter().sum(), each: false }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn size_label(len: u64) -> String {
    match len {
        l if l >= GIB && l % GIB == 0 => format!("{} GiB", l / GIB),
        l if l >= MIB && l % MIB == 0 => format!("{} MiB", l / MIB),
        l if l % KIB == 0 => format!("{} KiB", l / KIB),
        l => format!("{l} B"),
    }
}

/// The bytes of the file `name` (its path under the files directory, `/`
/// between its parts): BLAKE3's extended output of the name, so files are
/// alike on every machine and incompressible (a filesystem or a drive that
/// compresses would read zeros from storage almost for free).
fn contents(name: &str) -> blake3_servil::OutputReader {
    blake3_servil::Hasher::new().update(name.as_bytes()).finalize_xof()
}

/// Make `dir/name` hold `len` bytes of its contents, unless it holds them: a
/// file of the right length whose first 32 bytes match is kept (the rest
/// cannot change a hash's speed; regenerating a gigabyte each run would
/// only cost time). Written files are synced, so a later eviction finds
/// them clean.
fn ensure_file(dir: &Path, name: &str, len: u64) {
    let path = dir.join(name);
    let mut first = [0u8; 32];
    contents(name).fill(&mut first);
    let head_len = len.min(32) as usize;
    if let Ok(mut file) = File::open(&path) {
        let mut head = [0u8; 32];
        if file.metadata().map(|m| m.len()).ok() == Some(len) && file.read_exact(&mut head[..head_len]).is_ok() && head[..head_len] == first[..head_len] {
            return;
        }
    }
    let mut file = File::create(&path).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    let mut reader = contents(name);
    let mut buffer = vec![0u8; MIB as usize];
    let mut left = len;
    while left > 0 {
        let take = left.min(MIB) as usize;
        reader.fill(&mut buffer[..take]);
        file.write_all(&buffer[..take]).unwrap();
        left -= take as u64;
    }
    file.sync_all().unwrap();
}

// ---------- The page cache ----------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Cache {
    /// The files in the page cache: read moments before.
    Warm,
    /// The files out of the page cache: evicted before each run.
    Cold,
}

impl Cache {
    fn name(self) -> &'static str {
        match self {
            Cache::Warm => "warm",
            Cache::Cold => "cold",
        }
    }
    fn heading(self) -> &'static str {
        match self {
            Cache::Warm => "Files in the page cache (warm: read moments before)",
            Cache::Cold => "Files read from storage (cold: evicted from the page cache before each run)",
        }
    }
}

/// Read every file once, untimed, so the page cache holds it.
fn warm(files: &[PathBuf], buffer: &mut [u8]) {
    for path in files {
        let mut file = File::open(path).unwrap();
        while file.read(buffer).unwrap() > 0 {}
    }
}

/// Drop the files' pages from the page cache, without root, where the
/// operating system offers a way: Linux `posix_fadvise(DONTNEED)` (the
/// files are clean, synced when made); macOS `msync(MS_INVALIDATE)` over
/// a mapping of each file. Whether a run then read from storage is in its
/// counts, and the report checks it.
fn evict(files: &[PathBuf]) {
    for path in files {
        imp::evict(&File::open(path).unwrap());
    }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod imp {
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn posix_fadvise(fd: i32, offset: i64, len: i64, advice: i32) -> i32;
    }
    const POSIX_FADV_DONTNEED: i32 = 4;

    pub const EVICTS: bool = true;

    pub fn evict(file: &std::fs::File) {
        // Sound: a plain call on an open descriptor.
        let rc = unsafe { posix_fadvise(file.as_raw_fd(), 0, 0, POSIX_FADV_DONTNEED) };
        assert_eq!(rc, 0, "posix_fadvise(DONTNEED)");
    }

    /// The filesystem holding `dir`: the longest mount point above it in
    /// /proc/self/mounts.
    pub fn filesystem(dir: &std::path::Path) -> String {
        let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let mut best = (0, "unknown".to_owned());
        for line in mounts.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            if fields.len() > 2 && dir.starts_with(fields[1]) && fields[1].len() >= best.0 {
                best = (fields[1].len(), format!("{} ({} on {})", fields[2], fields[0], fields[1]));
            }
        }
        best.1
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
        fn msync(addr: *mut u8, len: usize, flags: i32) -> i32;
        fn munmap(addr: *mut u8, len: usize) -> i32;
        fn statfs(path: *const i8, buf: *mut u8) -> i32;
    }
    const PROT_READ: i32 = 1;
    const MAP_SHARED: i32 = 1;
    const MS_INVALIDATE: i32 = 2;

    pub const EVICTS: bool = true;

    pub fn evict(file: &std::fs::File) {
        let len = file.metadata().unwrap().len() as usize;
        if len == 0 {
            return;
        }
        // Sound: a fresh read-only shared mapping of an open file, unmapped below.
        unsafe {
            let at = mmap(std::ptr::null_mut(), len, PROT_READ, MAP_SHARED, file.as_raw_fd(), 0);
            assert!(at as isize != -1, "mmap for eviction");
            assert_eq!(msync(at, len, MS_INVALIDATE), 0, "msync(MS_INVALIDATE)");
            assert_eq!(munmap(at, len), 0, "munmap");
        }
    }

    /// The filesystem holding `dir`: statfs's f_fstypename and f_mntonname.
    pub fn filesystem(dir: &std::path::Path) -> String {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).unwrap();
        let mut buf = vec![0u8; 4096];
        // Sound: `buf` is writable and larger than a struct statfs.
        if unsafe { statfs(path.as_ptr(), buf.as_mut_ptr()) } != 0 {
            return "unknown".to_owned();
        }
        // struct statfs (64-bit inodes): f_fstypename at 72 (16 bytes), f_mntonname at 88.
        let text = |bytes: &[u8]| String::from_utf8_lossy(&bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]).into_owned();
        format!("{} (on {})", text(&buf[72..88]), text(&buf[88..88 + 1024]))
    }
}

#[cfg(not(unix))]
mod imp {
    pub const EVICTS: bool = false;
    pub fn evict(_: &std::fs::File) {
        unreachable!("no eviction here");
    }
    pub fn filesystem(_: &std::path::Path) -> String {
        "unknown".to_owned()
    }
}

/// Whether cold runs can be measured on this filesystem: the platform
/// evicts, and the filesystem keeps files in storage (tmpfs and ramfs keep
/// them in memory alone).
fn cold_possible(filesystem: &str) -> bool {
    imp::EVICTS && !filesystem.starts_with("tmpfs") && !filesystem.starts_with("ramfs")
}

// ---------- Contenders ----------

struct Contender {
    name: String,
    program: PathBuf,
    args: Vec<String>,
    /// The executable's BLAKE3 digest, which names the build exactly.
    digest: String,
    version: String,
}

fn contender(spec: &str) -> Contender {
    let (name, command) = spec.split_once('=').unwrap_or_else(|| panic!("a contender is NAME=COMMAND, found {spec:?}"));
    assert!(!name.is_empty() && !name.contains(['\t', ' ', '|']), "a contender's name has no spaces, tabs, or bars: {name:?}");
    let mut words = command.split_whitespace().map(str::to_owned);
    let program = PathBuf::from(words.next().unwrap_or_else(|| panic!("contender {name} names no program")));
    let program = if program.components().count() > 1 { std::fs::canonicalize(&program).unwrap_or_else(|e| panic!("{}: {e}", program.display())) } else { program };
    let bytes = std::fs::read(&program).or_else(|_| which(&program).map(std::fs::read).expect("the program is in PATH")).unwrap_or_else(|e| panic!("cannot read {}: {e}", program.display()));
    let output = Command::new(&program).arg("--version").output().unwrap_or_else(|e| panic!("cannot run {}: {e}", program.display()));
    Contender {
        name: name.to_owned(),
        program,
        args: words.collect(),
        digest: blake3_servil::hash(&bytes).to_hex().to_string(),
        version: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    }
}

/// `program` found in PATH, for a bare name.
fn which(program: &Path) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(program)).find(|path| path.is_file())
}

// ---------- Measuring ----------

/// One run's counts beyond its wall time, as the samples file's comment
/// lines keep them.
fn counts_line(name: &str, cache: Cache, input: &str, round: u64, run: &clocks::child::Run) -> String {
    let opt = |v: Option<u64>| v.map_or("-".to_owned(), |v| v.to_string());
    let c = run.counts;
    let level = |f: fn(&clocks::Counts) -> u64| opt(c.as_ref().map(f));
    format!(
        "# counts\t{name}\t{}\t{input}\t{round}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        cache.name(), run.wall_ns, opt(run.cpu_ns), opt(run.max_rss_bytes), opt(run.storage_read_bytes), opt(run.major_faults),
        level(|c| c.p.cycles), level(|c| c.e.cycles), level(|c| c.p.time_ns), level(|c| c.e.time_ns),
    )
}

pub fn command(args: &[String]) {
    let mut quick = false;
    let mut rounds = None;
    let mut files_dir = PathBuf::from("b3sum-files");
    let mut specs = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--quick" => quick = true,
            "--rounds" => rounds = Some(it.next().expect("--rounds N").parse::<u64>().expect("--rounds takes a whole number")),
            "--files" => files_dir = PathBuf::from(it.next().expect("--files DIR")),
            flag if flag.starts_with("--") => panic!("unknown option {flag}; see README, \"b3sum\""),
            spec => specs.push(contender(spec)),
        }
    }
    assert!(specs.len() >= 2, "name two contenders or more, NAME=COMMAND; the first is the one the others are compared with");
    let mut names: Vec<&str> = specs.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), specs.len(), "each contender has a name of its own");
    let rounds = rounds.unwrap_or(if quick { 5 } else { 15 });
    assert!(rounds >= 2, "two rounds at least");

    eprintln!("bench-hashes b3sum: making the input files in {} (kept for later runs)", files_dir.display());
    let inputs = inputs(&files_dir, quick);
    let files_dir = fs::canonicalize(&files_dir).unwrap();
    let filesystem = imp::filesystem(&files_dir);
    let caches: Vec<Cache> = if cold_possible(&filesystem) { vec![Cache::Warm, Cache::Cold] } else { vec![Cache::Warm] };
    let mut machine = machine_metadata();

    let mut buffer = vec![0u8; MIB as usize];
    let run_files = |c: &Contender, input: &Input, files: &[PathBuf]| -> clocks::child::Run {
        let mut command = Command::new(&c.program);
        command.args(&c.args).args(files).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        let run = clocks::child::run(&mut command);
        if !run.success {
            // Show what it said, then stop: a failing contender measures nothing.
            let _ = Command::new(&c.program).args(&c.args).args(files).stdout(Stdio::null()).status();
            panic!("contender {} failed on {}", c.name, input.label);
        }
        run
    };
    let run_once = |c: &Contender, input: &Input| -> clocks::child::Run {
        if !input.each {
            return run_files(c, input, &input.files);
        }
        // A process per file: the runs' sums (their peak memory, the largest).
        let runs: Vec<_> = input.files.iter().map(|f| run_files(c, input, std::slice::from_ref(f))).collect();
        let sum = |f: fn(&clocks::child::Run) -> Option<u64>| runs.iter().map(f).sum::<Option<u64>>();
        clocks::child::Run {
            started_ns: runs[0].started_ns,
            wall_ns: runs.iter().map(|r| r.wall_ns).sum(),
            success: true,
            cpu_ns: sum(|r| r.cpu_ns),
            max_rss_bytes: runs.iter().map(|r| r.max_rss_bytes).max().flatten(),
            storage_read_bytes: sum(|r| r.storage_read_bytes),
            major_faults: sum(|r| r.major_faults),
            counts: runs.iter().map(|r| r.counts).reduce(|a, b| a.zip(b).map(|(a, b)| a.plus(b))).flatten(),
        }
    };
    // Every contender once on every input, untimed, so the executables and
    // the files start in the page cache.
    for input in &inputs {
        warm(&input.files, &mut buffer);
        for c in &specs {
            run_once(c, input);
        }
    }
    // cells[cache][input][contender]: each round's run.
    let mut cells: Vec<Vec<Vec<Vec<clocks::child::Run>>>> = caches.iter().map(|_| inputs.iter().map(|_| specs.iter().map(|_| Vec::new()).collect()).collect()).collect();
    let started = clocks::now();
    for round in 0..rounds {
        for (ci, &cache) in caches.iter().enumerate() {
            for (i, input) in inputs.iter().enumerate() {
                if cache == Cache::Warm {
                    warm(&input.files, &mut buffer);
                }
                // The order rotates with the round and the input, so drift
                // and each run's after-effects fall on every contender.
                for k in 0..specs.len() {
                    let c = (k + round as usize + i) % specs.len();
                    if cache == Cache::Cold {
                        evict(&input.files);
                    }
                    cells[ci][i][c].push(run_once(&specs[c], input));
                }
            }
        }
        eprintln!("bench-hashes b3sum: round {} of {rounds} done, {} s", round + 1, clocks::since_ns(started) / 1_000_000_000);
    }
    machine.load = clocks::load::windows();
    machine.power[1] = super::Power::read();

    let directory = output_directory(&machine);
    fs::create_dir_all(&directory).unwrap();
    let mut tsv = format!("{SAMPLES_VERSION}\n# timestamp: {}\n# bench-hashes version: {}\n# git commit: {}\n# cpu type: {}\n# cpu count: {}\n# os type: {}\n# files: {} on {filesystem}\n# rounds: {rounds}\n",
        machine.timestamp, super::BENCH_VERSION, super::GIT_COMMIT, machine.cpu_type, machine.cpu_count, machine.os_type, files_dir.display());
    for c in &specs {
        tsv += &format!("# contender {}: {} {} (blake3 {}; {})\n", c.name, c.program.display(), c.args.join(" "), c.digest, c.version);
    }
    tsv += &format!("# power: {}\n# load: {}\n", machine.describe_power(), clocks::load::describe(&machine.load));
    tsv += "# counts\tcontender\tcache\tinput\tround\twall_ns\tcpu_ns\tmax_rss_bytes\tstorage_read_bytes\tmajor_faults\tp_cycles\te_cycles\tp_time_ns\te_time_ns\n";
    for (ci, &cache) in caches.iter().enumerate() {
        for (i, input) in inputs.iter().enumerate() {
            for (c, spec) in specs.iter().enumerate() {
                for (round, run) in cells[ci][i][c].iter().enumerate() {
                    tsv += &counts_line(&spec.name, cache, &input.label, round as u64, run);
                    tsv += "\n";
                }
            }
        }
    }
    tsv += SAMPLES_COLUMNS;
    tsv += "\n";
    for (ci, &cache) in caches.iter().enumerate() {
        for (i, input) in inputs.iter().enumerate() {
            for (c, spec) in specs.iter().enumerate() {
                let runs = &cells[ci][i][c];
                let samples: Vec<String> = runs.iter().map(|r| format!("{}/{}", r.wall_ns, input.bytes)).collect();
                let starts: Vec<String> = runs.iter().map(|r| (r.started_ns / 1_000_000).to_string()).collect();
                tsv += &format!("{}\t{}\tb3sum\t{}\tB\t{}\t{}\n", spec.name, cache.name(), input.label, samples.join(","), starts.join(","));
            }
        }
    }
    let samples_path = directory.join("b3sum.samples.tsv");
    fs::write(&samples_path, tsv).unwrap();
    super::map::write(&directory);
    let report = report(&specs, &inputs, &caches, &cells, &machine, &filesystem, quick, rounds);
    fs::write(directory.join("b3sum.result.txt"), &report).unwrap();
    print!("{report}");
    eprintln!("bench-hashes b3sum: report, samples, and map in {}", directory.display());
}

// ---------- The report ----------

/// A run's time per input, its mean (one sample a run), as a per-unit value.
fn per_run(run: &clocks::child::Run, bytes: u64) -> u128 {
    clocks::summary::mean([(run.wall_ns, bytes)])
}

/// `value` over `scale` with three significant digits.
fn three(value: u128, scale: u128) -> String {
    let mut decimals = 0u32;
    while decimals < 6 && value * 10u128.pow(decimals) < 100 * scale {
        decimals += 1;
    }
    let scaled = (value * 10u128.pow(decimals) + scale / 2) / scale;
    let p = 10u128.pow(decimals);
    if decimals == 0 { scaled.to_string() } else { format!("{}.{:0w$}", scaled / p, scaled % p, w = decimals as usize) }
}

/// Bytes over nanoseconds: gigabytes per second, three significant digits.
fn rate(bytes: u64, ns: u128) -> String {
    three(u128::from(bytes), ns.max(1))
}

/// Nanoseconds for people: "812 us", "1.23 ms", "2.05 s" (three significant digits).
pub(crate) fn time(ns: u128) -> String {
    match ns {
        n if n < 1_000 => format!("{n} ns"),
        n if n < 1_000_000 => format!("{} us", three(n, 1_000)),
        n if n < 1_000_000_000 => format!("{} ms", three(n, 1_000_000)),
        n => format!("{} s", three(n, 1_000_000_000)),
    }
}

#[allow(clippy::too_many_arguments)]
fn report(specs: &[Contender], inputs: &[Input], caches: &[Cache], cells: &[Vec<Vec<Vec<clocks::child::Run>>>], machine: &super::MachineMetadata, filesystem: &str, quick: bool, rounds: u64) -> String {
    let mut out = String::new();
    out += "bench-hashes b3sum: how long b3sum takes to hash files, from its start to its exit\n\n";
    out += &format!("machine:   {}, {} CPUs, {}\nfiles on:  {filesystem}\n", machine.cpu_type, machine.cpu_count, machine.os_type);
    out += &format!("load:      {}\n", clocks::load::describe(&machine.load));
    out += &format!("runs:      {rounds} rounds, each contender once on each input in each{}\n", if quick { " (--quick: a check, not a record)" } else { "" });
    if !caches.contains(&Cache::Cold) {
        out += "cold:      not measured: this platform or filesystem cannot evict one file from the page cache\n";
    }
    out += "\ncontenders:\n";
    for c in specs {
        out += &format!("  {:<14} {} {}   ({}, blake3 {})\n", c.name, c.program.display(), c.args.join(" "), c.version, &c.digest[..16]);
    }
    let base = &specs[0].name;
    for (ci, &cache) in caches.iter().enumerate() {
        out += &format!("\n{}\nmean time per run, lower is better; xN: against {base}, the median of the rounds' ratios, marked slower or faster where the rounds agree (3%)\n", cache.heading());
        let mut rows: Vec<Vec<String>> = vec![std::iter::once("input".to_owned()).chain(specs.iter().map(|c| c.name.clone())).collect()];
        for (i, input) in inputs.iter().enumerate() {
            let mut row = vec![input.label.clone()];
            for (c, _) in specs.iter().enumerate() {
                let runs = &cells[ci][i][c];
                let mean = ExactMean::of(&runs.iter().map(|r| Measured::new(r.wall_ns, input.bytes)).collect::<Vec<_>>()).fixed();
                let ns = (mean.0 * u128::from(input.bytes) + (1 << 63)) >> 64;
                let mut text = time(ns);
                if input.bytes >= MIB {
                    text += &format!(" {} GB/s", rate(input.bytes, ns));
                }
                if c > 0 {
                    let ratios: Vec<u64> = runs.iter().zip(&cells[ci][i][0]).map(|(r, b)| clocks::summary::ratio_permille(per_run(r, input.bytes), per_run(b, input.bytes))).collect();
                    let median = clocks::summary::median_permille(&ratios);
                    let mark = match clocks::summary::verdict(&ratios, MARGIN_PERMILLE) {
                        clocks::summary::Verdict::Slower => " slower",
                        clocks::summary::Verdict::Faster => " faster",
                        clocks::summary::Verdict::Level => "",
                    };
                    text += &format!(" x{}.{:03}{mark}", median / 1000, median % 1000);
                }
                row.push(text);
            }
            rows.push(row);
        }
        out += &table(&rows);
        out += "\nCPUs busy (CPU time over wall time) and peak memory, medians\n";
        let mut rows: Vec<Vec<String>> = vec![std::iter::once("input".to_owned()).chain(specs.iter().map(|c| c.name.clone())).collect()];
        for (i, input) in inputs.iter().enumerate() {
            let mut row = vec![input.label.clone()];
            for (c, _) in specs.iter().enumerate() {
                let runs = &cells[ci][i][c];
                let busy = median_u64(runs.iter().filter_map(|r| r.cpu_ns.map(|cpu| (cpu * 100 + r.wall_ns / 2) / r.wall_ns)).collect());
                let rss = median_u64(runs.iter().filter_map(|r| r.max_rss_bytes).collect());
                row.push(match (busy, rss) {
                    (Some(b), Some(m)) => format!("{}.{:02} CPUs, {} MiB", b / 100, b % 100, (m + MIB / 2) / MIB),
                    _ => "not counted here".to_owned(),
                });
            }
            rows.push(row);
        }
        out += &table(&rows);
        if cache == Cache::Cold {
            // From the counts: each cold cell's reads from storage.
            let mut warm_cells = Vec::new();
            for (i, input) in inputs.iter().enumerate() {
                for (c, spec) in specs.iter().enumerate() {
                    if let Some(read) = median_u64(cells[ci][i][c].iter().filter_map(|r| r.storage_read_bytes).collect()) {
                        if read * 2 < input.bytes {
                            warm_cells.push(format!("{} on {} read {} of {} from storage", spec.name, input.label, size_label_approx(read), size_label(input.bytes)));
                        }
                    }
                }
            }
            out += &if warm_cells.is_empty() {
                "\nEvery cold run read its files from storage (by the counts).\n".to_owned()
            } else {
                format!("\nNot cold: the page cache still served these runs (median reads): {}.\n", warm_cells.join("; "))
            };
        }
    }
    out
}

fn median_u64(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]).div_ceil(2) })
}

fn size_label_approx(bytes: u64) -> String {
    if bytes >= MIB { format!("{} MiB", (bytes + MIB / 2) / MIB) } else { format!("{} KiB", (bytes + KIB / 2) / KIB) }
}

/// Columns padded to their widest cell.
fn table(rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..rows[0].len()).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap()).collect();
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row.iter().zip(&widths).map(|(cell, &w)| format!("{cell}{}", " ".repeat(w - cell.chars().count()))).collect();
        out += cells.join("   ").trim_end();
        out += "\n";
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_read_three_digits() {
        assert_eq!(time(812), "812 ns");
        assert_eq!(time(1_234_567), "1.23 ms");
        assert_eq!(time(45_600), "45.6 us");
        assert_eq!(time(2_050_000_000), "2.05 s");
        assert_eq!(rate(GIB, 100_000_000), "10.7");
    }

    /// A file's bytes are BLAKE3's extended output of its name: the empty
    /// name's first 32 bytes are BLAKE3's published digest of the empty
    /// input (the official test vectors).
    #[test]
    fn the_contents_are_blake3_output_of_the_name() {
        let mut first = [0u8; 32];
        contents("").fill(&mut first);
        assert_eq!(blake3_servil::Hash::from(first).to_hex().as_str(), "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
    }

    /// The mixed tree's interleaving takes every file once.
    #[test]
    fn the_mixed_trees_order_is_a_permutation() {
        for classes in [MIXED, MIXED_QUICK] {
            let count: u64 = classes.iter().map(|&(n, _)| n).sum();
            let mut seen: Vec<u64> = (0..count).map(|j| j * 389 % count).collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..count).collect::<Vec<_>>());
        }
    }
}
