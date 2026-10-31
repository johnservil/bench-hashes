#!/usr/bin/env python3
"""How each nonstop cell's samples move within a visit, for the
continuous-load proposal's tests (FROZEN.md, October 31, 2026).

    python3 tools/settle.py RUN.tsv...

A nonstop cell's samples are its visits' trains, in order (the file's
`nonstop train` line gives S samples a visit); in a settling run
(`--train 0,N --nonstop-visits 1`) a cell's one train starts from the
machine as the contender before it left it. For each nonstop cell, solo
and shared (each step the two copies' samples together), its warm-up
curve: the time a unit over the train's first sample and over its first
5, 20, 50, 100, 200, 500 and 1000 ms (each taking the samples that start
within it), each against the train's last third, in permille, averaged
over its trains. A cell that needs no warm-up reads about 1000 from the
first sample on; where its curve reaches 1000 is the warm-up it needs.
Then, by use case, each column's median and largest. (Samples read
through tools/samples.py.)
"""
import sys

import samples

SPANS_MS = [5, 20, 50, 100, 200, 500, 1000]
AFTER_GAP = {"OneMessage", "ManyMessages", "ManyLongMessages", "IdleOneMessage", "IdleManyMessages",
             "InterleavedTurn", "IdleInterleavedTurn"}


def permille(part, tail):
    (ns, units), (tns, tunits) = part, tail
    return (ns * tunits * 1000 + units * tns // 2) // max(units * tns, 1)


def total(steps):
    return (sum(n for n, _ in steps), sum(u for _, u in steps))


def curve(steps):
    """A train's first sample, and the samples starting within each span,
    against its last third (permille); None for a span past the train."""
    tail = total(steps[len(steps) - -(-len(steps) // 3):])
    out = [permille(steps[0], tail)]
    length = sum(n for n, _ in steps)
    for span in SPANS_MS:
        span_ns = span * 1_000_000
        if span_ns > length:
            out.append(None)
            continue
        at = taken = 0
        while taken < len(steps) and at < span_ns:
            at += steps[taken][0]
            taken += 1
        out.append(permille(total(steps[:taken]), tail))
    return out


def main(paths):
    for path in paths:
        run = samples.read(path)
        train = run.headers.get("nonstop train")
        assert train, f"{path}: a `nonstop train` line (bench-hashes 0.19 or later)"
        per_visit = int(train.split(", ")[1].removesuffix(" samples a visit"))
        print(f"# {path}: nonstop train {train}")
        print("# the time a unit over each part of a train against its last third, permille (1000: as settled)")
        print("contender\tscenario\tuse case\tpoint\ttrains\tfirst sample\t" + "\t".join(f"first {s} ms" for s in SPANS_MS))
        groups = {}
        for (contender, scenario, use_case, label), cell in run.cells.items():
            if use_case in AFTER_GAP:
                continue
            steps = [(a[0] + b[0], a[1] + b[1]) for a, b in zip(cell[::2], cell[1::2])] if scenario == "shared" else cell
            assert len(steps) % per_visit == 0, f"{path}: {contender} {use_case} {label}: whole trains"
            curves = [curve(steps[i:i + per_visit]) for i in range(0, len(steps), per_visit)]
            mean = []
            for k in range(len(curves[0])):
                v = [c[k] for c in curves]
                mean.append(None if None in v else (sum(v) + len(v) // 2) // len(v))
            print(f"{contender}\t{scenario}\t{use_case}\t{label}\t{len(curves)}\t" + "\t".join("-" if v is None else str(v) for v in mean))
            groups.setdefault(f"{use_case} {scenario}", []).append(mean)
        print("# by use case: each column's median / largest, permille: first sample, first " + ", ".join(map(str, SPANS_MS)) + " ms")
        for group, means in groups.items():
            cols = []
            for k in range(len(SPANS_MS) + 1):
                v = sorted(m[k] for m in means if m[k] is not None)
                cols.append(f"{v[len(v) // 2]} / {v[-1]}" if v else "-")
            print(f"# {group}: {len(means)} cells: {', '.join(cols)}")


if __name__ == "__main__":
    assert len(sys.argv) > 1, __doc__
    main(sys.argv[1:])
