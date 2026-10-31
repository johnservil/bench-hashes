#!/usr/bin/env python3
"""Which points of the benchmark's size axes could go, losing least.

    python3 tools/points.py [--tolerance P,...] [--quantile Q] [--axes A,...] [--protect USECASE:LABEL...] RUN.tsv...

It reads the samples files through tools/samples.py. For each axis (a use case and a
scenario: OneMessage, IdleOneMessage, ManyMessages, IdleManyMessages,
ContinuousMessages, LentMessages, ContinuousBatches, LentBatches), every
series the runs measured (a run's contender in that use case and
scenario) gives its time a unit at each point. A dropped point is
predicted from its nearest kept neighbours by straight-line interpolation
of log time against log size, and its error is the ratio of the prediction
to the measurement. Points drop one at a time, always the one that keeps
the largest error over every dropped point of every series smallest,
while that largest error stays within the tolerance. The axis's two ends
and every protected point stay.

A protected point is kept for what no interpolation of today's results
can show: where a code path changes, or is likely to between now and 0.9
on some platform (a chunk, a SIMD width of chunks, the SME2 group, the
fork's split across threads, the GPU's lengths), and the lengths that are
no multiple of a chunk, where a kernel's tail runs (bench-hashes #63: a
generated x86 kernel 34.6% slower from 1152 to 1984 bytes, which no cell
showed until 1536 B was added). PROTECTED below lists them, each with its
reason; --protect adds more.

Output, per axis and tolerance: the points kept (protected ones marked),
and for each point dropped, its largest error and the series it came from.
"""
import argparse
import collections
import math
import re

import samples

AXES = ["OneMessage", "IdleOneMessage", "ManyMessages", "IdleManyMessages",
        "ContinuousMessages", "LentMessages", "ContinuousBatches", "LentBatches"]

# (use case or "*", label): why the point stays, whatever today's results.
PROTECTED = {
    ("OneMessage", "64 B"): "the overview's cell",
    ("OneMessage", "1 KiB"): "a chunk: one-chunk kernels end here; the overview's cell",
    ("OneMessage", "1536 B"): "a chunk and a half: the tail of a two-chunk input (bench-hashes #63)",
    ("OneMessage", "4 KiB"): "four chunks: 4-way NEON fills",
    ("OneMessage", "16 KiB"): "sixteen chunks: the SME2 group, AVX-512's 16 lanes",
    ("OneMessage", "512 KiB"): "servil mt splits across threads from here",
    ("OneMessage", "1 MiB"): "the overview's cell",
    ("OneMessage", "128 MiB"): "the overview's cell",
    ("IdleOneMessage", "1 KiB"): "a chunk",
    ("IdleOneMessage", "16 KiB"): "the SME2 group",
    ("IdleOneMessage", "512 KiB"): "servil mt's split",
    ("ManyMessages", "1024"): "the overview's cell",
    ("ManyMessages", "16"): "sixteen messages: servil's batch call switches; AVX-512's 16 lanes",
    ("ManyMessages", "2"): "two messages: servil's batch call switches",
    ("ManyMessages", "4"): "four messages: NEON's batch width",
    ("IdleManyMessages", "16"): "servil's batch switch",
    ("ContinuousMessages", "64 KiB"): "the overview's cell; a stream's piece",
    ("ContinuousMessages", "4 MiB"): "the GPU stream's shortest message (the Metal work)",
    ("ContinuousMessages", "64 MiB"): "the GPU stream's longest message",
    ("LentMessages", "64 KiB"): "a piece",
    ("LentMessages", "4 MiB"): "a GPU's lengths, should the lent calls take it",
}


def axis_value(label):
    """A point's place on its axis from its label: bytes ("1536 B", "3 MiB")
    or messages ("1024"); None for a label of no size ("git objects")."""
    m = re.fullmatch(r"(\d+)(?: (B|KiB|MiB))?", label)
    if not m:
        return None
    return int(m.group(1)) * {None: 1, "B": 1, "KiB": 1 << 10, "MiB": 1 << 20}[m.group(2)]


def means(paths):
    """{(machine, contender, scenario, use case): {axis value: (label, ps a unit)}}:
    each machine's runs of a cell averaged (the geometric mean of their
    means), so that a point's departure from its neighbours counts as it
    repeats over runs rather than as one run's noise (tools/alike.py: two
    cells of the same calls in one run differ by up to 40%)."""
    runs = collections.defaultdict(lambda: collections.defaultdict(list))
    labels = {}
    for path in paths:
        run = samples.read(path)
        for (contender, scenario, use_case, label), cell in run.cells.items():
            x = axis_value(label)
            if x is not None:
                runs[(samples.machine(path), contender, scenario, use_case)][x].append(samples.mean_ps(cell))
                labels[(use_case, x)] = label
    series = {}
    for key, points in runs.items():
        series[key] = {x: (labels[(key[3], x)], math.exp(sum(map(math.log, v)) / len(v))) for x, v in points.items()}
    counts = collections.Counter(samples.machine(f) for f in paths)
    print("# runs a machine: " + ", ".join(f"{m} {n}" for m, n in sorted(counts.items())))
    return series


