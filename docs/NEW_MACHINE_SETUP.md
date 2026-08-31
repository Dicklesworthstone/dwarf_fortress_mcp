# New-machine setup: repository, Dwarf Fortress, and DFHack

This guide reproduces the development/reference environment assembled on Ubuntu x86-64 on
August 31, 2026. It installs the Rust workspace and a separately runnable Dwarf Fortress Classic
53.16 + DFHack 53.16-r1.1 reference stack.

It does **not** make this repository control Dwarf Fortress. The checked-in bridge is a
fail-closed compile placeholder, and the Rust `DfhackAdapter` is only a liveness probe. Read
[`../IMPLEMENTATION_STATUS.md`](../IMPLEMENTATION_STATUS.md) before interpreting a successful
build or DFHack launch.

## Reproduced versions and paths

The reference machine uses:

| Component | Version / revision |
|---|---|
| Dwarf Fortress Classic | 53.16 Linux x86-64 |
| DFHack binary release | 53.16-r1.1 Linux x86-64 |
| DFHack source | tag `53.16-r1.1`, commit `b638b59d0876d9bdf8b5f97e52714206ab7f3266` |
| Rust | repository-selected latest nightly; observed `1.100.0-nightly (2026-08-30)` |
| MCP dependency | exact `fastmcp_rust` revision from the workspace `Cargo.toml`/`Cargo.lock` |

The installation root used here is `/data/opt/dwarf_fortress_reference`. You may choose another
absolute path, but keep game binaries outside this Git repository.

```text
/data/opt/dwarf_fortress_reference/
├── downloads/
├── installs/
│   └── df-53.16-classic-dfhack-53.16-r1.1/
├── sources/
│   └── dfhack-53.16-r1.1/
└── current -> installs/df-53.16-classic-dfhack-53.16-r1.1
```

## 1. Preconditions and disk space

These instructions target Ubuntu/Debian x86-64. Confirm the architecture and available space
before downloading anything:

```bash
uname -m
df -h /data
```

The pinned archives, runnable stack, and recursive DFHack source checkout used about 400 MiB on
the reference machine. Rust artifacts are much larger: reserve at least 20 GiB for the repository's
full debug-and-release qualification. A full DFHack source build needs another 8–12 GiB of
headroom. The reference setup deliberately used the official DFHack binary release rather than
spending that space on a redundant full build.

Install the packages used for the repository, binary runtime, headless launch, and optional C++
smoke build:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential bzip2 ca-certificates ccache cmake curl git iproute2 mesa-utils \
  ninja-build pkg-config python3 ripgrep \
  protobuf-compiler libprotobuf-dev zlib1g-dev libsdl2-dev libsdl2-image-2.0-0 \
  libxml-libxml-perl libxml-libxslt-perl xvfb
```

## 2. Rust toolchain and repository dependencies

Install `rustup` using your operating-system package or the official instructions at
<https://rustup.rs>. If `rustup` is already present, do not reinstall it. From the repository
root, the checked-in `rust-toolchain.toml` selects nightly and installs `clippy`, `rustfmt`,
`rust-src`, and `llvm-tools-preview`. Clone the project if it is not already present:

```bash
sudo mkdir -p /data/projects
sudo chown "$(id -u):$(id -g)" /data/projects
git clone https://github.com/Dicklesworthstone/dwarf_fortress_mcp \
  /data/projects/dwarf_fortress_mcp
cd /data/projects/dwarf_fortress_mcp
rustup show active-toolchain
rustup component add clippy rustfmt rust-src llvm-tools-preview --toolchain nightly
cargo fetch --locked
cargo check --locked --workspace --all-targets --all-features
```

Do not replace the exact `fastmcp_rust` pin or enable its legacy feature graph during setup.
`cargo fetch --locked` populates Cargo's cache; later qualification deliberately resolves locked
metadata offline.

## 3. Download the pinned game and DFHack archives

Create explicit destinations. These commands stop rather than overwrite an existing install:

```bash
DFMCP_STACK_ROOT=/data/opt/dwarf_fortress_reference
DFMCP_INSTALL_DIR="$DFMCP_STACK_ROOT/installs/df-53.16-classic-dfhack-53.16-r1.1"

