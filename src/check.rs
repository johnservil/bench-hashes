//! `bench-hashes check --fork DIR [--against REV] [--old-bench REV] [--cells all] [--pairs N] [--keep DIR]`:
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
//!    build script, or what the fork's manifest builds (anything but
//!    its tests, examples, benches and dev-dependencies)
//!    checks every point (coverage cannot see into assembly or the build).
//!    `--cells all` checks every point regardless.
//! 3. Runs `bench-hashes regress` over the pairs: servil st and servil mt
//!    (the code that changes) at the regression check's visits
//!    (TO_CHECK_FOR_REGRESSIONS), judged all at once (clocks::summary).
//! 4. When no cell is slower, runs the cells `regress` names as suspects
//!    (level, the pairs' mean beyond the margin; at most 8) again on
//!    SECOND_PAIRS fresh pairs, each side relinked anew, judged among
//!    themselves: judged among every cell, a slowdown at one point is
//!    level unless it is many times its pairs' spread (bench-hashes #66).
//!
//! A change to the fork's API changes the bench with it, so one bench builds
//! against one side alone: `--old-bench REV` builds the old side with this
//! bench at REV (a commit of this bench's repository, in a worktree under
//! `target/check/`), the new side with the bench as it is. The cells are
//! keyed by use case and point, not by call, so the two sides' cells meet.
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
/// Pairs of the second stage: when the first finds no cell slower, the
/// cells it names as suspects (level, beyond the margin) run again on
/// fresh layouts and are judged among themselves. Judged among the 75
/// cells a hash() change reaches, a lone cell's paired t on 4 pairs must
/// pass about 15, and a slowdown planted at one point (+20% of calls,
/// its cell +23% to +77% in every pair) was judged level; among 8 cells
/// on 16 pairs the bar is about 3 (bench-hashes #66; the data branch,
/// 2026-10-24/check-one-point).
const SECOND_PAIRS: usize = 16;
/// The two sides' contenders: the code a change to the fork changes.
const SUBJECTS: [&str; 2] = ["blake3-servil-st", "blake3-servil-mt"];

