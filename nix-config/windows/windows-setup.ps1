# Windows bootstrap. Run this once on a fresh Windows install (after winget
# is installed and you have signed into the Microsoft Store). It:
#   1. Verifies winget + Windows 11
#   2. Clones the dotfiles repo into $env:USERPROFILE as a colocated git+jj
#      repo at $env:USERPROFILE\.git
#   3. Applies the WinGet DSC configuration
#   4. Copies windows/.wslconfig to %USERPROFILE%\.wslconfig
#   5. Wires the AHK Mac-keyboard script into Startup
#   6. Wires the PowerShell profile dot-source into the per-host $PROFILE
#   7. Prints manual-install reminders for non-winget apps
#
# Scope: gaming-focused install + NixOS-WSL2 dev environment. Linux dev
# work happens inside the WSL distro (see nixos/mattpc-wsl/).
#
# Run from an elevated PowerShell:
#   PS> Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
#   PS> .\scripts\windows-setup.ps1
#
# Re-running is safe: each step is idempotent.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Section($msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }

# --- Prereqs ------------------------------------------------------------------
Section 'Verifying prerequisites'

$os = Get-CimInstance Win32_OperatingSystem
if ([int]($os.BuildNumber) -lt 22000) {
    throw "Windows 11 (build >= 22000) required. Detected build $($os.BuildNumber)."
}
Write-Host "Windows 11 build $($os.BuildNumber)"

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    throw 'winget not found. Install "App Installer" from the Microsoft Store and re-run.'
}
winget --version | Out-Host

# --- git + gh + jj + GitHub auth (needed before dotfiles clone — private repo) --
Section 'Installing git + GitHub CLI + jj for clone+colocate bootstrap'

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    winget install --id Git.Git --silent --accept-package-agreements --accept-source-agreements
}
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    winget install --id GitHub.cli --silent --accept-package-agreements --accept-source-agreements
}
if (-not (Get-Command jj -ErrorAction SilentlyContinue)) {
    winget install --id jj-vcs.jj --silent --accept-package-agreements --accept-source-agreements
}
# Refresh PATH for the freshly-installed tools without requiring a new shell.
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
[Environment]::GetEnvironmentVariable('Path', 'User')

# gh auth status exits 1 when not signed in. PS 7.4+ throws on native non-zero
# under $ErrorActionPreference='Stop', so scope the call to 'Continue' and let
# the explicit $LASTEXITCODE check drive control flow.
& { $ErrorActionPreference = 'Continue'; gh auth status 2>&1 | Out-Null }
if ($LASTEXITCODE -ne 0) {
    Write-Host 'Authenticate with GitHub (this opens a browser):'
    gh auth login
}
gh auth setup-git

# --- zireael repo -------------------------------------------------------------
# Clone the zireael monorepo into $env:USERPROFILE\repos\zireael\ as a
# colocated git+jj repo. Same shape as bootstrap-common.sh's
# clone_zireael_via_gh: gh-authenticated clone, then jj colocate.
Section 'Cloning zireael'

$reposDir = Join-Path $env:USERPROFILE 'repos'
$zireaelDir = Join-Path $reposDir 'zireael'
$zireaelRepo = 'mattwilkinsonn/zireael'

if (-not (Test-Path (Join-Path $zireaelDir '.git'))) {
    New-Item -ItemType Directory -Path $reposDir -Force | Out-Null
    gh repo clone $zireaelRepo $zireaelDir

    # Colocate jj. ~/.config/jj/config.toml will land later via
    # home-manager activation on the WSL side; for the Windows-side
    # repo we just need the colocate + main-bookmark track once.
    if (-not (Test-Path (Join-Path $zireaelDir '.jj'))) {
        jj git init --colocate $zireaelDir
        jj -R $zireaelDir bookmark track main --remote=origin
    }
}
else {
    Write-Host "zireael already at $zireaelDir"
}

# --- privatefiles repo --------------------------------------------------------
# Optional private repo (CLAUDE.md, RTK.md, user-level SEAL.md,
# sealedsecurity workspace meta, Tailscale ACL). Skipped silently
# if the user doesn't have access — this is a fresh-Win box; the
# user will populate it themselves later if they want.
Section 'Cloning privatefiles (optional)'

$privatefilesDir = Join-Path $reposDir 'privatefiles'
$privatefilesRepo = 'mattwilkinsonn/privatefiles'

if (-not (Test-Path (Join-Path $privatefilesDir '.git'))) {
    try {
        gh repo clone $privatefilesRepo $privatefilesDir
        if (-not (Test-Path (Join-Path $privatefilesDir '.jj'))) {
            jj git init --colocate $privatefilesDir
            jj -R $privatefilesDir bookmark track main --remote=origin
        }
    }
    catch {
        Write-Host "Skipping privatefiles clone (gh access denied or repo unavailable)"
    }
}
else {
    Write-Host "privatefiles already at $privatefilesDir"
}

