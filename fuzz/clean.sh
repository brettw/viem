#!/bin/sh
# Remove the fuzz builds: fuzz/target and any separate fuzz/target-* build such
# as target-plain. Corpora, artifacts, logs and minimized inputs are kept.
set -eu
cd "$(dirname "$0")"
rm -rf target target-*
