#!/usr/bin/env python3
"""Check a run's text report against its samples, independently of the Rust.

    python3 tools/check-report.py benchmark-results/AppleM4Max.darwin25

Reads bench-hashes.samples.tsv (samples v3: each sample as measured,
ns/units) as exact fractions, recomputes every table cell of
bench-hashes.result.txt with the benchmark's rules, and compares them with
the report: the median (the middle sample, or the mean of the two middle
ones), the two-speed split (the widest gap between neighbouring sorted
samples of at least 4% of the median, with a tenth of the samples or more
on each side, and the slower side's median at least 1.25x the faster's,
in permille rounded half up), each figure shown in nanoseconds with three
decimals or more until three significant digits show (at most six),
rounded half up, and the ~ mark (a speed whose 95% bootstrap interval of
the median spans 5% of it or more; the bootstrap is the benchmark's:
SplitMix64 seeded by the sample count, 400 resamples, indices by the high
half of a 64 x 64-bit product). Exits 1 on the first cell that differs.
"""
import re
import sys
from fractions import Fraction
from pathlib import Path

HEADINGS = {
    "One input at a time": "OneMessage",
    "Batches of 64-byte messages": "ManyMessages",
    "Batches of 256-byte messages": "ManyMessages256",
    "One input arriving in 64 KiB pieces": "Streaming",
}
MASK = (1 << 64) - 1


def load(path):
    """{(contender, scenario, use_case, point): [Fraction ns per unit]}, and the contenders in order."""
    cells, order, header = {}, [], None
    for line in path.read_text().splitlines():
        if line.startswith("#"):
            assert not line.startswith("# bench-hashes samples v") or line == "# bench-hashes samples v3", line
            continue
        fields = line.split("\t")
        if header is None:
            header = fields
            assert header == ["contender", "scenario", "use_case", "point", "unit", "ns/units"], header
            continue
        contender, scenario, use_case, point, _unit, values = fields
        if contender not in order:
            order.append(contender)
        cells[(contender, scenario, use_case, point)] = [Fraction(*map(int, v.split("/"))) for v in values.split(",")]
    return cells, order


def median(sorted_values):
    n = len(sorted_values)
    return sorted_values[n // 2] if n % 2 else (sorted_values[n // 2 - 1] + sorted_values[n // 2]) / 2


def bootstrap(sorted_values):
    """(low, high): the 2.5th and 97.5th percentiles of 400 resampled medians."""
    n = len(sorted_values)
    state = n

    def next_value():
        nonlocal state
        state = (state + 0x9E3779B97F4A7C15) & MASK
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    medians = []
    for _ in range(400):
        # Resample ranks: the sorted resample is the sorted ranks' values.
        ranks = sorted((next_value() * n) >> 64 for _ in range(n))
        medians.append(median([sorted_values[r] for r in ranks]))
    medians.sort()
    return medians[10], medians[390]


def permille(a, b):
    """a / b in permille, rounded half up."""
    return int(a * 1000 / b + Fraction(1, 2))


def speeds(values):
    """[(median, low, high)] for one speed or two, faster first."""
    v = sorted(values)
    n, whole = len(v), median(v)
    side = max(1, -(-n * 100 // 1000))
    best = None
    for split in range(side, n - side + 1):
        gap = v[split] - v[split - 1]
        if gap * 1000 >= whole * 40 and (best is None or gap > best[1]):
            best = (split, gap)
    if best:
        parts = [v[:best[0]], v[best[0]:]]
        if permille(median(parts[1]), median(parts[0])) >= 1250:
            return [(median(p), *bootstrap(p)) for p in parts]
    return [(whole, *bootstrap(v))]


def shown(ns):
    """Nanoseconds as the report shows them."""
    decimals = 3
    while decimals < 6 and ns < Fraction(1, 10 ** (decimals - 2)):
        decimals += 1
    rounded = int(ns * 10 ** decimals + Fraction(1, 2))
    return f"{rounded // 10 ** decimals}.{rounded % 10 ** decimals:0{decimals}d}"


def expected(values):
    found = speeds(values)
    wide = any(permille(high - low, mid) >= 50 for mid, low, high in found)
    return "|".join(shown(mid) for mid, _, _ in found) + ("~" if wide else "")


def main():
    record = Path(sys.argv[1])
    cells, order = load(record / "bench-hashes.samples.tsv")
    report = (record / "bench-hashes.result.txt").read_text().splitlines()
    scenario, use_case, columns, checked = None, None, None, 0
    for line in report:
        if line.startswith("SOLO:") or line.startswith("SHARED:"):
            scenario = "solo" if line.startswith("SOLO:") else "shared"
            continue
        if line.startswith(("CHECKS", "TWO SPEEDS", "KERNELS", "PROVENANCE")):
            scenario = None
        if scenario is None:
            continue
        heading = re.match(r"  (.*) \((ns/B|ns/msg)\)$", line)
        if heading:
            use_case = HEADINGS[heading.group(1)]
            present = {key[0] for key in cells if key[1] == scenario and key[2] == use_case}
            columns = [c for c in order if c in present]
            continue
        if use_case is None or not line.strip() or line.strip().startswith(("size", "messages")):
            continue
        tokens = line.split()
        figures = tokens[-len(columns):]
        label = " ".join(tokens[:-len(columns)])
        for contender, figure in zip(columns, figures):
            key = (contender, scenario, use_case, label)
            want = expected(cells[key])
            if figure != want:
                print(f"check-report: {key}: the report shows {figure}, the samples give {want}")
                return 1
            checked += 1
    assert checked > 0, "no table cells found"
    print(f"check-report: all {checked} cells agree with the samples")
    return 0


if __name__ == "__main__":
    sys.exit(main())
