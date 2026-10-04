#!/bin/sh
# Libra's own benchmark (benchmark/run.sh) over three builds of Libra, to see
# what faster hashing does for it:
#   L0  Libra as it is (git-internal 0.10.2, crates.io blake3);
#   L1  git-internal built on the servil fork's BLAKE3 (its kernels alone);
#   L2  L1 with BLAKE3 object IDs hashed in place: no "<type> <size>\\0"
#       header, a key-derivation context per object type instead
#       (gi-noheader.patch, libra-noheader.patch).
#   L3  L2 with `add` replaying its object-index markers in batches at its end
#       instead of one queued update (a connection, a transaction, its syncs)
#       per object (libra-batch-index.patch).
# The runner gains --object-format and an add_all scenario (libra-runner.patch).
#   sh apps/libra-bench/run.sh FORK WORK OUT [RUNS]
# FORK: a checkout of github.com/johnservil/BLAKE3; WORK: a directory for the
# clones and builds; OUT: where the result files go.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
fork=$(cd "$1" && pwd)
mkdir -p "$2" "$3"
work=$(cd "$2" && pwd)
out=$(cd "$3" && pwd)
runs=${4:-5}
LIBRA=85dcbb7383092d914b65d2e7d2973b396613f23a
GIT_INTERNAL=0d368889eb79f1179244436141266586378c6416
export LIBRA_SKIP_WEB_BUILD=1 LC_ALL=C CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
fetch() { # URL COMMIT DIR
    [ -d "$3/.git" ] || { git init -q "$3"; git -C "$3" fetch -q --depth 1 "$1" "$2"; }
    git -C "$3" checkout -q --force "$2"
}
fetch https://github.com/libra-tools/libra.git $LIBRA "$work/libra"
fetch https://github.com/libra-tools/git-internal.git $GIT_INTERNAL "$work/gi-servil"
fetch https://github.com/libra-tools/git-internal.git $GIT_INTERNAL "$work/gi-noheader"
git -C "$work/libra" apply "$here/libra-runner.patch"
for gi in gi-servil gi-noheader; do
    sed -i.orig 's|^blake3 = "1.8"$|blake3 = { package = "blake3-servil", path = "'"$fork"'" }|' "$work/$gi/Cargo.toml"
    grep -q blake3-servil "$work/$gi/Cargo.toml"
done
git -C "$work/gi-noheader" apply "$here/gi-noheader.patch"
target=$work/target
build() { # NAME [git-internal dir]
    if [ $# -gt 1 ]; then
        (cd "$work/libra" && cargo build -q --release --config "patch.crates-io.git-internal.path=\"$2\"" --target-dir "$target")
    else
        (cd "$work/libra" && cargo build -q --release --locked --target-dir "$target")
    fi
    # Libra runs itself as a helper by its name, so each build keeps it.
    mkdir -p "$work/$1"
    cp "$target/release/libra" "$work/$1/libra"
}
build L0
build L1 "$work/gi-servil"
git -C "$work/libra" apply "$here/libra-noheader.patch"
build L2 "$work/gi-noheader"
git -C "$work/libra" apply "$here/libra-batch-index.patch"
build L3 "$work/gi-noheader"
git -C "$work/libra" apply -R "$here/libra-batch-index.patch"
git -C "$work/libra" apply -R "$here/libra-noheader.patch"
[ "${LIBRA_BENCH_BUILD_ONLY:-}" = 1 ] && { echo "libra-bench: built $work/L0/libra, L1, L2"; exit 0; }
scenarios="--scenario status_dirty --scenario fsck_history --scenario add_all"
bench() { # NAME FORMAT PASS
    (cd "$work/libra" && benchmark/run.sh --binary "$work/$1/libra" $scenarios --object-format "$2" --runs $runs --warmup 1 --output "$out/$1-$2-$3.json")
}
# BLAKE3 in the order L0 L2 L3 L3 L2 L0 (L1, the fork's kernels alone,
# measured level with L0 in job 1202), so drift falls on every build alike.
for pass in 1 2; do
    case $pass in 1) order="L0 L2 L3" ;; 2) order="L3 L2 L0" ;; esac
    for build in $order; do bench $build blake3 $pass; done
done
echo "libra-bench: results in $out"
