use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};

use windows_sys::Win32::Foundation::HWND;

use super::log;

const DRAG_THRESHOLD_SQ: i32 = 16;

static GESTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static DRAG_TRIGGERED: AtomicBool = AtomicBool::new(false);
static CONSUME_NEXT_LWIN_KEYUP: AtomicBool = AtomicBool::new(false);

static CANDIDATE_HWND: AtomicIsize = AtomicIsize::new(0);
static START_X: AtomicI32 = AtomicI32::new(0);
static START_Y: AtomicI32 = AtomicI32::new(0);
static CURSOR_X: AtomicI32 = AtomicI32::new(0);
static CURSOR_Y: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy)]
pub(crate) struct DragSnapshot {
	pub(crate) hwnd: HWND,
	pub(crate) start_x: i32,
	pub(crate) start_y: i32,
	pub(crate) cursor_x: i32,
	pub(crate) cursor_y: i32,
}

pub(crate) fn on_down(lwin_down: bool, candidate_window: Option<HWND>, x: i32, y: i32) {
	let candidate = candidate_window.map(|hwnd| hwnd as isize).unwrap_or(0);
	let active = lwin_down && candidate != 0;

	CANDIDATE_HWND.store(if active { candidate } else { 0 }, Ordering::Relaxed);
	START_X.store(x, Ordering::Relaxed);
	START_Y.store(y, Ordering::Relaxed);
	CURSOR_X.store(x, Ordering::Relaxed);
	CURSOR_Y.store(y, Ordering::Relaxed);
	DRAG_TRIGGERED.store(false, Ordering::Relaxed);
	CONSUME_NEXT_LWIN_KEYUP.store(false, Ordering::Relaxed);
	GESTURE_ACTIVE.store(active, Ordering::Relaxed);

	log::event(&format!(
		"drag down active={} lwin_down={} x={} y={} candidate={:#x}",
		active, lwin_down, x, y, candidate
	));
}

pub(crate) fn on_move(x: i32, y: i32) {
	if !GESTURE_ACTIVE.load(Ordering::Relaxed) {
		return;
	}

	CURSOR_X.store(x, Ordering::Relaxed);
	CURSOR_Y.store(y, Ordering::Relaxed);

	if DRAG_TRIGGERED.load(Ordering::Relaxed) {
		return;
	}

	let dx = x - START_X.load(Ordering::Relaxed);
	let dy = y - START_Y.load(Ordering::Relaxed);
	if dx.saturating_mul(dx) + dy.saturating_mul(dy) < DRAG_THRESHOLD_SQ {
		return;
	}

	DRAG_TRIGGERED.store(true, Ordering::Relaxed);
	CONSUME_NEXT_LWIN_KEYUP.store(true, Ordering::Relaxed);
	let hwnd = CANDIDATE_HWND.load(Ordering::Relaxed);
	log::event(&format!(
		"drag trigger x={} y={} hwnd={:#x}",
		x, y, hwnd
	));
}

pub(crate) fn on_up() {
	GESTURE_ACTIVE.store(false, Ordering::Relaxed);
	DRAG_TRIGGERED.store(false, Ordering::Relaxed);
	CANDIDATE_HWND.store(0, Ordering::Relaxed);
	log::event("drag up");
}

pub(crate) fn on_cancel() {
	GESTURE_ACTIVE.store(false, Ordering::Relaxed);
	DRAG_TRIGGERED.store(false, Ordering::Relaxed);
	CANDIDATE_HWND.store(0, Ordering::Relaxed);
	log::event("drag cancel");
}

pub(crate) fn gesture_active() -> bool {
	GESTURE_ACTIVE.load(Ordering::Relaxed)
}

pub(crate) fn snapshot_for_worker() -> Option<DragSnapshot> {
	if !GESTURE_ACTIVE.load(Ordering::Relaxed) || !DRAG_TRIGGERED.load(Ordering::Relaxed) {
		return None;
	}
	let hwnd = CANDIDATE_HWND.load(Ordering::Relaxed);
	if hwnd == 0 {
		return None;
	}
	Some(DragSnapshot {
		hwnd: hwnd as HWND,
		start_x: START_X.load(Ordering::Relaxed),
		start_y: START_Y.load(Ordering::Relaxed),
		cursor_x: CURSOR_X.load(Ordering::Relaxed),
		cursor_y: CURSOR_Y.load(Ordering::Relaxed),
	})
}

pub(crate) fn take_consume_next_lwin_keyup() -> bool {
	let consume = CONSUME_NEXT_LWIN_KEYUP.swap(false, Ordering::Relaxed);
	log::event(&format!("drag consume_lwin_keyup={}", consume));
	consume
}
