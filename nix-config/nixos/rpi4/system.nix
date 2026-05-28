{
  pkgs,
  modulesPath,
  ...
}:

# rpi4 — Raspberry Pi 4B in a Canakit case with always-on fan.
# Server-only: Technitium DNS + Cockpit + Tailscale. Headless, no
# desktop, no special hardware (no NVMe HAT, no GPIO accessories).
# SD card boot.

{
  # SD image module — used to build the bootable .img for fresh provisioning.
  # Build via: nix build .#nixosConfigurations.rpi4.config.system.build.sdImage
  # The resulting image at result/sd-image/*.img.zst gets dd'd to the SD card.
  imports = [
    "${modulesPath}/installer/sd-card/sd-image-aarch64.nix"
  ];

  # Increase FIRMWARE partition from the ~30MB default. raspberry-pi-4.base's
  # uboot-builder writes the full Pi vendor firmware set (Pi 2/3/4/CM/Zero
  # variants) plus u-boot binaries — total ~80MB. uboot-builder also writes
  # via .tmp + rename, so it briefly needs ~2x file size during install.
  # 256MB is generous headroom that survives future firmware bloat.
  sdImage.firmwareSize = 256;

  # Pi 4 doesn't need the Argon-style EEPROM tweaks (different case, no
  # M.2 HAT). raspberry-pi-4.base from nixos-raspberrypi handles firmware
  # and kernel.

  environment.systemPackages = with pkgs; [
    lm_sensors # CLI thermal monitoring (`sensors`), useful headless too
  ];

  networking = {
    hostName = "rpi4";
    interfaces.end0 = {
      useDHCP = false;
      ipv4.addresses = [
        {
          # .50 — the network's DNS server. Router points here for DHCP DNS.
          address = "192.168.1.50";
          prefixLength = 24;
        }
      ];
    };
    defaultGateway = "192.168.1.1";
    # This Pi IS the DNS server. Loopback to local Technitium for
    # itself. Plain-DNS fallbacks listed in case Technitium is briefly
    # unhealthy during nix-switch / service restart.
    nameservers = [
      "127.0.0.1"
      "::1"
      "9.9.9.11"
      "149.112.112.11"
      "1.1.1.1"
      "1.0.0.1"
    ];

    # Fast failover for cases where local Technitium returns SERVFAIL
    # during restart — 1s × 2 attempts means ~2s before falling through
    # to upstream.
    resolvconf.extraOptions = [
      "timeout:1"
      "attempts:2"
    ];
  };

  networking.firewall = {
    enable = true;
    # 22   = SSH
    # 443  = Tailscale Serve (Technitium admin UI HTTPS)
    # 9090 = Cockpit web UI
    # 9443 = Tailscale Serve (Cockpit HTTPS)
    #
    # 53 (TCP+UDP) + 5380 are opened by services.technitium-dns-server
    # (openFirewall = true, port list restricted to those two — see
    # nixos/rpi4/dns.nix for why 53443 is excluded). Listed here for
    # visibility, not duplication.
    allowedTCPPorts = [
      22
      443
      9090
      9443
    ];
    trustedInterfaces = [ "tailscale0" ];
  };

  services.tailscale = {
    enable = true;
    openFirewall = true;
    extraUpFlags = [ "--ssh" ];
  };

  # Tailscale Serve mappings — same pattern as rpi5.
  # - https://rpi4.tail08a5c5.ts.net/      → Technitium admin (port 5380)
  # - https://rpi4.tail08a5c5.ts.net:9443/ → Cockpit         (port 9090)
  #
  # `tailscale serve --bg --https=443` also handles the HTTP→HTTPS
  # redirect on port 80, so `http://rpi4...` works from a browser.
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
        ${pkgs.tailscale}/bin/tailscale serve --bg --https=443 http://localhost:5380
        ${pkgs.tailscale}/bin/tailscale serve --bg --https=9443 https+insecure://localhost:9090
        # Remove the old :8443 mapping if it's still configured from
        # the initial Technitium migration. Idempotent: no-op if unset.
        ${pkgs.tailscale}/bin/tailscale serve --https=8443 off || true
      '';
    };
  };

  # SD card filesystems. The sd-image-aarch64 module sets these for
  # initial image build but doesn't persist them at runtime; declaring
  # explicitly here ensures /boot/firmware is mounted (required for
  # uboot-builder to install firmware files during nixos-rebuild).
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

  # No swap by default — Technitium + Cockpit use ~200MB total, fits
  # comfortably in 4GB+ RAM. Add swap later if memory pressure becomes
  # a real concern.

  # Pi-specific kernel tunings — same rationale as rpi5.
  boot.kernel.sysctl = {
    "vm.swappiness" = 10;
  };

  # NTP servers as IPs, not hostnames. Pi 4 has no RTC battery, so on
  # power-up it boots with a stale clock (often months in the past). If
  # NTP servers were hostnames, they'd need DNS to resolve, but DNS goes
  # through Technitium which is on this same host — chicken-and-egg,
  # plus any TLS handshake (incl. Tailscale auth) fails because the
  # clock makes all certs look "not yet valid". IP-based NTP servers
  # bypass DNS entirely and let the clock sync immediately on first
  # boot. Cloudflare + Google anycast NTP, four redundant entries.
  services.timesyncd = {
    enable = true;
    servers = [
      "162.159.200.123" # time.cloudflare.com
      "162.159.200.1"
      "216.239.35.0" # time.google.com
      "216.239.35.4"
    ];
  };

}
