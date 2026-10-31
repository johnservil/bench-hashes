#!/usr/bin/env python3
"""Where a nonstop cell's run-to-run spread comes from, and what more
visits or more samples a visit would buy (the continuous-load protocol's
settings, FROZEN.md, October 31, 2026).

    python3 tools/variance.py RUN.tsv RUN.tsv RUN.tsv...

Runs of one machine and one protocol (each file's `nonstop train` line:
S samples a visit, V visits a cell). For each nonstop cell, solo and
shared, the time a unit of each sample, visit and run, in permille of the
cell's mean over every run, splits into three parts (squared permille,
the classic nested estimates):
  within    sample to sample inside a visit
  visits    visit to visit inside a run, beyond what `within` explains
  runs      run to run, beyond what `visits` explains: placement, layout,
            the state a process settles into, which no visit count shrinks
and predicts the spread of a run's mean (its standard deviation, permille)
as runs + visits / V + within / (V S) for the settings given with
--settings V,S;V,S... (default: this run's, then 4,3 and 3,4 and 2,12),
each with its nonstop time a cell relative to this run's (V (W + S) for
W the warm-up in ms, a sample being about 1 ms). Then, by use case, the
median and 90th percentile of each prediction. A visit's samples follow
one another, so a drift inside a train counts as `within`: a prediction
for more samples a visit assumes they are independent, and is the less
certain; one for more visits is the better founded. (Samples read through
tools/samples.py; integers throughout, a square root at the end.)
"""
import math
import sys

import samples

AFTER_GAP = {"OneMessage", "ManyMessages", "ManyLongMessages", "IdleOneMessage", "IdleManyMessages",
             "InterleavedTurn", "IdleInterleavedTurn"}


def per_unit_ps(steps):
    ns = sum(n for n, _ in steps)
    units = sum(u for _, u in steps)
    return (ns * 1000 + units // 2) // units


def train_of(run):
    t = run.headers["nonstop train"]
    warm = int(t.split(" ms warm-up")[0])
    per_visit = int(t.split(", ")[1].removesuffix(" samples a visit"))
    return warm, per_visit


def components(cells_by_run, s):
    """cells_by_run: for each run, the cell's steps (whole visits of s).
    The nested ANOVA's mean squares on permille of the cell's grand mean."""
    runs = []
    for steps in cells_by_run:
        visits = [steps[i:i + s] for i in range(0, len(steps), s)]
        runs.append([[per_unit_ps([x]) for x in v] for v in visits])
    grand = sum(sum(sum(v) for v in r) for r in runs) // sum(sum(len(v) for v in r) for r in runs)
    pm = lambda x: (x * 1000 + grand // 2) // grand
    runs = [[[pm(x) for x in v] for v in r] for r in runs]
    R, V = len(runs), len(runs[0])
    assert all(len(r) == V for r in runs), "every run with the same visits"
    # sums of squares, scaled by s*V*R to stay integer-exact
    vmean = [[sum(v) for v in r] for r in runs]          # x s
    rmean = [sum(r) for r in vmean]                       # x s V
    tot = sum(rmean)                                      # x s V R
    ss_w = sum(sum(x * x for x in v) * s - sum(v) ** 2 for r in runs for v in r)                # x s
    ss_v = sum(sum(m * m for m in r) * V - sum(r) ** 2 for r in vmean)                         # x s^2 V
    ss_r = sum(m * m for m in rmean) * R - tot ** 2                                              # x (sV)^2 R
    # mean squares (squared permille, rounded down once each)
    ms_w = ss_w // (s * R * V * (s - 1))
    ms_v = ss_v // (s * V * R * (V - 1)) if V > 1 else ms_w
    ms_r = ss_r // (s * V * R * (R - 1))
    within = ms_w
    visits = max((ms_v - ms_w) // s, 0)
    runs_c = max((ms_r - ms_v) // (s * V), 0)
    return within, visits, runs_c


def spread(within, visits, runs_c, V, S):
    """A run mean's standard deviation, permille, in tenths."""
    return math.isqrt(100 * (runs_c + visits // V + within // (V * S)))


def tenths(x):
    return f"{x // 10}.{x % 10}"


def main(argv):
    settings = None
    if argv and argv[0] == "--settings":
        settings = [tuple(map(int, x.split(","))) for x in argv[1].split(";")]
        argv = argv[2:]
    runs = [samples.read(p) for p in argv]
    assert len(runs) >= 2, __doc__
    warm, s = train_of(runs[0])
    assert all(train_of(r) == (warm, s) for r in runs), "one protocol"
    keys = [k for k in runs[0].cells if k[2] not in AFTER_GAP]
    first = runs[0].cells[keys[0]]
    v_now = len(first) // s
    settings = settings or [(v_now, s), (4, 3), (3, 4), (2, 12), (6, 2)]
    print(f"# {len(runs)} runs, {warm} ms warm-up, {s} samples a visit, {v_now} visits; predicted spread of a run's mean, permille")
    print("contender\tscenario\tuse case\tpoint\twithin\tvisits\truns\t" + "\t".join(f"V{v}S{t}" for v, t in settings))
    groups = {}
    for key in keys:
        contender, scenario, use_case, label = key
        per_run = []
        for r in runs:
            cell = r.cells[key]
            steps = [(a[0] + b[0], a[1] + b[1]) for a, b in zip(cell[::2], cell[1::2])] if scenario == "shared" else cell
            per_run.append(steps)
        if any(len(x) != len(per_run[0]) or len(x) % s for x in per_run):
            continue
        w, v, rr = components(per_run, s)
        pred = [spread(w, v, rr, V, S) for V, S in settings]
        print(f"{contender}\t{scenario}\t{use_case}\t{label}\t{w}\t{v}\t{rr}\t" + "\t".join(tenths(p) for p in pred))
        groups.setdefault(f"{use_case} {scenario}", []).append((w, v, rr, pred))
    cost = lambda V, S: V * (warm + S)
    print("# cost a cell (nonstop time), permille of this run's: " + ", ".join(f"V{v}S{t} {cost(v, t) * 1000 // cost(v_now, s)}" for v, t in settings))
    print("# by use case: median / p90 of the predicted spread, permille, per setting; and of the three parts (sqrt)")
    allp = [[] for _ in settings]
    for g, rows in sorted(groups.items()):
        cols = []
        for k in range(len(settings)):
            x = sorted(r[3][k] for r in rows)
            allp[k] += x
            cols.append(f"{tenths(x[len(x) // 2])}/{tenths(x[(len(x) * 9) // 10])}")
        parts = []
        for k in range(3):
            x = sorted(math.isqrt(100 * r[k]) for r in rows)
            parts.append(tenths(x[len(x) // 2]))
        print(f"# {g}: {len(rows)} cells: " + ", ".join(f"V{v}S{t} {c}" for (v, t), c in zip(settings, cols)) + f"; parts within/visits/runs {', '.join(parts)}")
    print("# every nonstop cell: " + ", ".join(f"V{v}S{t} {tenths(sorted(x)[len(x)//2])}/{tenths(sorted(x)[(len(x)*9)//10])}" for (v, t), x in zip(settings, allp)))


if __name__ == "__main__":
    main(sys.argv[1:])
