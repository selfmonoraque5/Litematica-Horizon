# Display tab

[Back to the overview](Home.md)

## World overlay

| Control | Default | What it does | Command |
|---|---|---|---|
| **Rendering** | On | Draws the overlay at all | `/lm render [on\|off]`, key **V** |
| **See through blocks** | Off | Draws the overlay through blocks in the world | `/lm through [on\|off]` |
| **Mode** | Ghost blocks | Translucent real block models, or simple boxes | `/lm mode ghost\|boxes` |
| **Ghost opacity** | 55% | How solid the ghost blocks look, from 10% to 100% | — |
| **Render radius** | 32 blocks | How far around you the overlay is built. The slider goes from 16 to 1024 | `/lm radius <blocks>`, with no upper limit |
| **Radius = view distance** | Off | Uses your view distance instead of the radius | `/lm radius view` |
| **Face limit** | 300k | The most faces the overlay keeps: 300k, 1 M, 2 M, 5 M, 10 M or no limit | `/lm faces <n>\|none`, such as `2m` or `2.5m` |
| **Hide water and lava** | Off | Water and lava are whole cubes as ghosts, so they can get in the way | `/lm fluids off` hides them, `on` shows them |
| **Boxes around placements** | On | The frames around placements | — |
| **Show extra blocks** | On | Marks blocks in the world that the schematic doesn't have | — |
| **Show wrong blocks** | On | Marks blocks that differ from the schematic | — |
| **Highlight missing blocks** | Off | Marks missing blocks in blue through everything, however far away | `/lm highlight missing [on\|off]` |
| **Highlight wrong blocks** | Off | Marks wrong blocks in red and wrong states in orange the same way | `/lm highlight wrong [on\|off]` |
| **Strict states** | Off | Fence, pane, wall and redstone connections and stair shapes also count as wrong | — |
| **Redstone on/off counts as wrong** | On | Powered, lit and similar states count as wrong | — |

## HUD

| Control | Default | What it does | Command |
|---|---|---|---|
| **Info HUD** | On | The info lines while playing | — |
| **Material list HUD** | Off | The material list in the corner while playing | `/lm materials [on\|off]`, key **J** |
| **Copy the material list** | | Copies it to the clipboard as a table. See [Materials](Materials.md) | `/lm materials copy` |

## What the overlay costs

The bottom of the tab shows how many faces the overlay holds, in how many sections, and about how much video memory
they take. **FULL** means the face limit is reached, so the farthest parts aren't built.

A face takes about 180 bytes of video memory:

| Faces | Video memory |
|---|---|
| 300k | about 50 MB |
| 1 M | about 180 MB |
| 2 M | about 350 MB |
| 5 M | about 880 MB |
| 10 M | about 1.8 GB |

The overlay is built nearest first, so a limit that is too low shows up as a missing far end.
