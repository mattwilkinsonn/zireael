# mattpc — bare-metal dual-boot install

Turns the gaming PC into a **bare-metal NixOS daily driver** dual-booting
Windows (games only). NixOS takes **Disk 0** (the 2 TB Samsung 980 PRO that
held the WSL `ext4.vhdx`); **Windows stays on Disk 1, untouched**.

## Hardware (confirmed from the running WSL host)

| Disk | Serial (Windows-reported) | Was | Becomes |
| --- | --- | --- | --- |
| **0** | `0025_38BA_21A1_303A` | `G:` — 99% the WSL vhdx | **NixOS** (wiped) |
| **1** | `0025_38B2_4140_9DA2` | `C:` Windows + 200 MB ESP | **untouched** |

Both are the **identical model**, so the installer targets Disk 0 by
`/dev/disk/by-id` (serial), never `/dev/nvmeXn1`.

Layout (disko): 2 GiB ESP (vfat `/boot`) · 64 GiB swap (= RAM, hibernate) ·
btrfs root with `@` → `/`, `@home` → `/home`, `@nix` → `/nix` (zstd).

## 0. Pre-flight (before touching the disk)

1. **Push every agent's work** and anything uncommitted in the WSL distro —
   the wipe destroys the WSL distro on Disk 0.
2. Confirm nothing else on `G:` matters (it's ~0.3 GB beyond the vhdx).
3. Have a NixOS installer USB (any recent unstable ISO) + this repo reachable.

## 1. Boot the installer + identify Disk 0

Boot the NixOS installer USB. Identify the target by **elimination** (the
serials look alike, so verify by content):

```bash
lsblk -f          # find the two nvme disks
ls -l /dev/disk/by-id/ | grep -i nvme
```

- **Disk 1 (Windows — DO NOT TOUCH):** the NVMe with a ~200 MB vfat ESP + a
  large NTFS partition (Windows `C:`).
- **Disk 0 (target):** the NVMe with a single large NTFS labelled `Games` and
  nothing else.

Copy Disk 0's stable path, e.g. `/dev/disk/by-id/nvme-Samsung_SSD_980_PRO_2TB_<serial>`.

## 2. Fill the install-time placeholder

Clone the repo in the installer, then edit:

- `nixos/mattpc/disko.nix` → set `device` to Disk 0's by-id path (replaces
  `REPLACE-WITH-DISK0-BY-ID`).

That's the only install-time placeholder. The Windows dual-boot needs no config
placeholder — it's selected through the firmware-native Windows Boot Manager
entry (§8), not a chainloaded systemd-boot entry.

## 3. Partition + format + mount (disko)

```bash
# DESTROYS Disk 0. Double-check the device is Disk 0, not Windows.
sudo nix --experimental-features "nix-command flakes" run github:nix-community/disko -- \
  --mode disko ./nixos/mattpc/disko.nix
```

disko partitions, formats (btrfs subvols + swap + ESP), and mounts everything
under `/mnt`.

## 4. Reconcile hardware config

```bash
sudo nixos-generate-config --no-filesystems --root /mnt
```

`--no-filesystems` keeps disko as the single source of the mount layout. Diff
the generated `/mnt/etc/nixos/hardware-configuration.nix` against this repo's
`nixos/mattpc/hardware-configuration.nix` and merge any extra
`boot.initrd.availableKernelModules` the scan found.

## 5. Install

```bash
sudo nixos-install --flake /mnt/etc/nixos-repo/nix-config#mattpc
# (point --flake at wherever you cloned the repo)
reboot
```

Set the user password on first login (or via `nixos-install`'s prompt).

> **Secure Boot on a from-scratch install:** the flake has lanzaboote enabled,
> so `nixos-install` signs the first generation and needs the keys present in
> the target first. Create them into `/mnt` before running the install above —
> see the reinstall note in §7. (Not needed for the already-installed machine,
> which converges to lanzaboote in §7 step 2.)

## 6. Provision secrets + first converge

On first boot, log in as `mattw` and run the bootstrap once:

```bash
bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-bootstrap.sh
# pass --auth-key <tskey> to skip the interactive tailscale login
```

It's the bare-metal analogue of `mattpc-wsl-bootstrap.sh` (single-phase — no
Windows copy, no user migration). Idempotent, so re-run it if a step fails. It:

1. Ensures `~/repos/zireael` (+ `privatefiles`) is cloned and jj-colocated.
2. Writes both 1Password service-account tokens to
   `~/.config/op/{service-account-token,team-service-account-token}` (mode
   600) — **prompts you to paste each one**. These are what
   `nixos/mattpc/home.nix` exports into the shell env; without them every
   `op://`-backed secret (API keys, etc.) stays unloaded.
3. Brings up this host's own tailscaled with `--ssh` (bare metal runs its own
   daemon; WSL borrowed Windows').
4. Re-converges (`nixos-rebuild switch --flake …#mattpc`) with the token in
   env so the op-backed home-manager activations actually fire.
5. Sets the `mattw` + `root` login/sudo passwords from a **single interactive
   prompt** — stored nowhere, never in 1Password (the service-account token
   must never be able to read the login password).

Operator input at run time is exactly: the two service-account tokens, and the
new login password (each prompted once).

> **Next:** Secure Boot is a separate, later step (§7). Leave Secure Boot
> **off** in firmware until the lanzaboote keys are enrolled, or the signed
> chain isn't in place yet and the machine won't boot.

## 7. Enable Secure Boot (lanzaboote keys + enrollment)

