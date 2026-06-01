//! Tests for the `jj-gt init` jjui action+binding merge logic.

use jj_gt::init::{
    AddedItems, InitOutcome, InitPlan, Prompter, ScriptedPrompter, add_jjui_actions, apply, plan,
};

#[test]
fn plan_with_yes_sets_flag() {
    let mut p = ScriptedPrompter::new(vec![true]);
    let plan = plan(&mut p).unwrap();
    assert!(plan.install_jjui_actions);
}

#[test]
fn plan_with_no_unsets_flag() {
    let mut p = ScriptedPrompter::new(vec![false]);
    let plan = plan(&mut p).unwrap();
    assert!(!plan.install_jjui_actions);
}

#[test]
fn add_jjui_actions_to_empty_config_installs_all_six() {
    let (output, added) = add_jjui_actions("").unwrap();
    // All six actions + bindings should be added.
    assert!(added.added_submit);
    assert!(added.added_submit_selected);
    assert!(added.added_fetch);
    assert!(added.added_track);
    assert!(added.added_track_selected);
    assert!(added.added_reconcile);
    assert!(added.added_binding_submit);
    assert!(added.added_binding_submit_selected);
    assert!(added.added_binding_fetch);
    assert!(added.added_binding_track);
    assert!(added.added_binding_track_selected);
    assert!(added.added_binding_reconcile);

    let parsed: toml::Table = output.parse().unwrap();
    let action_names: Vec<&str> = parsed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    for expected in [
        "jj-gt-submit",
        "jj-gt-submit-selected",
        "jj-gt-fetch",
        "jj-gt-track",
        "jj-gt-track-selected",
        "jj-gt-reconcile",
    ] {
        assert!(
            action_names.contains(&expected),
            "missing action `{expected}` in {action_names:?}",
        );
    }
}

#[test]
fn add_jjui_actions_default_keymap_uses_lowercase_for_selected() {
    // Matches the jj-hp 2026-05 swap: lowercase = selected (common
    // case), uppercase = whole stack.
    let (output, _) = add_jjui_actions("").unwrap();
    let parsed: toml::Table = output.parse().unwrap();
    let bindings = parsed["bindings"].as_array().unwrap();

    let mut got: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for b in bindings {
        let action = b
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let seq: Vec<String> = b
            .get("seq")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        got.insert(action, seq);
    }
    assert_eq!(got["jj-gt-submit"], vec!["x", "S"]);
    assert_eq!(got["jj-gt-submit-selected"], vec!["x", "s"]);
    assert_eq!(got["jj-gt-fetch"], vec!["x", "f"]);
    assert_eq!(got["jj-gt-track"], vec!["x", "T"]);
    assert_eq!(got["jj-gt-track-selected"], vec!["x", "t"]);
    assert_eq!(got["jj-gt-reconcile"], vec!["x", "r"]);
}

#[test]
fn add_jjui_actions_selected_lua_passes_commit_id_revset() {
    // The selected variants must invoke `jj-gt submit -r <commit>`
    // (or track -r) so the bookmark at the focused commit is what
    // gets acted on. Using `-r` rather than `-b <name>` because
    // jjui's lua context exposes commit_id but not bookmark_name.
    let (output, _) = add_jjui_actions("").unwrap();
    let parsed: toml::Table = output.parse().unwrap();
    let actions = parsed["actions"].as_array().unwrap();
    let submit_selected_lua = actions
        .iter()
        .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("jj-gt-submit-selected"))
        .and_then(|a| a.get("lua").and_then(|v| v.as_str()))
        .expect("missing jj-gt-submit-selected");
    assert!(
        submit_selected_lua.contains(r#""-r", context.commit_id()"#),
        "expected -r context.commit_id() in lua:\n{submit_selected_lua}",
    );
    let track_selected_lua = actions
        .iter()
        .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("jj-gt-track-selected"))
        .and_then(|a| a.get("lua").and_then(|v| v.as_str()))
        .expect("missing jj-gt-track-selected");
    assert!(
        track_selected_lua.contains(r#""-r", context.commit_id()"#),
        "expected -r context.commit_id() in lua:\n{track_selected_lua}",
    );
}

#[test]
fn add_jjui_actions_idempotent_on_second_run() {
    let (first, _) = add_jjui_actions("").unwrap();
    let (second, added) = add_jjui_actions(&first).unwrap();
    assert_eq!(first, second, "second run should be a no-op");
    // Nothing was added on the second run.
    assert_eq!(added, AddedItems::default());
}

#[test]
fn add_jjui_actions_preserves_existing_user_actions() {
    let existing = r#"
[[actions]]
name = "my-custom"
lua = "print('hi')"

[[bindings]]
action = "my-custom"
seq = ["q"]
scope = "revisions"
desc = "quit"
"#;
    let (output, added) = add_jjui_actions(existing).unwrap();
    assert!(added.added_submit, "should still add jj-gt-submit");
    // User's existing entry must be preserved.
    assert!(output.contains(r#"name = "my-custom""#), "got:\n{output}");
    let parsed: toml::Table = output.parse().unwrap();
    let action_names: Vec<&str> = parsed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(action_names.contains(&"my-custom"));
    assert!(action_names.contains(&"jj-gt-submit"));
}

#[test]
fn apply_skips_when_install_flag_false() {
    let tmp = tempfile::tempdir().unwrap();
    let jjui_path = tmp.path().join("jjui").join("config.toml");
    let plan = InitPlan {
        install_jjui_actions: false,
    };
    let outcome: InitOutcome = apply(&plan, Some(&jjui_path)).unwrap();
    assert_eq!(outcome.jjui_actions_added, AddedItems::default());
    // File should not have been created.
    assert!(!jjui_path.exists());
}

#[test]
fn apply_writes_jjui_config_when_requested() {
    let tmp = tempfile::tempdir().unwrap();
    let jjui_path = tmp.path().join("jjui").join("config.toml");
    let plan = InitPlan {
        install_jjui_actions: true,
    };
    let outcome = apply(&plan, Some(&jjui_path)).unwrap();
    assert!(outcome.jjui_actions_added.added_submit);

    let written = std::fs::read_to_string(&jjui_path).unwrap();
    assert!(written.contains(r#"name = "jj-gt-submit""#));
    assert!(written.contains(r#"name = "jj-gt-fetch""#));
    assert!(written.contains(r#"name = "jj-gt-reconcile""#));
}

#[test]
fn scripted_prompter_runs_out_of_answers_errors() {
    let mut p = ScriptedPrompter::new(vec![]);
    let res = p.confirm("anything?", true);
    assert!(res.is_err(), "expected error when out of canned answers");
}