# --- WinGet DSC apply ---------------------------------------------------------
Section 'Applying WinGet DSC configuration'

$dscFile = Join-Path $env:USERPROFILE 'repos\zireael\nix-config\windows\configuration.dsc.yaml'
if (-not (Test-Path $dscFile)) {
    throw "DSC file not found at $dscFile (zireael checkout incomplete?)"
}
winget configure --enable
winget configure --file $dscFile --accept-configuration-agreements --disable-interactivity

# --- .wslconfig ---------------------------------------------------------------
# Drops the WSL2 settings (mirrored networking, memory cap, sparse vhdx)
# at %USERPROFILE%\.wslconfig where wsl.exe reads it. Run `wsl --shutdown`
# after first install for it to take effect.
Section 'Installing .wslconfig'

$wslSrc = Join-Path $env:USERPROFILE 'repos\zireael\nix-config\windows\.wslconfig'
$wslDst = Join-Path $env:USERPROFILE '.wslconfig'
if (-not (Test-Path $wslSrc)) {
    throw ".wslconfig not found at $wslSrc (zireael checkout incomplete?)"
}
Copy-Item -Path $wslSrc -Destination $wslDst -Force
Write-Host "Wrote $wslDst (run 'wsl --shutdown' for it to take effect)"

# --- AHK auto-start (Mac-keyboard mapping) ------------------------------------
Section 'Wiring AHK Mac-mapping script into Startup'

$ahkScript = Join-Path $PSScriptRoot 'mac.ahk'
if (Test-Path $ahkScript) {
    $startup = [Environment]::GetFolderPath('Startup')
    $shortcut = Join-Path $startup 'mac-keyboard.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $sc = $shell.CreateShortcut($shortcut)
    $sc.TargetPath = $ahkScript
    $sc.WorkingDirectory = Split-Path $ahkScript
    $sc.Save()
    Write-Host "Created $shortcut -> $ahkScript"
}
else {
    Write-Warning "AHK script not found at $ahkScript - skipping Startup shortcut."
}

# --- PowerShell profile dot-source --------------------------------------------
# Wire the dotfiles-tracked profile source (nix-config/windows/profile.ps1)
# into the per-host $PROFILE. We don't drop content at
# Documents\PowerShell\... directly because Documents is often OneDrive-
# redirected on Win11 and the bare-repo checkout would land in the wrong
# tree -- whereas $HOME\nix-config\... lives directly under USERPROFILE.
Section 'Wiring PowerShell profile dot-source'

$myDocs = [Environment]::GetFolderPath('MyDocuments')
$ps7ProfileDir = Join-Path $myDocs 'PowerShell'
$ps7ProfilePath = Join-Path $ps7ProfileDir 'Microsoft.PowerShell_profile.ps1'

$profileSrc = Join-Path $env:USERPROFILE 'repos\zireael\nix-config\windows\profile.ps1'
if (-not (Test-Path $profileSrc)) {
    throw "profile source not found at $profileSrc (zireael checkout incomplete?)"
}
if (-not (Test-Path $ps7ProfileDir)) {
    New-Item -ItemType Directory -Path $ps7ProfileDir -Force | Out-Null
}

# Literal $HOME keeps the line portable across users.
$dotSource = '. "$HOME\nix-config\windows\profile.ps1"'
$existing = if (Test-Path $ps7ProfilePath) { Get-Content $ps7ProfilePath -Raw -ErrorAction SilentlyContinue } else { '' }
if ($existing -and ($existing -match [regex]::Escape($dotSource))) {
    Write-Host "$ps7ProfilePath already dot-sources profile.ps1"
}
else {
    Add-Content -Path $ps7ProfilePath -Value $dotSource -Encoding utf8
    Write-Host "Wired dot-source into $ps7ProfilePath"
}

# --- Manual-install reminders ------------------------------------------------
Section 'Manual installs (not winget-manageable)'

$manual = @(
    @{ name = 'NVIDIA App'; url = 'https://www.nvidia.com/en-us/software/nvidia-app/' }
    @{ name = 'Divvy'; url = 'https://mizage.com/divvy/' }
    @{ name = 'Insights Capture'; url = 'https://insights.gg/' }
    @{ name = 'Gigabyte Control Center'; url = 'https://www.gigabyte.com/Consumer/Software/GIGABYTE-Control-Center' }
    # AWCC was pulled from Microsoft Store in 2024 and isn't on winget.
    # Needed for the Alienware monitor (refresh rate, AlienFX lighting,
    # OSD shortcuts). Dell's evergreen driver-details URL.
    @{ name = 'Alienware Command Center'; url = 'https://www.dell.com/support/home/en-us/drivers/driversdetails?driverid=00wmc' }
    # Pulled from winget (flagged as PUA). Install via the official site.
    @{ name = 'RustDesk'; url = 'https://rustdesk.com/' }
    # winget manifest hash drifts behind the live Riot installer; install
    # via Riot Client (already there from LoL) or the official site.
    @{ name = 'Valorant'; url = 'https://playvalorant.com/' }
    # Intel pulled the pinned patch version; winget manifest hasn't caught up.
    @{ name = 'Intel Extreme Tuning Utility (XTU)'; url = 'https://www.intel.com/content/www/us/en/download/17881/intel-extreme-tuning-utility-intel-xtu.html' }
    @{ name = 'MicSwitch'; url = 'https://github.com/iXab3r/MicSwitch' }
)
foreach ($m in $manual) {
    Write-Host (" - {0}: {1}" -f $m.name, $m.url)
}

