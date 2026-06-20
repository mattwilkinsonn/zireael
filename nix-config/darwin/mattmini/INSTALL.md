# mattmini — Install Procedure

Apple Silicon Mac mini (M2 Pro) self-hosted Buildkite CI agent host
for the sealedsecurity org (macOS arm64 pool, queue
`macos-arm64-selfhosted`).

**Hardware:**

- Apple Mac mini (M2 Pro)
- 10- or 12-core M2 Pro (verify P-core count with
  `sysctl -n hw.perflevel0.physicalcpu`)
- 16 GB unified memory
- 512 GB SSD

**OS strategy:** current macOS, installed natively. Keep the OS on
the latest point release; standard Software Update applies. The
sandbox-exec tests behave the same as production (no OS-version
divergence to work around).

## TL;DR

1. [Install macOS](#macos-install) (standard, native).
2. [Apply the basic hardening settings](#network--basic-hardening) in
   System Settings.
3. Plug in a USB stick containing this `nix-config` repo (or `gh repo
   clone mattwilkinsonn/zireael /tmp/dotfiles` if you have an SSH/LAN
   path in already).
4. `bash <path-to>/nix-config/darwin/scripts/mattmini-bootstrap.sh`.

The bootstrap script handles everything else: Xcode CLT, Nix
(upstream nixos.org installer), Homebrew, `gh` + zireael checkout,
Buildkite agent token, `darwin-rebuild`, Tailscale auth. You'll get
three interactive prompts: `gh auth login` flow, Buildkite agent
token, Tailscale pre-auth key. That's it.

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` on the Mac to confirm).
- [ ] Tailscale pre-auth key ready at
      <https://login.tailscale.com/admin/settings/keys>. Single-use,
      tagged with `tag:ci-runner`. Paste at the bootstrap prompt.
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent
      Token) at
      <https://buildkite.com/organizations/sealedsecurity/agents>.

## macOS install

Native install — no OCLP. Apple Silicon runs current macOS directly.

1. **Initial setup**. Boot the mini, run through Setup Assistant.
   Create the `mattw` admin user (full name "Matt Wilkinson", short
   name `mattw`, password from 1Password — create the item
   `op://Dev/mattmini Password` if not already present). Skip iCloud,
   skip Siri, skip Find My, skip everything else — this is a headless
   CI host.
2. **Update to the latest point release**. System Settings → General →
   Software Update → install all pending updates before bootstrapping.

**Future macOS updates**: standard Software Update.

## Network + basic hardening

Plug the box into wired Ethernet. Then System Settings →

- **Sharing → Remote Login**: ON (lets you SSH in for the rest of the
  bootstrap; the system.nix activation locks this in via `systemsetup`
  too, but ticking it now means you can run the bootstrap remotely if
  you'd rather).
- **Sharing → Remote Management**: OFF (use Tailscale SSH + screen
  sharing over Tailscale instead).
- **Energy Saver / Battery → Power Adapter**:
  - Prevent automatic sleep when the display is off: ON
  - Start up automatically after a power failure: ON
  - Wake for network access: ON

  The system.nix `pmset` activation enforces all of these
  declaratively too — clicking now means the box stays up even
  before the first `darwin-rebuild` runs.
- **Login Items → Open at Login**: remove anything pre-populated.

## Bootstrap

Get the `nix-config` repo onto the box (USB or `gh repo clone` —
whichever's easier; the script handles either path), then:

```bash
bash <path-to>/nix-config/darwin/scripts/mattmini-bootstrap.sh
```

The script is fully self-contained. It walks 11 steps in an order
that gets Tailscale + SSH up as early as possible so the rest is
debuggable remotely (no more USB-shuffling to copy errors back):

1. **Xcode Command Line Tools** — triggers `xcode-select --install`
   if missing, waits for the GUI prompt to complete.
2. **macOS hostname** — sets `HostName` / `LocalHostName` /
   `ComputerName` all to `mattmini` via `scutil`. nix-darwin will
   re-apply later; doing it now means Tailscale registers with the
   correct name in step 4.
3. **Homebrew** — `/opt/homebrew` prefix (Apple Silicon). Needed for
   the Tailscale cask + gh + op in the next steps.
4. **Tailscale** — installs the `tailscale` Homebrew **formula**
   (not the `tailscale-app` cask), starts `tailscaled` as a launchd
   system daemon via `sudo brew services start tailscale`, prompts
   for a pre-auth key (or reads `--auth-key` / `TAILSCALE_AUTH_KEY`),
   and runs `tailscale up --ssh --hostname=mattmini`. Once this
   completes you can SSH from your laptop:
   `ssh mattw@mattmini.tail08a5c5.ts.net` — and re-run the script
   over SSH instead of via the console.

   The formula is used instead of the cask because the cask is a
   Catalyst GUI whose CLI shim is broken on recent macOS (running
   the bundle exe errors with *"The current bundleIdentifier is
   unknown to the registry"*, and the in-app "Install CLI" menu
   item no longer works). The formula gives you upstream Go-built
   `tailscaled` + `tailscale` binaries — same as Linux.
5. **gh + dotfiles** — installs `gh`, prompts for `gh auth login`,
   clones your dotfiles into `~` (colocated git+jj at `~/.git`).
6. **Nix** — upstream nixos.org multi-user installer (not Determinate's
   fork). The standard daemon lets nix-darwin manage the daemon plist
   declaratively (`nix.enable = true`); same installer machinery
   handles the synthetic-fs dance + daemon plist.
7. **Buildkite agent token** — prompts for the org agent token and
   writes it to `/etc/buildkite-agent/agent-token` (mode 600
   root:wheel). Must be in place before step 8 since the agent
   daemon reads it at launch.
8. **darwin-rebuild switch** — keeps native sshd disabled (Tailscale
   SSH is the only access path), locks pmset, lays down the agent
   LaunchDaemon + the decrypt-agent-secrets daemon (start immediately),
   and Glances + tailscale-serve-glances.
9. **Sanity checks** — confirms native sshd is off, Tailscale SSH +
   agents + Glances are all up.

The script is re-runnable: every step skips if already done. Safe to
ctrl-C at any point and resume — including via SSH after step 4.

**Security posture:** mattmini deliberately keeps no 1Password
service-account tokens on disk — the host runs untrusted CI
workflows via the self-hosted Buildkite agent pool, so any SA
accessible to processes here is a credential a compromised agent
could exfiltrate. The agent token in step 7 is the only secret,
mode-600 root-wheel. Earlier bootstrap versions provisioned a
personal SA into Keychain at step 7 and fetched a shared
`inter-server` SSH key — both retired. If your host still has either,
clean them up:

```bash
security delete-generic-password -a "$USER" -s OP_SERVICE_ACCOUNT_TOKEN
rm -f ~/.ssh/id_ed25519_inter_server ~/.ssh/id_ed25519_inter_server.pub
```

After it completes, you should be able to SSH from your MBP:

```bash
ssh mattw@mattmini.tail08a5c5.ts.net
```

And the agent should appear at
<https://buildkite.com/organizations/sealedsecurity/agents>
with the tag `queue=macos-arm64-selfhosted`.

## Glances dashboard

System metrics dashboard at
`https://mattmini.tail08a5c5.ts.net:9443/` (after Tailscale is up
and the `glances` + `tailscale-serve-glances` launchd daemons have
run at least once). Reachable from any device on your tailnet.

If the page returns 404 on first visit, check
`/var/log/tailscale-serve-glances.log` — the serve hook re-runs
idempotently on every nix-switch, so a `sudo darwin-rebuild switch
--flake .#mattmini` typically fixes it.

## Permissions / Gatekeeper

The agent binary lives in the nix store; the hand-rolled launchd
daemon (`darwin/mattmini/system.nix`) runs it as the
`_buildkite-agent` user. Gatekeeper doesn't gate launchd-launched
binaries in the nix store path, so no first-launch
`xattr -d com.apple.quarantine` dance is needed.

If a CI job tries to do something macOS gates (Accessibility access,
Screen Recording, Files & Folders), you'll see a prompt — pre-approve
under System Settings → Privacy & Security. The agent workload
shouldn't trip these, but `cargo nextest` with seal's sandbox tests
exercises `sandbox-exec`, which is built into macOS and works without
approval.

## Updates / re-patches

After every macOS point release:

1. Apply the update via System Settings → Software Update; reboot.
2. SSH back in and `sudo darwin-rebuild switch --flake .#mattmini`
   to re-apply pmset + sshd (point releases sometimes reset
   SystemSetup defaults).

> **⚠️ A macOS reinstall can wipe `/nix`.** If a full reinstall (or a
> botched OS update) empties the nix store, `/nix` ends up gone (store
> missing, `nix` not on PATH, no `/nix/var/nix/profiles`) while the
> nix-darwin `/etc` symlinks (`/etc/zshrc → /etc/static → /nix/store/…`)
> are left behind pointing at a store path that no longer exists.
> Symptoms: SSH lands in a **bare prompt** (dangling `/etc/static`
> means the login shell can't source `path_helper`), and system tools
> fail with `scutil: command not found` / `shutdown: command not found`
> because `/usr/sbin` + `/sbin` never made it onto PATH.
>
> **Diagnosis:** `ls /nix/var/nix/profiles/` (empty) + `which nix`
> (not found) + `ls -d /run/current-system` (missing) = nix store
> wiped, full re-bootstrap needed (not a re-activate).
>
> **Recovery:**
>
> 1. Restore a usable PATH for the session:
>
>    ```bash
>    export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/opt/homebrew/sbin:$PATH"
>    ```
>
> 2. Re-run the bootstrap from scratch — it reinstalls Nix, Homebrew,
>    re-clones zireael, re-prompts for the agent token, and
>    `darwin-rebuild switch` recreates `/run/current-system`, which
>    repairs the dangling `/etc/static` symlinks:
>
>    ```bash
>    bash ~/repos/zireael/nix-config/darwin/scripts/mattmini-bootstrap.sh
>    ```
>
>    (The bootstrap script self-exports the base PATH at its top, so it
>    survives this broken state even if you forget step 1. If the
>    zireael checkout itself was also wiped, `gh repo clone
>    mattwilkinsonn/zireael ~/repos/zireael` first — needs `gh` auth,
>    or copy the repo from a USB stick.)

## Rotating the agent token

When the agent token rotates:

```bash
# BSD `install` can't read /dev/stdin, so lay down the file empty with
# the right mode first, then write the token via tee.
sudo install -m 600 -o root -g wheel /dev/null /etc/buildkite-agent/agent-token
printf '%s' 'new-token...' | sudo tee /etc/buildkite-agent/agent-token >/dev/null
sudo launchctl kickstart -k system/com.sealedsecurity.decrypt-agent-secrets
sudo launchctl kickstart -k \
  system/com.sealedsecurity.buildkite-agent-sealed-macos
```

No `darwin-rebuild` needed — the decrypt daemon re-stages the token
and the agents re-read it on restart.
