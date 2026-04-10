# oxmap

Terminal mind map and flowchart editor with Mermaid `.mmd` export.

## Commands

| Key / Command | Action |
| --- | --- |
| `a` | Add a note and start editing it |
| `f` then `<key>` | Select a note by key |
| `i` | Edit the selected note inline |
| `o` | Open the selected note in your configured editor |
| `x` | Delete the selected note |
| `m` `f` `<key>` | Create a relation from the selected note |
| `u` `f` `<key>` | Remove the relation from the selected note to the target |
| `h` `j` `k` `l` | Move selected note, or pan when nothing is selected |
| `<count>` then motion | Repeat a move, e.g. `8l` |
| `s` / `d` | Zoom in / out |
| `G` | Fit all notes on screen |
| `Esc` | Cancel pending mode or clear selection |
| `:` | Enter command mode |
| `:w` | Save to `.mmd` |
| `:wq` | Save and quit |
| `:q` | Quit if clean |
| `:q!` | Force quit |
| `:export` | Write a Mermaid `.mmd` export |
| `:editor <cmd>` | Change editor command and save it to config |

## Config

Config lives at `~/.config/oxmap/config.json`.

```json
{
  "movement_step": 6,
  "editor": "~/nvim-macos-arm64/bin/nvim"
}
```
