{ pkgs, inputs, ... }:
# zireael dev shell — the single source of the dev + CI toolchain. The split:
#
#   proto  — owns the language/runtime toolchains (bun, node, moon), pinned in
#            .prototools. Activated on shell entry below.
#   fenix  — the exact Rust toolchain from rust-toolchain.toml (the monorepo's
#            Rust pin), shared by the dev shell + CI.
#   devenv — provides proto itself plus everything non-language: the nix + shell
#            + toml linters the gate runs, actionlint, markdownlint, jj, hk, and
#            the hook frameworks jj-hooks' integration tests drive.
#
# `moon ci` is the local + CI gate; the hk pre-push hook is a thin shell over it.
let
  # Exact Rust toolchain from rust-toolchain.toml, built by fenix; the dev shell
  # and CI share this one derivation, so rust-toolchain.toml is the single
  # Rust-version source.
  rustToolchain = inputs.fenix.packages.${pkgs.stdenv.system}.fromToolchainFile {
    file = ./rust-toolchain.toml;
    # Bumping rust-toolchain.toml's channel invalidates this hash — update it
    # from the mismatch error's `got:` line (fenix has no lockfile for it).
    sha256 = "sha256-mvUGEOHYJpn3ikC5hckneuGixaC+yGrkMM/liDIDgoU=";
  };
in
{
  packages = with pkgs; [
    # Language/runtime manager. Pins bun/node/moon via .prototools.
    proto

    # Nix linters — the same set the pre-push gate runs.
    nixfmt-rfc-style
    deadnix
    statix
    nil

    # Shell + TOML linters.
    shellcheck
    shfmt
    taplo

    # CI / workflow + docs tooling.
    actionlint
    markdownlint-cli2

    # VCS: jj (Matt's review tool; the release script shells out to it) + hk,
    # the pre-push hook runner (from its own flake — not in nixpkgs).
    jujutsu
    inputs.hk.packages.${pkgs.stdenv.system}.hk

    # Rust toolchain (jj-hooks, jj-gt). fenix provides the rust-toolchain.toml
    # pin; cc/ld for cargo's link step; cargo-nextest is the test runner.
    rustToolchain
    stdenv.cc
    cargo-nextest

    # jj-hooks' integration tests drive real hook frameworks, so they need the
    # backends on PATH: pre-commit, prek, lefthook (+ hk above), and pkl (hk
    # reads hk.pkl). markdownlint-cli2 + actionlint (above) cover its doc /
    # workflow hooks.
    pre-commit
    prek
    lefthook
    pkl
  ];

  enterShell = ''
    # Activate the proto-managed toolchains (bun/node/moon from .prototools).
    export PROTO_HOME="''${PROTO_HOME:-$HOME/.proto}"
    export PATH="$PROTO_HOME/shims:$PROTO_HOME/bin:$PATH"
    # Use the pinned nix proto (the shims dir is ahead on PATH, so a bare
    # `proto` would resolve a host install instead of this one).
    ${pkgs.proto}/bin/proto install
    # Install the pre-push git hook (a thin shell over `moon ci`). hk install is
    # idempotent, so re-run it on every entry to pick up hk.pkl changes.
    if command -v hk >/dev/null 2>&1; then
      hk install >/dev/null 2>&1 || echo "devenv: hk install failed; run 'hk install' to enable the pre-push gate"
    fi
  '';
}
