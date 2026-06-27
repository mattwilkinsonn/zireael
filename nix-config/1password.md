# 1Password Architecture

Canonical reference for the 1Password setup across all hosts in this
flake. Two accounts (personal + sealedsecurity team), vaults split by
security domain inside each, service accounts scope-limit
non-interactive access, `load-secrets` exports per-shell env vars from
two `op inject` passes on each interactive shell startup.

## Accounts

| Account | Shortname | Used for |
| --- | --- | --- |
| Personal | `my.1password.com` | Personal identity + personal working creds + server-host fetcher creds |
| Sealed Security (team) | `sealedsecurity.1password.com` | Sealed identity + Sealed working creds. Future home for hires + shared team scope. |

The `op` CLI is account-aware (`op signin --account ...`), but service
account tokens are single-account — one `OP_SERVICE_ACCOUNT_TOKEN`
value can only see one account's vaults. The dual-account loaders below
work around this by swapping the env var per `op inject` call.

## Vaults

### Personal account (`my.1password.com`)

| Vault | Tier | Contents | Scope of compromise |
| --- | --- | --- | --- |
| **Personal** | Identity | GitHub.com login + 2FA codes, email passwords, banking, Netflix, life subscriptions, AWS root account, Cloudflare account login, 1Password master password | Full life — keep nothing automated reading from here |
| **Dev** | Working | GitHub PAT (single-account, scoped per use), Cloudflare API token, OpenRouter API key, Anthropic API key, Personal Claude Code OAuth token, Neon API key, NPM token, GHCR push token, NixOS initial hashed password, host-rotation passwords | Personal projects + dev tooling |

### Sealed Security team (`sealedsecurity.1password.com`)

| Vault | Tier | Contents | Scope of compromise |
| --- | --- | --- | --- |
| **Employee** | Identity | Sealed master login, work email + 2FA codes, work bank/payroll if any, recovery codes for Sealed-managed accounts | Full Sealed identity — keep nothing automated reading from here |
| **Local Dev** | Working | Sealed Claude Code OAuth token, Linear API key, CodeRabbit API key, OpenAI API key, Buildkite API token, Graphite API token, Sealed AWS credentials, internal Sealed API tokens, Sealed CI tokens, work VPN | Sealed working scope |
| **Shared** | Identity | (future) Joint team identity — onboard creds for new hires, etc. | Whole team |

### Identity vs Working tier

Identity credentials (master logins, 2FA codes, AWS root) live in the
identity vaults (Personal, Employee). Working credentials *derived*
from those identities (PATs, SSH keys, API tokens) live in the working
vaults (Dev, Local Dev). If a working credential leaks, you revoke
and regenerate. If an identity credential leaks, you have a much
bigger recovery problem. Service accounts read working vaults only —
that boundary is load-bearing.

## Service accounts

### Personal-account SAs (per-host)

Each dev host has its own SA so blast-radius of a leaked token is
single-host. These read `op://Dev` and, where a local workflow still
needs it, `op://Server`. Token storage matches the host's available
mechanism (Keychain on Mac, 600-perm file on dev hosts). Sealed server
service accounts live with the sealed infra modules that consume them.

| Service account | Used by | Token storage |
| --- | --- | --- |
| **macbook-svc** | Mac interactive shell | macOS Keychain entry `OP_SERVICE_ACCOUNT_TOKEN` |
| **pc-svc** | mattpc-wsl interactive shell + Windows host (mattpc) | `~/.config/op/service-account-token` (mode 0600) + Windows `%USERPROFILE%\.config\op\service-account-token` (icacls-locked) |
| **personal-gha-svc** | Personal-project GitHub Actions workflows | GitHub Actions secret `OP_SERVICE_ACCOUNT_TOKEN` per personal repo |

### Team-account SA (shared)

Single SA across all interactive shells — only 2 secrets currently
need it (Sealed Claude OAuth + Linear API key), low isolation value.
Split per-host later if blast-radius separation becomes useful.

| Service account | Vault scope | Used by | Token storage |
| --- | --- | --- | --- |
| **matt-dev-svc** | Local Dev (read) | Every interactive shell on every dev host | macOS Keychain `OP_TEAM_SERVICE_ACCOUNT_TOKEN` / `~/.config/op/team-service-account-token` (Linux + Windows) |
| **sealed-gha-svc** | Local Dev (read) | Sealed repo GitHub Actions workflows | GitHub Actions secret per Sealed repo |