def error_at(curves, kept, x):
    """Point x's largest interpolation error over the curves, from its
    nearest kept neighbours (permille), and the curve it came from."""
    result = (0, None)
    for key, curve in curves.items():
        if x not in curve:
            continue
        have = sorted(k for k in curve if k in kept)
        lo = max((k for k in have if k < x), default=None)
        hi = min((k for k in have if k > x), default=None)
        if lo is None or hi is None:
            return (math.inf, key)
        t = (math.log(x) - math.log(lo)) / (math.log(hi) - math.log(lo))
        pred = math.exp((1 - t) * math.log(curve[lo]) + t * math.log(curve[hi]))
        err = abs(pred / curve[x] - 1) * 1000
        if err > result[0]:
            result = (err, key)
    return result


def errors_at(curves, kept, x):
    """Point x's interpolation error (permille) in every curve that has it."""
    out = []
    for curve in curves.values():
        if x not in curve:
            continue
        have = sorted(k for k in curve if k in kept)
        lo = max((k for k in have if k < x), default=None)
        hi = min((k for k in have if k > x), default=None)
        if lo is None or hi is None:
            return [math.inf]
        t = (math.log(x) - math.log(lo)) / (math.log(hi) - math.log(lo))
        pred = math.exp((1 - t) * math.log(curve[lo]) + t * math.log(curve[hi]))
        out.append(abs(pred / curve[x] - 1) * 1000)
    return sorted(out)


def quantile(v, q):
    return v[min(len(v) - 1, int(q * len(v)))] if v else 0


def cost(curves, kept, xs, q):
    """The largest, over the dropped points, of a point's q-quantile error over the curves."""
    return max((quantile(errors_at(curves, kept, x), q) for x in xs if x not in kept), default=0)


def worst(curves, kept):
    """The largest interpolation error (permille away from 1000) over every
    dropped point of every curve, and where: (error, curve key, axis value)."""
    result = (0, None, None)
    for key, curve in curves.items():
        xs = sorted(curve)
        have = [x for x in xs if x in kept]
        for x in xs:
            if x in kept:
                continue
            lo = max((k for k in have if k < x), default=None)
            hi = min((k for k in have if k > x), default=None)
            if lo is None or hi is None:
                return (math.inf, key, x)
            t = (math.log(x) - math.log(lo)) / (math.log(hi) - math.log(lo))
            pred = math.exp((1 - t) * math.log(curve[lo]) + t * math.log(curve[hi]))
            err = abs(pred / curve[x] - 1) * 1000
            if err > result[0]:
                result = (err, key, x)
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tolerance", type=lambda v: [int(x) for x in v.split(",")], default=[30, 50, 100],
                    help="largest interpolation errors to accept, permille, A,B,... (default 30,50,100)")
    ap.add_argument("--protect", nargs="*", default=[], help="more points to keep, USECASE:LABEL")
    ap.add_argument("--quantile", type=int, default=100,
                    help="judge a dropped point by this percentile of its errors over the series (default 100: the largest)")
    ap.add_argument("--axes", type=lambda v: v.split(","), default=AXES, help="the use cases to study, A,B,...")
    ap.add_argument("runs", nargs="+")
    args = ap.parse_args()
    protected = dict(PROTECTED)
    for p in args.protect:
        uc, label = p.split(":", 1)
        protected[(uc, label)] = "named on the command line"
    series = means(args.runs)
    print(f"# {len(args.runs)} runs, {len(series)} series")
    q = args.quantile / 100
    for use_case in args.axes:
        for scenario in ["solo", "shared"]:
            curves = {k: {x: ps for x, (_, ps) in v.items()} for k, v in series.items()
                      if k[2] == scenario and k[3] == use_case and len(v) >= 3}
            if not curves:
                continue
            labels = {}
            for k, v in series.items():
                if k[2] == scenario and k[3] == use_case:
                    labels.update({x: label for x, (label, _) in v.items()})
            xs = sorted(labels)
            stay = {xs[0], xs[-1]} | {x for x in xs if (use_case, labels[x]) in protected}
            print(f"\n## {use_case} {scenario}: {len(xs)} points, {len(curves)} series, {len(stay)} kept by rule")
            for tol in args.tolerance:
                kept = set(xs)
                dropped = []
                while True:
                    best = None
                    for x in sorted(kept - stay):
                        c = cost(curves, kept - {x}, xs, q)
                        if c <= tol and (best is None or c < best[1]):
                            best = (x, c)
                    if best is None:
                        break
                    kept.discard(best[0])
                    dropped.append(best[0])
                # Each dropped point's error at the quantile, and its largest, with the final kept set.
                detail = []
                for x in sorted(dropped):
                    errs = errors_at(curves, kept, x)
                    err, key = error_at(curves, kept, x)
                    detail.append(f"{labels[x]} (p{args.quantile} {quantile(errs, q):.0f}, largest {err:.0f}: {key[1]}, {key[0][:24]})")
                show = [labels[x] + ("*" if x in stay else "") for x in xs if x in kept]
                print(f"  within {tol} permille: keep {len(kept)} of {len(xs)}: {', '.join(show)}")
                if detail:
                    print(f"    dropped (largest error, permille, and its series): {'; '.join(detail)}")


if __name__ == "__main__":
    main()
