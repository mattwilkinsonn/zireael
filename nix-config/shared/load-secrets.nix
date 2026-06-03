_:

# Cross-platform 1Password CLI secret loading. Imported only on hosts
# that should auto-load API keys into shell env (Mac, mattfw,
# mattserver, mattpc-wsl — all dev hosts). The Pis intentionally
# skip this — they're headless servers and we don't want every
# interactive shell pulling the full secret bundle into env. For
# one-off shell needs on a Pi, copy-paste the env var directly.
#
# Two-account model (May 2026): personal items (Anthropic, OpenRouter,
# CF, Neon, etc.) live in the personal account's Dev vault and are
# read using OP_SERVICE_ACCOUNT_TOKEN. Sealed items (Sealed Claude
# OAuth, Linear) live in the sealedsecurity team's `Employee Dev`
# vault and are read using OP_TEAM_SERVICE_ACCOUNT_TOKEN. Each `op
# inject` invocation is scoped to one account by swapping the env var
# in/out — op CLI has no native multi-account-svc-token mode.

{
  programs.zsh.initContent = ''
          # API keys from 1Password CLI. `op` is provided per-host
          # (1Password.app cask on Mac, NixOS env packages on Linux), so
          # `command -v op` gracefully no-ops on hosts where the module
          # gets pulled in but op isn't installed yet. The unlocked 1P
          # desktop app handles auth transparently — reads are silent and
          # sub-second when paired with the OP_SERVICE_ACCOUNT_TOKEN
          # exported by the platform's mkBefore block.
          #
          # Defined as a function (not a one-shot block) so it can be re-run
          # in the current shell after unlocking / signing in — `load-secrets`
          # is the exact command the warning below tells you to run.
          #
          # The op-failure path is the load-bearing bit: a bare
          # `export FOO=$(op read … 2>/dev/null)` *sets* `FOO=""` when `op`
          # fails, and any process spawned from this shell inherits the
          # empty value. Tools like seal then silently send empty
          # credentials and get an opaque 401 instead of a "credential
          # missing" error. Skipping the export entirely lets each tool
          # fall back to its own credential discovery and surface a clear
          # error if nothing is configured.
          if command -v op >/dev/null; then
            # _op_inject_with_token TOKEN_VAR_NAME LABEL TEMPLATE
            #
            # Runs `op inject` against TEMPLATE with OP_SERVICE_ACCOUNT_TOKEN
            # temporarily set to the value of the named env var, then eval's
            # the rendered output. LABEL is purely for the warning message
            # — `Personal` / `Team` so a failure points to the right token.
            #
            # Caller passes the env-var *name* (not value) so a missing
            # token short-circuits with a useful warning instead of an
            # `op inject failed` that hides which account misfired.
            _op_inject_with_token() {
              local token_var=$1 label=$2 template=$3 rendered
              local token=''${(P)token_var:-}
              if [[ -z $token ]]; then
                print -u2 "warning: $token_var unset — $label secrets not loaded."
                return 1
              fi
              # Subshell so OP_SERVICE_ACCOUNT_TOKEN swap is per-call.
              rendered=$(OP_SERVICE_ACCOUNT_TOKEN=$token printf '%s' "$template" | OP_SERVICE_ACCOUNT_TOKEN=$token op inject 2>/dev/null)
              if [[ -n $rendered ]]; then
                eval "$rendered"
              else
                print -u2 "warning: op inject failed for $label — $token_var may be invalid. Env vars not loaded."
                return 1
              fi
            }

            # Claude Code OAuth token swap. Defined here (before
            # load-secrets) because load-secrets calls one of them on
            # every shell start based on the user's marker file —
            # function-resolution is at call time in zsh, so the
            # helpers must exist when load-secrets first runs.
            claude-sealed() {
              local val
              if val=$(OP_SERVICE_ACCOUNT_TOKEN=''${OP_TEAM_SERVICE_ACCOUNT_TOKEN:-} op read "op://Shared Development/Claude Code OAuth Token matt sealed/credential" 2>/dev/null) && [[ -n $val ]]; then
                export CLAUDE_CODE_OAUTH_TOKEN=$val
                print "CLAUDE_CODE_OAUTH_TOKEN → Sealed"
              else
                print -u2 "failed to read Sealed claude token (OP_TEAM_SERVICE_ACCOUNT_TOKEN unset or invalid?)"
                return 1
              fi
            }
            claude-personal() {
              local val
              if val=$(op read "op://Dev/Personal Claude Code OAuth Token/credential" 2>/dev/null) && [[ -n $val ]]; then
                export CLAUDE_CODE_OAUTH_TOKEN=$val
                print "CLAUDE_CODE_OAUTH_TOKEN → Personal"
              else
                print -u2 "failed to read Personal claude token (op locked or signed out?)"
                return 1
              fi
            }

            # Persist the default Claude token to use for every NEW
            # shell. Writes ~/.config/claude-code/default-account;
            # load-secrets reads it at shell startup. The current
            # shell's env is also flipped immediately for convenience.
            # No nix-switch needed — this is plain runtime state.
            claude-default() {
              local choice=''${1:-}
              case "$choice" in
                personal|sealed) ;;
                *)
                  print -u2 "usage: claude-default <personal|sealed>"
                  return 2
                  ;;
              esac
              install -d -m 700 "$HOME/.config/claude-code"
              print "$choice" > "$HOME/.config/claude-code/default-account"
              print "default Claude account → $choice (persisted; new shells will use this)"
              case "$choice" in
                personal) claude-personal ;;
                sealed) claude-sealed ;;
              esac
            }

            load-secrets() {
              # Personal account: items in op://Dev/... Read with
              # OP_SERVICE_ACCOUNT_TOKEN (matt-dev-svc / per-host SAs).
              local personal_template
              personal_template='export ANTHROPIC_API_KEY="{{ op://Dev/Anthropic API Key/credential }}"
    export OPENROUTER_API_KEY="{{ op://Dev/OpenRouter API Key/credential }}"
    export GITHUB_PERSONAL_ACCESS_TOKEN="{{ op://Dev/GitHub Personal Access Token/token }}"
    export CLOUDFLARE_API_TOKEN="{{ op://Dev/Personal Cloudflare API Token/token }}"
    export NEON_API_KEY="{{ op://Dev/Neon API Key/credential }}"
    export PULUMI_ACCESS_TOKEN="{{ op://Dev/Personal Pulumi Access Token/token }}"'
              _op_inject_with_token OP_SERVICE_ACCOUNT_TOKEN Personal "$personal_template"

              # Team account (sealedsecurity.1password.com): items in
              # op://Employee Dev/... Read with OP_TEAM_SERVICE_ACCOUNT_TOKEN
              # (matt-dev-svc). CLAUDE_CODE_OAUTH_TOKEN is intentionally
              # NOT loaded here — it's set below from a per-user marker
              # file so the default account is user-controllable without
              # an edit-and-nix-switch cycle.
              local team_template
              team_template='export LINEAR_API_KEY="{{ op://Employee Dev/Linear API Key/credential }}"
    export CODERABBIT_API_KEY="{{ op://Employee Dev/CodeRabbit API Key/credential }}"'
              _op_inject_with_token OP_TEAM_SERVICE_ACCOUNT_TOKEN Team "$team_template"

              # Choose which Claude OAuth token to load by default.
              # Marker at ~/.config/claude-code/default-account holds
              # `personal` or `sealed`; missing/unknown values fall
              # back to `sealed` (the original behavior). Use
              # `claude-default <personal|sealed>` to flip it — takes
              # effect on every new shell, no nix-switch required.
              local claude_default=sealed
              if [[ -r "$HOME/.config/claude-code/default-account" ]]; then
                IFS= read -r claude_default < "$HOME/.config/claude-code/default-account"
              fi
              case "$claude_default" in
                personal) claude-personal >/dev/null ;;
                *) claude-sealed >/dev/null ;;
              esac
            }

            # Auto-invoke at shell startup only when stdout is a tty — a
            # real interactive terminal session, not a non-interactive
            # subshell like VS Code's env-resolver (`zsh -l -i -c env`).
            # Service account auth means no prompts anywhere, but skipping
            # non-tty contexts still avoids adding ~500ms HTTPS-call
            # latency to every subshell startup (cron, systemd, env-
            # resolvers, scripted shells, etc.).
            if [ -t 1 ]; then
              load-secrets
            fi
          fi
  '';
}
