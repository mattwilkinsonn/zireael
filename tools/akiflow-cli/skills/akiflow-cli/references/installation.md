# Installation

## Option 1: npm (Recommended for Bun users)

```bash
bun install -g akiflow-cli
```

This installs the `af` command globally.

## Option 2: Prebuilt Binary (No runtime needed)

Download the binary for your platform from [GitHub Releases](https://github.com/code-yeongyu/akiflow-cli/releases):

### macOS (Apple Silicon)

```bash
curl -L https://github.com/code-yeongyu/akiflow-cli/releases/latest/download/af-darwin-arm64 -o af
chmod +x af
sudo mv af /usr/local/bin/
```

### macOS (Intel)

```bash
curl -L https://github.com/code-yeongyu/akiflow-cli/releases/latest/download/af-darwin-x64 -o af
chmod +x af
sudo mv af /usr/local/bin/
```

### Linux (x64)

```bash
curl -L https://github.com/code-yeongyu/akiflow-cli/releases/latest/download/af-linux-x64 -o af
chmod +x af
sudo mv af /usr/local/bin/
```

## Option 3: Build from Source

```bash
git clone https://github.com/code-yeongyu/akiflow-cli.git
cd akiflow-cli
bun install
bun run build
sudo mv af /usr/local/bin/
```

## Verify Installation

```bash
af --help
```

## First-time Setup

After installation, extract credentials from your browser:

```bash
af auth
```

Requires Akiflow to be logged in via browser (Chrome, Firefox, Safari, Arc, Brave, Edge supported).
