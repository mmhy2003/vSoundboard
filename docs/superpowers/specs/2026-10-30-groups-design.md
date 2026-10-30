# vSoundboard Groups: Design

Date: 2026-10-30
Status: Draft, awaiting approval
Builds on: `docs/superpowers/specs/2026-10-26-vsoundboard-design.md` (the app).

## Intent

The owner uses vSoundboard in different situations: livestreaming, gaming, and general fooling around. Today there is one board, so every sound and every shortcut is mixed together. The owner wants **groups**, with one board per situation, such as "Livestream", "Gaming" and "Misc". Each group shows only its own sounds.

A group is a complete, separate board: its own pads in its own order, with its own shortcuts. You pick the group for the situation before you start, in the vSoundboard window, and then play from it.

## Goals

- The user can create, rename and delete groups.
- **Selecting a group swaps the whole board** to that group's pads.
- **Only the selected group's pad shortcuts are live.** Pressing a shortcut that belongs to a pad in another group does nothing.
- **Shortcuts can repeat across groups.** Ctrl+1 can play one sound in Gaming and a different one in Livestream.
- Existing users lose nothing when they update: all their current pads, shortcuts, volumes and order end up in the first group.

## Non-goals

- **Switching groups from outside the window:** no shortcut, tray menu entry or in-game switching. The owner said: "i don't want to switch" while in a game. Switching happens only by clicking a tab in the window.
- **Sharing one pad between groups.** To have an airhorn in two groups, the user adds it to each group separately. Each copy has its own name, shortcut and volume.
- **Moving or copying a pad between groups.** To fix a sound added to the wrong group, the user removes it and adds it again in the right group (the owner's choice).
- **Reordering tabs.** Tabs stay in the order they were created.
- **Per-group devices, volumes or Go Live mode.** Those stay global.

## Anti-goals

- **An update that empties someone's board** or loses shortcuts, volumes or pad order.
- **A shortcut from a hidden group firing.** That would be a sound the user can't see and didn't expect.
- **Switching tabs cuts off the sound that's playing.** Only a new sound, re-pressing the playing pad, or Stop does that.

## Behaviour

### The tab row (decided by the owner: mockup option A)

A row of tabs sits between the top bar (+ Add sound, 📌 Always on top, ⚙ Devices) and the pad grid. It has one tab per group, in creation order, followed by a **+** tab.

- **Clicking a tab** selects that group. Its pads fill the grid and its shortcuts become the live ones.
- **+** asks for a name (a small dialog like the existing Rename dialog), creates an empty group, and selects it.
- **Right-clicking a tab** opens Rename and Delete.
- The selected tab is clearly highlighted.

### Groups

- **Names** are trimmed, can't be empty, and must be unique, ignoring case. A clash or an empty name keeps the dialog open with a short message.
- **Delete** asks first: "Delete “Gaming” and its 8 sounds? The audio files stay on your PC." If the group is empty, it deletes without asking.
  - Deleting removes the group and its pads (never the audio files).
  - If one of its pads is playing, that sound stops.
  - The selection moves to a neighbouring tab.
- **The last group can't be deleted.** Its Delete entry is disabled, so there is always a board.
- **Remembered between restarts:** the groups, their order, and which group is selected. That includes starting hidden with Windows, where the remembered group's shortcuts are the live ones.

### Pads within a group

Everything that works on the board today works the same way within the selected group:

- **Adding sounds** (drag files onto the window, or + Add sound) adds them to the selected group.
- **Drag-to-reorder** works within the group. The "drop in empty space moves to end" rule means the end of this group.
- **Right-click** (Rename, Set shortcut…, Volume, Remove) is unchanged.
- **Empty group:** it shows the same "Drop MP3, WAV, OGG or FLAC files here…" message as an empty board does today.

### Shortcuts

- **Live shortcuts** are the selected group's pad shortcuts, plus the global Stop and Go Live shortcuts. Switching tabs re-registers the shortcuts.
- **A pad shortcut is checked for conflicts** against:
  - other pads **in the same group**;
  - the Stop and Go Live shortcuts.

  It is not checked against pads in other groups.
- **Setting the Stop or Go Live shortcut** is checked against pads in **every** group, because Stop and Go Live are live in every group. The message names the pad and its group, for example "Ctrl+1 is already used by “Airhorn” in Gaming."
- **If Windows refuses a shortcut because another app holds it,** the existing notice appears ("… is already used by another app."). It appears whenever that group's shortcuts are registered, including after switching tabs.

### Playback across a switch

- **A sound that is playing keeps playing** when the user switches tabs.
  - The bottom bar still shows its name and time.
  - ■ Stop and the Stop shortcut still stop it.
  - Its own pad shortcut is no longer live, so pressing it does nothing, as for any other group's pad.
- **Playing a pad in the new group** cuts the old sound off, as today.

## Settings file and migration

- `%APPDATA%\vSoundboard\config.json` gains:
  - a list of groups, each with a name and its own ordered pads;
  - which group is selected.
- **Pad ids stay unique across all groups.** The engine, playing-state and error maps already key on pad id.
- **Migration** happens on first start after the update, when an old file has a top-level `pads` list and no groups:
  - all existing pads, in order, move into one group named **"Main"**, which is selected;
  - nothing else changes.
- **A fresh install** starts with one empty group named "Main".
- **Back up before the first migration.** The first time the app migrates an old file, it copies it to `config.json.pre-groups` before saving. `main.rs` saves right after loading, so the backup has to be written inside or before `config::load`. Older versions don't know about groups: going back to one would show an empty board, and saving there could drop the groups. The backup is the way back, and the release notes should mention it. *(My call.)*
- **Broken group data is repaired on load:**
  - zero groups gets one empty "Main" group;
  - a selected group that doesn't exist selects the first group;
  - duplicate pad ids are given new ids;
  - duplicate or empty names get " 2", " 3"… or "Group N".

  This matches the existing "normalize on load" approach. *(My call.)*

## Constraints

- **Build on what's there:**
  - Rust, eframe/egui 0.37 and serde config;
  - `HotkeyRegistry::sync(&Config)`, which re-registers from `hotkeys::bindings(cfg)`;
  - `find_conflict` / `conflict_message`;
  - `Config::add_pad` / `remove_pad` / `move_pad`;
  - the pad drag-and-drop in `src/ui/board.rs`;
  - the Rename modal pattern.
- **The engine can still play any pad.** The engine's pad table is currently all pads (`engine_pads`). It must still include the pad that is playing after a switch, so the simplest route is to keep sending all pads from all groups. *(My call.)*
- **Follow the project's working rules:**
  - TDD, where each change starts from a failing test;
  - `make check` passes (fmt, clippy, tests);
  - work on `develop`;
  - a new feature means a **minor** version bump at release (v0.5.0), per CLAUDE.md.
- **Update the README:** the Features and Using it sections, and a screenshot showing the tabs.

## Left to the builder

- The exact config field names and serde layout, as long as old files migrate and the result round-trips.
- Tab widget details (egui `selectable_label`s in a horizontal scroll area are fine) and how many tabs fit before it scrolls.
- Exactly which neighbouring tab is selected after a delete.
- Where the group helpers live (`Config` methods vs a small `groups` module).
- How tests reach the migration. One option is a new v1 `config.json` fixture under `tests/fixtures/`, which currently only holds a GitHub release response.
