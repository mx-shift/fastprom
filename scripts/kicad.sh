#!/bin/sh
# Run KiCad 10's command-line tools or its Python from the official
# container, with the repository mounted at the same path, so generated
# files land in the tree with the caller's ownership.
#
#   scripts/kicad.sh cli  pcb drc ...      -> kicad-cli pcb drc ...
#   scripts/kicad.sh py   tools/x.py ...   -> python3 (with pcbnew) tools/x.py ...
#
# Set KICAD_IMAGE to use a different image; KICAD_ENGINE for docker.
set -e
ROOT=$(cd "$(dirname "$0")/.." && pwd)
IMAGE=${KICAD_IMAGE:-docker.io/kicad/kicad:10.0}
ENGINE=${KICAD_ENGINE:-podman}
mode=$1; shift
case "$mode" in
  cli) cmd=kicad-cli ;;
  py)  cmd=python3 ;;
  *) echo "usage: $0 cli|py args..." >&2; exit 2 ;;
esac
if [ "$ENGINE" = podman ]; then userns=--userns=keep-id; else userns="--user $(id -u):$(id -g)"; fi
exec "$ENGINE" run --rm $userns -v "$ROOT:$ROOT:z" -w "$PWD" -e HOME=/tmp \
  "$IMAGE" $cmd "$@"
