{ lib, ... }:

# mattserver user config. shared/load-secrets.nix is imported at the flake
# level so `op inject` runs on every interactive shell — same model as mattfw
# and mattpc-wsl.

{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattserver\" --show-trace";

  # 1Password service-account tokens from 600-perm files. Loaded before
  # shared/home.nix's op inject invocation. Same pattern as mattfw.
  # Written once by mattserver-bootstrap.sh; rotate by rewriting the
  # relevant file.
  #
  # Two tokens for two accounts:
  #
  #   ~/.config/op/service-account-token       → personal account (mattserver-svc).
  #     Scope: op://Dev/... + op://Server/...
  #   ~/.config/op/team-service-account-token  → sealedsecurity team (matt-dev-svc).
  #     Scope: op://Employee Dev/... (interactive shell only).
  #
  # Exported via `envExtra` (writes to ~/.zshenv) instead of
  # `initContent` (writes to ~/.zshrc) so non-interactive shells —
  # systemd user services, cron, scripted invocations — see the
  # tokens too.
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