pub fn command(arguments: &[String]) -> i32 {
    let usage = "usage: bench-hashes check --fork DIR [--against REV] [--old-bench REV] [--cells all] [--pairs N] [--keep DIR]";
    let (mut against, mut all, mut pairs, mut keep) =
        ("HEAD".to_string(), false, PAIRS, None::<PathBuf>);
    let mut old_bench_rev = None::<String>;
    let mut fork = None::<PathBuf>;
    let mut it = arguments.iter();
    while let Some(name) = it.next() {
        match name.as_str() {
            "--fork" => {
                fork = Some(PathBuf::from(it.next().unwrap_or_else(|| {
                    panic!("--fork needs the fork's checkout\n{usage}")
                })))
            }
            "--against" => {
                against = it
                    .next()
                    .unwrap_or_else(|| panic!("--against needs a revision\n{usage}"))
                    .clone()
            }
            "--old-bench" => {
                old_bench_rev = Some(
                    it.next()
                        .unwrap_or_else(|| panic!("--old-bench needs a revision\n{usage}"))
                        .clone(),
                )
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
    let fork = fork_checkout(
        &fork.unwrap_or_else(|| panic!("check: --fork DIR names the fork's checkout\n{usage}")),
    );
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
    // The old side's bench: this one, or this repository at --old-bench.
    let old_bench = match &old_bench_rev {
        None => bench.clone(),
        Some(rev) => {
            let commit = git(&bench, &["rev-parse", &format!("{rev}^{{commit}}")]);
            let dir = cache.join("old-bench");
            if !dir.exists() {
                git(&bench, &["worktree", "prune"]);
                git(
                    &bench,
                    &[
                        "worktree",
                        "add",
                        "--detach",
                        dir.to_str().unwrap(),
                        &commit,
                    ],
                );
            } else if git(&dir, &["rev-parse", "HEAD"]) != commit {
                git(
                    &dir,
                    &["checkout", "--quiet", "--force", "--detach", &commit],
                );
            }
            eprintln!(
                "check: the old side with the bench at {rev} ({})",
                &commit[..12]
            );
            dir
        }
    };
    let old_exe = side(&fork, &old_bench, &cache, "old", &old);
    let new_exe = side(&fork, &bench, &cache, "new", &new);
    // The cells a change touches, from the new side's coverage.
    // The cells a change touches: (contender, point), from the new side's
    // coverage; a run measures their contenders over their points.
    let (contenders, points): (Vec<String>, Option<Vec<String>>) = if all {
        (SUBJECTS.iter().map(|s| s.to_string()).collect(), None)
    } else {
        let cells = touched_cells(
            &fork,
            Side {
                bench: &old_bench,
                name: "old",
                commit: &old,
                exe: &old_exe,
            },
            Side {
                bench: &bench,
                name: "new",
                commit: &new,
                exe: &new_exe,
            },
            &cache,
        );
        if cells.is_empty() {
            println!("check: the change touches no line a cell executes; nothing to measure");
            return 0;
        }
        let contenders: BTreeSet<String> = cells.iter().map(|(c, _)| c.clone()).collect();
        let points: BTreeSet<String> = cells.iter().map(|(_, p)| p.clone()).collect();
        eprintln!(
            "check: {} cells execute a changed line: {} at {} points: {}",
            cells.len(),
            contenders.iter().cloned().collect::<Vec<_>>().join(" and "),
            points.len(),
            points.iter().cloned().collect::<Vec<_>>().join(", ")
        );
        (
            contenders.into_iter().collect(),
            Some(points.into_iter().collect()),
        )
    };
    let olds = layouts(&old_bench, &cache, "old", &old_exe, 0..pairs);
    let news = layouts(&bench, &cache, "new", &new_exe, pairs..2 * pairs);
    let mut regress: Vec<String> = vec![
        "regress".into(),
        "--pairs".into(),
        pairs.to_string(),
        "--contenders".into(),
        contenders.join(","),
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
    let (code, out) = run_regress(&new_exe, &bench, &regress);
    let suspects: Vec<String> = out
        .lines()
        .find_map(|l| l.strip_prefix("regress: suspects: "))
        .map(|s| s.split(';').map(str::to_string).collect())
        .unwrap_or_default();
    if code != 0 || suspects.is_empty() {
        return code;
    }
    // The second stage: the suspects on fresh layouts (seeds past both
    // sides' first ones), judged among themselves.
    eprintln!(
        "check: a second stage for {} suspects on {SECOND_PAIRS} fresh pairs: {}",
        suspects.len(),
        suspects.join(", ")
    );
    let field = |k: &String, i: usize| k.split('|').nth(i).expect("a cell key").to_string();
    let contenders: BTreeSet<String> = suspects.iter().map(|k| field(k, 0)).collect();
    let points: BTreeSet<String> = suspects
        .iter()
        .map(|k| {
            point_label(&field(k, 2), &field(k, 3)).expect("a suspect's use case is this bench's")
        })
        .collect();
    let first = 2 * pairs;
    let olds = layouts(
        &old_bench,
        &cache,
        "old",
        &old_exe,
        first..first + SECOND_PAIRS,
    );
    let news = layouts(
        &bench,
        &cache,
        "new",
        &new_exe,
        first + SECOND_PAIRS..first + 2 * SECOND_PAIRS,
    );
    let mut second: Vec<String> = vec![
        "regress".into(),
        "--pairs".into(),
        SECOND_PAIRS.to_string(),
        "--contenders".into(),
        contenders.into_iter().collect::<Vec<_>>().join(","),
        "--points".into(),
        points.into_iter().collect::<Vec<_>>().join(","),
        "--judge".into(),
        suspects.join(";"),
    ];
    if let Some(keep) = &keep {
        second.extend([
            "--keep".to_string(),
            keep.join("second").display().to_string(),
        ]);
    }
    second.extend(olds);
    second.push("--".into());
    second.extend(news);
    run_regress(&new_exe, &bench, &second).0
}

/// The `--points` label of a cell's point, from the use case's name and the
/// point as a cell key spells them ("IdleOneMessage", "256 B": "idle 256
/// B"); None for a use case of another bench version.
fn point_label(use_case: &str, point: &str) -> Option<String> {
    super::UseCase::ALL
        .into_iter()
        .find(|u| format!("{u:?}") == use_case)
        .map(|u| format!("{}{point}", u.label_prefix()))
}

/// `regress` with these arguments, its output passed through and kept.
fn run_regress(exe: &Path, bench: &Path, arguments: &[String]) -> (i32, String) {
    let out = Command::new(exe)
        .args(arguments)
        .current_dir(bench)
        .stderr(std::process::Stdio::inherit())
        .output()
        .expect("regress runs");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    print!("{text}");
    (out.status.code().unwrap_or(2), text)
}

/// The fork's checkout that --fork names: a git repository holding
/// `Cargo.toml` with `name = "blake3-servil"`, wherever it sits.
fn fork_checkout(dir: &Path) -> PathBuf {
    let fork = std::fs::canonicalize(dir)
        .unwrap_or_else(|e| panic!("check: --fork {}: {e}", dir.display()));
    let manifest = std::fs::read_to_string(fork.join("Cargo.toml")).unwrap_or_else(|e| {
        panic!(
            "check: {} holds no Cargo.toml: {e}; --fork names the fork's checkout",
            fork.display()
        )
    });
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
/// worktree for the fork, keeping the bench's pinned clocks, and a target
/// directory of the side's own, so an unchanged side rebuilds nothing.
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
/// maintainer measuring the working tree, run it from there). The two
/// checkouts sit anywhere; neither holds the other.
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
    // A directory no bench before the separation (0.18.3 and earlier) builds
    // in: those built in target/build-against, from a copy at
    // checkout/bench-hashes, under the same package name and version, so
    // Cargo gave both layouts' build scripts one unit. Its record of a file
    // inside the target directory (the copy here) is relative to the
    // target, so an old bench built after a new one checked the new copy's
    // build.rs, found it fresh, and ran the new build script (the Mac's job
    // 1781, October 31).
    let exe = build_against(
        &checkout,
        &bench,
        &target.join("bench-hashes-build"),
        "build-against",
    );
    println!("{}", exe.display());
    0
}

/// The copy of the bench a build against the fork compiles, in the build's
/// target directory: outside both checkouts, so neither reads as changed.
fn bench_copy(target: &Path) -> PathBuf {
    target.join("bench-src")
}

/// This bench, built against the fork checkout at `checkout` as it is:
/// its sources copied into the build's target directory (bench_copy),
/// Cargo pointed at the checkout for the fork (and build.rs told where it
/// is, BENCH_HASHES_FORK), keeping the bench's pinned clocks, a lock and a
/// target directory of the build's own (the committed lock is never
/// written); the executable's path. `check`'s two sides and the Mac
/// runner's benchmark jobs build this way, and so does a maintainer
/// measuring a working tree (`bench-hashes build-against DIR`).
pub fn build_against(checkout: &Path, bench: &Path, target: &Path, name: &str) -> PathBuf {
    let commit = git(checkout, &["rev-parse", "HEAD"]);
    let copy = bench_copy(target);
    copy_bench(bench, &copy);
    let target = target.to_path_buf();
    let patch_fork = fork_patch(&copy, &checkout);
    // The side's lock: the bench's, with the fork taken from the worktree.
    // Cargo patches at the locked version only, so a version change wants
    // `cargo update -p blake3-servil`.
    let out = Command::new("cargo")
        .args([
            "--config",
            &patch_fork,
            "update",
            "--quiet",
            "-p",
            "blake3-servil",
        ])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("BENCH_HASHES_CHECKOUT", bench)
        .env("BENCH_HASHES_FORK", checkout)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "check: {name}: cargo update: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("cargo")
        .args(["--config", &patch_fork, "build", "--release", "--quiet"])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("BENCH_HASHES_CHECKOUT", bench)
        .env("BENCH_HASHES_FORK", checkout)
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
    let exe = target.join("release").join("bench-hashes");
    require_sme2_kernel(&exe);
    exe
}

/// On a CPU with SME2, the build must carry the SME2 kernel (the fork
/// builds without it, with a warning, when the C compiler cannot assemble
/// SME2): a check would measure NEON alone and miss every SME2 change. The
/// kernel's own symbol in the executable says whether it is there.
fn require_sme2_kernel(exe: &Path) {
    let has_sme2 = if cfg!(target_os = "macos") {
        Command::new("sysctl")
            .args(["-n", "hw.optional.arm.FEAT_SME2"])
            .output()
            .is_ok_and(|o| o.stdout.trim_ascii() == b"1")
    } else {
        std::fs::read_to_string("/proc/cpuinfo").is_ok_and(|c| c.contains(" sme2"))
    };
    if !has_sme2 {
        return;
    }
    let bytes = std::fs::read(exe).expect("the executable reads");
    // The kernel's name without its crate prefix: `blake3_servil_` since BLAKE3 #164, `blake3_` before.
    let symbol = b"sme2_hash16_chunks_512";
    assert!(
        bytes.windows(symbol.len()).any(|w| w == symbol),
        "check: this CPU has SME2, and the fork was built without its SME2 kernel: point CC at a compiler that assembles SME2 (CC=clang-19 on Debian 12)"
    );
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
fn layouts(
    bench: &Path,
    cache: &Path,
    name: &str,
    exe: &Path,
    seeds: std::ops::Range<usize>,
) -> Vec<String> {
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
    let copy = bench_copy(&side_dir.join("target"));
    let checkout = side_dir.join("src");
    let patch_fork = fork_patch(&copy, &checkout);
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
            .env("BENCH_HASHES_CHECKOUT", bench)
            .env("BENCH_HASHES_FORK", &checkout)
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
    // The relinking runs on Linux (ld.lld) and macOS (its linker's order
    // file); elsewhere check stops here.
    #[cfg(unix)]
    std::os::unix::fs::symlink(found, &link).unwrap();
    #[cfg(not(unix))]
    panic!(
        "check relinks each side with a function order, on Linux and macOS only; {} is the ld.lld found",
        found.display()
    );
    #[cfg(unix)]
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

/// One side of a check: its bench, its name under the cache, the fork's
/// commit it builds, and its executable.
#[derive(Clone, Copy)]
struct Side<'a> {
    bench: &'a Path,
    name: &'a str,
    commit: &'a str,
    exe: &'a Path,
}

/// The points whose cells execute a line the change touches: the lines it
/// adds or changes, which the new side's cells execute, and the lines it
/// deletes or changes, which the old side's executed (a deleted line has
/// no new side: a change deleting a fast path still measures the cells
/// that took it). A coverage build of each side runs each point once
/// (servil st and mt) and keeps the lines each executes in the fork's
/// `src/`, cached by the bench's source and the fork's commit; the diff
/// (old..new, `src/`) gives the lines. Assembly, the build script or a
/// manifest changed: every point.
fn touched_cells(
    fork: &Path,
    old_side: Side,
    new_side: Side,
    cache: &Path,
) -> Vec<(String, String)> {
    let (bench, old, new, new_exe) = (
        new_side.bench,
        old_side.commit,
        new_side.commit,
        new_side.exe,
    );
    let changed_files = git(fork, &["diff", "--name-only", &format!("{old}..{new}")]);
    let every = || all_cells(new_exe, bench);
    if changed_files.lines().any(|f| {
        f.starts_with("c/")
            || f == "build.rs"
            || f.ends_with(".S")
            || f.ends_with(".asm")
            || (f == "Cargo.toml" && manifest_builds_differently(fork, old, new, f))
    }) {
        eprintln!("check: assembly, the build script or the build's manifest changed: every point");
        return every();
    }
    let (added, deleted) = touched_lines(&git(
        fork,
        &[
            "diff",
            "--unified=0",
            &format!("{old}..{new}"),
            "--",
            "src",
            "capi/src",
        ],
    ));
    if added.is_empty() && deleted.is_empty() {
        return Vec::new();
    }
    let reaches = |reached: &BTreeMap<String, BTreeSet<usize>>,
                   touched: &BTreeMap<String, BTreeSet<usize>>| {
        reached
            .iter()
            .any(|(f, lines)| touched.get(f).is_some_and(|t| !t.is_disjoint(lines)))
    };
    let new_reached = coverage(new_side, cache);
    let old_reached = if deleted.is_empty() {
        BTreeMap::new()
    } else {
        coverage(old_side, cache)
    };
    new_reached
        .iter()
        .filter(|(cell, files)| {
            reaches(files, &added) || old_reached.get(*cell).is_some_and(|f| reaches(f, &deleted))
        })
        .map(|(cell, _)| cell.clone())
        .collect()
}

/// A `--unified=0` diff's touched lines, by file: the new side's (each
/// hunk's `+c,d`) and the old side's (`-a,b`). A hunk that adds nothing
/// (`+c,0`) touches no new line, and one that deletes nothing no old one.
fn touched_lines(
    diff: &str,
) -> (
    BTreeMap<String, BTreeSet<usize>>,
    BTreeMap<String, BTreeSet<usize>>,
) {
    let (mut added, mut deleted) = (BTreeMap::new(), BTreeMap::new());
    let (mut old_file, mut new_file) = (None::<String>, None::<String>);
    let range = |r: &str| -> (usize, usize) {
        r.split_once(',')
            .map(|(s, c)| (s.parse().unwrap(), c.parse().unwrap()))
            .unwrap_or_else(|| (r.parse().unwrap(), 1))
    };
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("--- ") {
            old_file = f.strip_prefix("a/").map(str::to_string);
        } else if let Some(f) = line.strip_prefix("+++ ") {
            new_file = f.strip_prefix("b/").map(str::to_string);
        } else if line.starts_with("@@") {
            // "@@ -a,b +c,d @@ context"
            let mut parts = line.split_whitespace().skip(1);
            let (a, b) = range(parts.next().unwrap().trim_start_matches('-'));
            let (c, d) = range(parts.next().unwrap().trim_start_matches('+'));
            if let (Some(f), true) = (&old_file, b > 0) {
                deleted
                    .entry(f.clone())
                    .or_insert_with(BTreeSet::new)
                    .extend(a..a + b);
            }
            if let (Some(f), true) = (&new_file, d > 0) {
                added
                    .entry(f.clone())
                    .or_insert_with(BTreeSet::new)
                    .extend(c..c + d);
            }
        }
    }
    (added, deleted)
}

/// Every cell of the check's contenders, (contender, point as `--points`
/// takes it), as a one-round run measures them (a contender takes part
/// in its use cases alone).
fn all_cells(exe: &Path, bench: &Path) -> Vec<(String, String)> {
    let dir = bench.join("target").join("check").join("points");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(exe)
        .args(["--contenders", &SUBJECTS.join(","), "--rounds", "1"])
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
    let mut cells = BTreeSet::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("contender"))
    {
        let f: Vec<&str> = line.split('\t').collect();
        if let Some(label) = point_label(f[2], f[3]) {
            cells.insert((f[0].to_string(), label));
        }
    }
    cells.into_iter().collect()
}

/// Each point's executed lines of the fork's `src/`, from a coverage build
/// of `side` run one point at a time; cached as JSON-free TSV under the
/// cache, keyed by the bench's source and the fork's commit.
fn coverage(
    side: Side,
    cache: &Path,
) -> BTreeMap<(String, String), BTreeMap<String, BTreeSet<usize>>> {
    let (bench, new, new_exe) = (side.bench, side.commit, side.exe);
    let bench_source = {
        let mut h = blake3_servil::performance::Hasher::new(blake3_servil::Mode::Plain);
        for f in walk(&bench.join("src")) {
            h.update(&std::fs::read(&f).unwrap());
        }
        h.finalize().to_hex().to_string()
    };
    let cached = cache
        .join("coverage")
        .join(format!("{}-{}.tsv", &new[..12], &bench_source[..12]));
    if let Ok(text) = std::fs::read_to_string(&cached) {
        let mut map: BTreeMap<(String, String), BTreeMap<String, BTreeSet<usize>>> =
            BTreeMap::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            map.entry((f[0].to_string(), f[1].to_string()))
                .or_default()
                .insert(
                    f[2].to_string(),
                    f[3].split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.parse().unwrap())
                        .collect(),
                );
        }
        return map;
    }
    eprintln!(
        "check: recording which lines each point executes on the {} side (once per change to the bench or the fork)",
        side.name
    );
    let checkout = cache.join(side.name).join("src");
    let copy = bench_copy(&cache.join(side.name).join("target"));
    let target = cache.join(side.name).join("target-coverage");
    let patch_fork = fork_patch(&copy, &checkout);
    let out = Command::new("cargo")
        .args(["--config", &patch_fork, "build", "--release", "--quiet"])
        .current_dir(&copy)
        .env("CARGO_TARGET_DIR", &target)
        .env("RUSTFLAGS", "-C instrument-coverage")
        .env("BENCH_HASHES_CHECKOUT", bench)
        .env("BENCH_HASHES_FORK", &checkout)
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
    let mut map: BTreeMap<(String, String), BTreeMap<String, BTreeSet<usize>>> = BTreeMap::new();
    for (contender, label) in all_cells(new_exe, bench) {
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();
        let raw = work.join("p.profraw");
        let out = Command::new(&cov_exe)
            .args([
                "--contenders",
                &contender,
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
            "check: coverage run of {contender} at {label}: {}",
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
        map.insert((contender, label), files);
    }
    let mut text = String::new();
    for ((contender, label), files) in &map {
        for (f, lines) in files {
            text += &format!(
                "{contender}\t{label}\t{f}\t{}\n",
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

/// llvm-profdata or llvm-cov: the toolchain's own llvm-tools first
/// (`rustup component add llvm-tools`: its profile format is always the
/// compiler's), else on PATH, else the newest in /usr/lib/llvm-*/bin. An
/// LLVM on PATH older than the compiler's cannot read its profiles (Ubuntu
/// 26.04's LLVM 21 beside Rust 1.99's LLVM 23, EPYC 9255: bench-hashes #87).
fn llvm_tool(name: &str) -> PathBuf {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("rustc runs");
    let rustlib = Path::new(String::from_utf8_lossy(&sysroot.stdout).trim()).join("lib/rustlib");
    let in_bins = |dir: &Path| -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("bin").join(name))
            .filter(|p| p.exists())
            .collect();
        found.sort();
        found
    };
    let on_path = || {
        std::env::var_os("PATH").and_then(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(name))
                .find(|p| p.exists())
        })
    };
    in_bins(&rustlib)
        .pop()
        .or_else(on_path)
        .or_else(|| in_bins(Path::new("/usr/lib")).pop())
        .unwrap_or_else(|| {
            panic!(
                "check: choosing cells by coverage needs {name}: `rustup component add llvm-tools` (or, on Linux, apt install llvm); --cells all skips it"
            )
        })
}

/// Cargo's `--config` patch for a fork checkout. The bench keeps its pinned
/// clocks on both sides, so their timing and summary rules stay the same.
fn fork_patch(copy: &Path, checkout: &Path) -> String {
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
    format!(
        "patch.\"{git}\".blake3-servil.path=\"{}\"",
        checkout.display()
    )
}

/// Whether `manifest`'s change between `old` and `new` touches what the
/// bench's build of the fork uses: a changed line, on either side, in any
/// section but `[[test]]`, `[[example]]`, `[[bench]]` and
/// `[dev-dependencies]` (a test's entry changes no cell).
fn manifest_builds_differently(fork: &Path, old: &str, new: &str, manifest: &str) -> bool {
    let unused = |section: &str| {
        matches!(
            section,
            "[[test]]" | "[[example]]" | "[[bench]]" | "[dev-dependencies]"
        )
    };
    // Each line's section, in one side's file.
    let sections = |rev: &str| -> Vec<String> {
        let text = Command::new("git")
            .args(["show", &format!("{rev}:{manifest}")])
            .current_dir(fork)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let mut current = String::new();
        text.lines()
            .map(|l| {
                let t = l.trim();
                if t.starts_with('[') {
                    current = t.to_string();
                }
                current.clone()
            })
            .collect()
    };
    let (before, after) = (sections(old), sections(new));
    let diff = git(
        fork,
        &[
            "diff",
            "--unified=0",
            &format!("{old}..{new}"),
            "--",
            manifest,
        ],
    );
    for line in diff.lines().filter(|l| l.starts_with("@@")) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        for (field, side) in [
            (fields[1].trim_start_matches('-'), &before),
            (fields[2].trim_start_matches('+'), &after),
        ] {
            let (start, count) = field
                .split_once(',')
                .map(|(s, c)| (s.parse::<usize>().unwrap(), c.parse::<usize>().unwrap()))
                .unwrap_or_else(|| (field.parse().unwrap(), 1));
            // A pure insertion or deletion names the line before it (count 0).
            for n in start.max(1)..=(start + count.max(1) - 1) {
                if side.get(n - 1).is_none_or(|s| !unused(s)) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    /// The toolchain's llvm-profdata wins over one on PATH: an older LLVM
    /// there cannot read the compiler's profiles (bench-hashes #87, Sam's
    /// EPYC). Needs `rustup component add llvm-tools`, which the release
    /// machines have; without it the test fails, saying so.
    #[test]
    fn llvm_tool_takes_the_toolchains_copy_before_path() {
        let fake = std::env::temp_dir().join(format!("llvm-tool-test-{}", std::process::id()));
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(fake.join("llvm-profdata"), "#!/bin/sh\nexit 1\n").unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined =
            std::env::join_paths(std::iter::once(fake.clone()).chain(std::env::split_paths(&path)))
                .unwrap();
        // Sound: nextest runs each test in its own process, so no other
        // thread reads the environment meanwhile.
        unsafe { std::env::set_var("PATH", &joined) };
        let got = super::llvm_tool("llvm-profdata");
        unsafe { std::env::set_var("PATH", &path) };
        std::fs::remove_dir_all(&fake).unwrap();
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .output()
            .unwrap()
            .stdout;
        assert!(
            got.starts_with(String::from_utf8(sysroot).unwrap().trim()),
            "took {got:?}: install llvm-tools (rustup component add llvm-tools)"
        );
    }

    use super::touched_lines;
    use std::collections::BTreeSet;
    /// Every point the bench measures survives the trip through a cell key
    /// and back to the label that selects it: a second stage's suspects
    /// are cell keys, and "256 B" alone selects the call after other work,
    /// not the one after a pause (an IdleOneMessage suspect once went
    /// unmeasured so).
    #[test]
    fn a_deleted_line_is_touched_on_the_old_side() {
        let diff = "diff --git a/src/lanes.rs b/src/lanes.rs\n--- a/src/lanes.rs\n+++ b/src/lanes.rs\n\
                    @@ -1247 +1246,0 @@ fn run_members\n-                128 => return,\n\
                    @@ -1300,2 +1299,2 @@\n-a\n-b\n+c\n+d\n\
                    diff --git a/src/new.rs b/src/new.rs\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,3 @@\n+x\n+y\n+z\n";
        let (added, deleted) = touched_lines(diff);
        // The deleted arm: no new line, but its old one (the case the new
        // side alone missed: "nothing to measure", BLAKE3 #183).
        assert_eq!(deleted["src/lanes.rs"], BTreeSet::from([1247, 1300, 1301]));
        assert_eq!(added["src/lanes.rs"], BTreeSet::from([1299, 1300]));
        // A new file has no old side.
        assert_eq!(added["src/new.rs"], BTreeSet::from([1, 2, 3]));
        assert!(!deleted.contains_key("src/new.rs"));
    }

    #[test]
    fn every_cell_key_names_the_point_that_measures_it() {
        for (index, point) in super::super::POINTS.iter().enumerate() {
            let label = super::point_label(&format!("{:?}", point.use_case), point.label)
                .expect("this bench's use case");
            assert_eq!(super::super::point_named(&label), index, "{label}");
        }
    }
}
