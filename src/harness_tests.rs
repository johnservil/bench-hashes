//! Checks of the benchmark's own work, independent of speed: what a timed
//! interval holds besides the hash (zeroing), the realized
//! contender orders, what the guide and output directory show, and the
//! official crate's batch wrapper.
use super::*;

#[test]
fn batch_digest_storage_survives_a_smaller_cell_without_zeroing() {
    let mut large = take_batch_digests(8192);
    large.fill([0xa5; 32]);
    let address = large.as_ptr();
    keep_batch_digests(large);
    let small = take_batch_digests(16);
    keep_batch_digests(small);
    let large = take_batch_digests(8192);
    assert_eq!(large.as_ptr(), address, "the allocation stays mapped");
    assert!(large.iter().all(|digest| *digest == [0xa5; 32]),
        "switching sizes must preserve the output space, rather than zeroing the larger cell's outputs inside its interval");
    keep_batch_digests(large);
}

#[test]
fn official_batch_wrapper_hashes_each_message_with_plain_api_flags() {
    for messages in [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 128, 1024] {
        let input = make_input_seeded(messages * MESSAGE_LEN, 17);
        let expected: Vec<u8> = input
            .chunks_exact(MESSAGE_LEN)
            .flat_map(|message| *blake3::hash(message).as_bytes())
            .collect();
        let mut actual = Vec::new();
        blake3_batch(&input, MESSAGE_LEN, None, 2, |digests| {
            actual.extend_from_slice(digests)
        });
        assert_eq!(
            actual,
            expected.repeat(2),
            "{messages} separate hashes, in input order"
        );
    }
}

