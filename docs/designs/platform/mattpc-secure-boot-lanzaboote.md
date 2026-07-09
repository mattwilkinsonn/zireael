# Design: `mattpc` Secure Boot via lanzaboote

Status: **draft** — open questions pending (see end)
Domain: platform / bare-metal-boot

## Problem / Intent

`mattpc` is the bare-metal NixOS daily driver that dual-boots Windows (Disk 1,
games only) while NixOS owns Disk 0. Its current bootloader is **plain,
unsigned `systemd-boot`**. That cannot boot under UEFI Secure Boot: the firmware
rejects the unsigned `systemd-bootx64.efi`. Secure Boot must be **on** because
some Windows anti-cheats require it, and Secure Boot is a firmware-global
setting — enabling it for Windows enables it for the whole machine, so NixOS
must also boot under it.

Goal: replace the unsigned `systemd-boot` with **lanzaboote** (a signed
systemd-boot boot chain), keep the Windows dual-boot working, and keep the
NVIDIA/Hyprland desktop working — all under Secure Boot.

## Global Constraints

- **lanzaboote pinned to `v1.1.0`** (latest release, verified against the
  GitHub releases feed on 2026-07-09), added as a flake input that
  `follows` `nixpkgs-unstable` — matching how `disko` is wired, so its `lzbt`
  and the host's package set share the unstable channel.
- **`sbctl` must be present in the enrollment environment.** It is not currently
  in any package set (`shared/home.nix`, `shared/dev.nix`, `nixos/common.nix` —
  confirmed absent). lanzaboote's own services reference `${pkgs.sbctl}`; the
  manual key-gen/enroll steps need `sbctl` on the interactive shell too.
- **Microsoft keys MUST be enrolled alongside the machine-owner keys.** Windows'
  `bootmgfw.efi` is Microsoft-signed, and — critically for this box — the **RTX
  4080's UEFI option-ROM / VBIOS is Microsoft-signed too**. Enrolling
  owner-only keys can make the board refuse to POST. lanzaboote v1.1.0 encodes
  this as a hard assertion: `!autoEnrollKeys.allowBrickingMyMachine ->
  autoEnrollKeys.includeMicrosoftKeys` (module line 420), and `includeMicrosoftKeys`
  defaults `true`. The manual path uses `sbctl enroll-keys --microsoft`.
- **No kernel-module signing machinery.** The proprietary NVIDIA modules load
  under Secure Boot on NixOS because the generic kernel ships `MODULE_SIG = no`
  and `SECURITY_LOCKDOWN_LSM = no` (nixpkgs `common-config.nix:858`) — Secure
  Boot does not auto-arm integrity-lockdown as it does on Debian/Fedora. No MOK,
  no `.ko` signing, no `boot.kernelPatches`. Do not add any.
- **Keep `boot.loader.efi.canTouchEfiVariables = true`.** lanzaboote still writes
  NVRAM entries and enrollment touches EFI variables.
- **A recovery path is mandatory** given the POST-brick risk: the board's firmware
  Secure-Boot-disable / clear-keys sequence and, as last resort, a CMOS reset,
  documented so a bricked POST is recoverable at the console.
- **Sequencing:** NixOS must first boot with Secure Boot **off** (the current
  broken-install recovery is a separate, prior step), converge once, then land
  lanzaboote + keys, and only then enable Secure Boot in firmware. Secure Boot
  stays off until the signed chain is in place, or the boot re-bricks.
- **Attribution / VCS:** design record ships as its own PR, separate from the
  implementation; commit as Matt with the `Co-Authored-By: seal
  <noreply@sealedsecurity.com>` trailer; `gt` workflow; never merge.

## Approach

Adopt **lanzaboote** as the bootloader. It keeps systemd-boot as the boot
manager but signs `systemd-bootx64.efi` and installs per-generation **signed
UKIs** (`EFI/Linux/nixos-generation-*.efi`) with the machine-owner keys, so the
firmware accepts the chain under Secure Boot.

