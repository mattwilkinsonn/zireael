# mattlinuxpro — NixOS Install Procedure

2013 "trashcan" Mac Pro, converted from a retired macOS Buildkite runner
to a headless NixOS Linux CI agent host (SEA-839). Single-purpose box:
self-hosted Buildkite agents on the `linux-x64-selfhosted` queue,
alongside mattserver.

**Hardware:**

- Intel Xeon E5 (up to 12c/24t)
- 64 GB DDR3
- Internal PCIe/NVMe SSD → btrfs root
- Dual AMD FirePro GPUs (irrelevant headless)
- No T2 chip (2013 predates it) — clean EFI x86_64 box, no
  secure-boot / T2-storage headaches.

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` on the Mac to confirm).
- [ ] Tailscale: either approve interactively at first boot (the bootstrap
      runs `tailscale up --ssh` and prints a login URL — easiest on the
      headless console) or have a pre-auth key ready at
      <https://login.tailscale.com/admin/settings/keys> to pass via
      `--auth-key`.
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent Token; see
      "Buildkite agent token" section below).
- [ ] sealedsecurity-ci App `.pem` ready (see "Buildkite agent token").
- [ ] New `mattw` + `root` password ready to type at the bootstrap prompt
      (manually rotated — see "Security posture" below for why this isn't 1P-driven).
- [ ] Ventoy stick has the latest **NixOS 25.11 minimal or graphical ISO**
      from <https://nixos.org/download> (x86\_64-linux).

## Mac Pro firmware / boot

The trashcan boots via Apple EFI, which is quirky about third-party
bootloaders. Two things matter:

1. **Hold Option (⌥) at power-on** to reach the Mac boot picker, then
   pick the Ventoy USB ("EFI Boot"). A wired USB keyboard is the
   reliable way to catch the picker.
2. **No Secure Boot toggle exists** on this generation (no T2), so
   there's nothing to disable — systemd-boot's unsigned loader boots
   fine once installed. The installed ESP shows up in the Option picker
   as "EFI Boot" on subsequent boots.

Wiping macOS is just overwriting the internal SSD's partition table in
the partition step below — there's no T2 / Activation Lock to clear on
a 2013 box.

## Boot the installer

Insert Ventoy stick, hold ⌥ at power-on, select the NixOS ISO. From a
live TTY or Konsole, run everything below.

Get on the network first (wired ethernet is simplest on the trashcan):

```bash
ping 1.1.1.1
```

## Partition layout

Single internal SSD: ESP + swap + btrfs root. (Unlike mattserver, there's
no second disk / ZFS pool — this box only does CI.)

### Identify the disk

```bash
lsblk -d
```

Set the variable — **confirm before continuing** (NVMe is typically
`/dev/nvme0n1`; a SATA/AHCI SSD shows as `/dev/sda`):

```bash
DISK=/dev/nvme0n1
```

### Partition + format

```bash
sudo parted "$DISK" -- mklabel gpt
sudo parted "$DISK" -- mkpart ESP fat32 1MiB 1025MiB
sudo parted "$DISK" -- set 1 esp on
sudo parted "$DISK" -- mkpart swap linux-swap 1025MiB 17409MiB
sudo parted "$DISK" -- mkpart root btrfs 17409MiB 100%
```

Partition suffix differs by bus — NVMe uses `p1`/`p2`/`p3`, SATA uses
`1`/`2`/`3`. Adjust to match `lsblk`:

```bash
ESP="${DISK}p1"
SWAP="${DISK}p2"
ROOT="${DISK}p3"
```

```bash
sudo mkfs.fat -F 32 -n BOOT "$ESP"
sudo mkswap -L swap "$SWAP"
sudo mkfs.btrfs -f -L nixos "$ROOT"
```

Create subvolumes (layout matches mattserver/mattfw):

```bash
sudo mount "$ROOT" /mnt
sudo btrfs subvolume create /mnt/@
sudo btrfs subvolume create /mnt/@home
sudo btrfs subvolume create /mnt/@nix
sudo btrfs subvolume create /mnt/@log
sudo umount /mnt
```

Mount for nixos-install:

```bash
BTRFS_OPTS="compress=zstd:1,noatime,ssd,discard=async"

