#!/bin/bash
# Clippy's workspace wrapper at a checkout-local path, which Cargo hashes into
# every workspace artifact so checkouts sharing a target keep their clippy
# output apart. `cargo clippy` would force one shared clippy-driver path and
# lose that key; build_system/builder/gate/clippyrun.py runs clippy through
# this script instead.
exec clippy-driver "$@"
