# Litematica for Horizon

A native Rust schematic mod for Horizon’s Minecraft Java Edition 26.3 client. Open the mod with **M** or start with `/lm help` in a world.

- Load `.litematic` versions 1–7, Sponge `.schem`, legacy `.schematic`, and vanilla structure `.nbt`.
- Preview a build as ghost blocks, move or rotate the placement, mirror it, and show individual layers along X, Y, or Z.
- Compare your build to its schematic and view the required materials. Keep multiple placements and restore them per world and dimension.
- Capture an area and export its blocks and states as a `.litematic`. Easy Place and the optional printer are available from the mod menu.

## Build and install

The manifest targets **Horizon Nightly 2026-11-20 (`0a875da`)**. The renderer and placement helpers are compiled into that native client. To build it, open the Horizon launcher, install the mod from the `hmod/` folder, and choose **Build**. This is native code and the launcher requires native-code approval. The compatible game revision is pinned in `hmod/horizon-mod.toml`.

With the matching Horizon launcher and `cargo-horizon-mod` installed, package the mod with `python tools/package.py --launcher <path-to-horizon-launcher>`. Run `cargo test --workspace` for the portable Rust tests. Run `cargo run -p sample-hut -- sample_hut.litematic` to generate a small test schematic.

## Use it in a world

1. Run `/lm folder` to locate the instance’s `schematics/` directory and put your schematic there.
2. Press **M** and choose the file under **Load**, or run `/lm load <name>`.
3. Use the **Placement**, **Display**, and **Materials** tabs to position the ghost, configure its appearance, and check your build. `/lm layer axis x` switches to X-axis slicing.
4. To export, set two corners in **Save** or with `/lm pos1` and `/lm pos2`. Run `/lm save <name>` to write the selected area.

See the [in-game guide](docs/wiki/Home.md), [command reference](docs/wiki/Commands.md), and [port plan](PORTING.md). Imports are limited to 128 MiB on disk and 512 MiB of decoded NBT. Sponge, vanilla structure, and legacy schematic imports have separate cell limits of 64 Mi, 16 Mi, and 2 Mi respectively. The current area capture writes a single region of block states. Saving entity data, editing individual imported subregions, and compatibility with the Java Easy Place server protocol remain open.

## Credits and license

This independent Rust project builds on [Horizonmatica by keksi2k](https://github.com/keksi2k/horizonmatica). [Litematica by masa and Sakura-Ryoko](https://github.com/sakura-ryoko/litematica) is the reference for schematics and workflows. The printer is based on the behavior of [Litematica Printer by aleksilassila](https://github.com/aleksilassila/litematica-printer). Horizon provides the native loader, world access, renderer, and block-state data fixer. The Rust implementation is distributed under the [MIT license](LICENSE).
