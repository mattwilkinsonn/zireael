{ pkgs, lib, ... }:

# rpi4 — Technitium DNS Server. Replaces the Pi-hole + dnscrypt-proxy
# stack: Technitium does DoH/DoT/DoQ natively (no separate encrypted-DNS
# proxy) and exposes a REST API for all config, so adlists / forwarders
# / login password seed declaratively from this module — same end state
# as the previous Pi-hole gravity.db + env-file pattern, just one
# process.
#
# Today the Pi is tailnet-only DNS (router still points clients at the
# ISP upstream). When/if it graduates to LAN-wide DNS, switching the
# router's DHCP DNS to 10.0.0.50 is the only change needed.

let
  # Declarative blocklists. Same lists that were in gravity.db before.
  # Add a new URL here, nix-switch, and the seed unit POSTs the full
  # set on next boot. Technitium replaces the list each call so
  # re-POSTing the same lists is harmless.
  technitiumBlockLists = [
    "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/adblock/pro.txt"
    "https://cdn.jsdelivr.net/gh/hagezi/dns-blocklists@latest/adblock/tif.txt"
  ];

  # Encrypted upstreams. Technitium speaks DoH natively, so these go
  # straight into the forwarder list — no dnscrypt-proxy hop. Quad9
  # filtered + Cloudflare, matching the prior dnscrypt-proxy roster.
  technitiumForwarders = [
    "https://dns.quad9.net/dns-query"
    "https://cloudflare-dns.com/dns-query"
  ];

  # Per-item curl arg fragments. Technitium 14's API expects array-style
  # parameters as REPEATED query params, not a newline-joined string:
  #
  #   ?forwarders=url1&forwarders=url2     (correct)
  #   ?forwarders=url1%0Aurl2              (Technitium parses literally
  #                                         as ONE entry containing the
  #                                         newline — silently broken)
  #
  # First-time setup we landed in the broken state and had to recover
  # by hand. Build the curl arg list at Nix-eval time so the script
  # body stays simple.
  forwarderArgs = lib.concatMapStringsSep " " (
    url: ''--data-urlencode "forwarders=${url}"''
  ) technitiumForwarders;
  blockListArgs = lib.concatMapStringsSep " " (
    url: ''--data-urlencode "blockListUrls=${url}"''
  ) technitiumBlockLists;
