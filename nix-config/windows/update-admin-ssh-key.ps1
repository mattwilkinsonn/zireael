# Rotate Windows-side OpenSSH admin access to the current zireael key.
#
# Windows admin users do not use ~/.ssh/authorized_keys. OpenSSH reads
# C:\ProgramData\ssh\administrators_authorized_keys and rejects it unless
# the ACL is restricted to Administrators + SYSTEM.

$ErrorActionPreference = 'Stop'

function Section([string]$Name) {
    Write-Host ''
    Write-Host "==> $Name" -ForegroundColor Cyan
}

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this script from an elevated PowerShell.'
}

$keyFile = 'C:\ProgramData\ssh\administrators_authorized_keys'
$keyDir = Split-Path $keyFile
$adminPublicKey = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOfinoupMf/v8sM7ez4K7wc/lN1a6NgXxHpv9wls5Ra9'

Section 'Writing administrators_authorized_keys'

if (-not (Test-Path $keyDir)) {
    New-Item -ItemType Directory -Path $keyDir -Force | Out-Null
}

Set-Content -Path $keyFile -Value $adminPublicKey -Encoding ascii
Write-Host "Wrote current zireael admin key to $keyFile"

Section 'Locking ACLs'

icacls $keyFile /reset | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "icacls /reset failed with exit code $LASTEXITCODE"
}

icacls $keyFile /inheritance:r | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "icacls /inheritance:r failed with exit code $LASTEXITCODE"
}

icacls $keyFile /grant:r 'Administrators:F' 'SYSTEM:F' | Out-Null
if ($LASTEXITCODE -ne 0) {
    throw "icacls /grant:r failed with exit code $LASTEXITCODE"
}

Write-Host 'ACLs locked down (Administrators + SYSTEM only)'

Section 'Done'
Write-Host 'Verify from another tailnet host:'
Write-Host '  ssh mattw@mattpc.tail2be430.ts.net'
