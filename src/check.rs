//! `bench-hashes check [--against REV] [--cells all] [--pairs N] [--keep DIR]`:
//! did a change to the fork hurt its speed anywhere? Run from a checkout of
//! bench-hashes inside a checkout of the fork (the fork at `..`), it
//! compares the fork's working tree with a commit (`--against`, HEAD by
//! default) and prints the cells judged slower or faster, with their
//! ratios; exit 0 when no cell is slower, 1 when one is, 2 when a run's
//! load left no verdict.
//!
//! What a check does, every time, with nothing to choose:
//! 1. Builds bench-hashes twice, against the fork at each side (a commit
//!    of the working tree as `git add -A` would stage it, and the commit
//!    checked against), in worktrees of its own under `target/check/`, and
//!    relinks each side once per pair with its functions in a fresh random
//!    order: a binary's layout alone moves cells several percent (x86: 8-10%),
//!    the same for every run of that binary, so no sampling of one binary
//!    sees it; two sides with different layouts each pair do, and their
//!    seeds stay apart so identical code still meets a new layout in every run.
//! 2. Chooses the cells: those whose code the change touches. A coverage
//!    build of the new side runs each point once and records the fork's
//!    lines each point executes (cached under `target/check/` by the
//!    bench's source and the fork's commit); the change's diff names the
//!    lines it touches; the points that execute one are checked, the rest
//!    skipped, since nothing they run changed. A change to assembly, the
//!    build script or a manifest checks every point (coverage cannot see
//!    into assembly). `--cells all` checks every point regardless.
//! 3. Runs `bench-hashes regress` over the pairs: servil st and servil mt
//!    (the code that changes) at the regression check's visits
//!    (TO_CHECK_FOR_REGRESSIONS), judged all at once (clocks::summary).
//!
//! It replaces the fork's `tools/perf_regress.py` (October 19, 2026,
//! Zooko: one tool that does everything a change needs, at no cost to the
//! one who made it) and `REGRESS_POINTS`, a fixed list of five cells that
//! was blind to the cells after a gap and to the stream, where the fork
//! had regressed before.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pairs of runs a check takes (its layouts a side). Measured October 20,
/// 2026 (i7-12700K; the data branch, 2026-10-20/check-validation), 4 pairs
/// against 8: a hash() plant 6% a call caught in 48 cells against 60; a
/// plant in the pool's split path in 22 of its 28 cells against 28; the
/// A/A over the same 146 cells no cell slower either way; a check that
/// chooses 73 points 6 minutes against 14. Every plant is still found;
/// `--pairs 8` sees more of its cells.
const PAIRS: usize = 4;
/// The two sides' contenders: the code a change to the fork changes.
const SUBJECTS: &str = "blake3-servil-st,blake3-servil-mt";

