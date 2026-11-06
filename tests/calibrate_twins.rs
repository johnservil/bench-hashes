//! `--calibrate-only` writes an owned cell measured as its lent twin with
//! the twin's size (twins_take_their_sizes), as the run itself uses it: the
//! sizes file `regress` gives both sides. It held 1 there, and a new side
//! that measured such a cell itself timed one call a sample (bench #133).

mod require_nextest {
    include!("../tools/require_nextest.rs");
}

#[test]
fn calibrate_only_gives_twins_their_twins_sizes() {
    let path = std::env::temp_dir().join(format!("calibrate-twins-{}.tsv", std::process::id()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_bench-hashes"))
        .args(["--contenders", "blake3-servil-st", "--points", "continuous 4 KiB,lent 4 KiB", "--calibrate-only"])
        .arg(&path)
        .output()
        .expect("bench-hashes runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = std::fs::read_to_string(&path).expect("the sizes file");
    std::fs::remove_file(&path).unwrap();
    let size = |cell: &str| -> usize {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("blake3-servil-st\t{cell}\t")))
            .unwrap_or_else(|| panic!("no {cell} in\n{text}"))
            .parse()
            .unwrap()
    };
    assert!(size("lent 4 KiB") > 1, "the lent cell calibrated:\n{text}");
    assert_eq!(size("continuous 4 KiB"), size("lent 4 KiB"), "the owned cell takes its twin's size:\n{text}");
}
