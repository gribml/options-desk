#!/usr/bin/env bash
set -e
curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk build --release