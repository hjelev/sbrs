---
name: tui-widget
description: "Add or modify ratatui TUI widgets and layouts in sbrs (Shell Buddy / sb). Use when adding new UI panels, dialogs, popup overlays, table columns, or status bar sections. Covers ratatui 0.26 patterns, crossterm event handling, Color::Rgb usage, and rendering conventions specific to this codebase."
argument-hint: "Describe the widget or UI element to add or change"
---

# TUI Widget Patterns for sbrs

## When to Use
- Adding a new panel, dialog, popup, or overlay
- Modifying table columns or column widths
- Adding a new section to the status bar or header
- Implementing an input prompt inside the TUI (text entry modes)
- Changing colors, styles, or highlight behavior

## Key Rules

### Colors
Always use `ratatui::style::Color::Rgb(r, g, b)`. Never use raw ANSI escape literals or hardcoded terminal codes.

```rust
Style::default().fg(Color::Rgb(100, 200, 100))
```

### Layout
The render function receives a `Frame` and splits with `Layout::default()`. Match the existing constraint pattern:

```rust
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(3),   // header
        Constraint::Min(0),      // main content
        Constraint::Length(1),   // status bar
    ])
    .split(frame.size());
```

### Table Widget (main file list)
Column widths are hardcoded constraints — update all four together if changing layout:
- Name: `Constraint::Percentage(40)`
- Size: `Constraint::Length(12)`
- Permissions: `Constraint::Length(8)`
- Modified: `Constraint::Min(20)`

### Popup / Overlay Pattern
Render popups after the main content so they paint on top. Use a centered `Rect` helper:

```rust
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
```

Then clear and render:

```rust
frame.render_widget(Clear, popup_area);
frame.render_widget(block, popup_area);
```

### Input Buffer Mode
New text-entry modes follow the `Mode` enum pattern:

1. Add a variant to `Mode` (e.g., `Mode::Searching`)
2. Handle character keys in the event loop under that mode variant
3. Clear `app.input_buffer` on mode entry; read it on confirm (`Enter`)
4. Render the input buffer in a `Paragraph` widget inside a bordered `Block`

```rust
let input = Paragraph::new(app.input_buffer.as_str())
    .block(Block::default().borders(Borders::ALL).title("Search"));
frame.render_widget(input, area);
```

## Raw Terminal Safety
Any new code path that can panic or return early must restore terminal state:

```rust
crossterm::terminal::disable_raw_mode()?;
execute!(stdout(), LeaveAlternateScreen)?;
```

Prefer wrapping risky operations in a closure or guard struct so cleanup is guaranteed.

## Procedure for Adding a New Widget
1. Decide which `App` fields need to change (add to the `App` struct if new state is required)
2. Update `refresh_entries()` if the widget depends on directory state
3. Add a layout chunk or popup area in the render function
4. Render the widget using `frame.render_widget(...)` or `frame.render_stateful_widget(...)`
5. Handle relevant key events in the event loop under the correct `Mode` branch
6. Run `cargo check` to verify no compile errors before testing interactively
