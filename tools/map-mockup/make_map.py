#!/usr/bin/env python3
"""Draw the map of a run's results (mock-up 3): every measurement a small
chart, placed by what the program does, opening in place.

    python3 tools/map-mockup/make_map.py GRAPH.svg B3SUM.samples.tsv OUT.html

GRAPH.svg is a run's bench-hashes.graph.svg (its DATA block); the b3sum
samples file is `bench-hashes b3sum`'s. Each cell's value is its mean time
per unit, as the benchmark's own summaries take it."""
import json, re, sys
from fractions import Fraction

graph, b3sum, out = sys.argv[1:4]

d = json.loads(re.search(r'const DATA = (\{.*?\});\n', open(graph).read(), re.S).group(1))
# The hashing section: each plot's place, by what is hashed (row) and how
# the program calls (column).
PLACE = {"OneMessage": ("one", "busy"), "IdleOneMessage": ("one", "idle"), "LentMessages": ("one", "lent"), "ContinuousMessages": ("one", "piped"),
         "ManyMessages": ("batch", "busy"), "IdleManyMessages": ("batch", "idle"), "LentBatches": ("batch", "lent"), "ContinuousBatches": ("batch", "piped"),
         "Interleaved": ("many", "lent"), "Collection": ("coll", "lent"), "Outboard": ("outb", "lent")}
# Each cell names the call of BLAKE3 servil mt, the servil contender shown
# when the page opens, as every record since 0.13 makes it.
CALL = {"OneMessage": "hash_multithreaded", "IdleOneMessage": "hash_multithreaded", "LentMessages": "hash_multithreaded", "ContinuousMessages": "Queue::messages",
        "ManyMessages": "hash_many_multithreaded", "IdleManyMessages": "hash_many_multithreaded", "LentBatches": "hash_many_multithreaded", "ContinuousBatches": "Queue::fixed",
        "Interleaved": "update_multithreaded per piece", "Collection": "hash_multithreaded per item", "Outboard": "outboard_multithreaded_with"}
# Cells whose points are separate cases, drawn as dots.
CASES = {"Interleaved", "Collection"}
cells = {}
for p in d["plots"]:
    if p["use"] not in PLACE:
        continue
    row, col = PLACE[p["use"]]
    per = p["bytes"] if p["rateUnit"] == "GB/s" else p["x"]
    cells[f'{p["scenario"]}|{row}|{col}'] = {"sizes": p["sizes"], "unit": p["rateUnit"], "call": CALL[p["use"]], "cases": p["use"] in CASES,
        "per": per, "what": "a message" if p["rateUnit"] == "GB/s" else "a batch",
        "series": [None if s is None else [float(v) for v in s["med"]] for s in p["series"]]}
hashing = {
    "title": "How fast each hash runs, by what a program hashes and how it calls",
    "rows": [["one", "one message"], ["batch", "a batch of short messages"], ["many", "many messages at once, in pieces"],
             ["coll", "a collection of items"], ["outb", "a message with its outboard, for verified streaming"]],
    "cols": [["busy", "now and then, between other work"], ["idle", "now and then, after a pause"],
             ["lent", "nonstop, waiting for each call"], ["piped", "nonstop, pipelined"]],
    "layers": [["solo", "one program", ""], ["shared", "two programs at once", "measured nonstop"]],
    "names": d["names"], "colors": d["colors"], "shown": d["shown"], "cells": cells,
    "absent": "new; not in this record",
    "newRows": ["outb"],
}

# The b3sum section: each run's time from start to exit, a file or a tree of
# files (row), in the page cache or read from storage (column).
means, labels, units_of = {}, {}, {}
names = []
for line in open(b3sum):
    if line.startswith("#") or line.startswith("contender\t"):
        continue
    contender, cache, _, point, _, samples = line.rstrip("\n").split("\t")[:6]
    ns = units = 0
    for s in samples.split(","):
        a, b = s.split("/")
        ns += int(a); units += int(b)
    means[(contender, cache, point)] = ns / units
    units_of[point] = int(samples.split(",")[0].split("/")[1])
    if contender not in names:
        names.append(contender)
    labels.setdefault(cache, [])
    if point not in labels[cache]:
        labels[cache].append(point)
FILES = [p for p in labels["warm"] if re.fullmatch(r"\d+ [KMG]iB", p)]
TREES = [p for p in labels["warm"] if p not in FILES]
colors = {"official": d["colors"][d["names"].index("BLAKE3 official")], "servil": d["colors"][d["names"].index("BLAKE3 servil mt")]}
b3cells = {}
for cache, col in [("warm", "cached"), ("cold", "storage")]:
    for row, points in [("file", FILES), ("tree", TREES)]:
        if cache in labels:
            b3cells[f"solo|{row}|{col}"] = {"sizes": points, "unit": "GB/s", "call": "b3sum", "cases": row == "tree",
                "per": [units_of[p] for p in points], "what": "a run",
                "series": [[means[(c, cache, p)] for p in points] for c in names]}
b3 = {
    "title": "How fast b3sum hashes files, from its start to its exit",
    "rows": [["file", "a file"], ["tree", "a tree of files"]],
    "cols": [["cached", "in the page cache"], ["storage", "read from storage"]],
    "layers": [["solo", "one program", ""]],
    "names": [{"official": "b3sum 1.8.2 (official)", "servil": "b3sum, servil"}.get(n, n) for n in names],
    "colors": [colors.get(n, "#6b7280") for n in names], "shown": [True for n in names], "cells": b3cells,
    "absent": "", "newRows": [],
}

html = open(__file__.replace("make_map.py", "map3.template.html")).read()
open(out, "w").write(html.replace("@SECTIONS@", json.dumps([hashing, b3])))
print(f"map: {out}: {len(cells)} hashing charts, {len(b3cells)} b3sum charts")
