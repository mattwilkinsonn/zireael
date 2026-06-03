//! `jj-gt init` — interactive setup that optionally installs shell
//! aliases, gt repo config reminders, and jjui actions/bindings so
//! `jj-gt submit / fetch / track / reconcile` are reachable from
//! inside [jjui](https://github.com/idursun/jjui).
//!
//! Mirrors the shape of `jj-hooks::init` so the two tools are
//! ergonomically symmetric — same prompter trait, same outcome
//! shape, same `add_jjui_actions` pure entry-point. The jjui config
//! merge logic is duplicated rather than shared because extracting a
//! generic spec system into `jj-hooks` would widen its published-
//! crate API for a one-time symmetry win.

use std::path::{Path, PathBuf};

use crate::error::{JjGtError, Result};

pub trait Prompter {
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool>;
}

/// A prompter that returns pre-canned answers in order. Used in tests.
pub struct ScriptedPrompter {
    answers: Vec<bool>,
    cursor: usize,
}

impl ScriptedPrompter {
    pub fn new(answers: Vec<bool>) -> Self {
        Self { answers, cursor: 0 }
    }
}

impl Prompter for ScriptedPrompter {
    fn confirm(&mut self, _message: &str, _default: bool) -> Result<bool> {
        let answer = *self
            .answers
            .get(self.cursor)
            .ok_or_else(|| JjGtError::Invalid("scripted prompter ran out of answers".into()))?;
        self.cursor += 1;
        Ok(answer)
    }
}

/// Interactive prompter backed by `dialoguer`.
pub struct InteractivePrompter;

impl Prompter for InteractivePrompter {
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool> {
        dialoguer::Confirm::new()
            .with_prompt(message)
            .default(default)
            .interact()
            .map_err(|e| JjGtError::Io(std::io::Error::other(format!("prompt failed: {e}"))))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitPlan {
    pub install_jjui_actions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AddedItems {
    pub added_submit: bool,
    pub added_submit_selected: bool,
    pub added_fetch: bool,
    pub added_track: bool,
    pub added_track_selected: bool,
    pub added_reconcile: bool,
    pub added_restack: bool,
    pub added_binding_submit: bool,
    pub added_binding_submit_selected: bool,
    pub added_binding_fetch: bool,
    pub added_binding_track: bool,
    pub added_binding_track_selected: bool,
    pub added_binding_reconcile: bool,
    pub added_binding_restack: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitOutcome {
    pub jjui_actions_added: AddedItems,
}

/// Print the same setup reminders the previous `jj-gt init` produced.
/// Kept as a separate entry-point so the interactive flow can call it
/// at the top, and shell scripts that just want the reminders can
/// invoke it through whatever invocation we end up exposing.
pub fn print_setup_reminders() {
    println!(
        "jj-gt setup reminders
=====================

Suggested zsh aliases:
    alias jgs='jj-gt submit'
    alias jgss='jj-gt submit --no-edit'
    alias jgf='jj-gt fetch'
    alias jgst='jj-gt status'
    alias jgr='jj-gt reconcile'

Tab completion (zsh):
    eval \"$(jj-gt completions zsh)\"

Tab completion (bash):
    eval \"$(jj-gt completions bash)\"

Tab completion (fish):
    jj-gt completions fish | source

Per-repo bootstrap:
    Run `gt init --trunk main` once per repo to create the
    .git/.graphite_repo_config sidecar that gt expects. jj-gt reads
    the trunk name from that file.
"
    );
}

/// Build an [`InitPlan`] by asking the user which optional
/// integrations to install.
pub fn plan(prompter: &mut dyn Prompter) -> Result<InitPlan> {
    let install_jjui_actions = prompter.confirm(
        "Install jjui actions/bindings so jj-gt submit / fetch / track / reconcile \
         are reachable from inside jjui?",
        false,
    )?;
    Ok(InitPlan {
        install_jjui_actions,
    })
}

/// Apply an [`InitPlan`] by merging jjui actions/bindings into the
/// jjui config file when requested.
///
/// `jjui_config_path`: where to merge. `None` resolves to the
/// canonical path (`$JJUI_CONFIG_DIR/config.toml` or
/// `~/.config/jjui/config.toml`).
pub fn apply(plan: &InitPlan, jjui_config_path: Option<&Path>) -> Result<InitOutcome> {
    let mut outcome = InitOutcome {
        jjui_actions_added: AddedItems::default(),
    };

    if plan.install_jjui_actions {
        let path = match jjui_config_path {
            Some(p) => p.to_path_buf(),
            None => default_jjui_config_path()?,
        };
        outcome.jjui_actions_added = apply_jjui_config(&path)?;
    }

    Ok(outcome)
}

/// Resolve the canonical jjui config path: `$JJUI_CONFIG_DIR/config.toml`
/// if set, otherwise `$XDG_CONFIG_HOME/jjui/config.toml`, otherwise
/// `~/.config/jjui/config.toml`.
fn default_jjui_config_path() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("JJUI_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            JjGtError::Io(std::io::Error::other(
                "neither JJUI_CONFIG_DIR, XDG_CONFIG_HOME, nor HOME is set",
            ))
        })?;
        PathBuf::from(home).join(".config")
    };
    Ok(base.join("jjui").join("config.toml"))
}

