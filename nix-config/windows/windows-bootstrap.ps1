# Windows pre-bootstrap. Run this ONCE on a fresh Windows install before
# windows-setup.ps1. It:
#   1. Verifies winget + Windows 11 + admin shell
#   2. Installs the GUI apps + tools needed to bring up remote access:
#        - 1Password    (to surface the Mac's SSH public key)
#        - VS Code      (general dev tool)
#        - Tailscale    (to put this box on the tailnet)
#        - PowerShell 7 (so SSH sessions default to pwsh, not cmd.exe)
#   3. Enables the OpenSSH Server, sets it to Automatic, opens port 22,
#      AND opens port 2222 for the NixOS-WSL distro's sshd (sits behind
#      the Windows firewall under mirrored networking).,
#      and points DefaultShell at pwsh.exe so incoming SSH sessions land
#      directly in PowerShell 7
#   4. Pre-creates C:\ProgramData\ssh\administrators_authorized_keys with the
#      ACLs sshd requires for admin users (Administrators + SYSTEM only)
#   5. Prompts for the Mac's SSH PUBLIC key (paste from 1Password) and appends
#      it to administrators_authorized_keys
#   6. Prints the remaining manual steps (Tailscale sign-in, then SSH from the
#      Mac and run windows-setup.ps1)
#
# Run from an elevated PowerShell:
#   PS> Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
#   PS> .\windows-bootstrap.ps1
#
# Re-running is safe: each step is idempotent.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Section($msg) { Write-Host "`n=== $msg ===" -ForegroundColor Cyan }

# Prereqs
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

# Refresh the local source cache so --exact queries don't miss recently
# updated packages. DSC apply does this implicitly; bare 'winget install'
# does not, which is what bit the first run of this script (tailscale's
# manifest update hadn't synced).
winget source update | Out-Null

$identity  = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script from an elevated PowerShell (the OpenSSH + ACL steps need admin).'
}

# winget installs
Section 'Installing 1Password, VS Code, Tailscale, PowerShell 7'

function Install-WingetPkg($id) {
    $listed = winget list --id $id --exact 2>&1 | Out-String
    if ($listed -match [regex]::Escape($id)) {
        Write-Host "  already installed: $id"
        return
    }
    Write-Host "  installing: $id"
    winget install --id $id --exact --silent `
        --accept-package-agreements --accept-source-agreements
}

Install-WingetPkg 'AgileBits.1Password'
Install-WingetPkg 'Microsoft.VisualStudioCode'
Install-WingetPkg 'Tailscale.Tailscale'

# PowerShell 7 needs to be machine-scope: the OpenSSH DefaultShell registry
# key is machine-wide, so the binary must live under C:\Program Files. As of
# the 7.7.0-preview manifest, winget's `Microsoft.PowerShell` package is
# MSIX-only -- single-user, sandboxed, with a version-specific path inside
# C:\Program Files\WindowsApps\, which doesn't work as a stable DefaultShell.
# Install the MSI directly from PowerShell's GitHub releases instead; that
# still lands at C:\Program Files\PowerShell\7\pwsh.exe and supports the
# OpenSSH-as-default-shell flow.
$pwshExe = "$env:ProgramFiles\PowerShell\7\pwsh.exe"
if (-not (Test-Path $pwshExe)) {
    Write-Host "  installing: PowerShell 7 (MSI from GitHub releases)"
    $release  = Invoke-RestMethod 'https://api.github.com/repos/PowerShell/PowerShell/releases/latest'
    $msiAsset = $release.assets |
        Where-Object { $_.name -match 'win-x64\.msi$' -and $_.name -notmatch '-preview' } |
        Select-Object -First 1
    if ($null -eq $msiAsset) { throw 'no win-x64.msi found in latest PowerShell GitHub release' }
    $msi = Join-Path $env:TEMP $msiAsset.name
    Invoke-WebRequest -Uri $msiAsset.browser_download_url -OutFile $msi -UseBasicParsing
    Start-Process msiexec.exe -ArgumentList "/i `"$msi`" /quiet /norestart" -Wait
    Remove-Item $msi -Force -ErrorAction SilentlyContinue
    if (-not (Test-Path $pwshExe)) {
        throw "PowerShell 7 MSI install completed but pwsh.exe still missing at $pwshExe"
    }
} else {
    Write-Host "  already installed at machine scope: PowerShell 7"
}

# Refresh PATH so freshly-installed tools (notably the 'code' shim) are
# reachable in this shell without needing a relaunch.
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
            [Environment]::GetEnvironmentVariable('Path', 'User')

# OpenSSH Server
Section 'Enabling OpenSSH Server'

$sshCap = Get-WindowsCapability -Online |
    Where-Object { $_.Name -like 'OpenSSH.Server*' } |
    Select-Object -First 1
if ($null -eq $sshCap) {
    throw 'OpenSSH.Server capability not available on this Windows edition.'
}
if ($sshCap.State -ne 'Installed') {
    Write-Host "Installing capability: $($sshCap.Name)"
    Add-WindowsCapability -Online -Name $sshCap.Name | Out-Null
} else {
    Write-Host "OpenSSH Server capability already installed"
}

