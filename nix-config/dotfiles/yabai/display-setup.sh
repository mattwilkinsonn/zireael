#!/usr/bin/env bash
# Apply per-display layout defaults. Identifies the laptop by its display
# width (1728 logical px). Display UUIDs can change on reconnect, so width
# is the more stable match.

export PATH="/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"

# Laptop's first space → stack layout. Other displays left at bsp.
laptop_space=$(yabai -m query --displays |
	jq -r '[.[] | select(.frame.w == 1728)] | .[0].spaces[0] // empty')

if [[ -n $laptop_space ]]; then
	yabai -m space "$laptop_space" --layout stack 2>/dev/null || true
fi
