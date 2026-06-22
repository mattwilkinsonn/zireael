# mattpc — Windows 11 + NixOS-WSL2 Install / Reclaim Procedure

One-time procedure for the **i9-14900KS + RTX 4080 + 64 GB DDR5** gaming
PC. Two M.2 2 TB NVMe drives. Goal state: **Windows 11 owns all 4 TB**
across both drives. Linux dev work happens inside a NixOS-WSL2 distro
running under Windows.

This doc covers two starting points:

- **Part A (reclaim)**: existing dual-boot install with Bazzite on
  Drive A, Windows on Drive B (1 TB used, 1 TB free), rEFInd as the
  picker. Wipe Bazzite, extend Windows over both drives, drop rEFInd,
  install NixOS-WSL.
- **Part B (fresh install)**: empty drives. Single-boot Windows
  install across both drives, then NixOS-WSL.

Both paths converge at the **WSL setup** section.

## Part A: reclaim from existing dual-boot

Existing layout (per the old `bazzite/INSTALL.md`):

| Drive | Contents | Owner |
| ---- | -------- | ----- |
| nvme0n1 (Drive A, 2 TB) | EFI + `/boot` + Btrfs root, rEFInd at `/boot/efi/EFI/refind/` | Bazzite |
| nvme1n1 (Drive B, 2 TB) | EFI + MSR + 1 TB Windows C: + recovery, 1 TB unallocated | Windows |

Drive B's EFI is self-contained — wiping Drive A doesn't break Windows
boot. Firmware-level `Windows Boot Manager` UEFI NVRAM entry survives.
The `rEFInd Boot Manager` UEFI entry will dangle and can be deleted
after the wipe; Windows Boot Manager takes over as the firmware default.

### A.1 Pre-reclaim checklist

- [ ] Anything on Drive A worth keeping is backed up. The Bazzite home
      partition is going away — Steam library, dotfiles state, etc. all
      get nuked.
- [ ] `~/.git` dotfiles are pushed to GitHub.
- [ ] `~/repos/zireael/nix-config` (this repo) is pushed.
- [ ] 1Password vault has the `mattpc-wsl Password` item created in the
      `Dev` vault (used by `mattpc-wsl-bootstrap.sh` step 4).
- [ ] You have another tailnet device available — you'll need to SSH
      into Windows-side OpenSSH from the Mac during the WSL setup.

### A.2 Boot to Windows, wipe Drive A

From an elevated PowerShell:

```powershell
# Identify the disks. Drive B (Windows) is whichever has the
# `Healthy (Boot, Page File, Crash Dump, Primary Partition)` label.
Get-Disk

# Replace <N> with Drive A's disk number (NOT Windows's).
Clear-Disk -Number <N> -RemoveData -RemoveOEM -Confirm:$false
```

After `Clear-Disk`, Drive A is unallocated. Confirm in Disk Management
(`diskmgmt.msc`) — Drive A should show 1.8 TiB unallocated, no
partitions, no labels.

### A.3 Set up Drive A as G:

Drive A is now empty. Create a single 2 TB NTFS volume on it for Steam
games + the WSL2 vhdx + general scratch.

```powershell
# Identify Drive A's disk number again (was <N> in A.2).
Get-Disk

# Create the volume in one shot — initialize, partition, format, assign G:
New-Partition -DiskNumber <N> -UseMaximumSize -DriveLetter G |
  Format-Volume -FileSystem NTFS -NewFileSystemLabel "Data" -Confirm:$false
```

After this `Get-Volume G` should show ~1.8 TiB NTFS, drive letter `G`.

### A.4 Extend C: across Drive B's free space

Drive B currently looks like:

```text
[ EFI ][ MSR ][ Windows C: 1 TB ][ Recovery ~1 GB ][ unallocated 1 TB ]
```

Windows can extend C: into **contiguous** unallocated space — but the
Recovery partition sits between C: and the unallocated tail, blocking
the extend in Disk Management. Two paths:

- **A.4a (recommended): delete + recreate Recovery.** Detaches WinRE,
  removes the Recovery partition, extends C: to take all 2 TB minus a
  small reserve, recreates Recovery at the new tail, re-attaches WinRE.
  WinRE keeps working; you keep the in-Windows "Reset this PC" /
  "Startup Repair" / "Advanced startup" options.
