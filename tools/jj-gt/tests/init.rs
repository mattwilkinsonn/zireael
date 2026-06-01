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
fn add_jjui_actions_orders_by_frequency() {
    // The menu order matters: jjui's `x`-prefix overlay surfaces
    // candidates top-down in the order they appear in the config.
    // Most-frequent operations first so the muscle-memory keystroke
    // is the shortest path through the menu.
    //
    // Pin the order here so a future reshuffle has to update both
    // the SPECS array in init.rs AND this explicit list.
    let (output, _) = add_jjui_actions("").unwrap();
    let parsed: toml::Table = output.parse().unwrap();
    let action_order: Vec<&str> = parsed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(
        action_order,
        vec![
            "jj-gt-submit-selected", // daily ship-my-bookmark
            "jj-gt-fetch",           // sync trunk + cleanup
            "jj-gt-track-selected",  // recovery for a single bookmark
            "jj-gt-submit",          // whole-stack submit (less common)
            "jj-gt-track",           // whole-stack track
            "jj-gt-reconcile",       // last-resort recovery
        ],
        "action order in SPECS drifted from frequency-ordered list",
    );
    let binding_order: Vec<&str> = parsed["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("action").and_then(|n| n.as_str()))
        .collect();
    assert_eq!(
        binding_order,
        vec![
            "jj-gt-submit-selected",
            "jj-gt-fetch",
            "jj-gt-track-selected",
            "jj-gt-submit",
            "jj-gt-track",
            "jj-gt-reconcile",
        ],
        "binding order in SPECS drifted from frequency-ordered list",
    );
}

#[test]
fn scripted_prompter_runs_out_of_answers_errors() {
    let mut p = ScriptedPrompter::new(vec![]);
    let res = p.confirm("anything?", true);
    assert!(res.is_err(), "expected error when out of canned answers");
}

#[test]
fn readme_toml_matches_generated_jjui_config() {
    // Drift guard: the README's "If you'd rather hand-edit ..."
    // TOML block must mirror what `add_jjui_actions("")` produces.
    // Asserts both the set of (action_name, seq) pairs AND the
    // order they appear in. Ordering matters because jjui's
    // `x`-prefix overlay surfaces candidates in the order they
    // appear in the config; reordering the SPECS array in the
    // code without updating the README would silently produce
    // different menu sort orders for the two install paths
    // (auto via `jj-gt init` vs hand-paste from README).
    let readme_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let readme = std::fs::read_to_string(&readme_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", readme_path.display()));

    // Find the first ```toml fenced block in the jjui-integration
    // section. The regex deliberately keeps the search greedy-on-
    // first-fence so a future second toml block doesn't shift it.
    let start_marker = "```toml\n";
    let start = readme
        .find(start_marker)
        .expect("no ```toml block in README");
    let body_start = start + start_marker.len();
    let end = readme[body_start..]
        .find("\n```")
        .expect("toml block has no closing fence");
    let readme_toml = &readme[body_start..body_start + end];

    let readme_parsed: toml::Table = readme_toml
        .parse()
        .unwrap_or_else(|e| panic!("parse README TOML: {e}\n---\n{readme_toml}\n---"));

    let readme_action_order: Vec<&str> = readme_parsed
        .get("actions")
        .and_then(|v| v.as_array())
        .expect("README TOML has no [[actions]]")
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    let readme_binding_order: Vec<(&str, Vec<&str>)> = readme_parsed
        .get("bindings")
        .and_then(|v| v.as_array())
        .expect("README TOML has no [[bindings]]")
        .iter()
        .filter_map(|v| {
            let action = v.get("action").and_then(|n| n.as_str())?;
            let seq: Vec<&str> = v
                .get("seq")
                .and_then(|n| n.as_array())?
                .iter()
                .filter_map(|s| s.as_str())
                .collect();
            Some((action, seq))
        })
        .collect();

    // Generate the canonical config from an empty input.
    let (generated, _) = add_jjui_actions("").unwrap();
    let generated_parsed: toml::Table = generated.parse().unwrap();
    let generated_action_order: Vec<&str> = generated_parsed["actions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    let generated_binding_order: Vec<(&str, Vec<&str>)> = generated_parsed["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| {
            let action = v.get("action").and_then(|n| n.as_str())?;
            let seq: Vec<&str> = v
                .get("seq")
                .and_then(|n| n.as_array())?
                .iter()
                .filter_map(|s| s.as_str())
                .collect();
            Some((action, seq))
        })
        .collect();

    assert_eq!(
        readme_action_order, generated_action_order,
        "README action ORDER drifted from generated config (jjui menu sort \
         order will differ between auto-install and copy/paste)",
    );
    assert_eq!(
        readme_binding_order, generated_binding_order,
        "README binding ORDER drifted from generated config",
    );
}