Start-Service sshd
Set-Service -Name sshd -StartupType Automatic
Write-Host "sshd service running and set to Automatic"

# The capability install usually creates this rule, but be defensive.
if (-not (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' `
        -DisplayName 'OpenSSH Server (sshd)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 |
        Out-Null
    Write-Host "Created firewall rule for sshd on port 22"
} else {
    Write-Host "Firewall rule for sshd already present"
}

# WSL2 mirrored-networking firewall hole for the NixOS-WSL distro's sshd
# on port 2222. With networkingMode=mirrored in .wslconfig, inbound
# packets to WSL services hit the Windows TCP/IP stack first and are
# evaluated against Windows Defender Firewall — so the WSL distro
# needs its own rule even though sshd lives inside Linux.
# nixos/mattpc-wsl/system.nix moves WSL's sshd to 2222 to avoid
# colliding with Windows OpenSSH on 22.
if (-not (Get-NetFirewallRule -Name 'WSL-SSH-In-TCP' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name 'WSL-SSH-In-TCP' `
        -DisplayName 'WSL NixOS SSH (sshd on 2222)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 2222 |
        Out-Null
    Write-Host "Created firewall rule for WSL sshd on port 2222"
} else {
    Write-Host "Firewall rule for WSL sshd already present"
}

# Default shell for incoming SSH sessions. Without this, sshd hands ssh
# clients a cmd.exe prompt, which is useless for the dotfiles workflow.
# sshd reads HKLM:\SOFTWARE\OpenSSH\DefaultShell at session-spawn time, so
# no service restart is required after writing it.
Section 'Setting OpenSSH default shell to PowerShell 7'

$pwshPath = "$env:ProgramFiles\PowerShell\7\pwsh.exe"
if (-not (Test-Path $pwshPath)) {
    throw "pwsh.exe not found at $pwshPath -- Microsoft.PowerShell install failed?"
}
if (-not (Test-Path 'HKLM:\SOFTWARE\OpenSSH')) {
    New-Item -Path 'HKLM:\SOFTWARE\OpenSSH' -Force | Out-Null
}
Set-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -Value $pwshPath -Type String
Write-Host "OpenSSH DefaultShell -> $pwshPath"

# administrators_authorized_keys
#
# Admin accounts on Windows do NOT use ~/.ssh/authorized_keys. sshd reads keys
# for all admin users from a single file: administrators_authorized_keys.
# It silently rejects that file unless ownership is locked to Administrators
# or SYSTEM, with no inherited ACL entries -- hence /inheritance:r below.
Section 'Preparing administrators_authorized_keys'

$keyFile = 'C:\ProgramData\ssh\administrators_authorized_keys'
$keyDir  = Split-Path $keyFile
if (-not (Test-Path $keyDir)) { New-Item -ItemType Directory -Path $keyDir -Force | Out-Null }
if (-not (Test-Path $keyFile)) {
    New-Item -ItemType File -Path $keyFile -Force | Out-Null
    Write-Host "Created empty $keyFile"
} else {
    Write-Host "$keyFile already exists; leaving contents intact"
}

icacls $keyFile /inheritance:r | Out-Null
icacls $keyFile /grant 'Administrators:F' 'SYSTEM:F' | Out-Null
Write-Host "ACLs locked down (Administrators + SYSTEM only)"

# Prompt for the SSH pubkey and append it to administrators_authorized_keys.
# This script is already elevated, so the ACL-locked file is writable here --
# unlike from a non-elevated VS Code instance.
Section 'Adding SSH public key to administrators_authorized_keys'

Write-Host 'Sign into 1Password (system tray) and copy your Mac SSH PUBLIC key.'
Write-Host 'Paste it at the prompt below (or leave blank to skip and add manually later).'
Write-Host ''
$pubkey = (Read-Host 'Public key').Trim()

if (-not $pubkey) {
    Write-Warning "No key entered. Add it manually later to: $keyFile"
} elseif ($pubkey -notmatch '^(ssh-(rsa|ed25519|dss)|ecdsa-sha2-\S+) ') {
    throw "That doesn't look like an SSH public key (should start with 'ssh-ed25519' / 'ssh-rsa' / 'ecdsa-sha2-*')."
} else {
    $existing = if (Test-Path $keyFile) { @(Get-Content $keyFile -ErrorAction SilentlyContinue) } else { @() }
    if ($existing -contains $pubkey) {
        Write-Host "  key already present in $keyFile - skipping"
    } else {
        Add-Content -Path $keyFile -Value $pubkey -Encoding ascii
        Write-Host "  appended public key to $keyFile"
    }
}

# Manual steps
Section 'Manual steps'

@(
    "1. Launch Tailscale from the Start menu and sign in. Confirm this device"
    "   appears in the Tailscale admin console."
    "2. Print this box's tailnet address:"
    "      tailscale ip -4"
    "3. From the Mac, test the SSH connection:"
    "      ssh $env:USERNAME@<tailscale-name-or-ip>"
    "4. Once SSH works, run windows-setup.ps1 (locally on this box, or remotely"
    "   via SSH from the Mac) to clone dotfiles and finish the install."
) | ForEach-Object { Write-Host $_ }