None of these have access to identity vaults (Personal, Employee).
A compromised SA can re-issue tokens but can't reach the master
logins that would let an attacker pivot into your full identity.

### GHA usage pattern

Both GHA service accounts work the same way — store token as repo
secret, use [`1Password/load-secrets-action`](https://github.com/1Password/load-secrets-action):

```yaml
- uses: 1password/load-secrets-action@v2
  with:
    export-env: true
  env:
    OP_SERVICE_ACCOUNT_TOKEN: ${{ secrets.OP_SERVICE_ACCOUNT_TOKEN }}
    GITHUB_TOKEN: op://Dev/GitHub Personal Access Token/token
    CF_TOKEN: op://Dev/Cloudflare API Token/credential
```

Personal repos reference `op://Dev/...` with `personal-gha-svc`;
Sealed repos reference `op://Local Dev/...` with `sealed-gha-svc`.
The service account token determines which account+vault is reachable
— a personal workflow accidentally referencing an `op://Local Dev/`
path simply won't resolve, and vice versa. Defense in depth.

## Loaders

### Eager batch loader — `load-secrets`

Lives in `shared/load-secrets.nix` (`programs.zsh.initContent` on
NixOS/Mac); the tables below describe the Nix loader. `windows/profile.ps1`
(PowerShell) loads its own overlapping set; `TAILSCALE_API_KEY`,
`CODERABBIT_API_KEY`, and `OPENAI_API_KEY` are Nix-only (not exported
there). Runs on
every interactive shell startup; non-interactive shells (systemd
units, cron, ssh-running-a-command, bash subshells) intentionally
skip it.

Runs **two `op inject` passes** — one per account. The two passes
are independent: if either token is unset or invalid, that pass logs
a one-line warning and the other proceeds. Each pass swaps
`OP_SERVICE_ACCOUNT_TOKEN` to the corresponding account's token for
the duration of the call.

**Personal account** (`OP_SERVICE_ACCOUNT_TOKEN`, scope `op://Dev/...`):

| op:// reference | Env var |
| --- | --- |
| `op://Dev/Anthropic API Key/credential` | `TESTING_ANTHROPIC_API_KEY` |
| `op://Dev/OpenRouter API Key/credential` | `OPENROUTER_API_KEY` |
| `op://Dev/GitHub Personal Access Token/token` | `GITHUB_PERSONAL_ACCESS_TOKEN` |
| `op://Dev/Personal Cloudflare API Token/token` | `CLOUDFLARE_API_TOKEN` |
| `op://Dev/Neon API Key/credential` | `NEON_API_KEY` |
| `op://Dev/Personal Pulumi Access Token/token` | `PULUMI_ACCESS_TOKEN` |

**Team account** (`OP_TEAM_SERVICE_ACCOUNT_TOKEN`, scope `op://Local Dev/...`):

| op:// reference | Env var |
| --- | --- |
| `op://Local Dev/Linear API Key/credential` | `LINEAR_API_KEY` |
| `op://Local Dev/CodeRabbit API Key/credential` | `CODERABBIT_API_KEY` |
| `op://Local Dev/OpenAI API Key mattdev/credential` | `OPENAI_API_KEY` |
| `op://Local Dev/Tailscale API Key/credential` | `TAILSCALE_API_KEY` |

`CLAUDE_CODE_OAUTH_TOKEN` is loaded by `load-secrets` from either
`op://Dev/Personal Claude Code OAuth Token/credential` or
`op://Local Dev/Claude Code OAuth Token matt sealed/credential`
depending on the per-user marker file — see the *Claude Code token
swap* section below.

The subscription token is an OAuth token (`sk-ant-oat…`), not an API
key, so it lives in `CLAUDE_CODE_OAUTH_TOKEN` — the OAuth slot. The
Anthropic API key is now parked under `TESTING_ANTHROPIC_API_KEY`, not
the magic `ANTHROPIC_API_KEY` name: OMP and the Anthropic SDKs
auto-detect `ANTHROPIC_API_KEY` and would bill the pay-per-token API
instead of the claude.ai subscription OAuth. Rename it back only if a
tool genuinely needs a raw `sk-ant-api…` key.

If `op` fails (token unset, app locked, signed out), each export is
*skipped* rather than set to empty — downstream tools fall back to
their own credential discovery (gh credential helper, etc.) and surface
clear errors instead of inheriting `""` and silently sending empty
credentials.

GitHub PAT uses the `/token` field (rather than `/credential`) to
match `gh`'s convention, so any `op://Dev/GitHub Personal Access
Token/token` reference in this file resolves the same way regardless
of where it's read from.

### Claude Code token swap — `claude-default` / `claude-personal` / `claude-sealed`

`load-secrets` chooses which Claude OAuth token to export based on the
marker file `~/.config/claude-code/default-account` (contents:
`personal` or `sealed`; defaults to `sealed` if missing). The marker
is read on every new shell, so changing it flips the default for every
future shell without a nix-switch.

```bash
claude-default personal   # persist + flip current shell to Personal
claude-default sealed     # persist + flip current shell to Sealed
claude-personal           # one-shot: flip ONLY the current shell to Personal
claude-sealed             # one-shot: flip ONLY the current shell to Sealed
```

`claude-sealed` temporarily swaps `OP_SERVICE_ACCOUNT_TOKEN` to the
team token to do the read, then restores it. `claude-personal` uses
the personal token (already in env).

Defined alongside `load-secrets` in `shared/load-secrets.nix` /
`windows/profile.ps1`. Use `claude-default` when you want a sticky
preference (most people most of the time); use the one-shot helpers
when you just need to flip a single terminal.

### One-shot activation — `graphiteAuth`

Lives in `shared/dev.nix`'s home-manager activations. Runs once on
the first `nix-switch` after `graphite-cli` is installed: reads
`op://Local Dev/Graphite API Token/credential` (team SA scope)
and runs `gt auth --token <…>`, which writes
`~/.config/graphite/user_config` (gt 1.5+; the legacy path was
`~/.graphite_user_config`). Subsequent nix-switches no-op because
the activation checks for that file's presence.

Different mechanism from `load-secrets` because the credential
isn't an env var the user wants in their shell — it's a one-shot
disk write that gt then reads on every invocation. Same shape as
`installHk` / `installAkiflowCli`: capability provisioning on
nix-switch, not env-var loading on shell startup.

The team SA token is read from
`~/.config/op/team-service-account-token` directly (not from
`OP_TEAM_SERVICE_ACCOUNT_TOKEN` in env). home-manager activations
run with a stripped environment — the `envExtra` / `initContent`
exports that warm the user shell aren't inherited. Reading from
disk mirrors the obsidian-bootstrap wrapper script in the same
file and works without any pre-activation shell state.

Mac hosts store the team SA token in Keychain rather than a 600-
perm file, so the activation no-ops there and `gt auth --token`
runs manually once per Mac host. Wiring a `security
find-generic-password` branch into the activation is a follow-up
once a second Mac dev host appears; one-shot manual auth per host
is cheap enough.

## op installation per host

`op` is installed per host rather than via `home.packages`, so each
host gets a copy aligned with its update channel and (where relevant)
its desktop-app integration:

| Host | op source |
| --- | --- |
| Mac (`Matts-MacBook-Pro`) | Bundled with the 1Password.app cask; on PATH via `brew shellenv` |
| mattpc-wsl (`mattpc-wsl`) | NixOS `environment.systemPackages` in `nixos/common.nix` |
| WSL standalone (`mattw`) | Manual `apt`/`brew` install — no system layer to declare it in |

## Onboarding a new machine

1. Install `op` via the appropriate path for the host (see table above).
2. Create per-host personal SA + obtain team-account SA token (matt-dev-svc).
3. Run the host bootstrap script (`darwin/scripts/mac-setup.sh`,
   `nixos/scripts/framework-bootstrap.sh`, etc.) — it prompts for both
   tokens and writes them to the appropriate storage (Keychain on Mac,
   `~/.config/op/{service,team-service}-account-token` elsewhere).
4. Open a new shell; `load-secrets` auto-fires and warms both env-var
   sets. Both `op` accounts are reachable for interactive use.
5. Reference secrets via `op://Vault/Item/field` format. Account
   disambiguation is implicit via the token-swap inside
   `load-secrets` / `claude-sealed` — no `--account` flag needed in
   day-to-day use.
