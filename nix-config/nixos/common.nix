{
  config,
  lib,
  pkgs,
  ...
}:

{
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    trusted-users = [
      "root"
      "mattw"
    ];
    # Auto-accept flake-declared substituters/keys. Our top-level
    # flake.nix advertises the nixos-raspberrypi cachix in nixConfig;
    # without this, every `nix eval` against the flake spams
    # `Pass '--accept-flake-config' to trust it` warnings. Accepting
    # is safe here because the flake is single-author and the only
    # extras it adds (the nixos-raspberrypi cache + its key) are
    # known-good — same trust scope as the rest of the flake content.
    accept-flake-config = true;
  };
  nixpkgs.config.allowUnfree = true;

  time.timeZone = "America/New_York";
  i18n.defaultLocale = "en_US.UTF-8";

  environment.systemPackages = with pkgs; [
    vim
    git
    curl
    wget
    htop
    # op — interactive 1Password CLI for SSH'd-in shell sessions. Pi
    # container modules also reference pkgs._1password-cli directly via
    # ${pkgs._1password-cli}/bin/op for service-level use, independent
    # of this.
    _1password-cli
    # bubblewrap — unprivileged user-namespace sandbox launcher. seal's
    # OS-level command sandbox (SEA-10) shells out to `bwrap` on Linux
    # for every `command_run` spawn; macOS uses sandbox-exec instead.
    # Installed system-wide so /run/current-system/sw/bin/bwrap is on
    # the daemon's PATH regardless of which shell launched it.
    bubblewrap
    # socat — bridges the daemon's host-side proxy listeners
    # (HTTP-CONNECT + SOCKS5) into the bwrap network namespace via a
    # UDS bind-mount + in-namespace TCP forwarder. Required pair with
    # bubblewrap for the SEA-10 proxy chokepoint.
    socat

    # ── system profiling / hardware introspection ──────────────────
    # Diagnostic toolkit kept on every NixOS host so first-touch
    # debugging on an unfamiliar machine doesn't bounce through
    # `nix run nixpkgs#...` every command. All are tiny and worth
    # the closure cost.

    # SMBIOS / DMI reader. Authoritative source for DIMM, board,
    # BIOS, CPU pinout info. Use `sudo dmidecode -t 17` for memory
    # slots (part numbers + speeds, useful for matching RAM
    # upgrades without opening the chassis).
    dmidecode
    # Full hardware tree walker, JSON output. `lshw -json` is the
    # single-shot probe for figuring out what's in a box.
    lshw
    # PCI / USB device enumeration. lspci + lsusb.
    pciutils
    usbutils
    # Disk SMART + NVMe vendor logs. `smartctl -a /dev/nvme0n1` for
    # wear data, self-test history, error counters.
    smartmontools
    # NVMe-specific (queue depth, namespace mgmt, vendor commands
    # beyond what smartctl exposes).
    nvme-cli

    # ── live performance probing ───────────────────────────────────
    # htop above covers process-level CPU/mem; these fill the gaps.

    # Per-process I/O bandwidth. Pairs with htop for the "what's
    # thrashing the disk" question.
    iotop
    # Per-process network bandwidth. nethogs answers "which
    # process is hogging the uplink".
    nethogs
    # Historical perf collectors (`iostat`, `mpstat`, `pidstat`,
    # `sar`). `sar` is the one that pays back tomorrow — it
    # writes rolling /var/log/sa/* snapshots so "what was the box
    # doing at 3am last Tuesday" becomes answerable. Defaults
    # collect every 10 min, retained 7 days.
    sysstat

    # ── storage ────────────────────────────────────────────────────
    # Interactive disk-usage browser. Faster than `du | sort` for
    # the "what's eating the disk" question.
    ncdu

    # ── network debugging ──────────────────────────────────────────
    # Packet-level capture for protocol debugging.
    tcpdump
    # Combined ping + traceroute. mtr -r reports loss% per hop,
    # the right tool for "is this network path lossy".
    mtr
    # DNS lookup utilities (`dig`, `nslookup`).
    dnsutils

    # ── system / process ───────────────────────────────────────────
    # Trace syscalls of a running process. Used heavily in seal
    # sandbox debugging; also useful for "why won't this binary
    # start".
    strace
    # "What's holding /that/ file/socket open" — the lsof toolbox.
    lsof
    # pstree + killall + fuser. pstree visualises the process
    # tree, killall kills by name, fuser shows what process is
    # using a file or filesystem. Comes from `psmisc`.
    psmisc
    # Pipe-viewer for progress bars on long-running data moves
    # (`pv < file.bin > /dev/sdX`, etc.).
    pv

    # ── temperature / power ────────────────────────────────────────
    # CPU + mainboard temperature sensors (`sensors` after
    # `sensors-detect`). Lives in nixos/mattserver too;
    # promoting here so every Linux host can `sensors` without
    # asking.
    lm_sensors

    # ── network throughput / link debugging ────────────────────────
    # NIC driver/link state, ring-buffer + offload settings.
    ethtool
    # Bandwidth benchmarking client+server. `iperf3 -s` on one
    # host, `iperf3 -c <host>` on the other.
    iperf3
    # Modern per-process bandwidth viewer (Rust). Higher-resolution
    # alternative to nethogs; uses pcap so needs root.
    bandwhich

    # ── modernised coreutils + nicer terminal UX ───────────────────
    # System-wide (vs home-manager) on purpose: root sessions and
    # one-off `sudo` invocations should get the same tools as your
    # interactive shell. Home-manager-only installs leave
    # `sudo rg foo` failing with command-not-found, which is
    # exactly when you want it most. Cost is a few MB of closure
    # per host.
    ripgrep # rg — grep replacement, respects .gitignore
    fd # find replacement, sane defaults
    bat # cat with syntax highlight + paging
    eza # ls replacement, git-aware, tree mode
    btop # htop replacement, prettier + scrollable history
    jq # already pulled in by lots of stuff but make it explicit
    yq-go # YAML/TOML/XML equivalent of jq
  ];

  users.users.mattw = {
    isNormalUser = true;
    # video + render for DRM/GPU access (cosmic-comp + other Wayland
    # compositors fail to start without these on Pi 5). networkmanager for
    # parity with the upstream installer profile if WiFi is ever needed.
    extraGroups = [
      "wheel"
      "video"
      "render"
      "networkmanager"
      "podman"
    ];
    shell = pkgs.zsh;
    initialHashedPassword = "$6$G/9ni/JGJ1OjjHuX$Wyic5QUkdhS0Gr5WYonErBUoR3Wlvrwd9ik3Lh/CdgCnon0Kfkif08bdlqSVUBfdCnEM.eOmucqV49Aj10ljF/";
    openssh.authorizedKeys.keys = [
      # Personal SSH key — biometric-gated via the 1Password SSH agent on
      # Mac. Used for matt's daily SSH access from his Mac.
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOfinoupMf/v8sM7ez4K7wc/lN1a6NgXxHpv9wls5Ra9"
    ];
  };
  users.users.root.initialHashedPassword = "$6$s5sbuiyTY/LhtPY3$i1/wNklH7HJrP6DYjsUZ.OdULaxFJ6MHfpupdY09yp.Nbkslng93AVXiHxfyoU7xIBhbjH6CdT/IcMsD8INhm0";
  programs.zsh.enable = true;

  # Lets generic dynamically-linked Linux binaries (e.g. VSCode Remote's bundled
  # node, language servers, prebuilt CLI tools) find their loader on NixOS.
  programs.nix-ld = {
    enable = true;
    libraries = with pkgs; [
      stdenv.cc.cc
      zlib
      openssl
      curl
      glib
      icu
    ];
  };

  virtualisation.podman = {
    enable = true;
    dockerCompat = true;
    defaultNetwork.settings.dns_enabled = true;
    # Expose the root podman socket as a Docker-API-compatible
    # endpoint at /run/podman/podman.sock. Containers on rpis are
    # declared as root-owned systemd units (oci-containers in
    # containers.nix), so user tools (lazydocker, podman client
    # over SSH from Mac) need to talk to the *root* socket — not a
    # rootless user socket.
    dockerSocket.enable = true;
  };

  # Relax /run/podman/podman.sock permissions so members of the
  # `podman` group can read it. mattw is added to the group below;
  # without this the socket is root:root mode 0660 and only sudo
  # works.
  systemd.sockets.podman.socketConfig = {
    SocketUser = "root";
    SocketGroup = "podman";
    SocketMode = "0660";
  };
  users.groups.podman = { };

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      PermitRootLogin = "no";
    };
  };

  # Cockpit — web-based system management dashboard. Reachable via
  # Tailscale Serve at https://<host>.tail2be430.ts.net:9443/ from any
  # tailnet host (the per-host system.nix wires the `tailscale serve`
  # mapping). Federate via the UI's "add new host" workflow to manage
  # multiple Pis from a single pane.
  #
  # `WebService.Origins` is hostname-templated so each Pi accepts the
  # WebSocket session from its own tailnet URL. Without this Cockpit
  # rejects the proxied login with "unexpected error while connecting
  # to the machine" — its default Origin allowlist only matches the
  # local hostname, not `<host>.tail2be430.ts.net:9443`.
  # `AllowUnencrypted = true` is safe here: the unencrypted hop is
  # loopback (`tailscale serve` → `localhost:9090`); the public-facing
  # `:9443` stays full-TLS via tailscale's LetsEncrypt cert.
  # `ProtocolHeader = X-Forwarded-Proto` tells Cockpit to trust the
  # header tailscale serve sets so its self-knowledge of the request
  # protocol stays correct through the proxy hop.
  services.cockpit = {
    enable = true;
    openFirewall = true;
    settings.WebService = {
      # mkForce because the upstream cockpit module already sets
      # Origins = "https://localhost:9090" at default priority — without
      # the override, both definitions collide. mkForce overwrites
      # (it's a scalar string, not a list — there's no merge), so the
      # upstream localhost entry is preserved here explicitly. Both
      # https:// and wss:// variants per origin so the initial page
      # load AND the WebSocket upgrade pass the allowlist check.
      Origins = lib.mkForce "https://localhost:9090 wss://localhost:9090 https://${config.networking.hostName}.tail2be430.ts.net:9443 wss://${config.networking.hostName}.tail2be430.ts.net:9443";
      ProtocolHeader = "X-Forwarded-Proto";
      AllowUnencrypted = true;
    };
  };

  system.stateVersion = "25.11";
}
