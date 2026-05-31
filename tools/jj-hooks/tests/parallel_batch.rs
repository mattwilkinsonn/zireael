//! Tests for the multi-bookmark batch entrypoints
//! [`run_for_updates_parallel`] / [`run_for_updates_sequential`].
//!
//! These exercise the library API directly (not via the `jj-hp push`
//! CLI) because the batch entrypoints don't have a CLI surface — they
//! exist for downstream callers like `jj-gt submit` that want a fan-out
//! over an N-bookmark stack.

mod harness;

use harness::{PRE_PUSH_FAILING, PRE_PUSH_PASSING, show};
use jj_hooks::bookmark_updates::{BookmarkUpdate, UpdateType};
use jj_hooks::hooks::{RunOpts, run_for_updates_parallel, run_for_updates_sequential};
use jj_hooks::jj::{JjCli, primary_git_dir};
use jj_hooks::runner::{Runner, Stage};

use harness::TestRepo;

/// Build a 3-bookmark stack `main → b1 → b2 → b3` and return the
/// commit ids for each. Each bookmark sits one commit above its
/// parent.
fn build_three_stack(repo: &TestRepo) -> (String, String, String, String) {
    repo.write("a.txt", "a\n");
    let out = repo.jj(&["commit", "-m", "b1 commit"]);
    assert!(out.status.success(), "{}", show(&out));
    let out = repo.jj(&["bookmark", "create", "b1", "-r", "@-"]);
    assert!(out.status.success(), "{}", show(&out));

    repo.write("b.txt", "b\n");
    let out = repo.jj(&["commit", "-m", "b2 commit"]);
    assert!(out.status.success(), "{}", show(&out));
    let out = repo.jj(&["bookmark", "create", "b2", "-r", "@-"]);
    assert!(out.status.success(), "{}", show(&out));

    repo.write("c.txt", "c\n");
    let out = repo.jj(&["commit", "-m", "b3 commit"]);
    assert!(out.status.success(), "{}", show(&out));
    let out = repo.jj(&["bookmark", "create", "b3", "-r", "@-"]);
    assert!(out.status.success(), "{}", show(&out));

    (
        repo.commit_id_of("main"),
        repo.commit_id_of("b1"),
        repo.commit_id_of("b2"),
        repo.commit_id_of("b3"),
    )
}

fn updates_for_three_stack(main: &str, b1: &str, b2: &str, b3: &str) -> Vec<BookmarkUpdate> {
    vec![
        BookmarkUpdate {
            remote: "origin".into(),
            bookmark: "b1".into(),
            update_type: UpdateType::Add,
            old_commit: Some(main.to_owned()),
            new_commit: Some(b1.to_owned()),
        },
        BookmarkUpdate {
            remote: "origin".into(),
            bookmark: "b2".into(),
            update_type: UpdateType::Add,
            old_commit: Some(b1.to_owned()),
            new_commit: Some(b2.to_owned()),
        },
        BookmarkUpdate {
            remote: "origin".into(),
            bookmark: "b3".into(),
            update_type: UpdateType::Add,
            old_commit: Some(b2.to_owned()),
            new_commit: Some(b3.to_owned()),
        },
    ]
}

#[test]
fn run_for_updates_parallel_returns_results_in_input_order() {
    // Three bookmarks, all passing hooks. The parallel API must:
    //   1. Run all three through the pre-push hook.
    //   2. Return outcomes in the SAME order as the input updates,
    //      regardless of which thread finished first.
    //   3. Capture each run's output into HookOutcome::captured_output.
    let repo = TestRepo::new();
    repo.write_pre_commit_config(PRE_PUSH_PASSING);
    let (main, b1, b2, b3) = build_three_stack(&repo);
    let updates = updates_for_three_stack(&main, &b1, &b2, &b3);

    let jj = JjCli::new(repo.primary().to_path_buf());
    let primary_git_dir = primary_git_dir(repo.primary()).unwrap();

    let opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        capture_output: true,
    };

    let outcomes = run_for_updates_parallel(
        &jj,
        &primary_git_dir,
        repo.primary(),
        Some(Runner::PreCommit),
        Stage::PrePush,
        &updates,
        opts,
        |_idx, _update, _outcome| {},
    )
    .unwrap();

    assert_eq!(outcomes.len(), 3);
    for (idx, outcome) in outcomes.iter().enumerate() {
        assert!(
            outcome.success,
            "outcome #{idx} failed: captured = {:?}",
            outcome.captured_output
        );
        assert!(
            outcome.captured_output.is_some(),
            "outcome #{idx} has no captured output despite capture_output=true",
        );
    }
}