pub fn command(arguments: &[String]) -> i32 {
    let usage = "usage: bench-hashes check [--against REV] [--cells all] [--pairs N] [--keep DIR]";
    let (mut against, mut all, mut pairs, mut keep) =
        ("HEAD".to_string(), false, PAIRS, None::<PathBuf>);
    let mut it = arguments.iter();
    while let Some(name) = it.next() {
        match name.as_str() {
            "--against" => {
                against = it
                    .next()
                    .unwrap_or_else(|| panic!("--against needs a revision\n{usage}"))
                    .clone()
            }
            "--cells" => {
                let value = it
                    .next()
                    .unwrap_or_else(|| panic!("--cells takes `all`\n{usage}"));
                assert_eq!(value, "all", "--cells takes `all`\n{usage}");
                all = true;
            }
            "--pairs" => {
                pairs = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n >= 3)
                    .unwrap_or_else(|| panic!("--pairs takes a count of 3 or more\n{usage}"))
            }
            "--keep" => {
                keep =
                    Some(PathBuf::from(it.next().unwrap_or_else(|| {
                        panic!("--keep needs a directory\n{usage}")
                    })))
            }
            other => panic!("unknown option {other}\n{usage}"),
        }
    }
    let bench = std::env::current_dir().expect("the current directory");
    let fork = fork_root(&bench);
    let old = git(&fork, &["rev-parse", &format!("{against}^{{commit}}")]);
    let new = working_tree_commit(&fork);
    eprintln!(
        "check: the fork's working tree ({}) against {} ({})",
        &new[..12],
        against,
        &old[..12]
    );
    let cache = fork.join("target").join("check");
    std::fs::create_dir_all(&cache).expect("the check's cache directory");
    let old_exe = side(&fork, &bench, &cache, "old", &old);
    let new_exe = side(&fork, &bench, &cache, "new", &new);
    // The cells a change touches, from the new side's coverage.
    let points = if all {
        None
    } else {
        Some(touched_points(&fork, &bench, &cache, &old, &new, &new_exe))
    };
    if let Some(points) = &points {
        if points.is_empty() {
            println!("check: the change touches no line a cell executes; nothing to measure");
            return 0;
        }
        eprintln!(
            "check: {} points execute a changed line: {}",
            points.len(),
            points.join(", ")
        );
    }
    let olds = layouts(&cache, "old", &old_exe, 0..pairs);
    let news = layouts(&cache, "new", &new_exe, pairs..2 * pairs);
    let mut regress: Vec<String> = vec![
        "regress".into(),
        "--pairs".into(),
        pairs.to_string(),
        "--contenders".into(),
        SUBJECTS.into(),
    ];
    if let Some(points) = &points {
        regress.extend(["--points".to_string(), points.join(",")]);
    }
    if let Some(keep) = &keep {
        regress.extend(["--keep".to_string(), keep.display().to_string()]);
    }
    regress.extend(olds);
    regress.push("--".into());
    regress.extend(news);
    let status = Command::new(&new_exe)
        .args(&regress)
        .current_dir(&bench)
        .status()
        .expect("regress runs");
    status.code().unwrap_or(2)
}

/// The fork checkout this bench sits in: `..` from the bench, a git
/// repository holding `Cargo.toml` with `name = "blake3-servil"`.
fn fork_root(bench: &Path) -> PathBuf {
    let fork = bench
        .parent()
        .expect("the bench sits inside a checkout of the fork")
        .to_path_buf();
    let manifest = std::fs::read_to_string(fork.join("Cargo.toml")).unwrap_or_else(|e| panic!("check: {} holds no Cargo.toml: {e}; run from a bench-hashes checkout inside the fork's", fork.display()));
    assert!(
        manifest.contains("name = \"blake3-servil\""),
        "check: {} is not the fork",
        fork.display()
    );
    fork
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {} in {} failed: {}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_string()
}

/// A commit of the working tree as `git add -A` would stage it, HEAD its
/// parent, through a copy of the index and with fixed dates, so one tree
/// always gives one commit and nothing of the user's changes.
fn working_tree_commit(fork: &Path) -> String {
    let real = git(fork, &["rev-parse", "--git-path", "index"]);
    let real = if Path::new(&real).is_absolute() {
        PathBuf::from(real)
    } else {
        fork.join(real)
    };
    let copy = fork.join("target").join("check-index");
    std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
    std::fs::copy(&real, &copy).expect("the index copies");
    let with = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(fork)
            .env("GIT_INDEX_FILE", &copy)
            .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    with(&["add", "-A"]);
    let tree = with(&["write-tree"]);
    with(&[
        "commit-tree",
        &tree,
        "-p",
        "HEAD",
        "-m",
        "check: the working tree",
    ])
}

/// One side's bench-hashes executable, built against the fork at `commit`:
/// a worktree of the fork under the cache (moved only when the commit
/// changes), this bench's sources copied beside it, Cargo pointed at the
/// worktree for the fork and its clocks crate, and a target directory of
/// the side's own, so an unchanged side rebuilds nothing.
fn side(fork: &Path, bench: &Path, cache: &Path, name: &str, commit: &str) -> PathBuf {
    let checkout = cache.join(name).join("src");
    if !checkout.exists() {
        git(fork, &["worktree", "prune"]);
        std::fs::create_dir_all(checkout.parent().unwrap()).unwrap();
        git(
            fork,
            &[
                "worktree",
                "add",
                "--detach",
                checkout.to_str().unwrap(),
                commit,
            ],
        );
    } else if git(&checkout, &["rev-parse", "HEAD"]) != commit {
        git(
            &checkout,
            &["checkout", "--quiet", "--force", "--detach", commit],
        );
    }
    build_against(&checkout, bench, &cache.join(name).join("target"), name)
}

