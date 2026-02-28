# Win+Drag Approach Notes

## Goal
Add `LWIN + left-drag` window movement without:
- opening the Start menu after drag
- freezing or stalling mouse input
- breaking existing Win shortcuts (`Win+1..9`, `Win+Enter`, `Win+Q`)

## What We Learned

1. Doing expensive work directly in `WH_MOUSE_LL` is fragile.
- Earlier versions that performed move logic inline caused long input stalls.

2. Suppressing all move events (`WM_MOUSEMOVE -> return 1`) breaks cursor updates.
- This produced "cursor stuck / small move then snap back" behavior.

3. Stable behavior came from keeping hook callbacks minimal.
- Hook should only capture state and decide which button events to swallow.
- Movement should happen outside the low-level hook callback.

4. `LWIN` must be explicitly intercepted and replayed conditionally.
- If we let native `LWIN` pass through, Start behavior becomes timing-sensitive.
- Current approach intercepts `LWIN` keydown and on keyup:
  - replays a synthetic Win tap if no drag/combo happened
  - otherwise swallows it

5. Click suppression should apply to drag-captured button events, not cursor movement.
- Swallowing drag `LButtonDown`/`LButtonUp` prevents accidental click-through.
- Letting `WM_MOUSEMOVE` pass preserves cursor and system responsiveness.

## Final Implemented Architecture

1. Keyboard hook (`WH_KEYBOARD_LL`)
- Intercepts physical `LWIN` keydown.
- Tracks whether a Win combo was used.
- On `LWIN` keyup:
  - cancel drag state
  - if no drag/combo: replay synthetic `LWIN` tap
  - swallow original `LWIN` keyup

2. Mouse hook (`WH_MOUSE_LL`)
- `WM_LBUTTONDOWN`:
  - if `LWIN` and target window is draggable, start drag capture and swallow down event
- `WM_MOUSEMOVE`:
  - update drag state only, do **not** swallow move events
- `WM_LBUTTONUP`:
  - end drag; swallow up event if drag gesture was active

3. Drag state
- Atomic state for low-overhead cross-thread reads:
  - active/triggered flags
  - candidate window handle
  - drag start cursor position
  - latest cursor position
  - one-shot consume flag for `LWIN` keyup

4. Move worker thread
- Fixed cadence (`~16ms`) loop reads latest drag snapshot.
- Initializes move session (window rect) once per gesture.
- Applies movement with `MoveWindow` while drag is active.
- No per-mouse-event move syscalls in the hook.

## Why This Version Was Kept
- No reproducible mouse freeze in the final tested iteration.
- Start menu suppression is deterministic and tied to actual drag/combo behavior.
- Underlying app click actions are blocked during captured drags.
- Existing Win-based shortcuts continue to work.
