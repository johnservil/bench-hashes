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