fn apply_jjui_config(path: &Path) -> Result<AddedItems> {
    let existing = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };

    let (merged, added) = add_jjui_actions(&existing)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, merged)?;
    Ok(added)
}

/// Merge jj-gt's actions + bindings into a jjui config TOML string.
///
/// Per-action lookup is name-based. Each action's `seq` follows the
/// same lowercase=selected, uppercase=all convention as the
/// jj-hooks 2026-05 swap:
///
///   x S → jj-gt-submit (whole stack)
///   x s → jj-gt-submit-selected (just the bookmark at focused commit)
///   x f → jj-gt-fetch
///   x T → jj-gt-track (whole stack)
///   x t → jj-gt-track-selected
///   x r → jj-gt-reconcile
///   x R → jj-gt-restack (rebase every local stack onto trunk)
///
/// Idempotent — running twice on the same input is a no-op.
pub fn add_jjui_actions(existing: &str) -> Result<(String, AddedItems)> {
    let mut doc: toml::Table = if existing.trim().is_empty() {
        toml::Table::new()
    } else {
        existing
            .parse()
            .map_err(|e: toml::de::Error| JjGtError::Invalid(format!("jjui config: {e}")))?
    };

    let mut added = AddedItems::default();

    let specs = action_specs();

    // -- Actions --
    let actions = doc
        .entry("actions")
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let actions_arr = actions
        .as_array_mut()
        .ok_or_else(|| JjGtError::Invalid("jjui config: `actions` is not an array".into()))?;

    for spec in specs {
        if !actions_has_name(actions_arr, spec.action_name) {
            let mut t = toml::Table::new();
            t.insert("name".into(), toml::Value::String(spec.action_name.into()));
            t.insert("lua".into(), toml::Value::String(spec.lua.into()));
            actions_arr.push(toml::Value::Table(t));
            set_added_action_flag(&mut added, spec.action_name);
        }
    }

    // -- Bindings --
    let bindings = doc
        .entry("bindings")
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let bindings_arr = bindings
        .as_array_mut()
        .ok_or_else(|| JjGtError::Invalid("jjui config: `bindings` is not an array".into()))?;

    for spec in specs {
        if !bindings_has_action(bindings_arr, spec.action_name) {
            bindings_arr.push(make_binding(
                spec.action_name,
                spec.seq,
                spec.scope,
                spec.desc,
            ));
            set_added_binding_flag(&mut added, spec.action_name);
        }
    }

    let serialized = toml::to_string_pretty(&doc)
        .map_err(|e| JjGtError::Invalid(format!("serializing jjui config: {e}")))?;

    Ok((serialized, added))
}

struct ActionSpec {
    action_name: &'static str,
    lua: &'static str,
    seq: &'static [&'static str],
    desc: &'static str,
    scope: &'static str,
}

