# mattmacpro — Install Procedure

Mac Pro 2013 "trashcan" repurposed as a self-hosted Buildkite CI agent
host for the sealedsecurity org (macOS x64 pool, queue `macos-x64-selfhosted`).

**Hardware:**

- Apple Mac Pro 6,1 (Late 2013)
- Intel Xeon E5 (E5-1650v2 / 2697v2 — 6c/12t typical; verify with `sysctl -n hw.ncpu`)
- 64 GB DDR3 ECC
- Dual AMD FirePro (D300 / D500 / D700 — irrelevant; box is headless)
- Apple SSD (~256 GB – 1 TB depending on variant)

**OS strategy:** OCLP-installed macOS Sonoma 14.x. The Mac Pro 2013 is
natively stuck on macOS Monterey (Apple dropped it from Ventura+), but
Monterey is already past security-update EOL and the Homebrew / CI
toolchain ecosystem is starting to drop it. Sonoma via OpenCore Legacy
Patcher keeps the box inside a supported window through ~late 2026.

## TL;DR

1. [Install macOS Sonoma via OCLP](#oclp-install).
2. [Apply the basic hardening settings](#network--basic-hardening) in
   System Settings.
3. Plug in a USB stick containing this `nix-config` repo (or `gh repo
   clone mattwilkinsonn/zireael /tmp/dotfiles` if you have an SSH/LAN
   path in already).
4. `bash <path-to>/nix-config/darwin/scripts/mattmacpro-bootstrap.sh`.

The bootstrap script handles everything else: Xcode CLT, Nix
(upstream, via the Determinate installer), Homebrew, `gh` + zireael
checkout, Buildkite agent token, `darwin-rebuild`, Tailscale auth.
You'll get three interactive prompts: `gh auth login` flow, Buildkite
agent token, Tailscale pre-auth key. That's it.

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` on the Mac to confirm).
- [ ] Tailscale pre-auth key ready at
      <https://login.tailscale.com/admin/settings/keys>. Single-use,
      tagged with `tag:ci-runner`. Paste at the bootstrap prompt.
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent
      Token) at
      <https://buildkite.com/organizations/sealedsecurity/agents>.
- [ ] Bootable Sonoma installer USB created via OpenCore Legacy Patcher
      (see [OCLP install](#oclp-install) below).

## OCLP install

One-time OS migration. Skip the bullet points you've already done.

1. **Boot the current Monterey install on the Mac Pro.** Sign in as an
   admin user.
2. **Download OCLP** from
   <https://github.com/dortania/OpenCore-Legacy-Patcher/releases>
   (latest stable). Run the `.dmg`, drag to `/Applications/`.
3. **Create the macOS Sonoma installer**. In OCLP → Create macOS
   Installer → "Download macOS Installer" → pick Sonoma 14.x →
   download. Then "Create macOS Installer" → flash to a 16 GB+ USB.
4. **Build OpenCore + install to disk**. OCLP → "Build and Install
   OpenCore" → "Install to Disk" → pick the internal SSD. Reboot.
5. **Reboot with OPTION held.** Pick "EFI Boot", then pick the Sonoma
   installer.
6. **Wipe + install Sonoma**. Disk Utility → erase internal SSD as
   APFS, name it "Macintosh HD". Quit Disk Utility, run the Sonoma
   installer. The box will reboot a few times — at each reboot hold
   OPTION and pick "EFI Boot" (or "Macintosh HD" once the first apply
   pass completes).
7. **Initial setup**. Create the `mattw` admin user (full name
   "Matt Wilkinson", short name `mattw`, password from 1Password —
   create the item `op://Dev/mattmacpro Password` if not already
   present). Skip iCloud, skip Siri, skip Find My, skip everything
   else — this is a headless CI host.
8. **Run OCLP root patches**. After login, open OCLP →
   "Post-Install Root Patch" → "Start Root Patching". Reboot when
   prompted. Applies FirePro GPU root patches + USB fixes. GPU
   patches will degrade Metal perf vs. native — irrelevant for a
   headless box.

**Future macOS updates**: every macOS point release (14.x → 14.y),
re-run the root-patch step after applying the update. OCLP catches
this automatically and prompts. See
<https://dortania.github.io/OpenCore-Legacy-Patcher/UNIVERSAL.html>.

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
bash <path-to>/nix-config/darwin/scripts/mattmacpro-bootstrap.sh
```

The script is fully self-contained. It walks 11 steps in an order
that gets Tailscale + SSH up as early as possible so the rest is
debuggable remotely (no more USB-shuffling to copy errors back):

1. **Xcode Command Line Tools** — triggers `xcode-select --install`
   if missing, waits for the GUI prompt to complete.
2. **macOS hostname** — sets `HostName` / `LocalHostName` /
   `ComputerName` all to `mattmacpro` via `scutil`. nix-darwin will
   re-apply later; doing it now means Tailscale registers with the
   correct name in step 4.
3. **Homebrew** — `/usr/local` prefix (x86_64). Needed for the
   Tailscale cask + gh + op in the next steps.
4. **Tailscale** — installs the `tailscale` Homebrew **formula**
   (not the `tailscale-app` cask), starts `tailscaled` as a launchd
   system daemon via `sudo brew services start tailscale`, prompts
   for a pre-auth key (or reads `--auth-key` / `TAILSCALE_AUTH_KEY`),
   and runs `tailscale up --ssh --hostname=mattmacpro`. Once this
   completes you can SSH from your laptop:
   `ssh mattw@mattmacpro.tail08a5c5.ts.net` — and re-run the script
   over SSH instead of via the console.

   The formula is used instead of the cask because the cask is a
   Catalyst GUI whose CLI shim is broken on recent macOS (running
   the bundle exe errors with *"The current bundleIdentifier is
   unknown to the registry"*, and the in-app "Install CLI" menu
   item no longer works). The formula gives you upstream Go-built
   `tailscaled` + `tailscale` binaries — same as Linux.
5. **gh + dotfiles** — installs `gh`, prompts for `gh auth login`,
   clones your dotfiles into `~` (colocated git+jj at `~/.git`).
6. **Nix** — Determinate Systems installer minus the `--determinate`
   flag (their proprietary fork doesn't ship x86_64-darwin). Upstream
   Nix; same installer machinery handles the synthetic-fs dance +
   daemon plist.
7. **Buildkite agent token** — prompts for the org agent token and
   writes it to `/etc/buildkite-agent/agent-token` (mode 600
   root:wheel). Must be in place before step 8 since the agent
   daemons read it at launch.
8. **darwin-rebuild switch** — keeps native sshd disabled (Tailscale
   SSH is the only access path), locks pmset, lays down the two agent
   LaunchDaemons + the decrypt-agent-token daemon (start immediately),
   and Glances + tailscale-serve-glances.
9. **Sanity checks** — confirms native sshd is off, Tailscale SSH +
   agents + Glances are all up.

The script is re-runnable: every step skips if already done. Safe to
ctrl-C at any point and resume — including via SSH after step 4.

**Security posture:** mattmacpro deliberately keeps no 1Password
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
ssh mattw@mattmacpro.tail08a5c5.ts.net
```

And the agents should appear at
<https://buildkite.com/organizations/sealedsecurity/agents>
with the tag `queue=macos-x64-selfhosted`.

## Glances dashboard

System metrics dashboard at
`https://mattmacpro.tail08a5c5.ts.net:9443/` (after Tailscale is up
and the `glances` + `tailscale-serve-glances` launchd daemons have
run at least once). Reachable from any device on your tailnet.

If the page returns 404 on first visit, check
`/var/log/tailscale-serve-glances.log` — the serve hook re-runs
idempotently on every nix-switch, so a `sudo darwin-rebuild switch
--flake .#mattmacpro` typically fixes it.

## Permissions / Gatekeeper

The agent binary lives in the nix store; the hand-rolled launchd
daemon (`darwin/mattmacpro/system.nix`) runs it as the
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

After every macOS Sonoma point release:

1. Open OCLP → it'll prompt to re-run root patches. Confirm; reboot.
2. SSH back in and `sudo darwin-rebuild switch --flake .#mattmacpro`
   to re-apply pmset + sshd (point releases sometimes reset
   SystemSetup defaults).

After a major macOS release (Sonoma 14 → 15+), OCLP support for the
Mac Pro 2013 may lag — wait for the OCLP project to officially bless
the new version before updating. See
<https://dortania.github.io/OpenCore-Legacy-Patcher/MODELS.html>.

## Rotating the agent token

When the agent token rotates:

```bash
sudo install -m 600 -o root -g wheel \
  /dev/stdin /etc/buildkite-agent/agent-token <<< 'new-token...'
sudo launchctl kickstart -k system/com.sealedsecurity.decrypt-agent-token
sudo launchctl kickstart -k \
  system/com.sealedsecurity.buildkite-agent-sealed-macos \
  system/com.sealedsecurity.buildkite-agent-sealed-macos-2
```

No `darwin-rebuild` needed — the decrypt daemon re-stages the token
and the agents re-read it on restart.