Enabling lanzaboote sets `boot.loader.external.enable = true` internally, which
is mutually exclusive with the in-tree `systemd-boot` installer — so the config
must set `boot.loader.systemd-boot.enable = lib.mkForce false`. That has two
consequential effects the current `system.nix` depends on:

### 1. The chainloaded Windows entry is removed (it no-ops under lanzaboote)

The current config boots Windows via
`boot.loader.systemd-boot.windows."windows"`, which launches the bundled
**edk2 UEFI shell** to chainload Disk 1's `bootmgfw.efi`, plus a
`boot.loader.systemd-boot.edk2-uefi-shell` entry. Both live inside nixpkgs
`systemd-boot.nix`'s `config = mkIf cfg.enable { … }` block (lines 540-645), so
with `systemd-boot.enable = false` they are **never generated** — setting them
under lanzaboote is a silent no-op. Worse, the edk2 shell binary is **unsigned**
and lanzaboote signs only UKIs + systemd-boot itself, so even if hand-installed
the shell would be Secure-Boot-rejected. **Remove both entries.**

### 2. Windows boot-selection moves to the firmware-native entry

Because the chainload entry is gone, `bootctl set-oneshot windows` no longer has
a target. The replacement — robust precisely because Windows is on a *separate*
disk with its own ESP — is the firmware's own **Windows Boot Manager** UEFI
NVRAM entry (Windows created it on Disk 1's ESP; `bootmgfw.efi` is
Microsoft-signed, so it boots under Secure Boot natively once Microsoft keys are
enrolled). Select it over SSH:

```bash
efibootmgr -v                              # one-time: find the "Windows Boot Manager" entry number
sudo efibootmgr --bootnext <N> && sudo reboot
```

`--bootnext` is a genuine UEFI one-shot: the firmware consumes it on the next
boot, then reverts to `BootOrder` (NixOS). This preserves the exact "game, then
a normal Windows restart returns to NixOS" flow, over SSH, from anywhere on the
tailnet. At the console, the firmware's F-key boot menu is the manual fallback.
`bootctl` / the systemd-boot menu still work for NixOS generations.

### NVIDIA note

