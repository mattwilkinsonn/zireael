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
    # `iss` is the App ID as a JSON integer (unquoted %s — appId is
    # numeric), matching GitHub's spec + octokit.
    payload="$(printf '{"iat":%d,"exp":%d,"iss":%s}' "$((now - 60))" "$((now + 540))" "${appId}")"
    unsigned="$(printf '%s' "$header" | b64url).$(printf '%s' "$payload" | b64url)"
    sig="$(printf '%s' "$unsigned" | openssl dgst -sha256 -sign "$key_file" | b64url)"
    jwt="$unsigned.$sig"

    api() {
      curl -sS --connect-timeout 10 --max-time 30 --retry 2 --retry-delay 1 \
        -H "Authorization: Bearer $jwt" \
        -H "Accept: application/vnd.github+json" \
        -H "X-GitHub-Api-Version: 2022-11-28" "$@"
    }

    # Resolve the installation via the ORG-level endpoint — stable
    # regardless of which repo is queried (a per-repo endpoint would
    # 404 if that repo were renamed/archived/uninstalled, silently
    # killing every agent checkout). The response is compact JSON on a
    # single line with several "id" fields (the installation id first,
    # then account.id, etc.); `grep -o` prints EVERY match on the line,
    # so pipe through `head -n1` to take only the first. `grep -oE`
    # (ERE) keeps `+` portable to macOS's BSD grep (grep isn't in
    # runtimeInputs, so mattmacpro uses /usr/bin/grep).
    install_id="$(api "https://api.github.com/orgs/sealedsecurity/installation" \
      | grep -oE '"id":[[:space:]]*[0-9]+' | head -n1 | grep -oE '[0-9]+')"
    [ -n "$install_id" ] || { echo "buildkite-git-credential-app: no installation id" >&2; exit 0; }

    token="$(api -X POST "https://api.github.com/app/installations/$install_id/access_tokens" \
      | sed -n 's/.*"token":[[:space:]]*"\([^"]*\)".*/\1/p')"
    [ -n "$token" ] || { echo "buildkite-git-credential-app: token mint failed" >&2; exit 0; }

    printf 'username=x-access-token\n'
    printf 'password=%s\n' "$token"
  '';
}
