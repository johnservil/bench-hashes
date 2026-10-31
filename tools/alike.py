#!/usr/bin/env python3
"""Two measurements of the same calls in one run: the continuous cells
with buffers owned and lent, wherever they run the same calls on the same
input (bench-hashes' owned_runs_lent: every contender but the fork's
stream, at messages up to 64 KiB and at batches).

    python3 tools/alike.py RUN.tsv...

For each such pair, the owned cell's mean against the lent cell's
(permille), then by file the pairs' median, 90th percentile, and largest
departure from 1000: how far one run's cells stray from what they
measure, a run's own A/A. (Samples read through tools/samples.py.)
"""
import sys

import samples

PIECE = 64 << 10
PAIRS = [("ContinuousMessages", "LentMessages"), ("ContinuousBatches", "LentBatches")]


def size(label):
    n, _, unit = label.partition(" ")
    return int(n) * {"": 1, "B": 1, "KiB": 1 << 10, "MiB": 1 << 20}[unit]


def main(paths):
    for path in paths:
        run = samples.read(path)
        print(f"# {path}")
        print("contender\tscenario\tuse case\tpoint\towned against lent (permille)")
        departures = []
        for (contender, scenario, use_case, label), cell in sorted(run.cells.items()):
            for owned, lent in PAIRS:
                # owned_runs_lent, as bench-hashes says it (keep the two alike).
                if use_case != owned or contender == "blake3-servil-mt":
                    continue
                if owned == "ContinuousMessages" and size(label) > PIECE:
                    continue
                twin = run.cells.get((contender, scenario, lent, label))
                if twin is None:
                    continue
                a, b = samples.mean_ps(cell), samples.mean_ps(twin)
                permille = (a * 1000 + b // 2) // b
                print(f"{contender}\t{scenario}\t{owned}\t{label}\t{permille}")
                departures.append(abs(permille - 1000))
        departures.sort()
        if departures:
            n = len(departures)
            print(f"# {n} pairs: departure from 1000 median {departures[n // 2]} permille, "
                  f"90th percentile {departures[n * 9 // 10]}, largest {departures[-1]}")


if __name__ == "__main__":
    assert len(sys.argv) > 1, __doc__
    main(sys.argv[1:])
