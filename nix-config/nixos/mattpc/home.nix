{ pkgs, lib, ... }:
# mattpc user config — bare-metal daily driver. Same op-token loading as the
# other Linux dev hosts, plus the Hyprland session (this host's DE). The
# Hyprland block is a functional baseline — extend keybinds/monitors/bar to
# taste; it's plain home-manager config, no separate flake input needed
# (Hyprland comes from nixpkgs-unstable, enabled system-side in system.nix).
{
  programs.zsh.shellAliases.nix-switch = lib.mkForce "sudo nixos-rebuild switch --flake \"$HOME/repos/zireael/nix-config#mattpc\" --show-trace";

  # 1Password service-account tokens (same model as mattpc-wsl / mattfw /
  # mattserver): 600-perm files loaded via .zshenv so non-interactive shells
  # see them too. Guarded by `[ -r ]`, so absent files are a silent no-op.
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

  # Session tools the Hyprland baseline references.
  home.packages = with pkgs; [
    waybar # status bar
    fuzzel # Wayland app launcher
    mako # notification daemon
    wl-clipboard # clipboard
    grim # screenshot
    slurp # region select
    brightnessctl
    playerctl
    exfatprogs # fsck/mkfs for exfat USB sticks (udiskie mounts them)
  ];

  # Automount removable drives (USB sticks). The minimal Hyprland session
  # ships no file manager, so nothing triggers udisks to mount a hotplugged
  # disk the way the installer ISO's GNOME desktop did — udiskie is that
  # trigger. Runs as a user service under graphical-session.target; mounts
  # to /run/media/mattw/<label>, pops a mako notification, no tray icon.
  # Needs services.udisks2 system-side (system.nix).
  services.udiskie = {
    enable = true;
    automount = true;
    notify = true;
    tray = "never";
  };

  # Hyprland session config. `enable` writes ~/.config/hypr/hyprland.conf from
  # `settings`; the compositor + portals are enabled system-side (system.nix).
  wayland.windowManager.hyprland = {
    enable = true;
    # Pin the hyprlang generator (the pre-26.05 default) — silences the
    # "default changed to lua" warning and keeps the settings below as
    # hyprlang, which is what they're written in.
    configType = "hyprlang";
    settings = {
      # Single monitor, preferred mode. Adjust to the real display/refresh
      # (e.g. "DP-1,3440x1440@144,0x0,1") once known.
      monitor = ",preferred,auto,1";

      "$mod" = "SUPER";
      "$terminal" = "ghostty";
      "$menu" = "fuzzel";

      exec-once = [
        "waybar"
        "mako"
      ];

      # NVIDIA + Wayland env hints for the session.
      env = [
        "LIBVA_DRIVER_NAME,nvidia"
        "GBM_BACKEND,nvidia-drm"
        "__GLX_VENDOR_LIBRARY_NAME,nvidia"
        # RTX 4080 + wlroots: without this the cursor is invisible until a
        # session restart. Harmless on other GPUs.
        "WLR_NO_HARDWARE_CURSORS,1"
      ];

      bind = [
        "$mod, Return, exec, $terminal"
        "$mod, Q, killactive"
        "$mod, E, exec, $menu"
        "$mod, F, fullscreen"
        "$mod, Space, togglefloating"
        "$mod SHIFT, Q, exit"
        # Focus movement.
        "$mod, H, movefocus, l"
        "$mod, L, movefocus, r"
        "$mod, K, movefocus, u"
        "$mod, J, movefocus, d"
        # Workspaces 1-5.
        "$mod, 1, workspace, 1"
        "$mod, 2, workspace, 2"
        "$mod, 3, workspace, 3"
        "$mod, 4, workspace, 4"
        "$mod, 5, workspace, 5"
        "$mod SHIFT, 1, movetoworkspace, 1"
        "$mod SHIFT, 2, movetoworkspace, 2"
        "$mod SHIFT, 3, movetoworkspace, 3"
        "$mod SHIFT, 4, movetoworkspace, 4"
        "$mod SHIFT, 5, movetoworkspace, 5"
      ];
      bindm = [
        "$mod, mouse:272, movewindow"
        "$mod, mouse:273, resizewindow"
      ];
    };
  };
}