Section 'Post-install checklist'
@'
1. PowerToys -> Keyboard Manager: remap Win <-> Ctrl (the Mac-keyboard mode
   sends Win for the Cmd key; AHK + this remap gives Mac-style shortcuts).
   Remap Win+C to Ctrl+C - disables Copilot shortcut and allows Ctrl+C to work in the terminal.
   Command Palette -> Map shortcut to Ctrl+Space (will be Win+Space with the remap), matches Spotlight
   FancyZones -> Configure layout
2. Open 1Password -> Developer -> Turn on the SSH agent. Add your GitHub
   key, then point Git at it (config is already in dotfiles).
3. Tailscale: log in via the system tray. Confirm the device appears in
   the admin console — WSL2 uses this same connection via mirrored networking.
4. Start AHK script & verify the AHK Startup shortcut works after a logout/login cycle.

5. Install NixOS-WSL (one-time, two-phase bootstrap):
     # Download nixos-wsl.tar.gz from
     # https://github.com/nix-community/NixOS-WSL/releases
     wsl --install --no-launch
     New-Item -ItemType Directory -Path G:\WSL -Force | Out-Null
     # Distro vhdx goes on G: (Drive A, dedicated 2 TB NVMe — no
     # Windows OS / pagefile / driver I/O contention).
     wsl --import NixOS G:\WSL\NixOS <path-to-tarball>
     wsl --set-default NixOS
     wsl -d NixOS
     # inside WSL as the default `nixos` user (phase 1 — copies
     # dotfiles from /mnt/c, runs first nixos-rebuild creating mattw):
     bash /mnt/c/Users/$env:USERNAME/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
     exit
     # Then back in PowerShell:
     wsl --shutdown
     wsl -d NixOS
     # inside WSL as `mattw` (phase 2 — migrates dotfiles,
     # stores op token, rotates password):
     bash ~/repos/zireael/nix-config/nixos/scripts/mattpc-wsl-bootstrap.sh
   See nix-config/windows/INSTALL.md sections W.4 and W.5 for the
   full two-phase walkthrough.
6. Podman Desktop -> Settings -> Resources -> Create new container
   engine -> SSH connection at ssh://mattw@localhost:2222 (the WSL
   distro). Mirrored networking exposes the WSL sshd at localhost.
   The Mac SSH key in 1Password's SSH agent is automatically
   authorized inside WSL via nixos/common.nix's declarative
   authorizedKeys — no manual key-copy step.
7. (Inside WSL, as mattw) rclone config — set up the 'gdrive' remote
   so Berkeley Mono fonts auto-sync. See INSTALL.md section W.8.

8. Driver + firmware update pass (run all three before gaming):
     - NVIDIA App      -> Drivers -> Check for updates
     - MSI Center      -> Live Update
     - Intel Driver and Support Assistant (system tray) -> Scan
8. ExplorerPatcher: open Properties from the Start menu, then set:
     - Taskbar -> Taskbar style: Windows 10
     - Taskbar -> Primary taskbar alignment: Left
     - Restart Explorer when prompted
9. Make Arc the default browser:
     Settings -> Apps -> Default apps -> Arc -> Set default
10. Sign into the GUI apps that need accounts:
     - Arc (Browser Company account; enable sync)
     - VS Code (Settings Sync via GitHub or Microsoft account)
     - Steam
     - Discord
     - Obsidian (link vault sync)
     - Claude desktop
11. RustDesk (once the winget install succeeds):
     - On Windows: open RustDesk -> Settings -> Security -> set a
       permanent password.
     - On Mac: install RustDesk client, enter this box's ID + password,
       and verify the remote desktop is reachable.
12. Setup autologon with `autologon` in Powershell. Also add a Task Scheduler task with:
        Name: StartWSL2
        User: mattw
        Run whether user is logged on or not
        Run with highest privileges
        Trigger: At log on of mattw
        Action: Start a program: wsl.exe -u root sleep infinity
        (This keeps the WSL2 distro running in the background for instant-on dev, even before login.)
13.  Setup FanControl and iCUE fan curves with a minimum 0 fan speed so the fans turn off while the computer is idle.
'@ | Write-Host