in
{
  services.technitium-dns-server = {
    enable = true;
    # 53 (DNS TCP+UDP) + 5380 (admin UI HTTP). Skip 53443 (Technitium's
    # built-in TLS port) — webServiceEnableTls defaults to false, so
    # nothing's listening there; keeping the firewall rule open just
    # makes the port look advertised. Tailscale Serve handles HTTPS
    # termination at :443 against the plain :5380 backend (see
    # tailscale-serve.service in nixos/rpi4/system.nix).
    openFirewall = true;
    firewallTCPPorts = [
      53
      5380
    ];
    firewallUDPPorts = [ 53 ];
  };

  # Seed Technitium config from the declarations above via its REST
  # API. Replaces the old pihole-adlists-seed + pihole-env-refresh
  # pair.
  #
  # First boot: log in with default admin/admin, set the admin password
  # from 1P, configure forwarders + blocklists + tailnet PTR forwarder.
  # Subsequent boots: log in with the real password and re-POST the
  # declared settings (idempotent — Technitium replaces lists, no
  # duplicates).
  #
  # Admin password is set once. Rotating it later means editing in the
  # web UI; we don't run an env-refresh loop because Technitium stores
  # the password hash in its own config DB, not an env file.
  systemd.services.technitium-seed = {
    description = "Seed Technitium DNS config from declarative Nix";
    wantedBy = [ "multi-user.target" ];
    after = [
      "technitium-dns-server.service"
      "network-online.target"
    ];
    requires = [ "technitium-dns-server.service" ];
    wants = [ "network-online.target" ];

    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      LoadCredentialEncrypted = "op-pi-svc:/etc/op-pi-svc-token.cred";
      RuntimeDirectory = "technitium-seed";
      Environment = [ "HOME=%t/technitium-seed" ];
      ExecStart = pkgs.writeShellScript "technitium-seed" ''
        set -euo pipefail

        export OP_SERVICE_ACCOUNT_TOKEN="$(cat "$CREDENTIALS_DIRECTORY/op-pi-svc")"
        OP="${pkgs._1password-cli}/bin/op"
        CURL="${pkgs.curl}/bin/curl --silent --show-error"
        JQ="${pkgs.jq}/bin/jq"
        API="http://127.0.0.1:5380/api"

        PASSWORD=$("$OP" read 'op://Server/Technitium Admin Password/password')
        [ -n "$PASSWORD" ] || { echo "ERROR: Technitium admin password empty from 1P" >&2; exit 1; }

        # Wait for the web API to come up. Technitium takes ~5-10s on
        # first boot (config DB init) and ~1s on warm restart. Probe
        # /api/user/login with a syntactically-valid but credential-
        # wrong call: a reachable API returns a status:error body in
        # 200 OK, an unreachable one fails the TCP connect.
        for i in $(seq 1 60); do
          if $CURL --max-time 2 -o /dev/null "$API/user/login?user=probe&pass=probe&includeInfo=false" 2>/dev/null; then
            break
          fi
          if [ "$i" = "60" ]; then
            echo "ERROR: Technitium web API not reachable after 2 minutes" >&2
            exit 1
          fi
          sleep 2
        done

        # Login helper. Returns the raw JSON body on stdout. Caller
        # extracts the token (or detects status:error).
        login() {
          local user="$1" pass="$2"
          $CURL --get \
            --data-urlencode "user=$user" \
            --data-urlencode "pass=$pass" \
            --data "includeInfo=false" \
            "$API/user/login"
        }

        # Try the real password first. If that fails, fall back to the
        # default admin/admin and rotate. Detects the "fresh server,
        # never seeded" state without needing a marker file — the API
        # response is the source of truth.
        RESP=$(login "admin" "$PASSWORD")
        TOKEN=$(echo "$RESP" | $JQ -r '.token // empty')

        if [ -z "$TOKEN" ]; then
          echo "Real password rejected — trying default admin/admin (first-boot path)"
          RESP=$(login "admin" "admin")
          TOKEN=$(echo "$RESP" | $JQ -r '.token // empty')
          if [ -z "$TOKEN" ]; then
            echo "ERROR: both real password and admin/admin rejected by Technitium" >&2
            echo "Login response: $RESP" >&2
            exit 1
          fi
          # changePassword needs BOTH `pass` (current) and `newPass`
          # (new) in Technitium 14. We just authenticated with
          # admin/admin, so that's the current pass.
          echo "First-boot: rotating admin password to 1P value"
          ROT=$($CURL --get \
            --data-urlencode "token=$TOKEN" \
            --data-urlencode "pass=admin" \
            --data-urlencode "newPass=$PASSWORD" \
            "$API/user/changePassword")
          if [ "$(echo "$ROT" | $JQ -r '.status // empty')" != "ok" ]; then
            echo "ERROR: changePassword failed: $ROT" >&2
            exit 1
          fi
          # New password takes effect immediately; reuse the existing
          # session token for the rest of this run.
        fi

        # Convenience wrapper — every authenticated call needs the
        # token and we want a fail-loud default. Technitium returns
        # 200 OK for both success and error envelopes (errors carry
        # `status: error` in the JSON body), so parsing the body is
        # the only way to detect a failed call. Bare `-o /dev/null`
        # like the first seed used silently swallows real failures.
        api_call() {
          local path="$1"
          shift
          local resp
          resp=$($CURL --get --data-urlencode "token=$TOKEN" "$@" "$API$path")
          local status
          status=$(echo "$resp" | $JQ -r '.status // empty')
          if [ "$status" != "ok" ]; then
            echo "ERROR: API $path returned status=$status" >&2
            echo "Response: $resp" >&2
            exit 1
          fi
        }

        # Settings: forwarders + blocklists + resolver mode.
        #
        # - recursion=Allow: switches from the default
        #   "AllowOnlyForPrivateNetworks" mode (which makes the
        #   resolver attempt full recursion against the roots and
        #   ignore the forwarder list) to "Allow" (use forwarders for
        #   every query). Without this every external query returns
        #   SERVFAIL because Technitium tries DNSSEC-validated
        #   recursion through residential ISP routing.
        # - forwarderProtocol=Https: REQUIRED in the same call as the
        #   forwarder URLs. Technitium parses each forwarder string
        #   in context of the active protocol — without Https, it
        #   strips `https://...` to a hostname and downgrades to
        #   plain UDP on port 53 (which DoH endpoints don't serve).
        # - dnssecValidation=true: validate upstream responses.
        # - forwarders / blockListUrls: REPEATED query params (see
        #   note on forwarderArgs / blockListArgs above).
        api_call /settings/set \
          --data "recursion=Allow" \
          --data "forwarderProtocol=Https" \
          --data "dnssecValidation=true" \
          --data "enableBlocking=true" \
          ${forwarderArgs} \
          ${blockListArgs}
        echo "Settings applied (recursion=Allow, forwarderProtocol=Https,"
        echo "  forwarders=${toString (builtins.length technitiumForwarders)},"
        echo "  blockListUrls=${toString (builtins.length technitiumBlockLists)})"

        # Force an immediate blocklist fetch. Without this, blocking
        # is "enabled" but the lists won't actually download until
        # the next scheduled update (default 24h).
        #
        # /blockList/forceUpdate is special — Technitium 14 returns
        # HTTP 200 with an EMPTY body for success (fire-and-forget;
        # the download runs async in the background). The generic
        # api_call wrapper requires `.status == "ok"` in the JSON body
        # and would fail this. Use a custom call that treats empty
        # body as success.
        BLOCKLIST_RESP=$($CURL --get --data-urlencode "token=$TOKEN" "$API/blockList/forceUpdate")
        if [ -n "$BLOCKLIST_RESP" ]; then
          BLOCKLIST_STATUS=$(echo "$BLOCKLIST_RESP" | $JQ -r '.status // empty')
          if [ "$BLOCKLIST_STATUS" != "ok" ]; then
            echo "ERROR: /blockList/forceUpdate returned: $BLOCKLIST_RESP" >&2
            exit 1
          fi
        fi
        echo "Blocklist fetch triggered (downloads + parses async; ~30-60s)"

        # Tailnet PTR forwarding. Tailscale assigns 100.64.0.0/10 IPs;
        # rDNS lives under 100.in-addr.arpa. Forwarding that zone to
        # MagicDNS (100.100.100.100) means the query log shows
        # `mattserver.tail08a5c5.ts.net` instead of bare 100.x IPs.
        # Without this, every tailnet client logs as an IP only — the
        # exact pain point Pi-hole had.
        #
        # Technitium 14 requires `forwarder` (and `protocol`) in the
        # SAME /zones/create call when type=Forwarder — there's no
        # separate /zones/records/add step needed. zones/create is
        # idempotent-ish: a second create returns status:error
        # "zone already exists"; we tolerate that specific message
        # and bail on anything else.
        api_call_tolerant() {
          local path="$1" tolerated_msg="$2"
          shift 2
          local resp
          resp=$($CURL --get --data-urlencode "token=$TOKEN" "$@" "$API$path")
          local status
          status=$(echo "$resp" | $JQ -r '.status // empty')
          if [ "$status" = "ok" ]; then
            return 0
          fi
          local err
          err=$(echo "$resp" | $JQ -r '.errorMessage // empty')
          if echo "$err" | grep -qiE "$tolerated_msg"; then
            return 0
          fi
          echo "ERROR: API $path returned status=$status, errorMessage=$err" >&2
          echo "Response: $resp" >&2
          exit 1
        }

        api_call_tolerant /zones/create "already exists" \
          --data "zone=100.in-addr.arpa" \
          --data "type=Forwarder" \
          --data "protocol=Udp" \
          --data-urlencode "forwarder=100.100.100.100"
        echo "Tailnet PTR forwarder ensured (100.in-addr.arpa → 100.100.100.100)"

        $CURL --get --data-urlencode "token=$TOKEN" "$API/user/logout" >/dev/null || true
        echo "Technitium seed complete at $(date -Iseconds)"
      '';
    };
  };
}
