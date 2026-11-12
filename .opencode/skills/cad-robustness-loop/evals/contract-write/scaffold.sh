#!/usr/bin/env bash
set -eu
python3 "$(dirname "$0")/../fixture.py" scaffold contract-write "$PWD/fixture"