No change and no signing needed (see Global Constraints). Confirmed by the
lanzaboote maintainer running the identical proprietary-NVIDIA + Secure Boot
setup (lanzaboote issue #319).

### Alternative considered and rejected

**Sign a custom edk2 shell to preserve the systemd-boot chainload entry.**
Rejected: lanzaboote does not sign arbitrary ESP binaries, so this means
maintaining an out-of-band signing step for the shell on every generation, for
no benefit — the firmware-native Windows Boot Manager entry is already
Microsoft-signed, works cleanly across separate disks, and needs zero extra
machinery. The `efibootmgr --bootnext` path is strictly simpler and more robust.

## Evidence base (source-verified this session)

- **lanzaboote v1.1.0 module** (`nix/modules/lanzaboote.nix`, read directly):
  `boot.lanzaboote.{enable,pkiBundle,publicKeyFile,privateKeyFile,settings,
  configurationLimit,allowUnsigned}`; `autoGenerateKeys.enable` runs
  `${pkgs.sbctl}/bin/sbctl create-keys` (line 536); `autoEnrollKeys.{enable,
  includeMicrosoftKeys=true,autoReboot,allowBrickingMyMachine}` drives
  `sbctl enroll-keys … --microsoft` (line 569); brick-guard assertion line 420;
  `boot.loader.external.enable = true` set internally (line 447). The
  `enrollKeys` option was **removed** in v1.1.0 (`mkRemovedOptionModule`).
- **lanzaboote/nixpkgs chainload analysis** (librarian, source-cited): the
  `windows.<name>` + `edk2-uefi-shell` entries are gated behind
  `systemd-boot.enable`; `lzbt` writes only signed UKIs + systemd-boot +
  `loader.conf`, no `extraEntries`/`extraFiles` passthrough, signs no arbitrary
  binaries.
- **NVIDIA-under-SB analysis** (librarian, source-cited): NixOS generic kernel
  `MODULE_SIG = no`, `SECURITY_LOCKDOWN_LSM = no`
  (`pkgs/os-specific/linux/kernel/common-config.nix:858`); lanzaboote #319
  maintainer confirmation; the real risk is the Microsoft-signed GPU option-ROM
  at key enrollment, not module loading.
- **Current repo state:** `nixos/mattpc/system.nix:31-47` (bootloader block);
  `nixos/mattpc/INSTALL.md` §2, §6-8; `nixos/scripts/mattpc-bootstrap.sh`
  step 7 (~line 162); `flake.nix:141-175` (`mattpc` nixosConfiguration already
  imports `inputs.disko.nixosModules.disko` — lanzaboote slots in identically).

## Plan

Land as one implementation PR (the change is small and coupled; the doc/script
rewrites can't be validated apart from the config change). Tasks:

### t1 — Add the lanzaboote flake input + wire the module

- Add to `flake.nix` inputs (after `disko`):

  ```nix
  lanzaboote = {
    url = "github:nix-community/lanzaboote/v1.1.0";
    inputs.nixpkgs.follows = "nixpkgs-unstable";
  };
  ```

- Add `inputs.lanzaboote.nixosModules.lanzaboote` to the `mattpc`
  `nixosConfigurations` module list (`flake.nix:147-153`), beside
  `inputs.disko.nixosModules.disko`.
- Interfaces: consumes `inputs.lanzaboote`; produces the `boot.lanzaboote.*`
  option namespace on the `mattpc` host.
- Acceptance: `nix flake lock` resolves lanzaboote; `nix eval
  .#nixosConfigurations.mattpc.config.system.build.toplevel` evaluates (the flake
  currently only needs to *evaluate*; the host isn't built here).

### t2 — Rewrite the `system.nix` bootloader block

- Replace `boot.loader.systemd-boot = { … }` (including `windows."windows"` and
  `edk2-uefi-shell`) with:

  ```nix
  boot.loader.systemd-boot.enable = lib.mkForce false;
  boot.lanzaboote = {
    enable = true;
    pkiBundle = "/var/lib/sbctl";
  };
  boot.loader.efi.canTouchEfiVariables = true;
  ```

- Preserve `configurationLimit` intent via `boot.lanzaboote.configurationLimit`
  (defaults to `systemd-boot.configurationLimit`; set to `10` as today, or drop
  to inherit).
- Rewrite the block's comment to describe the signed-chain + firmware-native
  Windows selection on its own terms (no historical "used to chainload"
  narrative in the shipped config).
- Interfaces: consumes `boot.lanzaboote.{enable,pkiBundle,configurationLimit}`,
  `boot.loader.systemd-boot.enable` (forced off), `boot.loader.efi.canTouchEfiVariables`.
- Acceptance: `mattpc` config evaluates; no reference to `windows.` /
  `edk2-uefi-shell` remains; `grep` for them in `nixos/mattpc/` is empty.

### t3 — Rewrite `INSTALL.md` §2 + §6-8 for the key + Secure-Boot sequence

- §2: drop the edk2 `efiDeviceHandle` placeholder step entirely (no longer a
  config placeholder).
- New §7 (replacing the edk2-handle discovery): the key provisioning + Secure
  Boot enablement sequence — key generation, sign-verify (`sbctl verify`),
  firmware Setup Mode, Microsoft-inclusive enrollment, enable Secure Boot,
  confirm `bootctl status` shows `Secure Boot: enabled`. (Exact manual-vs-module
  form pending OQ1.)
- §8: replace the `bootctl set-oneshot windows` remote OS-select flow with the
  `efibootmgr --bootnext <N>` flow (one-time entry discovery + the one-shot
  reboot), keeping the "auto-returns to NixOS" explanation.
- Interfaces: documents `sbctl create-keys`, `sbctl enroll-keys --microsoft`,
  `sbctl verify`, `bootctl status`, `efibootmgr -v`, `efibootmgr --bootnext`.
- Acceptance: INSTALL.md has no `bootctl set-oneshot windows`, no edk2 handle
  discovery; the Windows-select and SB-enable flows read end-to-end; markdownlint
  clean.

### t4 — Rewrite `mattpc-bootstrap.sh` step 7 + closing heredoc

- Update the closing "Next steps" heredoc (~line 162) from
  `sudo bootctl set-oneshot windows && sudo reboot` to the `efibootmgr
  --bootnext <N>` mechanism, with the one-time entry-number discovery note.
- If OQ1 selects a bootstrap-driven enrollment, add the key step here; otherwise
  leave enrollment to INSTALL.md and only fix the Windows-select text.
- Interfaces: shell text only; no new bootstrap logic unless OQ1 says so. (The
  file is an allowlisted `.sh`; keep it bash, no new logic that would trip the
  no-bash gate's intent.)
- Acceptance: no `bootctl set-oneshot windows` in the script; the printed
  instruction matches INSTALL.md §8.

### t5 — Document the firmware recovery path

- Add a short "Secure Boot recovery" subsection to INSTALL.md: how to disable
  Secure Boot / clear keys from firmware on this board, and the CMOS-reset last
  resort, so a POST-brick from enrollment is recoverable at the console.
- Interfaces: prose; the exact keystroke/menu depends on OQ2 (board model).
- Acceptance: a reader who bricks POST during enrollment has a documented way
  back.

## Tasks

- [ ] t1 — lanzaboote flake input + module wired into `mattpc`
- [ ] t2 — `system.nix` bootloader block rewritten (lanzaboote on, systemd-boot
      forced off, windows/edk2 entries removed)
- [ ] t3 — `INSTALL.md` §2/§6-8 rewritten (key + SB sequence, `efibootmgr`
      Windows-select)
- [ ] t4 — `mattpc-bootstrap.sh` step 7 + heredoc rewritten
- [ ] t5 — firmware recovery path documented

## Open Questions

- **OQ1 (load-bearing): manual `sbctl` steps vs lanzaboote's key
  automation.** The config + INSTALL.md flow differs depending on this.
  - *Option A — manual (recommended):* leave `autoGenerateKeys`/`autoEnrollKeys`
    off; INSTALL.md documents explicit `sudo sbctl create-keys`, converge,
    `sudo sbctl enroll-keys --microsoft` (in firmware Setup Mode), then enable
    Secure Boot. Transparent, matches lanzaboote's canonical docs, easy to debug
    a failed enrollment step-by-step. One-time, done at the console during
    install anyway.
  - *Option B — module automation:* `boot.lanzaboote.autoGenerateKeys.enable =
    true; autoEnrollKeys.enable = true;` (`includeMicrosoftKeys` stays default
    `true`; the brick-guard assertion is active). Fewer manual commands, but
    enrollment fires from a boot service when the firmware is in Setup Mode,
    which is less obvious to reason about, and `autoReboot` behavior needs a
    decision. **Recommendation: A** — one-time cost, maximal clarity, least
    surprising; keep B's options documented as the alternative.

- **OQ2 (load-bearing): the exact motherboard model**, to write the precise
  firmware Secure-Boot-disable / clear-keys recovery steps in t5. It's the
  i9-14900KS + RTX 4080 gaming PC. **Recommendation: confirm the board vendor +
  model** (e.g. from `sudo dmidecode -s baseboard-product-name` on the running
  WSL host, or Matt's memory) so t5 documents the real keystroke, not a generic
  one.

- **OQ3 (non-load-bearing, deferred): drop the EDK2 UEFI shell entry entirely.**
  It is dead under Secure Boot and no longer generated once systemd-boot is
  forced off, so t2 removes it regardless. No emergency-shell replacement is
  planned (an unsigned rescue shell can't run under SB anyway; console firmware
  menu + the USB installer are the rescue path). Documented deferral: the design
  is correct without a shell entry.

- **OQ4 (non-load-bearing, deferred): Measured Boot / TPM sealing**
  (`boot.lanzaboote.measuredBoot`, PCR policy, LUKS auto-cryptenroll). Out of
  scope — this host's disko layout is unencrypted btrfs, so there's nothing to
  seal. Documented deferral; a later record can add measured boot if disk
  encryption is ever introduced.
