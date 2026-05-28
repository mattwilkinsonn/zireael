#!/usr/bin/env bash
# Cycle the focused window + focus across connected displays, wrapping at
# the ends. Usage: cycle-display.sh prev|next

export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

dir="${1:-next}"
case "$dir" in
prev) fallback="last" ;;
next) fallback="first" ;;
*)
	echo "usage: $0 prev|next" >&2
	exit 1
	;;
esac

# Move the window. If we're at the wrap end, fall back to first/last.
yabai -m window --display "$dir" 2>/dev/null ||
	yabai -m window --display "$fallback" 2>/dev/null ||
	true

# Focus the new display. Same fallback behavior.
yabai -m display --focus "$dir" 2>/dev/null ||
	yabai -m display --focus "$fallback" 2>/dev/null ||
	true