sudo mount -o "subvol=@,$BTRFS_OPTS" "$ROOT" /mnt
sudo mkdir -p /mnt/{home,nix,var/log,boot}
sudo mount -o "subvol=@home,$BTRFS_OPTS" "$ROOT" /mnt/home
sudo mount -o "subvol=@nix,$BTRFS_OPTS" "$ROOT" /mnt/nix
sudo mount -o "subvol=@log,$BTRFS_OPTS" "$ROOT" /mnt/var/log
sudo mount "$ESP" /mnt/boot
sudo swapon "$SWAP"
```

## Run the installer

Copy the flake from Ventoy and run nixos-install:

```bash
mkdir -p /tmp/flake
cp -r /run/media/*/Ventoy/home-manager/. /tmp/flake/

sudo nixos-generate-config --root /mnt --dir /tmp/nixos-gen
cat /tmp/nixos-gen/hardware-configuration.nix
# Check initrd.availableKernelModules for the trashcan's storage
# controller (AHCI / NVMe) — the generated list should cover it.

sudo nixos-install --flake /tmp/flake#mattlinuxpro --no-root-passwd
```

Reboot and pull the Ventoy stick:

```bash
sudo reboot
```

## First boot

Login as `mattw` (initial password from `nixos/common.nix`). Run the
bootstrap from the Ventoy stick (the repo hasn't been cloned yet):

```bash
ls /run/media/*/Ventoy/home-manager/nixos/scripts/mattlinuxpro-bootstrap.sh
bash /run/media/*/Ventoy/home-manager/nixos/scripts/mattlinuxpro-bootstrap.sh
```

## Security posture

mattlinuxpro runs untrusted PR-time CI workloads via the self-hosted
Buildkite agent pool. The threat model assumes a malicious workflow
could land code-exec as a `buildkite-agent-*` user — already mitigated
upstream by external-PR approval gates + dep-update cooldowns, but
defense-in-depth on the host shrinks the blast radius if those upstream
layers fail. Same posture as mattserver:

- **Zero standing 1Password service-account tokens.** No personal SA,
  no team SA. The bootstrap script (step 4) prompts for the user
  password manually rather than fetching from 1P. Any SA token on disk
  is something a compromised agent could exfiltrate; the simplest
  answer is "have no SA tokens." Password rotation is operator-driven
  (run the bootstrap script and type the new password); the upside is
  the agent has zero capability against the 1P account.
- **Agent token + ci-app-key encrypted at rest** via systemd-creds
  (host-bound machine-ID encryption). The encrypted files under
  `/etc/buildkite-agent/` can't be decrypted off this box; the
  plaintext only lives in tmpfs after `decrypt-agent-token.service`
  runs at boot. See "Buildkite agent token" below.
- **No native sshd on the LAN side.** `services.openssh.enable =
  lib.mkForce false` on this host (overrides the common.nix default).
  Tailscale SSH covers every interactive access path.
- **No Cockpit.** `services.cockpit.enable = lib.mkForce false` — this
  host inherits cockpit from common.nix but doesn't tailscale-serve it,
  so the only path was LAN inbound on :9090. Replaced operationally by
  Tailscale SSH + `btm` for live monitoring.
- **No host-level egress lockdown on the agent (yet).** PR jobs run
  inside the seal-ci container (the Buildkite docker plugin), where a
  GID-by-`output` nftables rule both mis-scopes (supplementary group)
  and misses the hook (container egress is `forward`/`postrouting`).
  Container-aware LAN egress isolation is tracked in **SEA-835**, shared
  with mattserver.

## Buildkite agent token

> The agents register with the Buildkite **Agent** token (org Agents
> page → Reveal Agent Token) — distinct from the `BUILDKITE_API_TOKEN`
> the `bk` CLI uses. The encrypted file at
> `/etc/buildkite-agent/agent-token.cred` must exist before the
> `buildkite-agent-*` units start, or they fail with no token to
> decrypt. The rest of the host works without it if you're not wiring
> up the agents yet.

The two agent instances (`sealed`, `sealed-2`) share a single
org-scoped agent token. The plaintext is encrypted at rest via
`systemd-creds` (host-bound — only this box's machine ID can decrypt)
and decrypted into tmpfs (`/run/buildkite-agent/agent-token`) at boot by
`decrypt-agent-token.service` (declared in the shared
`nixos/modules/buildkite-agent.nix` module).

Get the agent token from
<https://buildkite.com/organizations/sealedsecurity/agents> (Reveal
Agent Token). Then encrypt it with the helper script (prompts for the
token, writes the host-bound .cred file):

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattlinuxpro-encrypt-agent-token.sh
```

Also stage the sealedsecurity-ci App private key (the checkout git
credential helper signs JWTs with it to mint clone tokens):

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattlinuxpro-encrypt-ci-app-key.sh /path/to/sealedsecurity-ci.pem
shred -u /path/to/sealedsecurity-ci.pem
```

Each script verifies the decrypt path before exiting, so a successful
run means the agent units will pick up the same plaintext at boot.

> **Why systemd-creds instead of plaintext files:** the .cred files are
> decryptable only by this host (encryption is bound to the machine ID),
> so an attacker who exfiltrates `/etc/buildkite-agent/` off the box
> gets a useless blob. The plaintext only ever lives in tmpfs after
> decrypt — it disappears on shutdown.

Rebuild — both agent services come up and register:

```bash
nix-switch
sudo systemctl status \
  decrypt-agent-token \
  buildkite-agent-sealed \
  buildkite-agent-sealed-2
```

Confirm the two agents appear at
<https://buildkite.com/organizations/sealedsecurity/agents> with the tag
`queue=linux-x64-selfhosted` and `host=mattlinuxpro`.

To rotate the token, re-run the encrypt script with the new value and
restart the decrypt + agent units:

```bash
sudo bash ~/repos/zireael/nix-config/nixos/scripts/mattlinuxpro-encrypt-agent-token.sh
sudo systemctl restart decrypt-agent-token.service
sudo systemctl restart 'buildkite-agent-sealed*.service'
```

To expand the pool (sealed-3, etc.), add the name to
`sealed.buildkiteAgent.agentNames` in `nixos/mattlinuxpro/system.nix`
and rebuild — the module generates the agent + its decrypt-ordering unit
from the list, and the org-level agent token registers any number of
agents with no token change. (Going past 2 agents only pays off with
per-container CPU pinning — see SEA-844.)

## SSH from the Mac

Add to `~/.ssh/config` (Mac side) — substitute the actual tailnet name
from `tailscale status`:

```text
Host mattlinuxpro
  HostName mattlinuxpro.tail08a5c5.ts.net
  User mattw
  IdentityAgent ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock
```
