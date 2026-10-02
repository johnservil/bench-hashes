//! The charts drawn from a samples file: for the hashes
//! (`bench-hashes.chart.svg`), one 1 MiB message, BLAKE3 servil on every
//! core and on one, beside the fastest of each other family the run
//! measured, as bars in GB/s; for b3sum (`b3sum.chart.svg`), each input's
//! time per run for every build, warm and cold. Every run draws its chart
//! from the samples file it writes; `bench-hashes chart SAMPLES.tsv` draws
//! one from a stored file. The READMEs show the published records'.

use super::{read_samples, ExactMean};
use std::fmt::Write as _;

const SIZE: &str = "1 MiB";
/// The bars: BLAKE3 servil's two, then each other family's fastest member.
const BLAKE3: [(&str, &str); 2] = [("blake3-servil-mt", "BLAKE3, every core"), ("blake3-servil-st", "BLAKE3, one core")];
const FAMILIES: [(&str, &[&str], &str); 3] = [
    ("SHA-256", &["sha256", "sha256-ring", "sha256-cc"], "#c2410c"),
    ("SHA3-256", &["sha3-256"], "#db2777"),
    ("SHA-1", &["sha1dc"], "#8a7a1e"),
];
const BLAKE3_COLOR: &str = "#7c3aed";

const WIDTH: f64 = 720.0;
const LEFT: f64 = 170.0;
const BAR: f64 = 26.0;
const GAP: f64 = 10.0;
const PLOT: f64 = WIDTH - LEFT - 60.0;
const FONT: &str = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif";

