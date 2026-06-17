#!/bin/sh
set -eu

viewer_repo="${1:-../Viewer2000}"
rm -f tests/vectors/*.txt
cp "$viewer_repo"/contracts/vectors/*.txt tests/vectors/
