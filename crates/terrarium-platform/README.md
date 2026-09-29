# terrarium-platform

Operating-system integration that needs FFI.

## Owns

- Windows: the `AppBar` registration, window subclassing and fullscreen detection.
- macOS: the window level, collection behaviour and activation policy.
- Idle time, the login item, and power and display notifications.

## Why it is a crate of its own

FFI is reviewed line by line, so all of it lives in one small crate (I-9).

## Rules

- It is the only crate allowed `unsafe`, and every `unsafe` block carries a `SAFETY:` comment
  (`clippy::undocumented_unsafe_blocks` is denied).
- It depends on no other workspace crate.

## Requirements

Invariant I-9.