#[test]
fn sparse_runs_remove_a_previous_guide() {
    let directory =
        std::env::temp_dir().join(format!("bench-hashes-audit-output-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    for name in ["bench-hashes.guide.html", "bench-hashes.result.txt"] {
        fs::write(directory.join(name), "previous run").unwrap();
    }
    remove_guide(&directory);
    assert!(!directory.join("bench-hashes.guide.html").exists());
    assert!(directory.join("bench-hashes.result.txt").exists());
    remove_guide(&directory); // First sparse run has no guide.
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guide_means_match_the_exact_report_rounding() {
    let point = UseCase::Interleaved.points().start;
    let roster = Roster::new(
        vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring],
        false,
        Some(vec![point]),
        Some(2),
        &TO_CHECK_FOR_REGRESSIONS,
    );
    let statistics = Mean::of(&[Measured::new(2135, 400), Measured::new(2135, 400)]);
    let mut results = vec![vec![None; POINT_COUNT]; roster.len()];
    for cells in &mut results {
        cells[point] = Some(super::Cell {
            solo: statistics,
            shared: Some(statistics),
        });
    }
    let guide = generate_guide(&roster, &results, &machine_metadata());
    assert!(
        guide.contains("\"mean\":[5.338]"),
        "the report rounds 2135/400 to 5.338; the guide must too"
    );
}

/// A point's visits walk its Williams orders in turn, so no order is more
/// than one visit ahead of another, and every row holds only contenders
/// that take part; the count is the use case's, whatever the order count.
#[test]
fn sampled_visits_walk_the_orders_in_turn() {
    for n in 2..=Algorithm::ALL.len() {
        for use_case in UseCase::ALL {
            let orders = participating_orders(&Algorithm::ALL[..n], use_case);
            if orders.is_empty() {
                continue;
            }
            for rounds in [QUICK_ROUNDS, FULL_ROUNDS] {
                for offset in [0, 1, 7, POINT_COUNT - 1] {
                    let mut realized = Vec::new();
                    for round in 0..rounds {
                        if cell_wants_sample(
                            round + offset,
                            rounds,
                            use_case.visits(&TO_MEASURE_SPEED),
                        ) {
                            realized.push(orders[realized.len() % orders.len()].clone());
                        }
                    }
                    assert_eq!(
                        realized.len(),
                        use_case.visits(&TO_MEASURE_SPEED).min(rounds)
                    );
                    let mut counts = vec![0usize; orders.len()];
                    for row in &realized {
                        counts[orders.iter().position(|o| o == row).unwrap()] += 1;
                    }
                    assert!(
                        counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1,
                        "{use_case:?}, {n} contenders: {counts:?}"
                    );
                    for row in realized {
                        assert!(row.iter().all(|&a| Algorithm::ALL[a].takes_part(use_case)));
                    }
                }
            }
        }
    }
}

#[test]
fn sample_schedule_handles_short_explicit_round_counts() {
    for rounds in 1..=96 {
        for _orders in 1..=18 {
            for offset in [0, 1, 7, 95] {
                let count = (0..rounds)
                    .filter(|r| cell_wants_sample(r + offset, rounds, 12))
                    .count();
                assert_eq!(count, 12.min(rounds));
            }
        }
    }
}

#[test]
fn regress_judges_at_three_percent() {
    assert_eq!(REGRESS_MARGIN_PERMILLE, 30);
}

/// Each digest in its slot, in turn (FROZEN.md's use of each hash): one at a
/// time, a shorter one (SHA-1's 20 bytes), and a run that wraps the array.
#[test]
fn use_digest_stores_each_digest_in_its_slot() {
    let digest = |i: usize| [(i % 251) as u8; 32];
    let start = DIGESTS.with_borrow(|(_, next)| *next);
    use_digest(&digest(start));
    use_digest(&[7u8; 20]);
    let run: Vec<u8> = (start + 2..start + 2 + DIGEST_SLOTS + 5)
        .flat_map(digest)
        .collect();
    use_digest(&run);
    DIGESTS.with_borrow(|(slots, next)| {
        assert_eq!(*next, start + 2 + DIGEST_SLOTS + 5);
        for i in start + 7..*next {
            assert_eq!(slots[i % DIGEST_SLOTS], digest(i), "digest {i}");
        }
    });
}

/// The provenance every samples file's header carries, in order (the main
/// run's, b3sum's and leaves' all write theirs with samples_header): the
/// keys before the run's own.
pub(crate) const PROVENANCE: [&str; 16] = [
    "timestamp",
    "bench-hashes version",
    "bench-hashes source",
    "git commit",
    "git clean status",
    "dependency lock",
    "rust compiler",
    "c compiler",
    "rustflags",
    "build target",
    "target features",
    "sample clock",
    "cpu type",
    "cpu count",
    "os type",
    "cpu identity",
];

/// A samples file's header keys, in order.
pub(crate) fn header_keys(text: &str) -> Vec<&str> {
    text.lines()
        .skip(1)
        .take_while(|line| line.starts_with('#'))
        .filter_map(|line| line.strip_prefix("# ")?.split_once(": ").map(|(k, _)| k))
        .collect()
}

/// samples_header: the provenance, then the run's own keys, then power and
/// load, in the form read_samples takes.
#[test]
fn every_samples_file_names_what_measured_and_where() {
    let machine = machine_metadata();
    let header = samples_header(
        &machine,
        &[
            ("rounds".to_owned(), "3".to_owned()),
            ("contender".to_owned(), "a: /bin/a".to_owned()),
        ],
    );
    let keys = header_keys(&header);
    assert_eq!(&keys[..PROVENANCE.len()], PROVENANCE, "{header}");
    assert_eq!(
        &keys[PROVENANCE.len()..PROVENANCE.len() + 2],
        ["rounds", "contender"],
        "{header}"
    );
    assert!(
        header.starts_with(&format!("{SAMPLES_VERSION}\n"))
            && keys.contains(&"power")
            && keys.contains(&"load"),
        "{header}"
    );
    let path = std::env::temp_dir().join(format!("samples-header-{}.tsv", std::process::id()));
    std::fs::write(&path, format!("{header}{SAMPLES_COLUMNS}\n")).unwrap();
    let file = read_samples(path.to_str().unwrap());
    std::fs::remove_file(&path).unwrap();
    assert!(file
        .headers
        .iter()
        .any(|(k, v)| k == "contender" && v == "a: /bin/a"));
}

/// A pipelined collection hashes every item, in order, each to its own
/// digest: for every contender taking part, the digests it hands on match
/// each item hashed alone (BLAKE3 servil's stream included).
#[test]
fn a_pipelined_collection_hashes_every_item() {
    blake3_servil::initialize_multithreaded(None);
    for point in &POINTS[UseCase::ContinuousCollection.points()] {
        let input = make_input(point.bytes);
        let items = items_of(*point, &input);
        let want: Vec<[u8; 32]> = items
            .iter()
            .map(|&(o, l)| *blake3_servil::hash(&input[o..o + l]).as_bytes())
            .collect();
        for algorithm in [
            Algorithm::Blake3ServilSt,
            Algorithm::Blake3ServilMt,
            Algorithm::Blake3,
        ] {
            let mut got: Vec<[u8; 32]> = Vec::new();
            hash_batch(algorithm, &input, *point, 2, |d| {
                got.push(d.try_into().unwrap())
            });
            assert_eq!(
                got.len(),
                2 * want.len(),
                "{}: every item, each pass",
                algorithm.key()
            );
            assert!(
                got[..want.len()] == want[..] && got[want.len()..] == want[..],
                "{}: {}: each item's digest, in order",
                algorithm.key(),
                point.label
            );
        }
    }
}

#[test]
fn a_merkle_tree_has_the_root_of_its_layers_hashed_node_by_node() {
    blake3_servil::initialize_multithreaded(None);
    /* The root, a node at a time with one plain call each: each leaf as a
     * leaf, then each parent of two children's digests as a parent. */
    fn root(leaves: &[u8], leaf_len: usize, hash: impl Fn(Node, &[u8]) -> Vec<u8>) -> Vec<u8> {
        let mut layer: Vec<Vec<u8>> = leaves
            .chunks(leaf_len)
            .map(|leaf| hash(Node::Leaf, leaf))
            .collect();
        while layer.len() > 1 {
            layer = layer
                .chunks(2)
                .map(|pair| {
                    hash(
                        Node::Parent,
                        &[pair[0].as_slice(), pair[1].as_slice()].concat(),
                    )
                })
                .collect();
        }
        layer.pop().unwrap()
    }
    /* RFC 6962's prefix, then the message, in one buffer. */
    let with = |node: Node, m: &[u8]| [&[node.prefix()][..], m].concat();
    let point = POINTS[UseCase::MerkleTree.points()][0];
    let input = make_input(point.bytes);
    let blake3_root = root(&input, MERKLE.leaf_len, |node, m| {
        blake3::keyed_hash(node.key(), m).as_bytes().to_vec()
    });
    let sha256_root = root(&input, MERKLE.leaf_len, |node, m| {
        Sha256::digest(with(node, m)).to_vec()
    });
    let sha1_root = root(&input, MERKLE.leaf_len, |node, m| {
        sha1_checked::Sha1::try_digest(with(node, m))
            .hash()
            .to_vec()
    });
    let sha3_root = root(&input, MERKLE.leaf_len, |node, m| {
        sha3::Sha3_256::digest(with(node, m)).to_vec()
    });
    for algorithm in Algorithm::ALL
        .into_iter()
        .filter(|a| a.availability().is_ok() && a.takes_part(UseCase::MerkleTree))
    {
        let want = match algorithm {
            Algorithm::Blake3 | Algorithm::Blake3ServilSt | Algorithm::Blake3ServilMt => {
                &blake3_root
            }
            Algorithm::Sha256 | Algorithm::Sha256Ring | Algorithm::Sha256CommonCrypto => {
                &sha256_root
            }
            Algorithm::Sha1Dc => &sha1_root,
            Algorithm::Sha3_256 => &sha3_root,
            Algorithm::Blake3Rayon | Algorithm::Blake3Commonware => {
                unreachable!("{} takes no part in a Merkle tree", algorithm.key())
            }
        };
        let mut roots: Vec<Vec<u8>> = Vec::new();
        hash_batch(algorithm, &input, point, 2, |r| roots.push(r.to_vec()));
        assert_eq!(
            roots,
            vec![want.clone(), want.clone()],
            "{}: the root of {} leaves, each tree",
            algorithm.key(),
            point.messages
        );
    }
}

#[test]
fn a_merkle_tree_hashes_leaves_and_parents_apart() {
    blake3_servil::initialize_multithreaded(None);
    /* Two child digests side by side, hashed as a parent and as a leaf:
     * were they equal, the pair would pass for a leaf. */
    let pair = make_input(64);
    for algorithm in Algorithm::ALL
        .into_iter()
        .filter(|a| a.availability().is_ok() && a.takes_part(UseCase::MerkleTree))
    {
        let digest = |node| {
            let mut out = Vec::new();
            hash_tree_layer(algorithm, &pair, 64, node, |d| out.extend_from_slice(d));
            out
        };
        assert_ne!(
            digest(Node::Leaf),
            digest(Node::Parent),
            "{}: a leaf and a parent of the same bytes",
            algorithm.key()
        );
    }
}

/// The two visit counts, side by side: a run to measure speed and one to
/// check for regressions. A cell after a gap takes more visits in a check,
/// where the noise is; nonstop cells the same in both; both fit a full run.
#[test]
fn visits_follow_what_the_run_is_for() {
    assert_eq!(UseCase::OneMessage.visits(&TO_MEASURE_SPEED), 12);
    assert_eq!(UseCase::OneMessage.visits(&TO_CHECK_FOR_REGRESSIONS), 24);
    assert_eq!(
        UseCase::LentMessages.visits(&TO_MEASURE_SPEED),
        UseCase::LentMessages.visits(&TO_CHECK_FOR_REGRESSIONS)
    );
    for use_case in UseCase::ALL {
        for schedule in [&TO_MEASURE_SPEED, &TO_CHECK_FOR_REGRESSIONS] {
            assert!(use_case.visits(schedule) <= FULL_ROUNDS);
        }
    }
}
