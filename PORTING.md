# Blazematica port plan

## Target and source

- Target the Horizon Rust client for Minecraft Java Edition 26.3. Use Horizon’s native `.hmod` loader and pin the client and mixin revisions in the mod manifest.
- Use Sakura-Ryoko’s [Litematica for 26.3](https://github.com/sakura-ryoko/litematica/tree/5f1e405a00c661c04107b3d9b9cff809103935a1) as the reference for the file format and user workflows.
- Start from the [MIT-licensed Horizonmatica implementation](https://github.com/keksi2k/horizonmatica/tree/1b0fa918fa7206c4488809d6acb22d100c32c613). Keep its history and attribution. The Rust mod uses Horizon’s renderer, world, input and block registry directly. Java source and dependencies are unnecessary for this implementation.

## Implementation order

1. Bring up the native base under its own name and mod ID in a new public repository. Verify the manifest, resource paths, commands, and package name agree.
2. Check `.litematic` compatibility against the 26.3 writer, including palette packing, signed region dimensions, transforms, and tile data. Exercise the import, preview, move, verify, material count, and save workflows using synthetic schematics. Bound file and decompression work before handing data to the native renderer.
3. Build the pinned native target and smoke-test it in an isolated Horizon instance. Record the exact build and which in-game paths were exercised.
4. Track remaining Java behavior differences with reproducible acceptance cases. Priorities are editable subregions and multi-region capture, entity and block-entity export, and Easy Place server protocol compatibility. Verify format interoperability separately from UI and server behavior.
