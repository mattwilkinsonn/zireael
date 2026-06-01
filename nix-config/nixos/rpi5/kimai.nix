{ lib, pkgs, ... }:

# Kimai — contractor-facing time tracking for Sealed Security.
#
# Public URL:
#   https://hours.sealedsecurity.com
#
# Ingress model:
#   Cloudflare Tunnel (remote-managed in Cloudflare Zero Trust)
#     → rpi5 cloudflared connector
#     → http://127.0.0.1:8080
#     → local nginx vhost
#     → Kimai PHP-FPM
#     → local MariaDB
#
# The Cloudflare tunnel token is intentionally not stored in Nix. Encrypt it
# once on rpi5 with nixos/scripts/rpi5-encrypt-kimai-cloudflared-token.sh,
# which writes the host-bound systemd credential consumed by the service below.

let
  kimaiHost = "hours.sealedsecurity.com";
  kimaiOriginPort = 8080;
  cloudflaredTokenCredential = "/etc/cloudflared/hours-sealedsecurity-com-token.cred";
in
{
  services.kimai.sites.${kimaiHost} = {
    # Local MariaDB is created automatically by the upstream Kimai NixOS
    # module. For this small, contractor-facing workload, keeping DB + app
    # colocated on rpi5 is simpler than coupling it to mattserver.
    database = {
      createLocally = true;
      name = "kimai";
      user = "kimai";
    };

    # Default pool is sized for a larger host (max_children=32). Keep the Pi
    # comfortably small; this service is expected to have only a few users.
    poolConfig = {
      "pm" = "dynamic";
      "pm.max_children" = 8;
      "pm.start_servers" = 2;
      "pm.min_spare_servers" = 1;
      "pm.max_spare_servers" = 3;
      "pm.max_requests" = 500;
    };

    # Non-secret runtime overrides. Kimai's generated .env still carries the
    # database URL and app secret; these environment variables override the
    # mailer/host/proxy defaults for the Cloudflare Tunnel deployment.
    # NOTE: pkgs.writeText lands in the world-readable Nix store. Keep only
    # non-secret values here; secrets must go through a separate mechanism.
    environmentFile = pkgs.writeText "kimai-hours-sealedsecurity-com.env" ''
      MAILER_FROM=hours@sealedsecurity.com
      MAILER_URL=null://null
      TRUSTED_HOSTS=^hours[.]sealedsecurity[.]com$
      TRUSTED_PROXIES=127.0.0.1,::1
    '';
  };

  services.nginx = {
    recommendedGzipSettings = true;
    recommendedOptimisation = true;
    recommendedProxySettings = true;

    # The Kimai module creates the vhost; this narrows the listener to
    # loopback so it is reachable by cloudflared only. No LAN/public firewall
    # port is opened for Kimai.
    virtualHosts.${kimaiHost}.listen = lib.mkForce [
      {
        addr = "127.0.0.1";
        port = kimaiOriginPort;
      }
      {
        addr = "[::1]";
        port = kimaiOriginPort;
      }
    ];
  };

  systemd.services.cloudflared-kimai = {
    description = "Cloudflare Tunnel connector for Kimai (${kimaiHost})";
    wantedBy = [ "multi-user.target" ];
    after = [
      "network-online.target"
      "nginx.service"
      "phpfpm-kimai-${kimaiHost}.service"
    ];
    wants = [
      "network-online.target"
      "nginx.service"
      "phpfpm-kimai-${kimaiHost}.service"
    ];

    serviceConfig = {
      Type = "simple";
      DynamicUser = true;
      LoadCredentialEncrypted = "tunnel-token:${cloudflaredTokenCredential}";
      ExecStart = pkgs.writeShellScript "cloudflared-kimai" ''
        set -euo pipefail

        export TUNNEL_TOKEN="$(${pkgs.coreutils}/bin/cat "$CREDENTIALS_DIRECTORY/tunnel-token")"
        exec ${pkgs.cloudflared}/bin/cloudflared tunnel \
          --edge-ip-version 4 \
          --no-autoupdate \
          run
      '';
      Restart = "on-failure";
      RestartSec = "5s";

      # cloudflared only needs outbound network access plus the loaded token.
      NoNewPrivileges = true;
      PrivateTmp = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectKernelModules = true;
      ProtectKernelLogs = true;
      ProtectControlGroups = true;
      RestrictSUIDSGID = true;
      LockPersonality = true;
      RestrictRealtime = true;
      SystemCallArchitectures = "native";
      SystemCallFilter = [ "@system-service" ];
      RestrictAddressFamilies = [
        "AF_INET"
        "AF_INET6"
      ];
    };
  };
}
