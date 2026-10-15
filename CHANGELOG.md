# Changelog

## Unreleased

- Free pet positioning with corner anchoring only on layout changes and off-screen recovery.
- Panel stays above the pet while open; the pet regains topmost after it closes.
- Deferred native window moves so dragging across monitors no longer crashes.
- Reminder windows show the pet with entrance and idle motion, plus a bundled chime.
- New "reminder sound" preference toggle, persisted with backward-compatible defaults.
- Focus rounds linked to a task check it off when they finish naturally.
- Smoother pet animation sampling at 40 ms ticks.

## 0.2.1

- Inline task editing with separate drafts, Enter to save and Escape to cancel.
- Clear hover/press feedback and short acknowledgement-driven task animations.
- Quick add with an explicit HH:mm prefix; direct add, complete and pause controls.
- Preserve failed or unconfirmed drafts and avoid replaying animation on polling.

## 0.2.0

- Persistent daily to-dos, priorities, deadlines, current task and carry-over.
- Attached desktop note with three visible tasks, completion undo and focus actions.
- Office panel with task editing, settings and statistics tabs.
- Backward-compatible atomic local storage and Windows widget layout hardening.

## 0.1.0

- Original nine-frame PNG desktop pet with keyboard and mouse activity animation.
- Focus and break timer, local daily statistics, pause and restart recovery.
- Light/dark/system themes and adjustable pet size.
- System tray controls and completion reminders.
- Atomic local saves, write barriers and command acknowledgements.
