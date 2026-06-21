# dedicatedmacio-mini — Install Procedure

RENTED Apple Silicon Mac mini (M4, from [dedicatedmac.io](https://dedicatedmac.io)),
used as a **stopgap** macOS arm64 self-hosted Buildkite CI agent for the
sealedsecurity org (queue `macos-arm64-selfhosted` — the same pool the
owned `mattmini` and the AWS `awsmac` serve). Stands in while the
Buildkite-trial macOS minutes are exhausted and the owned mini isn't
racked yet.

**Hardware:**

- Apple Mac mini (M4, base — 10-core: 4 performance + 6 efficiency;
  verify P-core count with `sysctl -n hw.perflevel0.physicalcpu` and
  adjust `cargoBuildJobs` in `system.nix` if it's an M4 Pro)
- 16 GB unified memory
- Rented — provisioned + network-managed by dedicatedmac.io

**OS strategy:** current macOS, as provisioned by the host. The box
arrives with macOS already installed and an admin account created — no
clean install, no OCLP. The sandbox-exec tests behave the same as
production (no OS-version divergence to work around).

**Why a rental + the security model:** the box lives in dedicatedmac's
datacenter on a network we don't control, and the provider has
physical/root access. Two consequences shape this config:

1. **Tailnet is the only trust boundary.** Native sshd is disabled;
   Tailscale SSH + the tailnet ACLs (only the MBP can reach this host)
   are the entire access-control story. The pf agent-egress lockdown
   (in the shared `darwin/modules/buildkite-agent-macos.nix` module)
   cuts the agent UID off from the provider's RFC1918 subnet — even more
   warranted here than on the owned mini, since the surrounding LAN is
   shared rental infrastructure.
2. **Secrets are plaintext at rest, and that's fine because of the exit
   plan.** macOS has no `systemd-creds`; the agent token + CI App key
   live mode-600 root in `/etc/buildkite-agent` (same tradeoff the owned
   mini takes). We do **not** rotate them per-use. Instead, the
   [Rental exit](#rental-exit) is: wipe the box before the subscription
   ends. dedicatedmac re-images on return anyway, so the wipe is the
   revocation — the token + key become unrecoverable, no rotation
   needed.

## TL;DR

1. Get an admin shell on the box (provider's console/VNC, or their
   provisioned SSH).
2. Ensure the admin account is named `mattw` (see
   [macOS setup](#macos-setup)) — the module applies home-manager to
   `mattw`.
3. [Apply the basic hardening settings](#network--basic-hardening).
4. `gh repo clone mattwilkinsonn/zireael /tmp/dotfiles` (or scp the repo
   over), then
   `bash /tmp/dotfiles/nix-config/darwin/scripts/macos-runner-bootstrap.sh dedicatedmacio-mini`.

The bootstrap script handles everything else: Xcode CLT, Nix
(Determinate Systems installer), Homebrew, `gh` + zireael checkout,
Buildkite agent token, `darwin-rebuild`, Tailscale auth. You'll get
three interactive prompts: `gh auth login` flow, Buildkite agent token,
Tailscale pre-auth key.

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` on the Mac to confirm).
- [ ] Tailscale pre-auth key ready at
      <https://login.tailscale.com/admin/settings/keys>. Single-use,
      tagged with `tag:server,tag:ci-runner`. Paste at the bootstrap prompt.
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent
      Token) at
      <https://buildkite.com/organizations/sealedsecurity/agents>.
- [ ] sealedsecurity-ci App `.pem` ready (org → Developer settings →
      GitHub Apps → sealedsecurity-ci → Private keys). The bootstrap
      stages it to `/etc/buildkite-agent/ci-app-key.pem` for the
      checkout credential helper.

## macOS setup

The box arrives with macOS + an admin account already provisioned by
dedicatedmac. Two things to settle before bootstrapping:

1. **Account name must be `mattw`.** The module declares
   `users.users.mattw` (the `adminUser` default) and applies
   home-manager to it, and the `nix-switch` alias + agent state paths
   assume `/Users/mattw`. If dedicatedmac gave you a provider-named
   admin account:
   - Simplest: in System Settings → Users & Groups, create a new admin
     user `mattw` (short name `mattw`, password from 1Password — create
     `op://Dev/dedicatedmacio-mini Password` if not present), log in as
     it, and use that for the rest.
   - Renaming an existing macOS account in place is fiddly (home-dir +
     short-name + record name all have to move together) — creating a
     fresh `mattw` admin is less error-prone for a short-lived rental.
2. **Update to the latest point release**: System Settings → General →
   Software Update → install all pending updates before bootstrapping.

## Network + basic hardening

The box is on dedicatedmac's network (no physical access to plug in
Ethernet — it's already wired). In System Settings →

- **Sharing → Remote Login**: ON initially (lets you SSH in for the
  rest of the bootstrap; the module's activation disables native sshd
  once Tailscale SSH is up — Tailscale becomes the only path).
- **Sharing → Remote Management**: OFF (use Tailscale SSH + screen
  sharing over Tailscale instead).
- **Energy Saver → Power Adapter**:
  - Prevent automatic sleep when the display is off: ON
  - Start up automatically after a power failure: ON
  - Wake for network access: ON

  The module's `pmset` activation enforces all of these declaratively
  too — clicking now means the box stays up even before the first
  `darwin-rebuild` runs.
- **Login Items → Open at Login**: remove anything pre-populated.

## Bootstrap

Get the `nix-config` repo onto the box (`gh repo clone` or scp), then:

```bash
bash <path-to>/nix-config/darwin/scripts/macos-runner-bootstrap.sh dedicatedmacio-mini
```

The shared bootstrap is parameterized by hostname (admin user defaults
to `mattw`). It walks the steps in an order that gets Tailscale + SSH up
as early as possible so the rest is debuggable remotely:

1. **Xcode Command Line Tools** — triggers `xcode-select --install`
   if missing, waits for the GUI prompt to complete.
2. **macOS hostname** — sets `HostName` / `LocalHostName` /
   `ComputerName` all to `dedicatedmacio-mini` via `scutil`. nix-darwin
   re-applies later; doing it now means Tailscale registers with the
   correct name in step 4.
3. **Homebrew** — `/opt/homebrew` prefix (Apple Silicon). Needed for the
   Tailscale formula + gh + op.
4. **Tailscale** — installs the `tailscale` Homebrew **formula** (not
   the `tailscale-app` cask), starts `tailscaled` as a launchd system
   daemon via `sudo brew services start tailscale`, prompts for a
   pre-auth key (or reads `--auth-key` / `TAILSCALE_AUTH_KEY`), and runs
   `tailscale up --ssh --hostname=dedicatedmacio-mini`. Once this
   completes you can SSH from your laptop:
   `ssh mattw@dedicatedmacio-mini.tail08a5c5.ts.net` — and re-run the
   script over SSH instead of via the provider console.

   The formula is used instead of the cask because the cask is a
   Catalyst GUI whose CLI shim is broken on recent macOS (running the
   bundle exe errors with *"The current bundleIdentifier is unknown to
   the registry"*, and the in-app "Install CLI" menu item no longer
   works). The formula gives you upstream Go-built `tailscaled` +
   `tailscale` binaries — same as Linux.
5. **gh + dotfiles** — installs `gh`, prompts for `gh auth login`,
   clones your dotfiles into `~` (colocated git+jj at `~/.git`).
6. **Nix** — Determinate Systems installer (same as the MBP and awsmac).
   The upstream nixos.org installer's launchd daemon can crash-loop on
   provider-provisioned macOS (dyld library-validation rejects
   /nix/store dylibs in the hardened launchd context when /nix isn't a
   firmlink-blessed mount); Determinate sets the volume + firmlink +
   daemon up correctly. nix-darwin doesn't manage the daemon
   (`nix.enable = false`); the step also writes `/etc/nix/nix.custom.conf`
   for trust + binary caches.
7. **Buildkite agent token + CI App key** — prompts for the org agent
   token and writes it to `/etc/buildkite-agent/agent-token` (mode 600
   root:wheel); stages the sealedsecurity-ci App key to
   `/etc/buildkite-agent/ci-app-key.pem`. Both must be in place before
   step 8 since the agent daemon + checkout helper read them at launch.
8. **darwin-rebuild switch** — keeps native sshd disabled (Tailscale SSH
   is the only access path), locks pmset, lays down the agent
   LaunchDaemon + the decrypt-agent-secrets daemon (start immediately),
   the pf agent-egress lockdown, and Glances + tailscale-serve-glances.
9. **Sanity checks** — confirms native sshd is off, Tailscale SSH +
   agents + Glances are all up.

The script is re-runnable: every step skips if already done. Safe to
ctrl-C at any point and resume — including via SSH after step 4.

**Security posture:** dedicatedmacio-mini deliberately keeps no
1Password service-account tokens on disk — the host runs untrusted CI
workflows via the self-hosted Buildkite agent pool, so any SA accessible
to processes here is a credential a compromised agent could exfiltrate.
The agent token + CI App key in step 7 are the only secrets, mode-600
root-wheel. They are not rotated per-use — see [Rental exit](#rental-exit).

After it completes, you should be able to SSH from your MBP:

```bash
ssh mattw@dedicatedmacio-mini.tail08a5c5.ts.net
```

And the agent should appear at
<https://buildkite.com/organizations/sealedsecurity/agents> with the
tags `queue=macos-arm64-selfhosted` and `host=dedicatedmacio-mini`
(distinct from the owned mini's `host=mattmini` and the AWS box's
`host=awsmac`, so the three don't collide in the agents list).

## Glances dashboard

System metrics dashboard at
`https://dedicatedmacio-mini.tail08a5c5.ts.net:9443/` (after Tailscale
is up and the `glances` + `tailscale-serve-glances` launchd daemons have
run at least once). Reachable from any device on your tailnet.

If the page returns 404 on first visit, check
`/var/log/tailscale-serve-glances.log` — the serve hook re-runs
idempotently on every nix-switch, so a `sudo darwin-rebuild switch
--flake .#dedicatedmacio-mini` typically fixes it.

## Permissions / Gatekeeper

The agent binary lives in the nix store; the hand-rolled launchd daemon
(from the shared `darwin/modules/buildkite-agent-macos.nix` module) runs
it as the `_buildkite-agent` user. Gatekeeper doesn't gate
launchd-launched binaries in the nix store path, so no first-launch
`xattr -d com.apple.quarantine` dance is needed.

If a CI job tries to do something macOS gates (Accessibility access,
Screen Recording, Files & Folders), you'll see a prompt — pre-approve
under System Settings → Privacy & Security. The agent workload shouldn't
trip these, but `cargo nextest` with seal's sandbox tests exercises
`sandbox-exec`, which is built into macOS and works without approval.

## Updates / re-patches

After every macOS point release:

1. Apply the update via System Settings → Software Update; reboot.
2. SSH back in and `sudo darwin-rebuild switch --flake .#dedicatedmacio-mini`
   to re-apply pmset + sshd (point releases sometimes reset SystemSetup
   defaults).

## Rental exit

This is the deliberate revocation path — **do this before the
dedicatedmac subscription expires**, instead of pre-rotating secrets:

1. **Deregister the agent.** Stop the agent daemon and remove it from
   the Buildkite org so a stale agent doesn't linger in the pool:

   ```bash
   sudo launchctl bootout system/com.sealedsecurity.buildkite-agent-sealed-macos
   ```

   Then remove the agent at
   <https://buildkite.com/organizations/sealedsecurity/agents> (or it
   ages out on its own once it stops heartbeating).
2. **Untrack the Tailscale node** at
   <https://login.tailscale.com/admin/machines> so the tailnet name
   frees up and the device key is revoked.
3. **Wipe the box.** Erase All Content & Settings (System Settings →
   General → Transfer or Reset → Erase All Content and Settings), or
   just hand it back — dedicatedmac re-images on return either way. The
   plaintext `/etc/buildkite-agent/{agent-token,ci-app-key.pem}` go with
   it, so no rotation is needed; the wipe is the revocation.
4. **(Belt-and-suspenders)** If you want the token dead *before* the
   wipe completes — e.g. you're handing the box back without a personal
   erase — rotate the Buildkite agent token at the org Agents page and
   regenerate the sealedsecurity-ci App key. Both invalidate the
   on-disk copies immediately. For a self-erase this is optional.