/// The chart from a samples file: b3sum's for a b3sum run; else the
/// hashes', or `None` when the file lacks BLAKE3 servil's two 1 MiB cells
/// (a quick run stops below 1 MiB).
pub fn from_samples(path: &str) -> Option<String> {
    let file = read_samples(path);
    if file.cells.iter().any(|(key, _)| key.split('|').nth(2) == Some("b3sum")) {
        return Some(b3sum_chart(&file));
    }
    let header = |key: &str| file.headers.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("?");
    // Hundredths of GB/s from a cell's mean (ns per byte, Q64.64), rounded once.
    let centi = |key: &str| -> Option<u128> {
        let cell = format!("{key}|solo|OneMessage|{SIZE}");
        let (_, samples) = file.cells.iter().find(|(k, _)| *k == cell)?;
        let mean = ExactMean::of(samples).fixed().0;
        Some(((100u128 << 64) + mean / 2) / mean)
    };
    let mut bars: Vec<(String, u128, &str, bool)> = Vec::new();
    for (key, name) in BLAKE3 {
        bars.push((name.to_owned(), centi(key)?, BLAKE3_COLOR, true));
    }
    for (family, members, color) in FAMILIES {
        if let Some(best) = members.iter().filter_map(|m| centi(m)).max() {
            bars.push((family.to_owned(), best, color, false));
        }
    }
    bars.sort_by(|a, b| b.1.cmp(&a.1));
    let date = header("timestamp").split(' ').next().unwrap_or("?").to_owned();
    let servil = header("blake3-servil source");
    let commit = servil.split("commit ").nth(1).map_or("?", |c| &c[..7.min(c.len())]);
    let subtitle = format!("{}, {} CPUs · GB/s, faster to the right", header("cpu type"), header("cpu count"));
    let footer = [
        format!("bench-hashes {} on {date}, BLAKE3 servil {commit}", header("bench-hashes version").split('+').next().unwrap_or("?")),
        "each call after other work, as a program hashes between its tasks; each bar the mean over the run".to_owned(),
    ];
    let top = bars[0].1 as f64;
    let head = 100.0;
    let height = head + bars.len() as f64 * (BAR + GAP) + 44.0;
    let mut svg = String::new();
    writeln!(svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="{FONT}">"#).unwrap();
    writeln!(svg, r##"<rect width="{WIDTH}" height="{height}" fill="#ffffff"/>"##).unwrap();
    writeln!(svg, r##"<text x="{LEFT}" y="28" font-size="17" font-weight="600" fill="#111827">Hashing one {SIZE} message</text>"##).unwrap();
    writeln!(svg, r##"<text x="{LEFT}" y="50" font-size="13" fill="#4b5563">{}</text>"##, super::xml_escape(&subtitle)).unwrap();
    writeln!(svg, r##"<text x="{LEFT}" y="72" font-size="13" fill="#4b5563">BLAKE3's tree spreads one message over every core; the others use one.</text>"##).unwrap();
    for (i, (name, centi, color, bold)) in bars.iter().enumerate() {
        let y = head + i as f64 * (BAR + GAP);
        let w = PLOT * *centi as f64 / top;
        let weight = if *bold { r#" font-weight="600""# } else { "" };
        let ty = y + BAR * 0.68;
        let shown = if *centi >= 1000 { format!("{}", (centi + 50) / 100) } else { format!("{}.{:02}", centi / 100, centi % 100) };
        writeln!(svg, r##"<text x="{}" y="{ty:.1}" font-size="13" fill="#111827" text-anchor="end"{weight}>{name}</text>"##, LEFT - 10.0).unwrap();
        writeln!(svg, r#"<rect x="{LEFT}" y="{y}" width="{w:.1}" height="{BAR}" rx="3" fill="{color}"/>"#).unwrap();
        writeln!(svg, r##"<text x="{:.1}" y="{ty:.1}" font-size="13" fill="#111827"{weight}>{shown}</text>"##, LEFT + w + 6.0).unwrap();
    }
    for (i, line) in footer.iter().enumerate() {
        writeln!(svg, r##"<text x="{LEFT}" y="{:.1}" font-size="11" fill="#9a9a9a">{}</text>"##, height - 26.0 + i as f64 * 14.0, super::xml_escape(line)).unwrap();
    }
    svg.push_str("</svg>\n");
    Some(svg)
}

/// Contender colours for b3sum's chart, in the order the run named them.
const PALETTE: [&str; 6] = ["#6b7280", "#7c3aed", "#0f766e", "#c2410c", "#2563eb", "#db2777"];

/// b3sum's chart: a panel for each page-cache state, a row for each input,
/// and in each row a bar for each build, as long as its speed against the
/// row's fastest, with its mean time per run and its time against the
/// first build's (the median of the rounds' ratios, marked slower or
/// faster by clocks::summary::verdict, as the report does).
fn b3sum_chart(file: &super::SamplesFile) -> String {
    let header = |key: &str| file.headers.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("?");
    let parts = |key: &str| -> (String, String, String) {
        let f: Vec<&str> = key.split('|').collect();
        (f[0].to_owned(), f[1].to_owned(), f[3].to_owned())
    };
    let mut contenders: Vec<String> = Vec::new();
    let mut caches: Vec<String> = Vec::new();
    let mut inputs: Vec<String> = Vec::new();
    for (key, _) in &file.cells {
        let (c, cache, input) = parts(key);
        for (list, item) in [(&mut contenders, c), (&mut caches, cache), (&mut inputs, input)] {
            if !list.contains(&item) {
                list.push(item);
            }
        }
    }
    let cell = |c: &str, cache: &str, input: &str| -> &[super::Measured] {
        &file.cells.iter().find(|(k, _)| *k == format!("{c}|{cache}|b3sum|{input}")).expect("every cell measured").1
    };
    let row_h = 15.0;
    let block = contenders.len() as f64 * row_h + 10.0;
    let panel_head = 30.0;
    let top = 92.0;
    let height = top + contenders.len() as f64 * 18.0 - 18.0 + caches.len() as f64 * (panel_head + inputs.len() as f64 * block) + 40.0;
    let (left, plot) = (190.0, 300.0);
    let mut svg = String::new();
    writeln!(svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="{FONT}">"#).unwrap();
    writeln!(svg, r##"<rect width="{WIDTH}" height="{height}" fill="#ffffff"/>"##).unwrap();
    writeln!(svg, r##"<text x="20" y="28" font-size="17" font-weight="600" fill="#111827">How fast b3sum hashes files, from its start to its exit</text>"##).unwrap();
    let subtitle = format!("{}, {} CPUs, {} · files on {}", header("cpu type"), header("cpu count"), header("os type"), header("files").split_once(" on ").map_or("?", |(_, fs)| fs));
    writeln!(svg, r##"<text x="20" y="48" font-size="13" fill="#4b5563">{}</text>"##, super::xml_escape(&subtitle)).unwrap();
    // The legend: each build's name and its own --version line.
    for (i, c) in contenders.iter().enumerate() {
        let ly = 61.0 + i as f64 * 18.0;
        let version = header(&format!("contender {c}")).rsplit("; ").next().unwrap_or("").trim_end_matches(')');
        writeln!(svg, r#"<rect x="20" y="{ly:.1}" width="12" height="12" rx="2" fill="{}"/>"#, PALETTE[i % PALETTE.len()]).unwrap();
        writeln!(svg, r##"<text x="37" y="{:.1}" font-size="13" fill="#111827">{} <tspan fill="#6b7280">{}</tspan></text>"##, ly + 10.0, super::xml_escape(c), super::xml_escape(version)).unwrap();
    }
    let mut y = top + contenders.len() as f64 * 18.0 - 18.0;
    for cache in &caches {
        let title = if cache == "warm" { "Files in the page cache (read moments before)" } else { "Files read from storage (evicted before each run)" };
        writeln!(svg, r##"<text x="20" y="{:.1}" font-size="14" font-weight="600" fill="#111827">{title}</text>"##, y + 18.0).unwrap();
        y += panel_head;
        for input in &inputs {
            let runs: Vec<&[super::Measured]> = contenders.iter().map(|c| cell(c, cache, input)).collect();
            // Each build's mean time per run (ns), rounded.
            let times: Vec<u128> = runs.iter().map(|r| {
                let total: u128 = r.iter().map(|m| u128::from(m.ns)).sum();
                (total + r.len() as u128 / 2) / r.len() as u128
            }).collect();
            let fastest = *times.iter().min().unwrap();
            writeln!(svg, r##"<text x="{:.1}" y="{:.1}" font-size="13" fill="#111827" text-anchor="end">{}</text>"##, left - 10.0, y + block / 2.0, super::xml_escape(input)).unwrap();
            for (i, time) in times.iter().enumerate() {
                let w = plot * fastest as f64 / *time as f64;
                let by = y + i as f64 * row_h;
                writeln!(svg, r#"<rect x="{left}" y="{by:.1}" width="{w:.1}" height="{:.1}" rx="2" fill="{}"/>"#, row_h - 3.0, PALETTE[i % PALETTE.len()]).unwrap();
                let mut label = super::b3sum::time(*time);
                if i > 0 {
                    let ratios: Vec<u64> = runs[i].iter().zip(runs[0]).map(|(a, b)| clocks::summary::ratio_permille(u128::from(a.ns), u128::from(b.ns))).collect();
                    let median = clocks::summary::median_permille(&ratios);
                    let mark = match clocks::summary::verdict(&ratios, 30) {
                        clocks::summary::Verdict::Slower => ", slower",
                        clocks::summary::Verdict::Faster => ", faster",
                        clocks::summary::Verdict::Level => "",
                    };
                    label += &format!(" · x{}.{:02}{mark}", median / 1000, (median % 1000 + 5) / 10);
                }
                writeln!(svg, r##"<text x="{:.1}" y="{:.1}" font-size="11" fill="#374151">{label}</text>"##, left + w + 6.0, by + row_h - 5.0).unwrap();
            }
            y += block;
        }
    }
    let footer = format!("bench-hashes {} on {}, {} rounds: bars, speed against each row's fastest; xN, time against {}'s (median over the rounds)",
        header("bench-hashes version").split('+').next().unwrap_or("?"), header("timestamp").split(' ').next().unwrap_or("?"), header("rounds"), contenders[0]);
    writeln!(svg, r##"<text x="20" y="{:.1}" font-size="11" fill="#9a9a9a">{}</text>"##, height - 14.0, super::xml_escape(&footer)).unwrap();
    svg.push_str("</svg>\n");
    svg
}

/// `bench-hashes chart SAMPLES.tsv`: draw its chart beside the samples file
/// (bench-hashes.samples.tsv -> bench-hashes.chart.svg, b3sum.samples.tsv ->
/// b3sum.chart.svg).
pub fn command(args: &[String]) {
    let [path] = args else { panic!("usage: bench-hashes chart SAMPLES.tsv") };
    let svg = from_samples(path).unwrap_or_else(|| panic!("{path} holds no 1 MiB cells of BLAKE3 servil st and mt (a full run measures them)"));
    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).expect("a samples file name").replace(".samples.tsv", ".chart.svg");
    let out = std::path::Path::new(path).with_file_name(name);
    std::fs::write(&out, svg).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
    println!("# Speed chart (SVG) is in \"{}\" .", out.display());
}

#[cfg(test)]
mod tests {
    /// The chart from a samples file: 1 MiB in 100 us is 10.49 GB/s
    /// (1048576 / 100000 bytes a ns), drawn as "10"; in 400 us, 2.62.
    #[test]
    fn bars_from_a_samples_file() {
        let path = std::env::temp_dir().join(format!("bench-hashes-chart-{}.samples.tsv", std::process::id()));
        let row = |key: &str, ns: u64| format!("{key}\tsolo\tOneMessage\t1 MiB\tB\t{ns}/1048576,{ns}/1048576\t1,2\n");
        let text = format!("{}\n# timestamp: 2026-10-02 12:00:00 UTC\n# bench-hashes version: 0.13.0+abc\n# blake3-servil source: x; commit 0123456789abcdef; clean\n# cpu type: Test CPU\n# cpu count: 4\n# power: n/a\n# load: quiet\n{}\n{}{}{}",
            super::super::SAMPLES_VERSION, super::super::SAMPLES_COLUMNS, row("blake3-servil-mt", 100_000), row("blake3-servil-st", 400_000), row("sha256", 800_000));
        std::fs::write(&path, text).unwrap();
        let svg = super::from_samples(path.to_str().unwrap()).expect("a chart");
        std::fs::remove_file(&path).unwrap();
        assert!(svg.contains(">BLAKE3, every core</text>") && svg.contains(">10</text>"), "{svg}");
        assert!(svg.contains(">2.62</text>") && svg.contains(">1.31</text>"), "{svg}");
        assert!(svg.contains("Test CPU, 4 CPUs") && svg.contains("BLAKE3 servil 0123456"), "{svg}");
    }
}
