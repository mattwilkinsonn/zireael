#!/usr/bin/env bash
# Build debug binaries and install them to ~/.cargo/bin (a dir already on
# most PATHs). Dev convenience for running the just-built tools locally.
#
#   scripts/install-debug.sh              # all three tools
#   scripts/install-debug.sh jj-hooks     # jj-hooks + jj-hp
#   scripts/install-debug.sh jj-gt
#   scripts/install-debug.sh akiflow-cli  # the `af` binary
#
# On Linux, writing over an in-use executable fails with ETXTBSY (text
# file busy), so each binary is unlinked before the fresh copy lands (a
# running process keeps its inode). macOS lets you overwrite an active
# binary, so the unlink is a harmless no-op there; macOS also gets an
# ad-hoc codesign so the binary doesn't trip Gatekeeper.
set -euo pipefail

dest="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$dest"

install_bin() {
	# $1 = source path, $2 = basename to install as, $3 = codesign? (1/0)
	local src="$1" name="$2" sign="${3:-0}"
	rm -f "$dest/$name"
	cp "$src" "$dest/$name"
	if [ "$sign" = "1" ] && [ "$(uname)" = "Darwin" ]; then
		codesign -s - "$dest/$name" 2>/dev/null && echo "Codesigned $name" || true
	fi
}

debug_jj_hooks() {
	cargo build -p jj-hooks --bin jj-hooks --bin jj-hp
	install_bin "target/debug/jj-hooks" jj-hooks 1
	install_bin "target/debug/jj-hp" jj-hp 1
	echo "Installed debug builds (jj-hooks + jj-hp) to $dest"
}

debug_jj_gt() {
	cargo build -p jj-gt --bin jj-gt
	install_bin "target/debug/jj-gt" jj-gt 1
	echo "Installed debug build (jj-gt) to $dest"
}

debug_akiflow_cli() {
	# akiflow-cli compiles to a single `af` binary via `bun build
	# --compile`. bun's compiled binaries don't trip Gatekeeper (no
	# codesign needed).
	(
		cd tools/akiflow-cli
		bun install --frozen-lockfile
		bun build src/index.ts --compile --outfile af
	)
	install_bin "tools/akiflow-cli/af" af 0
	echo "Installed debug build (af) to $dest"
}

case "${1:-all}" in
all)
	debug_jj_hooks
	debug_jj_gt
	debug_akiflow_cli
	;;
jj-hooks) debug_jj_hooks ;;
jj-gt) debug_jj_gt ;;
akiflow-cli) debug_akiflow_cli ;;
*)
	echo "error: unknown tool '$1'" >&2
	echo "valid: all | jj-hooks | jj-gt | akiflow-cli" >&2
	exit 1
	;;
esac
