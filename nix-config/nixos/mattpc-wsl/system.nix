{ lib, ... }:

# mattpc-wsl — NixOS-WSL2 distro running under Windows 11 on the gaming PC
# (i9-14900KS + RTX 4080 + 64 GB DDR5). Windows is the bare-metal OS;
# this is the Linux dev environment.
#
# WSL2-specific deviations from a normal NixOS host:
#   - No bootloader (WSL provides its own kernel via wsl.exe).
#   - No filesystem declarations (WSL manages the rootfs vhdx).
#   - SSH on port 2222 — Windows-side OpenSSH already binds 22 for
#     `ssh mattw@mattpc` system monitoring (`btm`, etc.).
#   - No tailscale daemon — Windows runs tailscaled, mirrored
#     networking in windows/.wslconfig makes the tailnet reachable
#     from inside WSL with no extra setup.
#   - Cockpit disabled — pointless without external port mapping;
#     for whole-system monitoring, ssh into Windows and run btm.
#
# nixos/common.nix is shared with the other NixOS hosts; the lib.mkForce
# below selectively turns off pieces that don't apply in WSL.

{
  wsl = {
    enable = true;
    defaultUser = "mattw";
    # Mirrored networking is set Windows-side in .wslconfig, not here.
    # interop.includePath = false would prune the Windows PATH entries
    # from $PATH inside WSL — leaving them in lets `code.exe`,
    # `winget`, etc. work directly from a WSL shell, which is the
    # whole point of WSL2 + Windows interop.
  };

  networking.hostName = "mattpc-wsl";

  # SSH on 2222 inside WSL. Windows-side OpenSSH owns 22 (set up in
  # windows/windows-bootstrap.ps1) and `mirrored` networking shares
  # ports between Windows and WSL, so 22 would collide. Override the
  # default port set in nixos/common.nix.
  services.openssh.ports = lib.mkForce [ 2222 ];

  # Cockpit is enabled in nixos/common.nix for the Pi management UI;
  # disable it inside WSL since there's no realistic external access
  # path (no fixed tailnet IP for WSL — tailscale runs on the Windows
  # host) and `btm` over SSH is the actual monitoring path on this box.
  services.cockpit.enable = lib.mkForce false;

  # NixOS-WSL leaves the firewall off by default and that's the right
  # call for WSL: the Windows firewall is the gating layer (Hyper-V
  # Firewall integration is on by default with mirrored networking),
  # and a duplicate iptables firewall inside WSL just adds confusion
  # without adding security. Be explicit.
  networking.firewall.enable = false;

  # Honor /etc/hosts entries on the Windows side. WSL2 propagates
  # them through automatically; nothing to declare here.
}
