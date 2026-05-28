# PowerShell profile (Windows). Tracked here at nix-config/windows/profile.ps1
# and dot-sourced from the per-host $PROFILE by windows-setup.ps1, so the
# real content stays in the dotfiles tree (which avoids the OneDrive
# Documents-redirect trap that would otherwise hit a profile sitting at
# Documents\PowerShell\Microsoft.PowerShell_profile.ps1).

# 1Password service-account tokens. Mirrors the Linux dev-host pattern:
# plaintext files at ~/.config/op/*-token, ACL-locked to the current user
# (windows-setup.ps1 sets the icacls when prompting for the tokens). Mac
# uses Keychain via darwin/home.nix mkBefore. Two accounts:
#
#   $env:OP_SERVICE_ACCOUNT_TOKEN       — personal (read op://Dev + op://Server).
#   $env:OP_TEAM_SERVICE_ACCOUNT_TOKEN  — sealedsecurity team (read op://Employee Dev).
#
# Without the relevant token set, the corresponding `op inject` pass in
# load-secrets below silently no-ops and load-secrets prints a one-line
# warning naming which token is missing.
$opTokenFile = Join-Path $HOME '.config\op\service-account-token'
if (Test-Path $opTokenFile) {
    $env:OP_SERVICE_ACCOUNT_TOKEN = (Get-Content $opTokenFile -Raw).Trim()
}
$opTeamTokenFile = Join-Path $HOME '.config\op\team-service-account-token'
if (Test-Path $opTeamTokenFile) {
    $env:OP_TEAM_SERVICE_ACCOUNT_TOKEN = (Get-Content $opTeamTokenFile -Raw).Trim()
}

# Cross-platform 1P CLI secret loading. Same shape as the zsh load-secrets
# function in shared/load-secrets.nix: each account does one `op inject`
# pass against its template, then Invoke-Expression the rendered
# PowerShell to set env vars on the current process. Account swap is
# implemented by setting $env:OP_SERVICE_ACCOUNT_TOKEN before each call.
#
# Skipping the env exports entirely on op failure (rather than exporting
# empty strings) is the load-bearing bit: empty vars get inherited by
# spawned tools and silently turn into 401s instead of clear "credential
# missing" errors.
function load-secrets {
    if (-not (Get-Command op -ErrorAction SilentlyContinue)) { return }

    # Personal account — items in op://Dev/...
    $personalTemplate = @'
$env:ANTHROPIC_API_KEY = "{{ op://Dev/Anthropic API Key/credential }}"
$env:OPENROUTER_API_KEY = "{{ op://Dev/OpenRouter API Key/credential }}"
$env:GITHUB_PERSONAL_ACCESS_TOKEN = "{{ op://Dev/GitHub Personal Access Token/token }}"
$env:CLOUDFLARE_API_TOKEN = "{{ op://Dev/Personal Cloudflare API Token/token }}"
$env:NEON_API_KEY = "{{ op://Dev/Neon API Key/credential }}"
'@
    if ($env:OP_SERVICE_ACCOUNT_TOKEN) {
        $rendered = $personalTemplate | & op inject 2>$null | Out-String
        if ($rendered.Trim()) {
            Invoke-Expression $rendered
        } else {
            Write-Warning 'op inject failed for Personal account - OP_SERVICE_ACCOUNT_TOKEN may be invalid. Env vars not loaded.'
        }
    } else {
        Write-Warning 'OP_SERVICE_ACCOUNT_TOKEN unset - Personal secrets not loaded.'
    }

    # Team account (sealedsecurity.1password.com) — items in op://Employee Dev/...
    # Swap $env:OP_SERVICE_ACCOUNT_TOKEN for the call, then restore so the
    # rest of the session sees the personal token.
    $teamTemplate = @'
$env:CLAUDE_CODE_OAUTH_TOKEN = "{{ op://Employee Dev/Sealed Claude Code OAuth Token/credential }}"
$env:LINEAR_API_KEY = "{{ op://Employee Dev/Linear API Key/credential }}"
'@
    if ($env:OP_TEAM_SERVICE_ACCOUNT_TOKEN) {
        $savedTok = $env:OP_SERVICE_ACCOUNT_TOKEN
        $env:OP_SERVICE_ACCOUNT_TOKEN = $env:OP_TEAM_SERVICE_ACCOUNT_TOKEN
        try {
            $rendered = $teamTemplate | & op inject 2>$null | Out-String
            if ($rendered.Trim()) {
                Invoke-Expression $rendered
            } else {
                Write-Warning 'op inject failed for Team account - OP_TEAM_SERVICE_ACCOUNT_TOKEN may be invalid. Env vars not loaded.'
            }
        } finally {
            $env:OP_SERVICE_ACCOUNT_TOKEN = $savedTok
        }
    } else {
        Write-Warning 'OP_TEAM_SERVICE_ACCOUNT_TOKEN unset - Team secrets not loaded.'
    }
}

# Auto-invoke at startup only when stdout is a real tty -- skips
# non-interactive subshells (ssh-and-run-command, env-resolver scripts,
# etc.) so they don't pay the ~500ms HTTPS round-trip on every spawn.
if (-not [Console]::IsOutputRedirected) {
    load-secrets
}

# Claude Code OAuth token swap helpers. Each new shell starts with the
# Sealed default loaded by load-secrets above; these flip
# CLAUDE_CODE_OAUTH_TOKEN in the current shell to the other account
# without restarting it. New shells revert to Sealed.
function claude-sealed {
    if (-not $env:OP_TEAM_SERVICE_ACCOUNT_TOKEN) {
        Write-Error 'OP_TEAM_SERVICE_ACCOUNT_TOKEN unset - cannot read Sealed claude token.'
        return
    }
    $savedTok = $env:OP_SERVICE_ACCOUNT_TOKEN
    $env:OP_SERVICE_ACCOUNT_TOKEN = $env:OP_TEAM_SERVICE_ACCOUNT_TOKEN
    try {
        $val = & op read 'op://Employee Dev/Sealed Claude Code OAuth Token/credential' 2>$null
    } finally {
        $env:OP_SERVICE_ACCOUNT_TOKEN = $savedTok
    }
    if ($val) {
        $env:CLAUDE_CODE_OAUTH_TOKEN = $val
        Write-Host 'CLAUDE_CODE_OAUTH_TOKEN -> Sealed'
    } else {
        Write-Error 'failed to read Sealed claude token (OP_TEAM_SERVICE_ACCOUNT_TOKEN invalid?)'
    }
}

function claude-personal {
    $val = & op read 'op://Dev/Personal Claude Code OAuth Token/credential' 2>$null
    if ($val) {
        $env:CLAUDE_CODE_OAUTH_TOKEN = $val
        Write-Host 'CLAUDE_CODE_OAUTH_TOKEN -> Personal'
    } else {
        Write-Error 'failed to read Personal claude token (op locked or signed out?)'
    }
}
