{
  pkgs,
  ...
}:

# rpi5 — server-only config. Runs Cockpit + Tailscale exit node; Kimai
# for hours.sealedsecurity.com lives in ./kimai.nix. No desktop by default.
# To temporarily enable KDE for debug, add `./desktop.nix` to the rpi5
# modules list in flake.nix and nix-switch.

{
  # Migrate off deprecated "kernelboot" (the upstream default for raspberry-pi-5.base).
  # See nvmd/nixos-raspberrypi PR#61.
  boot.loader.raspberry-pi.bootloader = "kernel";

  # rpi-eeprom-update + rpi-eeprom-config — needed for one-off EEPROM updates
  # and for the Argon NEO 5 setup script. raspberrypi-utils is already
  # provided by nixos-raspberrypi's base module (gives vcgencmd).
  # lm_sensors bridges /sys/class/thermal into the `sensors` CLI (and KDE
  # System Monitor when desktop.nix is loaded).
  environment.systemPackages = with pkgs; [
    raspberrypi-eeprom
    lm_sensors
  ];

  # Argon EEPROM config — runs the Argon Python helper on every system
  # activation to ensure their EEPROM settings (BOOT_UART, WAKE_ON_GPIO,
  # POWER_OFF_ON_HALT, BOOT_ORDER=0xf416, PCIE_PROBE) are applied. The script
  # is idempotent: if bootconf already has the right values it exits early.
  # If changes are needed it stages an EEPROM update which the bootloader
  # applies at next boot. Pinned by sha256 so a compromised CDN can't inject
  # a malicious script — re-prefetch the URL to update.
  system.activationScripts.argonEepromConfig =
    let
      argonScript = pkgs.fetchurl {
        url = "https://download.argon40.com/scripts/argon-rpi-eeprom-config-default.py";
        sha256 = "04apxnl7rwzjwzpb6j4nkygqpzvbxlhkw9rdmwyk2lxr2c2mh2b0";
      };
    in
    ''
      echo "Checking Argon EEPROM config..."
      PATH="${pkgs.raspberrypi-eeprom}/bin:${pkgs.raspberrypi-utils}/bin:${pkgs.coreutils}/bin:${pkgs.gnused}/bin:${pkgs.gnugrep}/bin:$PATH" \
        ${pkgs.python3}/bin/python3 ${argonScript} || true
    '';

  networking = {
    hostName = "rpi5";
    interfaces.end0 = {
      useDHCP = false;
      ipv4.addresses = [
        {
          # .51 — Technitium DNS lives on rpi4 (.50).
          address = "192.168.1.51";
          prefixLength = 24;
        }
      ];
    };
    defaultGateway = "192.168.1.1";
    # rpi4 (.50) is now the network DNS server (Technitium).
    # Plain-DNS fallbacks listed in case rpi4 is down for maintenance.
    nameservers = [
      "192.168.1.50"
      "9.9.9.11"
      "149.112.112.11"
      "1.1.1.1"
      "1.0.0.1"
    ];

    # Fast failover if rpi4 DNS is briefly unhealthy. Default 5s × 2 attempts
    # is too long for interactive use; 1s × 2 keeps things snappy.
    resolvconf.extraOptions = [
      "timeout:1"
      "attempts:2"
    ];
  };

  networking.firewall = {
    enable = true;
    # 22  = SSH (LAN bootstrap; Tailscale SSH used after onboarding)
    # 9090 = Cockpit web UI (host-network)
    # 9443 = Tailscale Serve (Cockpit HTTPS)
    allowedTCPPorts = [
      22
      9090
      9443
    ];
    trustedInterfaces = [ "tailscale0" ];
  };

  services.tailscale = {
    enable = true;
    openFirewall = true;
    # Advertise as an exit node. Pi 5's 4× Cortex-A76 @ 2.4 GHz +
    # NVMe SSD is well within capacity for home broadband throughput —
    # the ISP uplink is the real bottleneck, not this CPU.
    extraUpFlags = [
      "--advertise-exit-node"
      "--ssh"
    ];
  };

  # Tailscale Serve mappings — expose container services on the tailnet
  # over HTTPS with auto-provisioned LetsEncrypt certs via MagicDNS.
  # Idempotent: re-running with the same args is a no-op.
  # - https://rpi.tail08a5c5.ts.net:9443/ → Cockpit (port 9090, self-signed
  #     HTTPS upstream — Tailscale handles cert termination at the edge).
  # Technitium admin UI lives on rpi4 (https://rpi4.tail08a5c5.ts.net:8443/).
  # OpenClaw moved to mattfw — see nixos/mattfw/containers.nix.
  systemd.services.tailscale-serve = {
    description = "Tailscale Serve mappings for container services";
    wantedBy = [ "multi-user.target" ];
    after = [
      "tailscaled.service"
      "network-online.target"
    ];
    wants = [
      "tailscaled.service"
      "network-online.target"
    ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = pkgs.writeShellScript "tailscale-serve-setup" ''
        ${pkgs.tailscale}/bin/tailscale serve --bg --https=9443 https+insecure://localhost:9090
        # Idempotent cleanup of removed mappings (no-op if not set).
        ${pkgs.tailscale}/bin/tailscale serve --https=443 off || true
        ${pkgs.tailscale}/bin/tailscale serve --https=8443 off || true
      '';
    };
  };

  # UDP GRO forwarding for exit-node throughput. Tailscale recommends
  # `rx-udp-gro-forwarding on, rx-gro-list off` on Linux 6.2+ kernels.
  # The interface name is the default-route NIC (end0 on rpi5).
  # Idempotent — re-setting the same flags is a fast no-op.
  systemd.services.tailscale-gro-tweak = {
    description = "Apply Tailscale UDP GRO forwarding tweak for exit-node throughput";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = pkgs.writeShellScript "tailscale-gro-tweak" ''
        IFACE=$(${pkgs.iproute2}/bin/ip -o route get 8.8.8.8 | ${pkgs.coreutils}/bin/cut -f 5 -d " ")
        ${pkgs.ethtool}/bin/ethtool -K "$IFACE" rx-udp-gro-forwarding on rx-gro-list off
      '';
    };
  };

  # Filesystems are populated by the installer. If labels differ after flashing,
  # update these via `blkid` output before the first rebuild.
  fileSystems."/" = {
    device = "/dev/disk/by-label/NIXOS_SD";
    fsType = "ext4";
    options = [ "noatime" ];
  };
  fileSystems."/boot/firmware" = {
    device = "/dev/disk/by-label/FIRMWARE";
    fsType = "vfat";
    options = [
      "noatime"
      "noauto"
      "x-systemd.automount"
    ];
  };

  swapDevices = [
    {
      device = "/swapfile";
      size = 4096;
    }
  ];

  # Kernel tunings.
  # vm.swappiness=10: kernel only swaps when really needed (vs default 60).
  # Pi has 8GB RAM, NVMe is fast enough that swap isn't routinely needed,
  # and biasing against swap keeps memory-resident workloads snappier.
  # net.ipv4.ip_forward + net.ipv6.conf.all.forwarding: required by
  # Tailscale exit node — the kernel must forward packets between the
  # tailscale0 interface and the physical NIC.
  boot.kernel.sysctl = {
    "vm.swappiness" = 10;
    "net.ipv4.ip_forward" = 1;
    "net.ipv6.conf.all.forwarding" = 1;
  };

  # Argon NEO 5 case + NVMe SSD: replicates the config.txt edits from
  # Argon's argonneo5.sh.
  # - nvme: enables the M.2 HAT's PCIe → NVMe bridge (Pi 5 often auto-detects;
  #   explicit is harmless).
  # - pciex1_gen=3: bumps PCIe link speed from Gen 2 to Gen 3 — roughly 2x NVMe
  #   throughput.
  # - usb_max_current_enable=1: lifts the USB power budget so heavier
  #   peripherals don't get current-limited.
  hardware.raspberry-pi.config.all = {
    base-dt-params = {
      nvme.enable = true;
      pciex1_gen = {
        enable = true;
        value = 3;
      };
    };
    options = {
      usb_max_current_enable = {
        enable = true;
        value = 1;
      };
    };
  };
}