fn action_specs() -> &'static [ActionSpec] {
    // Order matters: jjui's `x`-prefix overlay lists candidate
    // bindings in the order they appear here. Most-frequent
    // actions first so the menu reads top-down like a usage
    // frequency curve.
    static SPECS: &[ActionSpec] = &[
        // 1. submit-selected — the daily "ship my current
        //    bookmark" keystroke; same hand as `x` so it's the
        //    fastest reach.
        ActionSpec {
            action_name: "jj-gt-submit-selected",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"submit\", \"-r\", context.commit_id())\n  revisions.refresh()\n",
            seq: &["x", "s"],
            desc: "jj-gt submit selected bookmark(s)",
            scope: "revisions",
        },
        // 2. fetch — sync trunk + Graphite cleanup, run multiple
        //    times a day.
        ActionSpec {
            action_name: "jj-gt-fetch",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"fetch\")\n  revisions.refresh()\n",
            seq: &["x", "f"],
            desc: "jj-gt fetch",
            scope: "revisions",
        },
        // 3. track-selected — manual track invocation when a
        //    submit ran into trouble and you want to retry the
        //    track step in isolation.
        ActionSpec {
            action_name: "jj-gt-track-selected",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"track\", \"-r\", context.commit_id())\n  revisions.refresh()\n",
            seq: &["x", "t"],
            desc: "jj-gt track selected bookmark(s)",
            scope: "revisions",
        },
        // 4. submit (whole stack) — less frequent than submit-
        //    selected; usually `jj-gt submit --all` already kicks
        //    via the shell alias.
        ActionSpec {
            action_name: "jj-gt-submit",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"submit\")\n  revisions.refresh()\n",
            seq: &["x", "S"],
            desc: "jj-gt submit (whole stack)",
            scope: "revisions",
        },
        // 5. track (whole stack) — similar but rarer than its
        //    selected counterpart.
        ActionSpec {
            action_name: "jj-gt-track",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"track\")\n  revisions.refresh()\n",
            seq: &["x", "T"],
            desc: "jj-gt track (whole stack)",
            scope: "revisions",
        },
        // 6. reconcile — recovery flow; only reached when
        //    submit/fetch produced ambiguous state.
        ActionSpec {
            action_name: "jj-gt-reconcile",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"reconcile\")\n  revisions.refresh()\n",
            seq: &["x", "r"],
            desc: "jj-gt reconcile",
            scope: "revisions",
        },
        // 7. restack — explicit "rewrite every stack onto new
        //    trunk." Used when fetch deferred a conflicting
        //    rebase (#60) or when the user wants to opt into the
        //    full sync-with-restack cascade. Capital R so it
        //    doesn't collide with `r` (reconcile) and to signal
        //    the disruptive nature.
        ActionSpec {
            action_name: "jj-gt-restack",
            lua: "  jj_async(\"util\", \"exec\", \"--\", \"jj-gt\", \"restack\")\n  revisions.refresh()\n",
            seq: &["x", "R"],
            desc: "jj-gt restack (rebase all local stacks onto trunk)",
            scope: "revisions",
        },
    ];
    SPECS
}

/// Look up the right `AddedItems` field for `name` and set it to
/// `true`. Name-based instead of index-based so reordering
/// `action_specs()` doesn't silently misroute the flag.
fn set_added_action_flag(added: &mut AddedItems, name: &str) {
    match name {
        "jj-gt-submit" => added.added_submit = true,
        "jj-gt-submit-selected" => added.added_submit_selected = true,
        "jj-gt-fetch" => added.added_fetch = true,
        "jj-gt-track" => added.added_track = true,
        "jj-gt-track-selected" => added.added_track_selected = true,
        "jj-gt-reconcile" => added.added_reconcile = true,
        "jj-gt-restack" => added.added_restack = true,
        _ => unreachable!("unexpected action name in action_specs: {name}"),
    }
}

fn set_added_binding_flag(added: &mut AddedItems, name: &str) {
    match name {
        "jj-gt-submit" => added.added_binding_submit = true,
        "jj-gt-submit-selected" => added.added_binding_submit_selected = true,
        "jj-gt-fetch" => added.added_binding_fetch = true,
        "jj-gt-track" => added.added_binding_track = true,
        "jj-gt-track-selected" => added.added_binding_track_selected = true,
        "jj-gt-reconcile" => added.added_binding_reconcile = true,
        "jj-gt-restack" => added.added_binding_restack = true,
        _ => unreachable!("unexpected action name in action_specs: {name}"),
    }
}

fn actions_has_name(arr: &[toml::Value], name: &str) -> bool {
    arr.iter()
        .any(|v| v.get("name").and_then(|n| n.as_str()) == Some(name))
}

fn bindings_has_action(arr: &[toml::Value], action: &str) -> bool {
    arr.iter()
        .any(|v| v.get("action").and_then(|n| n.as_str()) == Some(action))
}

fn make_binding(action: &str, seq: &[&str], scope: &str, desc: &str) -> toml::Value {
    let mut t = toml::Table::new();
    t.insert("action".into(), toml::Value::String(action.into()));
    t.insert(
        "seq".into(),
        toml::Value::Array(
            seq.iter()
                .map(|s| toml::Value::String((*s).into()))
                .collect(),
        ),
    );
    t.insert("scope".into(), toml::Value::String(scope.into()));
    t.insert("desc".into(), toml::Value::String(desc.into()));
    toml::Value::Table(t)
}

/// Legacy print-only entry-point. Kept for backward-compat callers
/// (the old `Command::Init` path); the new interactive flow calls
/// `print_setup_reminders` directly.
pub fn print_init() {
    print_setup_reminders();
}
