# awsmac — Install Procedure

AWS EC2 `mac2-m2.metal` instance (Apple M2, 8-core, 24 GB), used as the
most-disposable **stopgap** macOS arm64 self-hosted Buildkite CI agent
for the sealedsecurity org (queue `macos-arm64-selfhosted` — the same
pool the owned `mattmini` serves). Stands in while the Buildkite-trial
macOS minutes are exhausted and neither the owned mini nor the
dedicatedmac.io rental is up yet.

**Instance:**

- `mac2-m2.metal` — Apple M2 (8-core: 4P + 4E), 24 GB, aarch64-darwin.
- Runs on a **dedicated host** (EC2 Mac requirement). Dedicated hosts
  have a **24-hour minimum allocation** — you cannot release the host
  (and stop billing) until 24h after allocation. On-demand
  `mac2-m2.metal` is ~$0.65/hr ≈ **$16/day**; fits the $100 credit
  budget with room to spare.
- Account is `ec2-user` (the AWS macOS AMI's default admin) — no
  account creation, unlike the owned-mini hosts.

**OS strategy:** current macOS as shipped by the AWS AMI. Pick the
latest `amzn-ec2-macos` (or a current Apple-provided) arm64 macOS AMI at
launch so the sandbox-exec tests behave the same as production.

## Cost + lifecycle guardrails

- **24h minimum.** Allocating the dedicated host commits you to 24h of
  billing even if you terminate the instance in 10 minutes. Plan to
  keep it up for at least a day.
- **Terminate + release to stop billing.** Terminating the *instance*
  isn't enough — the *dedicated host* keeps billing until released
  (after the 24h minimum). See [Teardown](#teardown).
- This is the throwaway tier: the moment the owned mattmini or the
  dedicatedmac.io rental comes online, tear this down.

## Launch the instance

From your machine with the AWS CLI configured (or the console):

1. **Allocate a dedicated host** for the `mac2-m2` family in a region
   that has EC2 Mac capacity (us-east-1, us-west-2, etc.):

   ```bash
   aws ec2 allocate-hosts \
     --instance-type mac2-m2.metal \
     --availability-zone us-east-1a \
     --auto-placement on \
     --quantity 1
   ```

2. **Launch the instance** onto that host. Pick a current macOS arm64
   AMI (`aws ec2 describe-images --owners amazon --filters
   'Name=name,Values=amzn-ec2-macos-*' 'Name=architecture,Values=arm64_mac'`),
   an existing keypair, and a security group. The SG only needs
   **outbound** open (Tailscale + HTTPS) plus a temporary inbound SSH
   (port 22) from your IP for the first connection — once Tailscale is
   up you can remove the inbound SSH rule entirely:

   ```bash
   aws ec2 run-instances \
     --instance-type mac2-m2.metal \
     --image-id ami-XXXXXXXX \
     --key-name <your-keypair> \
     --security-group-ids sg-XXXXXXXX \
     --placement 'Tenancy=host' \
     --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=awsmac}]'
   ```

3. EC2 Mac instances take ~10-15 min to pass status checks on first
   boot (the host scrubs + reimages). Wait for `2/2 checks passed`.

## First connection

SSH in as `ec2-user` with your keypair:

```bash
ssh -i ~/.ssh/<your-keypair>.pem ec2-user@<instance-public-ip>
```

`ec2-user` is an admin with passwordless sudo — no account setup
needed. (If you want a password for console/VNC fallback,
`sudo /usr/bin/dscl . -passwd /Users/ec2-user <pw>`, but it's optional
for a headless box reached over Tailscale.)

## Pre-install checklist

- [ ] `nix-config` repo pushed (run `config status` to confirm).
- [ ] Tailscale pre-auth key ready at
      <https://login.tailscale.com/admin/settings/keys>. Single-use,
      tagged with `tag:ci-runner`. Paste at the bootstrap prompt.
- [ ] Buildkite agent token ready (org Agents page → Reveal Agent
      Token) at
      <https://buildkite.com/organizations/sealedsecurity/agents>.
- [ ] sealedsecurity-ci App `.pem` ready (org → Developer settings →
      GitHub Apps → sealedsecurity-ci → Private keys). The bootstrap
      stages it to `/etc/buildkite-agent/ci-app-key.pem`.

## Bootstrap

Get the `nix-config` repo onto the box (`gh repo clone` or scp), then:

```bash
bash <path-to>/nix-config/darwin/scripts/awsmac-bootstrap.sh
```

The script is self-contained. It walks the steps in an order that gets
Tailscale up early so the rest is reachable over the tailnet (and you
can drop the SG's inbound-SSH rule):

1. **Xcode Command Line Tools** — `xcode-select --install` if missing.
   (AWS macOS AMIs usually ship the CLT pre-installed; this is a
   no-op then.)
2. **macOS hostname** — sets `HostName` / `LocalHostName` /
   `ComputerName` to `awsmac` via `scutil` so Tailscale registers with
   the right name in step 4.
3. **Homebrew** — `/opt/homebrew` prefix (Apple Silicon). Needed for
   the Tailscale formula + gh + op.
4. **Tailscale** — installs the `tailscale` Homebrew **formula** (not
   the cask), starts `tailscaled` as a launchd system daemon via
   `sudo brew services start tailscale`, prompts for a pre-auth key (or
   reads `--auth-key` / `TAILSCALE_AUTH_KEY`), and runs `tailscale up
   --ssh --hostname=awsmac`. Now reachable at
   `ssh ec2-user@awsmac.tail08a5c5.ts.net` — switch to that and you can
   delete the SG's inbound SSH rule.

   Formula (not cask) because the cask's CLI shim is broken on recent
   macOS; the formula gives upstream Go-built `tailscaled` +
   `tailscale` — same as Linux.
5. **gh + dotfiles** — installs `gh`, prompts for `gh auth login`,
   clones your dotfiles into `~` (colocated git+jj at `~/.git`).
6. **Nix** — upstream nixos.org multi-user installer; nix-darwin then
   manages the daemon plist declaratively (`nix.enable = true`).
7. **Buildkite agent token + CI App key** — writes the org agent token
   to `/etc/buildkite-agent/agent-token` (mode 600 root:wheel) and
   stages the sealedsecurity-ci App key to
   `/etc/buildkite-agent/ci-app-key.pem`. Both must be in place before
   step 8.
8. **darwin-rebuild switch** — keeps native sshd disabled (Tailscale
   SSH is the only access path), locks pmset, lays down the agent
   LaunchDaemon + decrypt-agent-secrets daemon, the pf agent-egress
   lockdown, and Glances + tailscale-serve-glances.
9. **Sanity checks** — confirms native sshd is off, Tailscale SSH +
   agents + Glances are all up.

Re-runnable: every step skips if already done.

**Security posture:** awsmac keeps no 1Password service-account tokens
on disk — the host runs untrusted CI workflows via the self-hosted
Buildkite agent pool, so any SA accessible to processes here is a
credential a compromised agent could exfiltrate. The agent token + CI
App key in step 7 are the only secrets, mode-600 root-wheel. They are
not rotated per-use — see [Teardown](#teardown).

After it completes, SSH from your MBP:

```bash
ssh ec2-user@awsmac.tail08a5c5.ts.net
```

And the agent should appear at
<https://buildkite.com/organizations/sealedsecurity/agents> with tags
`queue=macos-arm64-selfhosted` and `host=awsmac` (distinct from the
owned mini's `host=mattmini` and the rental's `host=dedicatedmacio-mini`
— the three don't collide in the agents list).

## Glances dashboard

System metrics dashboard at `https://awsmac.tail08a5c5.ts.net:9443/`
(after Tailscale is up and the `glances` + `tailscale-serve-glances`
launchd daemons have run at least once). Reachable from any tailnet
device.

If the page 404s on first visit, check
`/var/log/tailscale-serve-glances.log` — the serve hook re-runs
idempotently on every nix-switch, so `sudo darwin-rebuild switch
--flake .#awsmac` typically fixes it.

## Permissions / Gatekeeper

The agent binary lives in the nix store; the hand-rolled launchd daemon
(`darwin/awsmac/system.nix`) runs it as `_buildkite-agent`. Gatekeeper
doesn't gate launchd-launched binaries in the nix store path, so no
first-launch `xattr -d com.apple.quarantine` dance is needed.

`cargo nextest` with seal's sandbox tests exercises `sandbox-exec`,
which is built into macOS and works without approval.

## Teardown

This is the deliberate revocation path — termination destroys the
volume and the plaintext secrets with it, so **no secret rotation is
needed**. Do this the moment a longer-lived runner is up:

1. **Deregister the agent.** Stop the daemon so a stale agent doesn't
   linger in the pool:

   ```bash
   sudo launchctl bootout system/com.sealedsecurity.buildkite-agent-sealed-macos
   ```

   Then remove it at
   <https://buildkite.com/organizations/sealedsecurity/agents> (or it
   ages out once it stops heartbeating).
2. **Untrack the Tailscale node** at
   <https://login.tailscale.com/admin/machines>.
3. **Terminate the instance + release the host.** Terminating the
   instance stops instance billing; releasing the dedicated host (only
   possible ≥24h after allocation) stops host billing:

   ```bash
   aws ec2 terminate-instances --instance-ids i-XXXXXXXX
   # wait for the instance to reach 'terminated', then (>=24h after
   # allocate-hosts):
   aws ec2 release-hosts --host-ids h-XXXXXXXX
   ```

   The EBS root volume is destroyed on terminate (default for EC2 Mac),
   taking `/etc/buildkite-agent/{agent-token,ci-app-key.pem}` with it.
   Termination is the revocation.
4. **(Belt-and-suspenders)** If you want the token dead the instant you
   terminate — e.g. you're paranoid about the volume-scrub window —
   rotate the Buildkite agent token at the org Agents page and
   regenerate the sealedsecurity-ci App key. Optional; the volume
   destroy already covers it.