mkdir -p "$DFMCP_STACK_ROOT/downloads" "$DFMCP_STACK_ROOT/installs" "$DFMCP_STACK_ROOT/sources"
test ! -e "$DFMCP_INSTALL_DIR"

curl --fail --location --retry 3 \
  --output "$DFMCP_STACK_ROOT/downloads/df_53_16_linux.tar.bz2" \
  https://www.bay12games.com/dwarves/df_53_16_linux.tar.bz2

curl --fail --location --retry 3 \
  --output "$DFMCP_STACK_ROOT/downloads/dfhack-53.16-r1.1-Linux-64bit.tar.bz2" \
  https://github.com/DFHack/dfhack/releases/download/53.16-r1.1/dfhack-53.16-r1.1-Linux-64bit.tar.bz2
```

Verify before extraction:

```bash
printf '%s  %s\n' \
  2f9c0134b2465cccb705b8d3e322cdff07df7374ffbfafffe8f982f2ef7e7e7d \
  "$DFMCP_STACK_ROOT/downloads/df_53_16_linux.tar.bz2" \
  | sha256sum --check --strict

printf '%s  %s\n' \
  87e041a3e9d260fd9295170182a90eb27ea3c92f05471e4e65259b32f7cb0204 \
  "$DFMCP_STACK_ROOT/downloads/dfhack-53.16-r1.1-Linux-64bit.tar.bz2" \
  | sha256sum --check --strict
```

The DFHack digest matches the release checksum. The Bay 12 site did not provide a signed digest
alongside the Classic download; the DF digest above records the exact archive used for
reproducibility and is not an independent authenticity proof. Obtain both archives only from the
official Bay 12 and DFHack release locations.

Inspect the archive roots, then overlay DFHack onto the Classic directory as required by DFHack's
binary installation model:

```bash
tar -tjf "$DFMCP_STACK_ROOT/downloads/df_53_16_linux.tar.bz2" | head
tar -tjf "$DFMCP_STACK_ROOT/downloads/dfhack-53.16-r1.1-Linux-64bit.tar.bz2" | head

mkdir "$DFMCP_INSTALL_DIR"
tar -xjf "$DFMCP_STACK_ROOT/downloads/df_53_16_linux.tar.bz2" -C "$DFMCP_INSTALL_DIR"
tar -xjf "$DFMCP_STACK_ROOT/downloads/dfhack-53.16-r1.1-Linux-64bit.tar.bz2" -C "$DFMCP_INSTALL_DIR"
ln -s "$DFMCP_INSTALL_DIR" "$DFMCP_STACK_ROOT/current"
```

On a rerun, verify `readlink -f "$DFMCP_STACK_ROOT/current"` instead of replacing the symlink or
existing install blindly.

## 4. Configure the built-in DFHack remote service safely

DFHack's built-in remote service listens on TCP port 5000. Keep it loopback-only:

```json
{
  "allow_remote": false,
  "port": 5000
}
```

Save that as:

```text
/data/opt/dwarf_fortress_reference/current/dfhack-config/remote-server.json
```

This built-in endpoint uses DFHack's protobuf-over-TCP remote protocol. It is **not gRPC**, and it
is not the repository's proposed `dfmcp.proto` bridge.

Check runtime linkage before launch. No output from the second command is success:

```bash
cd "$DFMCP_STACK_ROOT/current"
ldd ./dwarfort
ldd ./dwarfort | rg 'not found'
```

## 5. Launch and verify DFHack

For a normal desktop session:

```bash
cd "$DFMCP_STACK_ROOT/current"
./dfhack
```

For a headless development smoke test, run this in one terminal:

```bash
cd "$DFMCP_STACK_ROOT/current"
env SDL_AUDIODRIVER=dummy LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a ./dfhack
```

Wait for `DFHack is ready`. In another terminal, verify the process and remote service:

```bash
DFMCP_STACK_ROOT=/data/opt/dwarf_fortress_reference
cd "$DFMCP_STACK_ROOT/current"
./dfhack-run RemoteFortressReader_version
ss -ltnp | rg '127\.0\.0\.1:5000'
```

The reproduced installation returned RemoteFortressReader `0.21.0`. Stop the smoke-test game
cleanly with:

```bash
./dfhack-run die
```

The launcher may report exit status 154 after DF exits; the important evidence is orderly
shutdown and removal of the listener, not a fabricated zero status.

## 6. Optional exact DFHack source checkout

The binary release is sufficient to run DFHack. Keep an exact recursive source checkout for API
research and future genuine plugin work:

```bash
DFMCP_STACK_ROOT=/data/opt/dwarf_fortress_reference
git clone --recursive --branch 53.16-r1.1 \
  https://github.com/DFHack/dfhack.git \
  "$DFMCP_STACK_ROOT/sources/dfhack-53.16-r1.1"

