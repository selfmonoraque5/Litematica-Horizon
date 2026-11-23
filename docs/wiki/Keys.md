# Keys

[Back to the overview](Home.md)

Every action is one of the game's key mappings: bind it in **Options › Controls**, like the game's own keys. (Up to
0.3.3 this was the menu's Keys tab; keys bound there are not carried over to 0.4.0.)

The defaults:

| Key | Action | Command |
|---|---|---|
| **M** | Opens the menu | — |
| **V** | Rendering on or off | `/lm render` |
| **Page Up** / **Page Down** | Layer up / down | `/lm layer up`, `/lm layer down` |
| **Home** | Next layer mode: all, this layer, above, below | — |
| **K** | Printer on or off | `/lm printer` |
| **J** | Material list in the HUD on or off | `/lm materials` |
| **N** | Move mode on or off | — |
| **Middle mouse button** | Takes the block the schematic wants where you look, see below | — |

No default key: selection tool, easy place, reload, rotate, mirror, move to me, show or hide, select next or previous
placement, unload, area corners and clear, copy material list, open schematics folder. Bind them in Options › Controls.

## In the menu

| Key | Action |
|---|---|
| **1** to **7** | Switch tabs (while no edit box is being typed in) |
| **M**, **Esc** | Close the menu |

Things that need a file or a name, such as loading a schematic or `/lm save <name>`, have no key. In the menu itself
**M** and **1** to **7** keep their meaning whatever is bound.

## Move mode

Only in this mode do the mouse and keys move the selected placement, so nothing gets moved by accident. Turn it on with
**N** or the **Move mode** button in the [Placement tab](Placement.md). A locked placement can't enter it.

| Input | Effect |
|---|---|
| Mouse wheel | Along your view (the closest axis, up and down too) |
| Arrow keys | Forward, back, left, right, relative to your view |
| Page Up / Page Down | Up / down, instead of changing layers |
| R | Rotate clockwise |
| Home | Move it to your position |
| Shift held | 5 blocks instead of 1 |
| N or Enter | Done |

The printer pauses while move mode is on.

## Picking a block from the schematic

Middle-click a ghost block to take that block, as the game's own pick block does: from the hotbar, from your inventory
into the hotbar, or in creative out of nothing. This makes building by hand easier. It works within your normal reach,
on any spot that is still missing or wrong, in the layers shown.

Everywhere else the middle button stays the game's own pick block, also with rendering off (**V**). With the selection
tool in hand, the middle button belongs to the tool.

## Selection tool

The mouse controls are listed in the [Tool tab](Tool.md).
