# Libra's own benchmark, with faster hashing

`run.sh` runs [Libra](https://github.com/libra-tools/libra)'s benchmark
(`benchmark/run.sh`) over three builds of Libra, from fixed commits of Libra
(85dcbb7) and git-internal (0d36888, 0.10.2):

- **L0**: Libra as it is (crates.io `blake3`);
- **L1**: git-internal built on the servil fork of BLAKE3, its assembly
  kernels alone;
- **L2**: L1 with BLAKE3 object IDs hashed in place: no `"<type> <size>\0"`
  header in what is hashed, a key-derivation context per object type
  instead (`gi-noheader.patch`, `libra-noheader.patch`; libra-tools/git-internal#183).

Libra's runner gains `--object-format` (fixtures in SHA-1, SHA-256, or BLAKE3)
and an `add_all` scenario, `libra add .` on 5,000 files of a source tree's
sizes (`libra-runner.patch`). Each build runs in the order L0 L1 L2 L2 L1 L0,
so drift falls on all alike; SHA-1 runs once, with L0, as Libra's default.

    sh apps/libra-bench/run.sh FORK WORK OUT [RUNS]

FORK is a checkout of github.com/johnservil/BLAKE3, WORK a directory for the
clones and builds (several GB), OUT the directory for the result files (one
JSON file per build, format, and pass, in Libra's own format). Needs what
Libra's runner needs: bash, cargo, perl, unzip, and `/usr/bin/time`.

## Results (Apple M4 Max, mains power, October 4, 2026; `results/AppleM4Max/`)

Medians of 5 runs, ms, each BLAKE3 build twice (passes 1, 2):

| Build | status_clean | status_dirty | log_history | fsck_history | add_all |
|---|---|---|---|---|---|
| L0, SHA-1 | 428 | 430 | 119 | 5,446 | 9,663 |
| L0, BLAKE3 | 423, 417 | 425, 419 | 119, 117 | 5,521, 5,567 | 9,781, 9,675 |
| L1, fork kernels | 418, 414 | 426, 421 | 119, 118 | 5,485, 5,506 | 9,753, 9,700 |
| L2, IDs in place | 420, 415 | 419, 419 | 117, 118 | 5,166, 5,142 | 8,461, 8,530 |

SHA-1 and BLAKE3 take the same time, and the fork's kernels change nothing:
hashing is a small part of each scenario. Hashing in place saves 7% in `fsck`
and 12% in `add`, by the copies it removes (each object's contents copied
behind its header into a new buffer before hashing). In a Linux VM on the
same Mac, `perf` and `/usr/bin/time` show where the time goes: `add .` on
5,000 files made 530,230 voluntary context switches (about 106 per file),
`fsck` over about 3,000 objects 989,764 (about 330 per object), with the
kernel's thread wake-ups 14% and 34% of their samples; BLAKE3 was 2% of
`add`'s.
