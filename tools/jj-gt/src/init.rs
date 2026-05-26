//! `jj-gt init` — prints suggested aliases + setup reminders. Doesn't
//! write to any config files; the user copies what they want.

pub fn print_init() {
    println!(
        "jj-gt setup reminders
=====================

Suggested zsh aliases:
    alias jgs='jj-gt submit --all'
    alias jgss='jj-gt submit --all --no-edit'
    alias jgf='jj-gt fetch'
    alias jgst='jj-gt status'

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
