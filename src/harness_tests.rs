//! Checks of the benchmark's own work, independent of speed: what a timed
//! interval holds besides the hash (zeroing), the realized
//! contender orders, what the guide and output directory show, and the
//! official crate's batch wrapper.
use super::*;

#[test]
fn asynchronous_labels_describe_the_measured_api() {
    for (use_case, api) in [
        (UseCase::ContinuousBatches, "Queue::fixed"),
        (UseCase::LentPieces, "Hasher::update_multithreaded"),
    ] {
        let kernels = detect_kernels(Algorithm::Blake3ServilMt, use_case);
        assert_eq!(kernels.platform, "API (kernel unreported)");
        assert_eq!(kernels.kernels.len(), 1);
        assert_eq!(kernels.kernels[0].name, api);
        assert_eq!(kernels.kernels[0].first, 0);
    }
    let messages = detect_kernels(Algorithm::Blake3ServilMt, UseCase::ContinuousMessages);
    assert_eq!(messages.platform, "API (kernel unreported)");
    assert_eq!(messages.kernels.len(), 2);
    assert_eq!(messages.kernels[messages.kernel_index_for(PIECE_LEN)].name, "Queue::messages");
    assert_eq!(messages.kernels[messages.kernel_index_for(PIECE_LEN + 1)].name, "Queue::pieces");
    assert!(messages.kernels.iter().all(|kernel| kernel.why.contains("unreported")));
}

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
        let expected: Vec<u8> = input.chunks_exact(MESSAGE_LEN)
            .flat_map(|message| *blake3::hash(message).as_bytes()).collect();
        let mut actual = Vec::new();
        blake3_batch(&input, MESSAGE_LEN, 2, |digests| actual.extend_from_slice(digests));
        assert_eq!(actual, expected.repeat(2), "{messages} separate hashes, in input order");
    }
}

#[test]
fn sparse_runs_remove_previous_visualizations() {
    let directory = std::env::temp_dir().join(format!("bench-hashes-audit-output-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    for name in ["bench-hashes.graph.svg", "bench-hashes.guide.html", "bench-hashes.result.txt"] {
        fs::write(directory.join(name), "previous run").unwrap();
    }
    remove_visualizations(&directory);
    assert!(!directory.join("bench-hashes.graph.svg").exists());
    assert!(!directory.join("bench-hashes.guide.html").exists());
    assert!(directory.join("bench-hashes.result.txt").exists());
    remove_visualizations(&directory); // First sparse run has neither file.
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guide_means_match_the_exact_report_rounding() {
    let point = UseCase::LentPieces.points().start;
    let roster = Roster::new(vec![Algorithm::Blake3ServilSt, Algorithm::Sha256Ring], false, Some(vec![point]), Some(2));
    let statistics = summarize_measured(&[Measured::new(2135, 400), Measured::new(2135, 400)]);
    let mut results = vec![vec![None; POINT_COUNT]; roster.len()];
    for cells in &mut results {
        cells[point] = Some(super::Cell { solo: statistics, shared: Some(statistics) });
    }
    let guide = generate_guide(&roster, &results, &machine_metadata());
    assert!(guide.contains("\"mean\":[5.338]"), "the report rounds 2135/400 to 5.338; the guide must too");
}

#[test]
fn sampled_visits_complete_the_participating_williams_design() {
    // Enumerate roster sizes, use cases, quick/full counts, and every
    // point offset. Include contenders absent from some use cases.
    for n in 2..=Algorithm::ALL.len() {
        for use_case in UseCase::ALL {
            let orders = participating_orders(&Algorithm::ALL[..n], use_case);
            if orders.is_empty() { continue; }
            let participants = &orders[0];
            for rounds in [QUICK_ROUNDS, FULL_ROUNDS] {
                for offset in 0..POINT_COUNT {
                    let mut realized = Vec::new();
                    for round in 0..rounds {
                        if cell_wants_sample(round + offset, rounds, orders.len()) {
                            realized.push(orders[realized.len() % orders.len()].clone());
                        }
                    }
                    assert_eq!(realized.len(), STEADY_SAMPLES.next_multiple_of(orders.len()));
                    let normalized: Vec<Vec<usize>> = realized.iter().map(|row| row.iter()
                        .map(|a| participants.iter().position(|p| p == a).unwrap()).collect()).collect();
                    if participants.len() > 1 {
                        assert_orders_balanced(&normalized, participants.len());
                    }
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
        for orders in 1..=18 {
            for offset in [0, 1, 7, 95] {
                let count = (0..rounds).filter(|r| cell_wants_sample(r + offset, rounds, orders)).count();
                assert_eq!(count, STEADY_SAMPLES.next_multiple_of(orders).min(rounds));
            }
        }
    }
}

#[test]
fn regress_measures_the_nonstop_use_cases_by_their_points_names() {
    for name in REGRESS_POINTS {
        let point = POINTS[point_named(name)];
        assert!(!point.use_case.after_gap(), "{name}: regress measures nonstop cells alone (layout luck after a gap)");
    }
    assert!(REGRESS_POINTS.iter().all(|name| matches!(POINTS[point_named(name)].use_case, UseCase::LentMessages | UseCase::LentPieces | UseCase::LentBatches)), "regress judges the lent cells alone");
    assert_eq!(REGRESS_MARGIN_PERMILLE, 30);
}
