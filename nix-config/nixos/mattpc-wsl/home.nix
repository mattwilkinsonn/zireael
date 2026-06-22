{ lib, ... }:

# mattpc-wsl user config — NixOS-WSL2 dev box on the Windows gaming PC.
# shared/load-secrets.nix is imported at the flake level so `op inject`
# runs on every interactive shell — same model as Mac, mattfw, mattserver.

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattpc-wsl\" --show-trace";

  # 1Password service-account tokens from 600-perm files. Loaded BEFORE
  # shared/home.nix's load-secrets auto-invoke (which uses `op inject`
  # against these tokens). No biometric, no consent dialog, no IPC to the
  # desktop app — token-based auth.
  #
  # Two tokens for two accounts:
  #
  #   ~/.config/op/service-account-token       → personal account (pc-svc).
  #     Scope: op://Dev/... + op://Server/...
  #   ~/.config/op/team-service-account-token  → sealedsecurity team (matt-dev-svc).
  #     Scope: op://Local Dev/... (interactive shell only).
  #
  # Stored once per host via mattpc-wsl-bootstrap.sh (or replace manually:
  #   install -m 600 -D /dev/stdin ~/.config/op/service-account-token      <<< 'ops_...'
  #   install -m 600 -D /dev/stdin ~/.config/op/team-service-account-token <<< 'ops_...'
  # ). Token rotation: rewrite the relevant file. Same pattern as mattfw
  # and mattserver.
  #
  # Exported via `envExtra` (writes to ~/.zshenv) instead of
  # `initContent` (writes to ~/.zshrc) so non-interactive shells —
  # systemd user services, cron, scripted invocations — see the
  # tokens too. .zshenv runs unconditionally on shell startup,
  # regardless of interactive/login state.
  #
  # Uses shell builtins (`read`, `[`, `export`) instead of `cat`
  # because systemd user services start with a near-empty PATH where
  # /run/current-system/sw/bin isn't reachable yet — `$(cat ...)`
  # silently returns empty in that environment and the token ends up
  # unset. Builtins are path-independent.
  programs.zsh.envExtra = ''
    if [ -r "$HOME/.config/op/service-account-token" ]; then
      IFS= read -r OP_SERVICE_ACCOUNT_TOKEN < "$HOME/.config/op/service-account-token"
      export OP_SERVICE_ACCOUNT_TOKEN
    fi
    if [ -r "$HOME/.config/op/team-service-account-token" ]; then
      IFS= read -r OP_TEAM_SERVICE_ACCOUNT_TOKEN < "$HOME/.config/op/team-service-account-token"
      export OP_TEAM_SERVICE_ACCOUNT_TOKEN
    fi
  '';
}
