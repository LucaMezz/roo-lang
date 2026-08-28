#!/bin/sh
set -e

version="$1"
shift

for dir in "$@"; do
    sed -i "0,/^version = \".*\"/s//version = \"$version\"/" "$dir/Cargo.toml"
done