- **A.4b (lazier): delete Recovery, don't recreate.** WinRE stops
  working in-Windows, but the Ventoy USB still has the Win11 installer
  which exposes the same recovery tools when booted from. Saves the
  shrink/recreate dance. Pick this if you're confident you'd reach for
  the install USB before Settings → Recovery anyway.

#### A.4a: keep WinRE

```powershell
# 1. Confirm WinRE is currently active
reagentc /info
# Look for "Windows RE status: Enabled" and a "Windows RE location"
# pointing at the Recovery partition.

# 2. Detach WinRE — moves winre.wim back to C:\Windows\System32\Recovery\
reagentc /disable

# 3. Delete the Recovery partition via diskpart
diskpart
```

```text
list disk
select disk 1
list partition
# Find the Recovery partition (Type "Recovery", ~1 GB, between
# C: and the unallocated tail). Note its number.
select partition <N>
delete partition override
exit
```

Open Disk Management (`diskmgmt.msc`). Right-click C: → **Extend
Volume** → take all but ~1 GB. (The wizard accepts a custom size in
MB; 2,096,128 MB leaves a 1024 MB tail for the new Recovery.)

```powershell
# 4. Recreate Recovery partition at the new tail
diskpart
```

```text
select disk 1
create partition primary size=1024 id=de94bba4-06d1-4d40-a16a-bfd50179d6ac
# id=... = "Windows Recovery" partition GUID
format fs=ntfs quick label="Recovery"
gpt attributes=0x8000000000000001
# 0x8000000000000001 = required + hidden, same attrs Windows installer uses
exit
```

```powershell
# 5. Re-attach WinRE — it auto-discovers the new Recovery partition
#    and moves winre.wim back into it
reagentc /enable
reagentc /info
# Should show "Enabled" + a path inside the new Recovery partition.
```

#### A.4b: skip Recovery recreation

If you'd rather just nuke Recovery and never re-create it:

```powershell
reagentc /disable
diskpart
```

```text
select disk 1
list partition
select partition <recovery-N>
delete partition override
exit
```

Then in Disk Management, right-click C: → Extend Volume → take all
remaining space.

WinRE is now off; if Windows ever needs in-OS repair, boot from the
Ventoy USB and use the recovery tools there instead.

#### Final layout (either path)

| Drive | Disk # | Layout |
| ---- | ------ | ------ |
| nvme0n1 (Drive A) | 0 | G: NTFS — 2 TB (Steam library + WSL vhdx + scratch) |
| nvme1n1 (Drive B) | 1 | EFI + MSR + C: 2 TB (or 2 TB - 1 GB) [+ Recovery] |

Two independent disks, each independently recoverable. Drive A failure
doesn't affect Windows; Drive B failure doesn't affect the Steam
library or WSL distro. Sequential read on a single modern NVMe is
plenty for both Windows and WSL workloads — random access (which
dominates real game loading + WSL filesystem use) is what matters and
is identical to a striped pair.

### A.5 Remove rEFInd from NVRAM

Drive A's wipe deleted the rEFInd binary on Drive A's old ESP, but the
firmware-level UEFI NVRAM entry still points at the now-missing file
and clutters the BIOS boot menu. Remove it:

```powershell
# List UEFI boot entries
bcdedit /enum firmware

# Find the {GUID} for "rEFInd Boot Manager" or similar, then:
bcdedit /delete {<guid>} /f
```

Reboot. Windows Boot Manager should now be the only UEFI entry; firmware
boots straight into Windows with no picker. Confirm: F11 / F2 at POST
shows only `Windows Boot Manager` under "boot override."

> **Secure Boot stays on the whole time.** Wiping Drive A doesn't
> touch the firmware's MOK store, but the only key that lived there
> was Rod Smith's rEFInd signing key, which now points at no binary.
> Harmless. Leave it; Vanguard (Valorant, League) needs Secure Boot
> on regardless.

## Part B: fresh single-boot Windows install

Skip if you did Part A.

### B.1 BIOS prep