/// `bench-hashes build-against DIR`: this bench built against the fork
/// checkout at DIR as it is, into DIR's `target/build-against/`; the
/// executable's path on stdout (the Mac runner's benchmark jobs, and a
/// maintainer measuring the working tree, run it from there).
pub fn build_against_command(arguments: &[String]) -> i32 {
    let usage = "usage: bench-hashes build-against DIR    (DIR: a checkout of the fork)";
    let [dir] = arguments else { panic!("{usage}") };
    let checkout = std::fs::canonicalize(dir).unwrap_or_else(|e| panic!("{dir}: {e}\n{usage}"));
    assert!(
        checkout.join("Cargo.toml").exists() && checkout.join("c").is_dir(),
        "{dir} is no checkout of the fork\n{usage}"
    );
    let bench = std::env::current_dir().expect("the current directory");
    // Where Cargo would build (CARGO_TARGET_DIR where set: the VM builds on
    // a tmpfs, since its shared mount runs no executable), else the
    // checkout's target/.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| checkout.join("target"));
    let exe = build_against(
        &checkout,
        &bench,
        &target.join("build-against"),
        "build-against",
    );
    println!("{}", exe.display());
    0
}

/// This bench, built against the fork checkout at `checkout` as it is:
/// its sources copied beside the fork (`checkout/bench-hashes`), Cargo
/// pointed at the checkout for the fork and its clocks crate, a lock and a
/// target directory of the build's own (the committed lock is never
/// written); the executable's path. `check`'s two sides and the Mac
/// runner's benchmark jobs build this way, and so does a maintainer
/// measuring the working tree (`bench-hashes build-against ..`).
pub fn build_against(checkout: &Path, bench: &Path, target: &Path, name: &str) -> PathBuf {
    let commit = git(checkout, &["rev-parse", "HEAD"]);
    let copy = checkout.join("bench-hashes");
    copy_bench(bench, &copy);
    let target = target.to_path_buf();
    let [patch_fork, patch_clocks] = patches(&copy, &checkout);
    // The side's lock: the bench's, with the fork taken from the worktree.
    // Cargo patches at the locked version only, so a version change wants
    // `cargo update -p blake3-servil`.
    let out = Command::new("cargo")
        .args([
            "--config",
            &patch_fork,
            "--config",
            &patch_clocks,
            "update",
            "--quiet",
            "-p",
            "blake3-servil",
            "-p",
            "clocks",
        ])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("BENCH_HASHES_CHECKOUT", bench)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "check: {name}: cargo update: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("cargo")
        .args([
            "--config",
            &patch_fork,
            "--config",
            &patch_clocks,
            "build",
            "--release",
            "--quiet",
        ])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("BENCH_HASHES_CHECKOUT", bench)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "check: {name}: building bench-hashes against the fork at {}:\n{}",
        &commit[..12],
        String::from_utf8_lossy(&out.stderr)
    );
    // The fork must come from the side's checkout, never the commit the
    // bench's lock pins: Cargo names each package's source as it builds.
    let lock = std::fs::read_to_string(copy.join("Cargo.lock")).expect("the side's lock");
    let entry = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"blake3-servil\""))
        .expect("the lock names blake3-servil");
    assert!(
        !entry.contains("source = "),
        "check: {name}: the bench built blake3-servil from its pinned source, not the checkout {}:\n{entry}",
        checkout.display()
    );
    target.join("release").join("bench-hashes")
}

