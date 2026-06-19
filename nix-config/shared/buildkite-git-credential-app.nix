# Git credential helper that mints a short-lived GitHub App installation
# token and hands it to git as the HTTPS password, so a self-hosted
# Buildkite agent can clone repos the App is installed on — public seal
# now, private sealed once the App is installed there.
#
# Self-hosted agents have no GitHub SSH key, and the Buildkite pipeline's
# repo URL is `git@github.com:…` (SSH); each host's git config rewrites
# that to `https://github.com/…` (insteadOf) so this helper applies.
# Checkout runs BEFORE the job's command phase, so the App key can't come
# from the per-job secret-env plugin — each host decrypts/stages it onto
# the box at boot and points `keyPath` at the result.
#
# Same JWT → installation-token flow as the seal repo's
# scripts/gh-app-token.sh. git invokes it as `<helper> get` and feeds
# key=value lines on stdin (protocol/host/…); we only handle github.com
# HTTPS and print `username` + `password`.
#
# Args:
#   pkgs    nixpkgs
#   appId   the GitHub App's numeric App ID (a public identifier, not a
#           secret — sealedsecurity-ci is 4045728)
#   keyPath absolute path to the staged App private-key .pem on the host
#           (mattserver: /run/buildkite-agent/ci-app-key.pem; mattmacpro:
#           /var/run/buildkite-agent/ci-app-key.pem)

{
  pkgs,
  appId,
  keyPath,
}:

pkgs.writeShellApplication {
  name = "buildkite-git-credential-app";
  runtimeInputs = with pkgs; [
    coreutils
    curl
    openssl
    gnused
  ];
  text = ''
    # git calls the helper with an operation arg (get/store/erase) and
    # feeds key=value lines on stdin. We only mint on `get`.
    op="''${1:-}"
    [ "$op" = "get" ] || exit 0

    host=""
    while IFS='=' read -r key value; do
      [ -z "$key" ] && break
      [ "$key" = "host" ] && host="$value"
    done

    # Only serve github.com over HTTPS; anything else, stay silent so git
    # falls through to its other helpers / default behaviour.
    [ "$host" = "github.com" ] || exit 0

    key_file="${keyPath}"
    [ -r "$key_file" ] || { echo "buildkite-git-credential-app: $key_file not readable" >&2; exit 0; }

    b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }

    now="$(date +%s)"
    header='{"alg":"RS256","typ":"JWT"}'
    payload="$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' "$((now - 60))" "$((now + 540))" "${appId}")"
    unsigned="$(printf '%s' "$header" | b64url).$(printf '%s' "$payload" | b64url)"
    sig="$(printf '%s' "$unsigned" | openssl dgst -sha256 -sign "$key_file" | b64url)"
    jwt="$unsigned.$sig"

    api() {
      curl -sS --connect-timeout 10 --max-time 30 --retry 2 --retry-delay 1 \
        -H "Authorization: Bearer $jwt" \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" "$@"
    }

    # Look up the installation on sealedsecurity/seal (the App is
    # installed org-wide; any repo it covers resolves the same
    # installation). The first "id" in the installation object is the
    # installation id.
    install_id="$(api "https://api.github.com/repos/sealedsecurity/seal/installation" \
      | grep -m1 -o '"id":[[:space:]]*[0-9]\+' | grep -o '[0-9]\+')"
    [ -n "$install_id" ] || { echo "buildkite-git-credential-app: no installation id" >&2; exit 0; }

    token="$(api -X POST "https://api.github.com/app/installations/$install_id/access_tokens" \
      | sed -n 's/.*"token":[[:space:]]*"\([^"]*\)".*/\1/p')"
    [ -n "$token" ] || { echo "buildkite-git-credential-app: token mint failed" >&2; exit 0; }

    printf 'username=x-access-token\n'
    printf 'password=%s\n' "$token"
  '';
}
