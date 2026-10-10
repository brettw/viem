#!/bin/bash
# Print what reference Vim does to a buffer, for comparing with Viem's
# literal views. Uses the installed vim with no user configuration and
# Viem's indentation defaults.
#
# Usage: vim_reference.sh CONTENT LINE COLUMN KEYS
#   CONTENT  printf-style text, for example 'a\nb\nc\n'
#   LINE     1-based cursor line
#   COLUMN   1-based cursor column (bytes)
#   KEYS     :normal keys; escape special keys as \<Esc>, \<CR>
#
# Example: vim_reference.sh 'x\n   b\n' 2 2 'diw'   # prints 'x\nb\n'
set -euo pipefail
file=$(mktemp "${TMPDIR:-/tmp}/vimref.XXXXXX")
trap 'rm -f "$file"' EXIT
printf "$1" > "$file"
vim -u NONE -i NONE -N -es \
  -c 'set nocp ai et sw=2 sts=2 ts=2 sta bs=indent,eol,start ff=unix fixeol' \
  -c "call cursor($2,$3)" -c "exe \"normal $4\"" -c 'wq' "$file"
python3 -c 'import sys; print(repr(open(sys.argv[1], "rb").read().decode()))' "$file"