/// This bench's sources, copied file by file where they differ (so Cargo
/// sees only real changes); `target`, results and the lock stay the copy's.
fn copy_bench(from: &Path, to: &Path) {
    let skip = |p: &Path| {
        p.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some("target" | "benchmark-results" | ".git" | "tmp")
            )
        })
    };
    let mut wanted = BTreeSet::new();
    for entry in walk(from) {
        let relative = entry.strip_prefix(from).unwrap().to_path_buf();
        if skip(&relative) || relative == Path::new("Cargo.lock") {
            continue;
        }
        wanted.insert(relative.clone());
        let dest = to.join(&relative);
        let bytes = std::fs::read(&entry).unwrap();
        if std::fs::read(&dest).ok().as_deref() != Some(&bytes[..]) {
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(&dest, bytes).unwrap();
        }
    }
    if !to.join("Cargo.lock").exists() {
        std::fs::copy(from.join("Cargo.lock"), to.join("Cargo.lock")).unwrap();
    }
    for entry in walk(to) {
        let relative = entry.strip_prefix(to).unwrap().to_path_buf();
        if !skip(&relative) && relative != Path::new("Cargo.lock") && !wanted.contains(&relative) {
            std::fs::remove_file(&entry).unwrap();
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| {
                    matches!(
                        n.to_str(),
                        Some(".git" | "target" | "tmp" | "benchmark-results")
                    )
                }) {
                    continue;
                }
                files.extend(walk(&path));
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// `exe` relinked once per seed, its functions in a fresh random order
/// each (a linker order file: lld on Linux, ld's -order_file on macOS).
/// The bench crate alone relinks; its dependencies stay built.
fn layouts(cache: &Path, name: &str, exe: &Path, seeds: std::ops::Range<usize>) -> Vec<String> {
    let nm = Command::new("nm").arg(exe).output().expect("nm runs");
    let text = String::from_utf8_lossy(&nm.stdout).into_owned();
    let mut symbols: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() == 3 && matches!(f[1], "t" | "T")).then(|| f[2])
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let side_dir = cache.join(name);
    let copy = side_dir.join("src").join("bench-hashes");
    let checkout = side_dir.join("src");
    let [patch_fork, patch_clocks] = patches(&copy, &checkout);
    let mut exes = Vec::new();
    for seed in seeds {
        shuffle(&mut symbols, seed as u64);
        let order = side_dir.join(format!("order-{seed}.txt"));
        std::fs::write(&order, symbols.join("\n") + "\n").unwrap();
        let flags: Vec<String> = if cfg!(target_os = "macos") {
            vec![format!("-Clink-arg=-Wl,-order_file,{}", order.display())]
        } else {
            let lld = lld_dir(cache);
            vec![
                "-Clink-arg=-fuse-ld=lld".into(),
                format!("-Clink-arg=-B{}", lld.display()),
                format!("-Clink-arg=-Wl,--symbol-ordering-file={}", order.display()),
                "-Clink-arg=-Wl,--no-warn-symbol-ordering".into(),
            ]
        };
        let out = Command::new("cargo")
            .args([
                "--config",
                &patch_fork,
                "--config",
                &patch_clocks,
                "rustc",
                "--release",
                "--quiet",
                "--bin",
                "bench-hashes",
                "--",
            ])
            .args(&flags)
            .current_dir(&copy)
            .env("CARGO_TARGET_DIR", side_dir.join("target"))
            .output()
            .expect("cargo runs");
        assert!(
            out.status.success(),
            "check: relinking {name} with seed {seed}:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let kept = side_dir.join(format!("bench-hashes-{seed}"));
        std::fs::copy(
            side_dir.join("target").join("release").join("bench-hashes"),
            &kept,
        )
        .unwrap();
        exes.push(kept.display().to_string());
    }
    exes
}

/// A directory holding `ld.lld`, for the C compiler's -B (Linux): the one
/// on PATH, else the newest versioned one.
fn lld_dir(cache: &Path) -> PathBuf {
    let link = cache.join("lld").join("ld.lld");
    if link.exists() {
        return link.parent().unwrap().to_path_buf();
    }
    let found = std::env::var_os("PATH")
        .and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join("ld.lld"))
                .find(|p| p.exists())
        })
        .or_else(|| {
            let mut v: Vec<PathBuf> = std::fs::read_dir("/usr/bin")
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("ld.lld-"))
                })
                .collect();
            v.sort();
            v.pop()
        })
        .expect("check: relinking needs ld.lld (apt install lld)");
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(found, &link).unwrap();
    link.parent().unwrap().to_path_buf()
}