The `mattpc` config boots through **lanzaboote**: systemd-boot and every NixOS
generation are signed with machine-owner keys the firmware verifies under Secure
Boot. Provision the keys, enroll them once (with Microsoft's), then turn Secure
Boot on. Do this **after** the machine is installed, booted, and bootstrapped
(§6), with Secure Boot still **off** in firmware.

> **Order matters — lanzaboote signs on every converge, so the keys must exist
> before the first one.** On this already-installed machine, step 1 sources
> `sbctl` transiently and creates the keys *before* step 2's converge (the
> converge is what puts `sbctl` on PATH permanently and activates the signed
> chain) — just follow the steps in order. On a *from-scratch reinstall*
> lanzaboote is in the config from the start, so `nixos-install` itself signs:
> create the keys into the target before §5 with
> `nix shell nixpkgs#sbctl --command sudo sbctl create-keys`, then
> `sudo mkdir -p /mnt/var/lib && sudo cp -a /var/lib/sbctl /mnt/var/lib/`.

1. **Create the machine-owner keys.** `sbctl` isn't on PATH yet — it arrives
   with the lanzaboote generation in step 2, but that generation can't sign
   until the keys exist. Source `sbctl` transiently for this one step:

   ```bash
   nix shell nixpkgs#sbctl --command sudo sbctl create-keys
   ```

   Keys land in `/var/lib/sbctl` (the private key is root-only). Steps 3/5/7 use
   the `sbctl` that step 2 puts on PATH permanently.

2. **Converge so lanzaboote signs the current generation** with the new keys.
   This also replaces the unsigned systemd-boot with the signed chain and puts
   `sbctl` + `efibootmgr` on PATH:

   ```bash
   nix-switch    # = sudo nixos-rebuild switch --flake ~/repos/zireael/nix-config#mattpc
   ```

3. **Verify every ESP boot file is signed.** The kernels under `EFI/nixos/` are
   expected to show *unsigned* — only the per-generation UKIs and systemd-boot
   itself are signed:

   ```bash
   sudo sbctl verify
   ```

4. **Put the firmware in Setup Mode** (MSI PRO Z690-A): reboot, **Del** at POST,
   **F7** for Advanced Mode, then **Settings → Security → Secure Boot**. Set
   *Secure Boot Mode* to **Custom**, then under **Key Management** reset/delete
   the **Platform Key (PK)** to enter Setup Mode. Leave Secure Boot itself
   **disabled** for now. **F10** to save, boot back into NixOS.

   > Do **not** choose "Clear/Delete all Secure Boot keys" wholesale — that also
   > drops the Forbidden Signature Database (dbx). Reset only the PK.

5. **Enroll the keys, including Microsoft's.** Mandatory on this box: Windows'
   `bootmgfw.efi` *and* the RTX 4080's UEFI option-ROM are Microsoft-signed, and
   owner-only keys can make the board refuse to POST:

   ```bash
   sudo sbctl enroll-keys --microsoft
   ```

6. **Enable Secure Boot** in firmware: reboot → **Del** → **Settings → Security
   → Secure Boot** → set **Secure Boot** to *Enabled*, **F10** to save.

7. **Confirm** from the booted system:

   ```bash
   bootctl status | grep -i 'secure boot'     # → Secure Boot: enabled (user)
   ```

### Secure Boot recovery (MSI PRO Z690-A)

If enrollment leaves the board unable to POST, or NixOS won't boot under Secure
Boot:

- **Disable Secure Boot in firmware:** **Del** at POST → **F7** (Advanced) →
  **Settings → Security → Secure Boot** → set **Secure Boot** to *Disabled*. An
  unsigned NixOS install / USB boots again with Secure Boot off.
- **Clear the custom keys** from that same **Secure Boot** menu (Key Management
  → reset to factory defaults / delete custom keys) if the enrolled set is what
  broke POST.
- **Last resort — clear CMOS (JBAT1 jumper):** power off, move the **JBAT1** cap
  from pins **1-2 to 2-3** for a few seconds, then back to **1-2**. JBAT1 is
  next to the CMOS battery near the board's bottom edge. This resets all firmware
  settings, including Secure Boot state and custom keys.

## 8. Windows dual-boot: select over SSH

Windows is on Disk 1 with its own ESP and its own Microsoft-signed
`bootmgfw.efi`, so the firmware keeps a native **Windows Boot Manager** UEFI
entry that boots under Secure Boot once Microsoft keys are enrolled (§7). Select
it with a genuine UEFI one-shot — no console needed:

```bash
efibootmgr -v                                 # one-time: find the "Windows Boot Manager" Boot#### number
sudo efibootmgr --bootnext <N> && sudo reboot
```

The firmware consumes `--bootnext` on the *next* boot only, then reverts to
`BootOrder` (NixOS). So after the gaming session a normal Windows restart lands
back in NixOS — the "game, then back to NixOS" flow, over SSH from anywhere on
the tailnet.

- **Manual fallback:** the firmware boot menu (**F11** at POST on this board)
  lists both disks / boot managers directly.
- **Persistent default swap** (rarely needed): `sudo efibootmgr --bootorder
  <N>,<rest…>` reorders the boot sequence; restore the NixOS-first order after.
- systemd-boot's own menu and `bootctl` still switch between **NixOS
  generations** as before.

## 9. Converge afterward

`nix-switch` on this host is aliased to
`sudo nixos-rebuild switch --flake ~/repos/zireael/nix-config#mattpc`.
