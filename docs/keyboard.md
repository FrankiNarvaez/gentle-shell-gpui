# Keyboard reference

Everything the app does from the keyboard. In the app, **⌘/** (Ctrl+Shift+/ on Windows and Linux; also ⌃Space `?`, the command palette's "Keyboard shortcuts" and the app menu) opens the same list as a searchable screen. That screen is generated from the live keymap, so it is always right; this page is the written version.

Notation: ⌘ is Cmd on macOS and Ctrl elsewhere unless a column says otherwise. Off macOS the global chords add Shift or Alt where a plain Ctrl+letter belongs to terminals (Ctrl+L clears the screen, Ctrl+D ends input).

## Three layers

1. **Direct chords**: one chord, one action. Always available.
2. **Leader** (⌃Space, then one key): short, mnemonic, no modifiers to hold. The tmux/herdr style.
3. **Pane mode** (⌃Space, `Space`): sticky, so you can move, resize and split with bare keys until Esc.

While a sequence is pending (after ⌃Space, or the first `g` of `gg`), a which-key panel at the bottom of the window lists the keys that can come next and what they do. It disappears when the sequence completes, when you press Esc or a key that continues nothing, or after about a second without a key. Pane mode shows its own keys in the same panel, and a PANE badge in the title bar.

## Direct chords

| Action | macOS | Windows / Linux |
| --- | --- | --- |
| New agent tab | ⌘T | Ctrl+T |
| New terminal tab | ⌘⇧T | Ctrl+Shift+T |
| New canvas tab | ⌘⇧E | Ctrl+Alt+Shift+E |
| Close pane (the last one closes the tab) | ⌘W | Ctrl+W |
| Close tab | ⌘⇧W | Ctrl+Shift+W |
| Reopen closed pane or tab | ⌥⌘T | Ctrl+Alt+Shift+T |
| Go to tab 1 … 8, 9 = last tab | ⌘1 … ⌘9 | Ctrl+1 … Ctrl+9 |
| Next / previous tab | ⌃Tab / ⌃⇧Tab | Ctrl+Tab / Ctrl+Shift+Tab |
| New pane (tiled) | ⌘D | Ctrl+Shift+D |
| Split down | ⌘⇧D | Ctrl+Shift+E |
| Next / previous pane | ⌘] / ⌘[ | Ctrl+] / Ctrl+[ |
| Focus the composer | ⌘L | Ctrl+Shift+L |
| Focus Files | ⌘⇧B | Ctrl+Shift+B |
| Focus Git changes | ⌘⇧G | Ctrl+Shift+G |
| Toggle the Files sidebar | ⌘B | Ctrl+B |
| Command palette | ⌘K | Ctrl+K |
| Workspaces | ⌃⌘K | Ctrl+Alt+K |
| Sessions | ⌘⇧O | Ctrl+Shift+O |
| Settings | ⌘, | Ctrl+, |
| **Keyboard shortcuts** | ⌘/ | Ctrl+Shift+/ |
| Zoom in / out / actual size | ⌘= ⌘- ⌘0 | Ctrl+= Ctrl+- Ctrl+0 |
| Quit | ⌘Q | Ctrl+Q |
| New session (in an agent pane) | ⌘N | Ctrl+N |
| Agents panel (in an agent pane) | ⌘⇧A | Ctrl+Shift+A |
| Stop the agent | ⌘. or Esc | Ctrl+. or Esc |

Esc closes the screen that is open; with none open, in an agent pane, it interrupts the turn.

## Leader: ⌃Space, then one key

⌃Space is the same on every platform. Press it, then one of:

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` | Focus the pane to the left / below / above / right (spatial) |
| `v` / `s` | Split right / split down |
| `x` | Close pane |
| `u` | Reopen the last closed pane or tab |
| `c` / `t` | New agent tab / new terminal tab |
| `n` / `p` | Next / previous tab |
| `1` … `9` | Go to tab N (9 = last tab) |
| `f` / `g` | Focus Files / focus Git changes |
| `i` | Focus the composer |
| `b` | Toggle the Files sidebar |
| `:` | Command palette |
| `?` | Keyboard shortcuts |
| `Space` | Enter pane mode |

## Pane mode

⌃Space then `Space` turns it on. It stays on until Esc or Enter, and while it is on every other key is swallowed, so you cannot type into a pane by accident. It also ends if a screen opens or the window loses focus.

| Key | Action |
| --- | --- |
| `h` `j` `k` `l` | Move focus left / down / up / right |
| `H` `J` `K` `L` (Shift) | Resize: move the focused pane's edge that way |
| `v` / `s` | Split right / split down |
| `x` | Close pane |
| `Esc` / `Enter` | Leave pane mode |

The palette also has Focus pane left/right/up/down, Split left/up and Resize pane left/right/up/down, each with its key.

## Files and Git changes

Click or focus a list (⌘⇧B for Files, ⌘⇧G for Changes, or ⌃Space `f` / `g`). Arrow keys and Enter always work. The letter keys below are **off by default**: turn on **Configuration → Appearance → Keyboard → Vim keys** (saved as `vimKeys` in the app's `settings.json`, effective at once). They never fire while a text field, such as the rename or commit box, has focus.

| Key | Files | Git changes | Needs Vim keys |
| --- | --- | --- | --- |
| ↑ ↓ | Previous / next | Previous / next | no |
| ← → | Collapse / expand | | no |
| ↵ | Open | Open the diff | no |
| Space | | Stage or unstage the selected file | no |
| `j` `k` | Next / previous | Next / previous | yes |
| `g g` / `G` | First / last | First / last | yes |
| `h` `l` | Collapse / expand | | yes |
| `o` | Open | | yes |
| `s` / `u` | | Stage / unstage | yes |
| `d` | | Open the diff | yes |

More Files keys (always on): open to the side ⌘↵, open in a new tab ⌥⌘↵, insert path in the composer ⌥↵, copy path ⌥⌘C, copy relative path ⌥⇧⌘C, reveal ⌥⌘R, rename F2, cut / copy / paste ⌘X / ⌘C / ⌘V, move to trash ⌘⌫ (Ctrl and Alt/Shift variants off macOS).

## Terminals

Terminal panes get every key that the app has no binding for. The app takes ⌃Space for the leader, so a terminal program never sees it: that is Ctrl+Space (NUL), which readline, emacs and tmux use for set-mark and as an alternative prefix. If you rely on it, run tmux with another prefix inside the app; the leader's key is configurable in `keymap.json` (see below).

The leader waits about a second for its second key.

## Coming from tmux / nvim

- **Leader = tmux prefix.** ⌃Space plays the part of `C-b`: press it, release, press one key. `v` and `s` split like `%` and `"` (right and down); `x` closes like `&`/`x`; `n` / `p` / `1`–`9` switch windows (tabs) like tmux's `n`, `p` and digits; `h j k l` move between panes like `select-pane` with vim keys; `c` creates; `:` is the command prompt, here the palette.
- **Pane mode = a resize/move submode**, like herdr's modes or tmux's repeatable keys: enter once, use bare keys, leave with Esc.
- **Vim keys = netrw/oil habits.** `j k gg G h l` in lists once the setting is on; `s` and `u` stage and unstage as in fugitive and lazygit.
- **`:` opens the palette**, which also takes `/` agent commands, so `⌃Space :` is your command line.
- **Discoverability**: press ⌃Space and wait; the which-key panel shows the rest, like which-key.nvim.

## Custom key bindings

Bindings and the leader can be changed in `keymap.json` (palette: **Open keymap file**, then **Reload keymap**). The format, action names, contexts and how to unbind a key are in [integrations: Keymap file](integrations.md#keymap-file-sharedkeymaprs-gentle_corekeymap). The cheat sheet and the palette hints show your bindings after a reload.
