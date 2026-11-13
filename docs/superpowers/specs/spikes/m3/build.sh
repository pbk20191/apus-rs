#!/bin/sh
# Builds the Swift module twice (plain and library-evolution) and runs every experiment.
set -e
cd "$(dirname "$0")"
swiftc -O -emit-library -module-name Spike -o libplain.dylib spike.swift
swiftc -O -emit-library -module-name Spike -enable-library-evolution -o libevolution.dylib spike.swift
cargo build -q
for mode in plain evolution; do for part in class struct protocol; do
  echo "===== $mode / $part"; ./target/debug/m3-spike $mode $part
done; done
