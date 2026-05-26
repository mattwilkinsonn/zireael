# Draft `seal.toml` for zireael

When you `cd ~/repos/zireael` and want to launch a seal agent scoped to *just* the monorepo (rather than the parent `~/repos/seal.toml`), copy the contents below into `~/repos/zireael/seal.toml` manually. The sandbox doesn't let me write `seal.toml` files myself.

For the migration session you can keep using the parent `~/repos/seal.toml` — the `~/repos/zireael/**` paths are already covered by its `**` allow patterns.

---

```toml
#:schema ./.seal/schemas/seal.toml.json
schema_version = 1
name = "zireael"

[model]
provider = "anthropic"
name = "claude-opus-4-7"

[capabilities.allow]
additional_directories = [
  "~/.seal/",
  "~/.claude/",
  "~/other-repos/",
  "~/.config/",
  "~/notes/",
  "/tmp",
  "~/Desktop/",
  "~/scripts/",
]

[capabilities.allow.read]
default_files = ["*"]
paths = [
  "**",
  "~/other-repos/**",
  "~/notes/**",
  "/tmp/**",
  "~/.config/**",
  "~/scripts/**",
  "~/Desktop/**",
  "~/.seal/**",
]

[capabilities.allow.write]
default_files = [
  "*.rs",
  "*.toml",
  "*.md",
  "*.ts",
  "*.js",
  "*.json",
  "*.jsonc",
  "*.html",
  "*.css",
  "*.txt",
  "Justfile",
  ".envrc",
  "*.sh",
  "*.svg",
  "*.yml",
  "*.py",
  ".gitignore",
  "*.lock",
  "*.mts",
  "*.rb",
  "*.pkl",
]
paths = [
  "**",
  "~/notes/**",
  "/tmp/**",
  # LICENSE-MIT / LICENSE-APACHE have no extension — list them
  # explicitly so the dual-license boilerplate writes pass.
  { path = ".", files = ["LICENSE-MIT", "LICENSE-APACHE", "LICENSE"] },
]

[capabilities.allow.commands]
default_env_vars = [
  "SEAL_KEY_BACKEND",
  "JJ_HOOKS_LOG",
  "JJ_GT_LIVE_GH",
  "JJ_GT_LIVE_SUBMIT",
  "JJ_GT_LIVE_REPO",
  "JJ_GT_LIVE_REPO_URL",
  "CARGO_TERM_COLOR",
  "RUST_LOG",
  "RUST_BACKTRACE",
  "XDG_BIN_HOME",
  "XDG_DATA_HOME",
]
commands = [
  "cargo:*",
  "cargo nextest:*",
  "cargo fmt:*",
  "cargo clippy:*",
  "cargo build:*",
  "cargo check:*",
  "cargo binstall:*",
  "cargo set-version:*",
  "cargo publish:*",
  "rustup:*",
  "rustc:*",
  "bun:*",
  "bunx:*",
  "npm:*",
  "jj:*",
  "gh:*",
  "gt:*",
  "git log:*",
  "git diff:*",
  "git status:*",
  "git remote:*",
  "git rev-parse:*",
  "git cat-file:*",
  "git ls-remote:*",
  "git show:*",
  "git push:*",
  "git fetch:*",
  "git pull:*",
  "git checkout:*",
  "git switch:*",
  "git branch:*",
  "git add:*",
  "git commit:*",
  "git tag:*",
  "git rebase:*",
  "git mv:*",
  "git worktree:*",
  "git config:*",
  "hk:*",
  "pkl:*",
  "lefthook:*",
  "markdownlint-cli2:*",
  "actionlint:*",
  "just:*",
  "brew:*",
  "ls:*",
  "cat:*",
  "head:*",
  "tail:*",
  "wc:*",
  "grep:*",
  "sed:*",
  "awk:*",
  "find:*",
  "rm:*",
  "rmdir:*",
  "mkdir:*",
  "touch:*",
  "cp:*",
  "mv:*",
  "chmod:*",
  "echo:*",
  "pwd:*",
  "which:*",
  "readlink:*",
  "stat:*",
  "date:*",
  "diff:*",
  "tar:*",
  "shasum:*",
  "sha256sum:*",
  "curl:*",
  "python3:*",
  "linear:*",
]

[capabilities.allow.network]
domains = [
  "github.com",
  "*.github.com",
  "raw.githubusercontent.com",
  "objects.githubusercontent.com",
  "codeload.github.com",
  "api.graphite.com",
  "graphite.com",
  "*.graphite.com",
  "crates.io",
  "*.crates.io",
  "static.crates.io",
  "index.crates.io",
  "registry.npmjs.org",
  "*.npmjs.org",
  "pkl-lang.org",
  "*.pkl-lang.org",
  "releases.bun.sh",
  "deno.land",
  "*.docker.io",
  "linear.app",
  "*.linear.app",
]
```

After dropping this in, relaunch seal so the daemon picks up the new manifest.
