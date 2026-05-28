{ lib, pkgs, ... }:

# shared/linux-build-deps.nix — Linux-only C/C++ build environment.
#
# Mirrors the apt-installed system packages from the Linux branch of
# seal's `just install-deps` recipe, plus equivalents for other Linux
# dev workflows. Imported only by Linux dev hosts (mattpc-wsl, mattfw).
# Mac handles the equivalent via brew + Xcode CLI Tools (see
# darwin/system.nix).
{
  home.packages = with pkgs; [
    # Rust build deps — paired with rustup-managed cargo (NOT nixpkgs-
    # managed cargo), so the .dev outputs need PKG_CONFIG_PATH plumbing
    # below for pkg-config to find their .pc files.
    pkg-config # build helper used by openssl-sys, libdbus-sys, ...
    openssl # libssl runtime
    openssl.dev # libssl-dev — Rustls / reqwest / openssl-sys headers
    dbus # libdbus-1 runtime
    dbus.dev # libdbus-1-dev — keyring crate (exercised by seal's
    # `just test-keychain`; the file-backed SEAL_KEY_BACKEND
    # path used by the rest of the suite skips it).
    mold # faster linker — seal's .cargo/config.toml selects it
    clang # build-essential C/C++ toolchain. Provides `cc`/`c++`/`cpp`
    # links that crate build scripts (cc-rs, openssl-sys, …) call
    # by default. Paired with mold per Bevy docs. `gcc` is omitted
    # deliberately — both packages drop `cc`, `c++`, `cpp`, `ld`,
    # `ar`, `addr2line`, etc. into bin/ at the same priority and
    # collide on a home-manager activation. clang is sufficient on
    # its own for the seal build path.
    gnumake # build-essential (make)

    # Tauri 2 desktop build deps. Used by sealed/helm/src-tauri (the
    # Sealed Helm desktop binary, SEA-693). Tauri 2 on Linux pulls
    # webkit2gtk for the webview, gtk-3 for windowing, libsoup-3 for
    # HTTP, libayatana-appindicator for the system tray, and librsvg
    # for SVG icon rendering. glib + pango + atk + cairo come in
    # transitively but are listed explicitly so their .pc files are
    # reachable via PKG_CONFIG_PATH (gtk-sys / glib-sys / atk-sys /
    # pango-sys / cairo-sys-rs all run pkg-config to resolve their
    # headers).
    #
    # gdk-pixbuf is intentionally NOT included at the package level —
    # it conflicts with librsvg at home-manager activation time
    # because both packages ship
    # `lib/gdk-pixbuf-2.0/2.10.0/loaders.cache` (every gdk-pixbuf
    # loader plugin registers itself there). gtk3 + librsvg already
    # pull gdk-pixbuf into the closure transitively for runtime; we
    # only need its `.pc` file at build time, which the
    # PKG_CONFIG_PATH ref below resolves via gdk-pixbuf.dev
    # (no loaders.cache in the .dev output, so no conflict).
    webkitgtk_4_1 # WebKit2GTK runtime + headers
    gtk3 # GTK-3 windowing toolkit
    gtk3.dev # GTK-3 .pc + headers
    libsoup_3 # HTTP layer Tauri 2 ships against
    libayatana-appindicator # system-tray support
    librsvg # SVG icon rendering (ships gdk-pixbuf-2.0 SVG loader +
    # its own loaders.cache — see comment above re gdk-pixbuf)
    glib # libglib-2.0 runtime
    glib.dev # libglib-2.0 .pc + headers
    pango # text layout
    atk # accessibility toolkit
    cairo # 2D graphics
    harfbuzz # text shaping — required transitively by pango.pc, but
    # pkg-config doesn't recurse into nix-store paths
    # automatically; harfbuzz.pc must be on PKG_CONFIG_PATH
    # explicitly or pango-sys's build script fails with
    # "Package 'harfbuzz', required by 'pango', not found".
    zlib # compression — required transitively by gdk-3.0.pc. Same
    # nix-store / pkg-config recursion gotcha as harfbuzz above,
    # plus a second gotcha: zlib's `.pc` lives in `share/pkgconfig`,
    # NOT `lib/pkgconfig` (nixpkgs intentionally moves it to dodge
    # downstream "colon in pathname" parsers). The PKG_CONFIG_PATH
    # builder below appends both `lib/pkgconfig` and
    # `share/pkgconfig` for every package, which covers this.
  ];

  # rustup-managed cargo can't see nix-installed `.dev` outputs without
  # PKG_CONFIG_PATH — `home.packages` puts them on the filesystem but
  # doesn't plumb their `lib/pkgconfig` directories into pkg-config's
  # search path. Set it here so `cargo build` from rustup's
  # stable/nightly toolchain finds openssl and dbus without a
  # per-project shell.nix or flake-devshell.
  #
  # Exported via `programs.zsh.envExtra` (appended to .zshenv,
  # unguarded) rather than `home.sessionVariables` (which writes to
  # hm-session-vars.sh, guarded by `$__HM_SESS_VARS_SOURCED`). The
  # guard is set process-wide on first source and inherited by every
  # child zsh — so when home-manager regenerates hm-session-vars.sh
  # with a *new* variable, long-lived parent shells (terminal
  # multiplexers, WSL session managers, Windows Terminal tabs) skip
  # the re-source on subsequent rebuilds and the new var never
  # propagates without a full session restart. .zshenv-with-envExtra
  # runs every zsh start regardless of that guard, so a fresh tab is
  # always sufficient.
  programs.zsh.envExtra = ''
    export PKG_CONFIG_PATH="${
      lib.concatStringsSep ":" (
        lib.flatten (
          map
            (p: [
              "${lib.getDev p}/lib/pkgconfig"
              "${lib.getDev p}/share/pkgconfig"
            ])
            [
              pkgs.openssl
              pkgs.dbus
              # Tauri 2 system libs (SEA-693). Same rationale as openssl +
              # dbus — rustup-managed cargo doesn't see nix-installed .dev
              # outputs without explicit PKG_CONFIG_PATH plumbing.
              pkgs.webkitgtk_4_1
              pkgs.gtk3
              pkgs.libsoup_3
              pkgs.libayatana-appindicator
              pkgs.librsvg
              pkgs.glib
              pkgs.pango
              pkgs.atk
              pkgs.cairo
              pkgs.harfbuzz
              pkgs.zlib
              pkgs.gdk-pixbuf
            ]
        )
      )
    }''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  '';

  # `rustup component add rust-src rustc-codegen-cranelift-preview` —
  # mirrors the cargo-tools step in seal's `just install-deps`. Both
  # adds are tolerated-as-failure: the rustupDefault hook (in
  # shared/dev.nix) sets stable as the default, and cranelift is a
  # nightly-only `-preview` component, so the cranelift add silently
  # no-ops against stable. rust-src succeeds on both channels.
  #
  # Per-project: seal's rust-toolchain.toml already pins
  # nightly-2026-04-18 + rust-src, and explicitly omits cranelift due
  # to a `catch_unwind` regression on that nightly (see the seal
  # repo's rust-toolchain.toml comment). This hook is belt-and-braces
  # for the default toolchain — projects that pin their own toolchain
  # don't depend on it.
  home.activation.rustupComponents = lib.hm.dag.entryAfter [ "rustupDefault" ] ''
    echo "Ensuring rustup components are present (best-effort)..."
    ${pkgs.rustup}/bin/rustup component add rust-src 2>/dev/null || true
    ${pkgs.rustup}/bin/rustup component add rustc-codegen-cranelift-preview 2>/dev/null || true
  '';
}
