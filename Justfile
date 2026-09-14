musl_target := "x86_64-unknown-linux-musl"

build-linux-musl target=musl_target:
    env -u NO_COLOR trunk build --config web/Trunk.toml --release
    cargo zigbuild --release --target {{target}} --features honk-probe
