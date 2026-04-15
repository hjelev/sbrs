---
description: "Scaffold a new keyboard shortcut in sbrs (Shell Buddy / sb). Provide the key and what it should do."
argument-hint: "key=<key> action=<description of what it does>"
agent: agent
---

Add a new keybinding to `src/main.rs` in sbrs (Shell Buddy / sb).

## Inputs
The user has specified:
- **Key**: the key or key combination to bind (e.g. `d`, `F3`, `Ctrl+r`)
- **Action**: what the keybinding should do (e.g. "delete the selected file with confirmation")

## Steps

1. **Identify the correct Mode branch** in the event loop. Most file-operation keys belong under `Mode::Browsing`. Only add to `Mode::Renaming` if the key modifies text input behavior.

2. **Add the key match arm** following the existing pattern:
   ```rust
   KeyCode::Char('x') => {
       // implementation
   }
   ```
   For function keys: `KeyCode::F(3)`. For ctrl combos: guard with `event.modifiers.contains(KeyModifiers::CONTROL)`.

3. **Add any required App state** — if the action needs new state (e.g. a confirmation flag, a new mode), add a field to the `App` struct and initialize it in `App::new()`.

4. **Add a new Mode variant if needed** — for actions that require a secondary UI state (confirmation dialog, text entry), add a variant to the `Mode` enum and handle rendering and key events for it.

5. **Update the Help overlay** — find the help text rendered under `Mode::Help` and add a line documenting the new key and its action.

6. **File operation conventions** — silently ignore errors (use `let _ = ...` or `if let Ok(...)`). Do not add `unwrap()` or `expect()` on file I/O.

7. **Raw terminal safety** — if the action spawns an external process (like `less` or `$EDITOR`), follow the existing pattern: disable raw mode + leave alternate screen before spawn, restore after.

8. Run `cargo check` to confirm the code compiles.

## Output
Show the diff of changes made and confirm the help text was updated.
