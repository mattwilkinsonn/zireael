#!/usr/bin/env bash
# Move the focused window to a named display, then focus follows. Displays
# are identified by their resolution (which is stable per monitor), then we
# look up the current macOS display index — that's what `yabai --display`
# actually accepts (it doesn't take UUIDs).
# Silent no-op if the target display isn't currently connected.

export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

name="${1:-}"
case "$name" in
laptop) match_w=1728 ;;
aw) match_w=3440 ;;
g9) match_w=5120 ;;
*)
	echo "usage: $0 {laptop|aw|g9}" >&2
	exit 1
	;;
esac

display_idx=$(yabai -m query --displays |
	jq -r --argjson w "$match_w" '.[] | select(.frame.w == $w) | .index // empty' |
	head -1)

[[ -z $display_idx ]] && exit 0

yabai -m window --display "$display_idx" 2>/dev/null || true
yabai -m display --focus "$display_idx" 2>/dev/null || true