1. Boot to BIOS (Del or F2 at POST).
2. **Secure Boot = Enabled** (Vanguard needs it).
3. **TPM 2.0 / Intel PTT = Enabled** (Win11 + Vanguard need it).
4. **Intel VMD = Disabled** — VMD hides NVMe behind a virtual RAID
   controller and breaks Windows installer's NVMe detection. On the
   MSI PRO Z690-A: Settings → IO Ports → VMD setup menu → "Enable
   VMD controller" = Disabled.
5. Boot priority: USB first.

### B.2 Authoring the Windows install USB

Windows 11 ISO from <https://www.microsoft.com/software-download/windows11>.
Use **Rufus** to write it: GPT, UEFI mode, NTFS (because `install.wim`
is >4 GB and FAT32 can't hold it). Rufus's "Windows User Experience"
dialog optionally bypasses the Microsoft account requirement and
pre-disables BitLocker — both worth taking.

### B.3 Install Windows on Drive B (2 TB)

1. Boot Windows installer.
2. At "Where do you want to install Windows?" — **delete every
   partition on both drives.**
3. Select Drive B's unallocated space. Click "New" → leave the size
   at default (uses all unallocated). "Apply" — Windows creates the
   EFI / MSR / C: / Recovery layout taking all 2 TB. (No need to
   carve out a 1 TB partition; we want C: to be the full drive from
   the start, so we can skip the A.4 extend dance entirely.)
4. Install Windows.
5. Complete first-boot: Microsoft account (or local), Windows Update,
   reboot.
6. Disable Fast Startup (Control Panel → Power Options → "Choose what
   the power buttons do" → uncheck "Turn on fast startup"). Without
   this, NTFS volumes get left in a hibernating state that WSL2 mounts
   may corrupt.
7. Disable BitLocker if it auto-enabled (search "Manage BitLocker" →
   "Turn off BitLocker"). Same reason.
8. After Windows is up, follow Part A.3 to set up G: on Drive A.
   (Skip A.4 — C: is already 2 TB. Skip A.5 — there's no rEFInd entry
   to clean up on a fresh install.)

## WSL setup (both paths converge here)

Now Windows is installed and stable. Time to wire up the dev environment.

### W.1 Run `windows-bootstrap.ps1`

This is the prereq for everything else — installs 1Password, VS Code,
Tailscale, sets up Windows-side OpenSSH on port 22 with the Mac's SSH
public key in `administrators_authorized_keys`.

```powershell
# From an elevated PowerShell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass

# windows-bootstrap.ps1 lives in the dotfiles repo. Easiest path: open
# Edge, sign into GitHub (1Password fills creds), download:
#   https://github.com/mattwilkinsonn/zireael/raw/main/nix-config/windows/windows-bootstrap.ps1
.\windows-bootstrap.ps1
```

When prompted, paste the Mac SSH public key from 1Password. Then sign
into Tailscale via the system tray. Confirm `tailscale ip -4` returns
a tailnet address.

### W.2 Run `windows-setup.ps1`

```powershell
# In the same elevated PowerShell
.\nix-config\windows\windows-setup.ps1
```

This:

- Installs git + gh + jj, authenticates to GitHub, clones the dotfiles
  repo into `%USERPROFILE%` as a colocated git+jj repo at
  `%USERPROFILE%\.git`.
- Applies `windows/configuration.dsc.yaml` (winget DSC) — installs
  Steam, Discord, Podman Desktop, NixOS apps, btm, etc.
- Copies `windows/.wslconfig` to `%USERPROFILE%\.wslconfig` (mirrored
  networking, 56 GB memory cap, sparse vhdx).
- Wires the AHK Mac-keyboard mapping into Startup.

After it completes, **reboot once** so PowerToys and the WSL feature
both fully register.

### W.3 Install NixOS-WSL

Prereq: G: exists (Part A.3 / B.3 set up the Drive A volume). Confirm
with `Get-Volume G` — should show ~2 TB NTFS, drive letter `G`. If it
doesn't, finish the partition step first; `wsl --import` to a
non-existent drive errors out partway and leaves an orphaned distro
registration.

Download the latest tarball from
<https://github.com/nix-community/NixOS-WSL/releases> — file name like
`nixos-wsl.tar.gz`. Save it under `%USERPROFILE%\Downloads\`.

```powershell
# From an elevated PowerShell
wsl --install --no-launch
# Distro vhdx goes on G: (Drive A, dedicated 2 TB NVMe). G: has no
# Windows OS / pagefile / driver I/O competing for bandwidth, so all
# 2 TB of the disk is available to WSL whenever it needs it.
New-Item -ItemType Directory -Path G:\WSL -Force | Out-Null
wsl --import NixOS G:\WSL\NixOS $env:USERPROFILE\Downloads\nixos-wsl.tar.gz
wsl --set-default NixOS

# Apply the .wslconfig we just dropped
wsl --shutdown

# Enter the new distro
wsl -d NixOS
```

You're now inside the NixOS-WSL distro as the `nixos` user (NixOS-WSL's
tarball default — our config-defined `mattw` doesn't exist yet).

### W.4 Bootstrap NixOS-WSL (phase 1, as `nixos`)

The bootstrap is two-phase by design — you start as `nixos`, the first
nixos-rebuild creates `mattw`, then you re-enter WSL as `mattw` and
re-run the same script. One script, two invocations, idempotent.

```bash
# inside WSL as `nixos`:
bash /mnt/c/Users/<your-windows-username>/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
```

(The path uses `/mnt/c` because `/home/nixos/nix-config` doesn't exist
yet — the script copies the dotfiles into `/home/nixos/.git` from the
Windows-side clone that `windows-setup.ps1` already laid down.)

Phase 1 does:

1. Copies the Windows-side dotfiles (`%USERPROFILE%\.git`) into
   `/home/nixos/.git` and resets the worktree into `/home/nixos/`.
2. Runs the first `nixos-rebuild switch --flake .#mattpc-wsl`. This
   applies the hostname (`mattpc-wsl`), opens sshd on port 2222,
   creates the `mattw` user with the Tailnet SSH public key already in
   `~/.ssh/authorized_keys` (declarative via `nixos/common.nix`), and
   sets `wsl.defaultUser = "mattw"` for future sessions.

When it finishes, exit and shut down WSL so the next session lands you
as `mattw`:

```bash
exit
```

```powershell
# Windows side
wsl --shutdown
wsl -d NixOS
```

### W.5 Bootstrap NixOS-WSL (phase 2, as `mattw`)

```bash
# inside WSL as `mattw`:
bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
```

(Path is now `~/repos/zireael/nix-config` because phase 1 placed dotfiles in
`/home/nixos/.git` and phase 2 migrates them to `/home/mattw/.git`.)

Phase 2 does:

1. Migrates `/home/nixos/.git` → `/home/mattw/.git` (sudo + chown), then
   colocates jj on top.
2. Stores both 1P service-account tokens (mode 600):
   - `~/.config/op/service-account-token` — personal account (pc-svc),
     reads `op://Dev/...` + `op://Server/...`.
   - `~/.config/op/team-service-account-token` — sealedsecurity team
     (matt-dev-svc), reads `op://Local Dev/...`.

   You'll be prompted to paste both `ops_…` tokens.
3. Re-runs `nixos-rebuild switch` with the token in env so home-manager
   activations that read 1Password (Berkeley Mono rclone sync,
   load-secrets warmup, etc.) actually fire.
4. Rotates `mattw` and `root` passwords from `op://Dev/mattpc-wsl
   Password`.
5. Fetches the inter-server SSH key from `op://Server/inter-server` into
   `~/.ssh/id_ed25519_inter_server` (used for host-to-host automation
   between mattpc-wsl, mattfw, mattserver, and the Pis).
6. Sanity-checks sshd-on-2222, podman socket, resolv.conf.

### W.6 Verify SSH from the Mac

```bash
# From the Mac
ssh mattw@mattpc.tail08a5c5.ts.net           # Windows side, port 22 — for `btm`-type system monitoring
ssh -p 2222 mattw@mattpc.tail08a5c5.ts.net   # WSL side, port 2222 — for Linux dev work
```

Both should land you in the right shell with no password prompt:

- The Mac's SSH key is in `administrators_authorized_keys` on Windows
  (set up by `windows-bootstrap.ps1`).
- The same key is wired into `users.users.mattw.openssh.authorizedKeys.keys`
  in `nixos/common.nix:52-77` and is dropped at
  `/etc/ssh/authorized_keys.d/mattw` on every NixOS host that imports
  common.nix — including this WSL distro. Declarative, no manual
  copy-paste needed.

### W.7 Wire up Podman Desktop (Windows GUI → WSL engine)

1. Open Podman Desktop from the Start menu.
2. Settings → Resources → "Create new..." → "SSH" provider.
3. URL: `ssh://mattw@localhost:2222`. Mirrored networking exposes the
   WSL sshd at `localhost` from Windows.
4. Identity: point at the same SSH key 1Password's SSH agent serves
   (or the Mac key if you replicated it locally). The WSL distro's
   `~/.ssh/authorized_keys` is the same file populated declaratively
   via `common.nix`, so any key authorized for `ssh -p 2222 mattw@…`
   from the Mac also works from Podman Desktop.

You should now see WSL's podman engine in Podman Desktop's UI.
Container actions issued from the Windows GUI execute inside WSL.

### W.8 First-time `rclone` setup (Berkeley Mono fonts)

`shared/linux.nix` has a home-manager activation that syncs Berkeley
Mono fonts from a `gdrive:` rclone remote. It skips silently if the
remote isn't configured, so without this step you fall back to
JetBrains Mono everywhere.

```bash
# inside WSL as mattw
rclone config
```

Walk the interactive prompts:

- `n` (new remote)
- name: `gdrive`
- storage: `drive` (Google Drive)
- `client_id` / `client_secret`: leave blank (uses rclone's defaults)
- scope: `1` (full access)
- service_account_file: blank
- "Use auto config?": `n` (we're headless inside WSL — auto config
  tries to open a browser via `xdg-open` which isn't reliable here).
  Follow the printed URL on the Windows side, paste the auth code back.
- team_drive: `n`
- confirm: `y`, then `q` to quit.

Verify:

```bash
rclone listremotes        # should show "gdrive:"
nix-switch                # syncBerkeleyMono activation runs cleanly now
fc-list | grep -i berkeley  # confirm fonts landed
```

### W.9 Verify monitoring paths

```bash
# Windows-side monitoring (from any tailnet host)
ssh mattw@mattpc.tail08a5c5.ts.net
btm                       # whole-Windows view: CPU, RAM, GPU, disks
exit

# WSL-side monitoring
ssh -p 2222 mattw@mattpc.tail08a5c5.ts.net
btm                       # WSL view (CPU + RAM allocated to WSL only)
```

`btm` on Windows shows the GPU; the WSL view doesn't, which is fine —
when you care about GPU stats, you're either gaming (Windows-side) or
inside whatever inference workload is running, in which case `nvtop`
or `nvidia-smi` from the WSL container is what you want anyway.

## Post-install reminders

- **Steam library**: install games to `G:\Steam\` (Drive A's volume).
  Steam → Settings → Storage → "+" → enter `G:\Steam` → make default.
- **Riot games**: install fresh on G: too. Drag the install path during
  the Riot installer dialog.
- **OBS / Insights / recording**: point at `G:\Recordings\`.
- **NVIDIA App + drivers**: run the driver update pass before launching
  any games.
- **WSL filesystem ceiling**: WSL2's default vhdx max is 1 TB; raise via
  `wsl --manage NixOS --set-vhd-size 2147483648` if you ever fill it.
  `sparseVhd=true` in `.wslconfig` keeps the actual on-disk size much
  smaller than the cap.

## Recovery

- **WSL distro corrupt**: `wsl --unregister NixOS` removes the distro
  cleanly (rootfs vhdx deleted, no Windows-side leftovers). Re-run
  W.3 + W.4 to reinstall.
- **Bootstrap script fails partway**: re-run it. Every step skips if
  already done.
- **Windows install borked but WSL fine**: a Windows in-place repair
  (boot installer USB → "Install Windows" → "Keep my files and apps")
  preserves the WSL distro since the vhdx lives on G: (`G:\WSL\NixOS\`,
  Drive A) and the repair only touches C: on Drive B. Worst case:
  backup the vhdx file first (`wsl --export NixOS backup.tar.gz`),
  reinstall Windows, re-import with
  `wsl --import NixOS G:\WSL\NixOS backup.tar.gz`.
