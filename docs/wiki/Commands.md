# Commands

[Back to the overview](Home.md)

All commands start with `/litematica` or `/lm`. `/lm help` lists them in game. A switch without `on` or `off`
toggles.

The chat completes the commands: type `/lm ` and it offers the subcommands, narrowing them down as you type. After
`load` it offers your schematic files, and after `select` and `unload` your placements. **Tab** takes a suggestion.

## Files

| Command | Effect | In the menu |
|---|---|---|
| `/lm list` | Lists the schematics in the folder | [Load](Load.md): file list |
| `/lm load <name>` | Loads one as another placement at your position and selects it | Load: **Load here** |
| `/lm reload` | Reads the last schematic again from its file, where it was | Load: **Reload last** |
| `/lm folder` | Prints the schematics folder's path | Load: **Open folder** opens it |

## Placements

| Command | Effect | In the menu |
|---|---|---|
| `/lm placements` | Lists the placements; `>` marks the selected one | [Placement](Placement.md): list |
| `/lm select <n>\|<name>\|next\|prev` | Selects a placement | Placement: list row |
| `/lm show [on\|off]`, `/lm hide` | Shows or hides the selected placement | Placement: **Show** / **Hide** |
| `/lm lock [on\|off]`, `/lm unlock` | Locks it where it is | Placement: **Lock** / **Unlock** |
| `/lm unload [<n>\|all]` | Unloads the selected one, another one, or all | Placement: **Unload**, **Unload all** |
| `/lm origin here` | Moves it to your position | Placement: **Move to me** |
| `/lm origin <x> <y> <z>` | Moves it to a position | Placement: **Origin** and **Set** |
| `/lm move <dx> <dy> <dz>` | Moves it by a distance | Placement: **-X +X -Y +Y -Z +Z** |
| `/lm rotate cw\|ccw\|180\|none` | Turns it | Placement: **Rotate left**, **Rotate right** |
| `/lm mirror none\|x\|z` | Mirrors it | Placement: **Mirror** |
| `/lm info` | Prints position, progress and errors | Placement: top lines |

## Layers

| Command | Effect | In the menu |
|---|---|---|
| `/lm layer all` | Shows all layers | Placement: **All layers** |
| `/lm layer up`, `/lm layer down` | Moves the shown layer | Placement: **Layer up**, **Layer down** |
| `/lm layer axis x\|y\|z` | Selects the world axis | — |
| `/lm layer <coordinate>`, `/lm layer here` | Shows one layer at the given coordinate or your position | — |
| `/lm layer above <coordinate>`, `/lm layer below <coordinate>` | Includes coordinates greater or smaller than the given endpoint, respectively | — |
| `/lm layer range <a> <b>` | Shows an inclusive range | — |

## Area selection and saving

| Command | Effect | In the menu |
|---|---|---|
| `/lm pos1`, `/lm pos2` | Sets a corner on the block you look at | [Save](Save.md): **Look** |
| `/lm pos1 here`, `/lm pos2 here` | Sets a corner where you stand | Save: **Here** |
| `/lm pos1 <x> <y> <z>`, `/lm pos2 <x> <y> <z>` | Sets a corner at a position | Save: fields and **Set** |
| `/lm area` | Prints the selection's corners and size | Save: shown in the tab |
| `/lm area clear` | Clears the selection | Save: **Clear selection** |
| `/lm save <name> [overwrite]` | Saves the area as `<name>.litematic` | Save: **Save**, **Overwrite** |
| `/lm save status` | Prints how far the save has got | Save: progress bar |
| `/lm save cancel` | Stops the save | Save: **Cancel** |

## Selection tool

| Command | Effect | In the menu |
|---|---|---|
| `/lm tool [on\|off]` | Turns the tool on or off. Without a word it only tells its state | [Tool](Tool.md): **Selection tool** |
| `/lm tool <item>` | Makes an item the tool and turns it on | Tool: item icons, **item id** |
| `/lm tool mode [area\|placement]` | What it works on. Without a word it switches, and it also turns the tool on | Tool: **Tool works on** |

## Display

| Command | Effect | In the menu |
|---|---|---|
| `/lm render [on\|off]` | Draws the overlay or not | [Display](Display.md): **Rendering** |
| `/lm through [on\|off]` | Draws it through blocks | Display: **See through blocks** |
| `/lm highlight missing\|wrong [on\|off]` | Marks missing or wrong blocks through everything | Display or [Materials](Materials.md#highlighting-blocks) |
| `/lm highlight item <item>\|all` | Marks only that item, or all again | Materials: click a row |
| `/lm highlight off` | Switches the highlights off | — |
| `/lm mode [ghost\|boxes]` | Ghost blocks or boxes. Without a word it switches | Display: **Mode** |
| `/lm radius <blocks>\|view` | How far the overlay reaches | Display: **Render radius**, **Radius = view distance** |
| `/lm faces <n>\|none` | The most faces it keeps, such as `300k` or `2m` | Display: **Face limit** |
| `/lm fluids [on\|off]` | Shows or hides water and lava | Display: **Hide water and lava** |

## Printer

| Command | Effect | In the menu |
|---|---|---|
| `/lm printer [on\|off]` | Starts or stops the printer | [Printer](Printer.md): **Printer** |
| `/lm printer speed <ticks>` | One block every 1 to 20 ticks | Printer: **One block every** |
| `/lm printer reach <blocks>` | Reach, from 1 to 6 blocks | Printer: **Reach** |
| `/lm printer air [on\|off]` | Placing without a neighbouring block | Printer: **Air place** |

## Easy place

| Command | Effect | In the menu |
|---|---|---|
| `/lm easyplace [on\|off]` | Easy place on or off | [Placement](Placement.md#easy-place): **Easy place** |
| `/lm easyplace every <ticks>` | One block every 1 to 10 ticks while the button is held | Placement: **Easy place every** |
| `/lm easyplace air [on\|off]` | Ghost blocks without a neighbouring block | Placement: **Easy air place** |

## Other

| Command | Effect | In the menu |
|---|---|---|
| `/lm materials [on\|off]` | The material list in the HUD | [Materials](Materials.md): **HUD list** |
| `/lm materials copy` | Copies the material list to the clipboard as a table | Materials: **Copy** |
| `/lm credits` | Prints who made the originals | **Credits** button: **Show the credits in chat** |
| `/lm help` | Lists the commands | — |
