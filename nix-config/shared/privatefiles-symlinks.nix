{ config, ... }:

# shared/privatefiles-symlinks.nix — declarative re-creation of the
# symlinks the migrate-from-dotfiles.sh script originally authored
# in-place.
#
# Why home-manager owns these now:
#   - Run-once scripted symlinks bit-rot. If a symlink is accidentally
#     deleted (sloppy `rm`, an installer that overwrites it, a fresh
#     bootstrap, …) home-manager re-creates it on the next nix-switch.
#   - One declarative answer for "what files in $HOME come from
#     privatefiles?" — readable in this file, not scattered across
#     bash.
#   - `nix-switch` becomes the single re-converge button for dev hosts.
#
# Why mkOutOfStoreSymlink (not source = ./path or text = readFile):
#   - The symlink target stays at the live path
#     `$HOME/repos/privatefiles/...`, so edits to privatefiles
#     content propagate without rebuilding nix-config.
#   - The privatefiles repo is a sibling of zireael (not vendored
#     into it), so referencing `../../privatefiles/...` from inside
#     this flake doesn't resolve at eval time.
#
# Imported ONLY by dev hosts (Matts-MacBook-Pro, mattfw, mattpc-wsl).
# Server / runner hosts (mattserver, mattmini, rpi4, rpi5) don't
# have privatefiles cloned and shouldn't try to symlink into a
# non-existent repo.

let
  privatefiles = "${config.home.homeDirectory}/repos/privatefiles";
  linkOut = path: config.lib.file.mkOutOfStoreSymlink "${privatefiles}/${path}";
in
{
  home.file = {

    # sealedsecurity workspace meta — the .code-workspace VS Code
    # multi-root config that opens the multi-repo workspace. References
    # the private sealed/ repo path so it stays in privatefiles. If the
    # workspace dir doesn't exist on a host the link lands in nowhere —
    # harmless; home.file mkdir's the parent on activation.
    "repos/sealedsecurity/sealedsecurity.code-workspace".source =
      linkOut "repos/sealedsecurity/sealedsecurity.code-workspace";

    # Top-level VS Code multi-root workspace at ~/repos/repos.code-workspace.
    # Points at zireael, privatefiles, snlfilm, sentinel, nix-config — the
    # set of repos open in a typical interactive dev session.
    "repos/repos.code-workspace".source = linkOut "repos/repos.code-workspace";
  };
}