git -C "$DFMCP_STACK_ROOT/sources/dfhack-53.16-r1.1" checkout --detach \
  b638b59d0876d9bdf8b5f97e52714206ab7f3266
git -C "$DFMCP_STACK_ROOT/sources/dfhack-53.16-r1.1" submodule update --init --recursive

git -C "$DFMCP_STACK_ROOT/sources/dfhack-53.16-r1.1" rev-parse HEAD
git -C "$DFMCP_STACK_ROOT/sources/dfhack-53.16-r1.1" submodule status --recursive
```

The expected top-level commit is
`b638b59d0876d9bdf8b5f97e52714206ab7f3266`. A leading `-` or `+` in submodule status means the
recursive checkout is absent or not at the recorded commit.

## 7. Qualify this repository

From a clean checkout:

```bash
cd /data/projects/dwarf_fortress_mcp
./scripts/qualify_local.sh
```

During review of uncommitted work only:

```bash
DFMCP_ALLOW_DIRTY=1 ./scripts/qualify_local.sh
```

The dirty-tree receipt is development evidence, not release evidence. Inspect the emitted
`target/qualification/<run>/qualification-receipt.json` and ensure every gate passed.

You may compile the proposed bridge target only as a fail-closed C++ smoke check:

```bash
DFMCP_BRIDGE_BUILD_DIR="$(mktemp -d /tmp/dfmcp-bridge-smoke.XXXXXXXX)"
cmake -S bridge/dfhack-plugin -B "$DFMCP_BRIDGE_BUILD_DIR"
cmake --build "$DFMCP_BRIDGE_BUILD_DIR"
```

Do **not** install the resulting `.so` into DFHack. It is not registered or linked as a genuine
DFHack plugin, and its initialization deliberately fails.

## Troubleshooting

- `error while loading shared libraries`: rerun both `ldd` commands and install the named Ubuntu
  runtime package; do not copy random shared libraries into the game directory.
- `cannot open display` or SDL video errors: use `xvfb-run -a` and software GL exactly as shown.
- audio-device errors: keep `SDL_AUDIODRIVER=dummy` for headless runs.
- port 5000 is already occupied: identify the owner with `ss -ltnp`; do not kill an unknown
  process or expose DFHack remotely.
- `dfhack-run` cannot connect: ensure the game reached `DFHack is ready`, the remote config is in
  the active install, and the listener is loopback-only.
- version mismatch: DF 53.16 and DFHack 53.16-r1.1 are a tested pair. Do not mix release families.
- Cargo `--offline` failure: run `cargo fetch --locked` once with network access, then retry.
- qualification rejects a dirty tree: commit/stash intentionally, or use
  `DFMCP_ALLOW_DIRTY=1` only for a development receipt.
- low disk space: check before qualification with `df -h .` and `du -sh target`; use
  `cargo clean --target-dir /absolute/disposable/target` for an explicitly identified build tree,
  or set `CARGO_TARGET_DIR` to a volume with at least 20 GiB free. Never delete the installation
  root through an unresolved variable or broad recursive command.

## Updating the reference stack

Treat any DF/DFHack upgrade as a new compatibility target:

1. create a new versioned install directory rather than overwriting `current`;
2. record source URLs, SHA-256 digests, DFHack tag/commit, submodule state, and package changes;
3. launch and verify the pair independently;
4. update the compatibility registries and this guide;
5. move `current` only after verification;
6. do not claim bridge compatibility until the live handshake/observation acceptance gate passes.

Primary upstream references: [Bay 12 downloads](https://www.bay12games.com/dwarves/),
[DFHack 53.16-r1.1 release](https://github.com/DFHack/dfhack/releases/tag/53.16-r1.1),
[DFHack installation](https://docs.dfhack.org/en/stable/docs/Installing.html), and
[DFHack remote API](https://docs.dfhack.org/en/stable/docs/dev/Remote.html).