/// Fisher-Yates with a small deterministic generator: the same seed gives
/// the same order on every machine.
fn shuffle(items: &mut [&str], seed: u64) {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..items.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

/// The points whose cells execute a line the change touches: a coverage
/// build of the new side runs each point once (servil st and mt) and the
/// lines each executes in the fork's `src/` are kept, cached by the bench's
/// source and the fork's commit; the change's diff (old..new, `src/`)
/// gives the touched lines. Assembly, the build script or a manifest
/// changed: every point.
fn touched_points(
    fork: &Path,
    bench: &Path,
    cache: &Path,
    old: &str,
    new: &str,
    new_exe: &Path,
) -> Vec<String> {
    let changed_files = git(fork, &["diff", "--name-only", &format!("{old}..{new}")]);
    let every = || all_points(new_exe, bench);
    if changed_files.lines().any(|f| {
        f.starts_with("c/")
            || f == "build.rs"
            || f.ends_with("Cargo.toml")
            || f.ends_with(".S")
            || f.ends_with(".asm")
    }) {
        eprintln!("check: assembly, the build script or a manifest changed: every point");
        return every();
    }
    let mut touched: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    let mut current = None;
    for line in git(
        fork,
        &[
            "diff",
            "--unified=0",
            &format!("{old}..{new}"),
            "--",
            "src",
            "capi/src",
        ],
    )
    .lines()
    {
        if let Some(f) = line.strip_prefix("+++ b/") {
            current = Some(f.to_string());
        } else if line.starts_with("@@") {
            if let Some(f) = &current {
                // "@@ -a,b +c,d @@ context": the new side's range is c,d.
                let range = line
                    .split_whitespace()
                    .nth(2)
                    .unwrap()
                    .trim_start_matches('+');
                let (start, count) = range
                    .split_once(',')
                    .map(|(s, c)| (s.parse::<usize>().unwrap(), c.parse::<usize>().unwrap()))
                    .unwrap_or_else(|| (range.parse().unwrap(), 1));
                touched
                    .entry(f.clone())
                    .or_default()
                    .extend(start..start + count);
            }
        }
    }
    if touched.is_empty() {
        return Vec::new();
    }
    let reached = coverage(bench, cache, new, new_exe);
    reached
        .into_iter()
        .filter(|(_, files)| {
            files
                .iter()
                .any(|(f, lines)| touched.get(f).is_some_and(|t| !t.is_disjoint(lines)))
        })
        .map(|(label, _)| label)
        .collect()
}

/// Every point's name as `--points` takes it, from a one-round run.
fn all_points(exe: &Path, bench: &Path) -> Vec<String> {
    let dir = bench.join("target").join("check").join("points");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(exe)
        .args(["--contenders", SUBJECTS, "--rounds", "1"])
        .current_dir(&dir)
        .output()
        .expect("the bench runs");
    assert!(
        out.status.success(),
        "check: a one-round run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let results = std::fs::read_dir(dir.join("benchmark-results"))
        .expect("the run's results")
        .flatten()
        .next()
        .expect("one machine's results")
        .path();
    let samples = results.join("bench-hashes.samples.tsv");
    let text = std::fs::read_to_string(samples).unwrap();
    let mut labels = BTreeSet::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("contender"))
    {
        let f: Vec<&str> = line.split('\t').collect();
        if let Some(use_case) = super::UseCase::ALL
            .into_iter()
            .find(|u| format!("{u:?}") == f[2])
        {
            labels.insert(format!("{}{}", use_case.label_prefix(), f[3]));
        }
    }
    labels.into_iter().collect()
}

/// Each point's executed lines of the fork's `src/`, from a coverage build
/// of the new side run one point at a time; cached as JSON-free TSV under
/// the cache, keyed by the bench's source and the fork's commit.
fn coverage(
    bench: &Path,
    cache: &Path,
    new: &str,
    new_exe: &Path,
) -> BTreeMap<String, BTreeMap<String, BTreeSet<usize>>> {
    let bench_source = {
        let mut h = blake3_servil::Hasher::new();
        for f in walk(&bench.join("src")) {
            h.update(&std::fs::read(&f).unwrap());
        }
        h.finalize().to_hex().to_string()
    };
    let cached = cache
        .join("coverage")
        .join(format!("{}-{}.tsv", &new[..12], &bench_source[..12]));
    if let Ok(text) = std::fs::read_to_string(&cached) {
        let mut map: BTreeMap<String, BTreeMap<String, BTreeSet<usize>>> = BTreeMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            map.entry(f[0].to_string()).or_default().insert(
                f[1].to_string(),
                f[2].split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().unwrap())
                    .collect(),
            );
        }
        return map;
    }
    eprintln!("check: recording which lines each point executes (once per change to the bench or the fork)");
    let checkout = cache.join("new").join("src");
    let copy = checkout.join("bench-hashes");
    let target = cache.join("new").join("target-coverage");
    let [patch_fork, patch_clocks] = patches(&copy, &checkout);
    let out = Command::new("cargo")
        .args([
            "--config",
            &patch_fork,
            "--config",
            &patch_clocks,
            "build",
            "--release",
            "--quiet",
        ])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTFLAGS", "-C instrument-coverage")
        .env("BENCH_HASHES_CHECKOUT", bench)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "check: the coverage build:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cov_exe = target.join("release").join("bench-hashes");
    let (profdata, llvm_cov) = (llvm_tool("llvm-profdata"), llvm_tool("llvm-cov"));
    let work = cache.join("coverage-work");
    let mut map: BTreeMap<String, BTreeMap<String, BTreeSet<usize>>> = BTreeMap::new();
    for label in all_points(new_exe, bench) {
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let raw = work.join("p.profraw");
        let out = Command::new(&cov_exe)
            .args([
                "--contenders",
                SUBJECTS,
                "--points",
                &label,
                "--rounds",
                "1",
            ])
            .current_dir(&work)
            .env("LLVM_PROFILE_FILE", &raw)
            .output()
            .expect("the bench runs");
        assert!(
            out.status.success(),
            "check: coverage run of {label}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let merged = work.join("p.profdata");
        let out = Command::new(&profdata)
            .args(["merge", "-sparse", "-o"])
            .arg(&merged)
            .arg(&raw)
            .output()
            .expect("llvm-profdata runs");
        assert!(
            out.status.success(),
            "llvm-profdata: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = Command::new(&llvm_cov)
            .args(["export", "--format=lcov", "--instr-profile"])
            .arg(&merged)
            .arg(&cov_exe)
            .output()
            .expect("llvm-cov runs");
        assert!(
            out.status.success(),
            "llvm-cov: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut files: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        let mut current: Option<String> = None;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(path) = line.strip_prefix("SF:") {
                current = Path::new(path)
                    .strip_prefix(&checkout)
                    .ok()
                    .filter(|r| r.starts_with("src") || r.starts_with("capi"))
                    .map(|r| r.display().to_string());
            } else if let (Some(f), Some(rest)) = (&current, line.strip_prefix("DA:")) {
                let mut parts = rest.split(',');
                let (number, count) = (
                    parts.next().unwrap().parse::<usize>().unwrap(),
                    parts.next().unwrap().parse::<u64>().unwrap(),
                );
                if count > 0 {
                    files.entry(f.clone()).or_default().insert(number);
                }
            }
        }
        map.insert(label, files);
    }
    let mut text = String::new();
    for (label, files) in &map {
        for (f, lines) in files {
            text += &format!(
                "{label}\t{f}\t{}\n",
                lines
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, text).unwrap();
    map
}

/// llvm-profdata or llvm-cov: on PATH, in the nightly sysroot's llvm-tools,
/// or in /usr/lib/llvm-*/bin.
fn llvm_tool(name: &str) -> PathBuf {
    if let Some(p) = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(name))
            .find(|p| p.exists())
    }) {
        return p;
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir("/usr/lib")
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("bin").join(name))
        .filter(|p| p.exists())
        .collect();
    candidates.sort();
    candidates.pop().unwrap_or_else(|| panic!("check: choosing cells by coverage needs {name} (apt install llvm); --cells all skips it"))
}

/// Cargo's `--config` patches that build a bench copy against a fork
/// checkout: the fork's repository as the copy's Cargo.toml names it (a
/// bench pinned to another fork patches that fork), its blake3-servil and
/// clocks from `checkout`.
fn patches(copy: &Path, checkout: &Path) -> [String; 2] {
    let manifest =
        std::fs::read_to_string(copy.join("Cargo.toml")).expect("the bench's Cargo.toml");
    let section = manifest
        .split("[dependencies.blake3-servil]")
        .nth(1)
        .expect("Cargo.toml names blake3-servil's dependency");
    let git = section
        .lines()
        .find_map(|l| l.trim().strip_prefix("git = "))
        .expect("blake3-servil comes by git")
        .trim_matches('"');
    [
        format!(
            "patch.\"{git}\".blake3-servil.path=\"{}\"",
            checkout.display()
        ),
        format!(
            "patch.\"{git}\".clocks.path=\"{}\"",
            checkout.join("clocks").display()
        ),
    ]
}