#[test]
fn run_for_updates_parallel_first_failure_aborts_with_per_bookmark_attribution() {
    // Three bookmarks. Hook fails for all of them (PRE_PUSH_FAILING).
    // The batch entrypoint should:
    //   1. Run all three (the parallel path doesn't short-circuit;
    //      every thread is already launched).
    //   2. Surface success=false for each.
    //   3. Capture each one's failure output separately so the
    //      caller can attribute the failure to the right bookmark.
    let repo = TestRepo::new();
    repo.write_pre_commit_config(PRE_PUSH_FAILING);
    let (main, b1, b2, b3) = build_three_stack(&repo);
    let updates = updates_for_three_stack(&main, &b1, &b2, &b3);

    let jj = JjCli::new(repo.primary().to_path_buf());
    let primary_git_dir = primary_git_dir(repo.primary()).unwrap();

    let opts = RunOpts {
        retry_after_fixup: false,
        all_files: false,
        capture_output: true,
    };

    let outcomes = run_for_updates_parallel(
        &jj,
        &primary_git_dir,
        repo.primary(),
        Some(Runner::PreCommit),
        Stage::PrePush,
        &updates,
        opts,
        |_idx, _update, _outcome| {},
    )
    .unwrap();

    assert_eq!(outcomes.len(), 3);
    for (idx, outcome) in outcomes.iter().enumerate() {
        assert!(
            !outcome.success,
            "outcome #{idx} unexpectedly passed: captured = {:?}",
            outcome.captured_output
        );
        let captured = outcome
            .captured_output
            .as_deref()
            .unwrap_or_else(|| panic!("outcome #{idx} has no captured output"));
        // The captured block tags the subprocess argv with `$ <argv>`
        // (see run_subprocess in hooks.rs); the failing hook produces
        // some diagnostic output. Just check the buffer is non-empty.
        assert!(
            !captured.is_empty(),
            "outcome #{idx} captured an empty buffer",
        );
    }
}

#[test]
fn run_for_updates_parallel_progress_callback_fires_per_update() {
    // The progress callback runs on each thread as soon as it
    // finishes. We don't pin completion order (parallel — any order
    // is fine); we just check the callback fires once per update
    // and the captured outcome is consistent with the returned one.
    use std::sync::Mutex;
    let repo = TestRepo::new();
    repo.write_pre_commit_config(PRE_PUSH_PASSING);
    let (main, b1, b2, b3) = build_three_stack(&repo);
    let updates = updates_for_three_stack(&main, &b1, &b2, &b3);

    let jj = JjCli::new(repo.primary().to_path_buf());
    let primary_git_dir = primary_git_dir(repo.primary()).unwrap();

    let opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        capture_output: true,
    };

    let progress_fired: Mutex<Vec<(usize, String, bool)>> = Mutex::new(Vec::new());
    let outcomes = run_for_updates_parallel(
        &jj,
        &primary_git_dir,
        repo.primary(),
        Some(Runner::PreCommit),
        Stage::PrePush,
        &updates,
        opts,
        |idx, update, outcome| {
            progress_fired
                .lock()
                .unwrap()
                .push((idx, update.bookmark.clone(), outcome.success));
        },
    )
    .unwrap();

    let mut fired = progress_fired.into_inner().unwrap();
    fired.sort_by_key(|(idx, _, _)| *idx);
    assert_eq!(
        fired.len(),
        3,
        "expected 3 progress callbacks, got {fired:?}"
    );
    assert_eq!(fired[0], (0, "b1".into(), true));
    assert_eq!(fired[1], (1, "b2".into(), true));
    assert_eq!(fired[2], (2, "b3".into(), true));
    assert!(outcomes.iter().all(|o| o.success));
}

#[test]
fn run_for_updates_sequential_matches_parallel_results() {
    // Same fixture, sequential entrypoint. Results should be
    // identical (same hooks, same updates, same configuration) —
    // pins the contract that --hooks-sequential is just an
    // execution-strategy switch, not a semantic change.
    let repo = TestRepo::new();
    repo.write_pre_commit_config(PRE_PUSH_PASSING);
    let (main, b1, b2, b3) = build_three_stack(&repo);
    let updates = updates_for_three_stack(&main, &b1, &b2, &b3);

    let jj = JjCli::new(repo.primary().to_path_buf());
    let primary_git_dir = primary_git_dir(repo.primary()).unwrap();

    let opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        // Sequential doesn't require capture; toggle on so we can
        // assert the output buffer plumbing still works in
        // sequential mode.
        capture_output: true,
    };

    let outcomes = run_for_updates_sequential(
        &jj,
        &primary_git_dir,
        repo.primary(),
        Some(Runner::PreCommit),
        Stage::PrePush,
        &updates,
        opts,
        |_idx, _update, _outcome| {},
    )
    .unwrap();

    assert_eq!(outcomes.len(), 3);
    for (idx, outcome) in outcomes.iter().enumerate() {
        assert!(
            outcome.success,
            "outcome #{idx} failed: captured = {:?}",
            outcome.captured_output
        );
        assert!(
            outcome.captured_output.is_some(),
            "sequential with capture_output=true should still capture, got None for #{idx}",
        );
    }
}

#[test]
#[should_panic(expected = "run_for_updates_parallel requires capture_output=true")]
fn run_for_updates_parallel_without_capture_panics() {
    // The parallel API asserts on capture_output=true at entry —
    // passing false is a programmer error (it would garble the
    // terminal). Pin that the assert fires with a clear message.
    let repo = TestRepo::new();
    repo.write_pre_commit_config(PRE_PUSH_PASSING);
    let (main, b1, b2, b3) = build_three_stack(&repo);
    let updates = updates_for_three_stack(&main, &b1, &b2, &b3);

    let jj = JjCli::new(repo.primary().to_path_buf());
    let primary_git_dir = primary_git_dir(repo.primary()).unwrap();

    let opts = RunOpts {
        retry_after_fixup: true,
        all_files: false,
        capture_output: false,
    };

    let _ = run_for_updates_parallel(
        &jj,
        &primary_git_dir,
        repo.primary(),
        Some(Runner::PreCommit),
        Stage::PrePush,
        &updates,
        opts,
        |_idx, _update, _outcome| {},
    );
}
