use std::sync::Mutex;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::{
	Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
	UI::{
		Input::KeyboardAndMouse::VK_LWIN,
		WindowsAndMessaging::{
			CallNextHookEx, GetWindowRect, MoveWindow, SetWindowsHookExW,
			UnhookWindowsHookEx, HC_ACTION, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONDOWN,
			WM_LBUTTONUP, WM_MOUSEMOVE,
		},
	},
};

use super::desktop::draggable_window_from_point;
use super::drag;
use super::key_is_down;
use crate::logging::{log_error, log_info};

static MOUSE_HOOK: Mutex<isize> = Mutex::new(0);
static MOVE_WORKER_STARTED: OnceLock<()> = OnceLock::new();
const APPLY_MOVEWINDOW: bool = true;

struct MoveSession {
	hwnd: isize,
	start_cursor_x: i32,
	start_cursor_y: i32,
	origin_left: i32,
	origin_top: i32,
	width: i32,
	height: i32,
}

pub(crate) fn bind_mouse_hook() {
	start_move_worker();
	unsafe {
		if let Ok(mut hook_guard) = MOUSE_HOOK.lock() {
			if *hook_guard == 0 {
				let hook = SetWindowsHookExW(
					WH_MOUSE_LL,
					Some(low_level_mouse_proc),
					std::ptr::null_mut() as HINSTANCE,
					0,
				);
				if hook.is_null() {
					log_error(
						"keys/mouse",
						&format!(
							"failed to install mouse hook: win32 error {}",
							GetLastError()
						),
					);
				} else {
					*hook_guard = hook as isize;
					log_info("keys/mouse", "mouse hook installed");
				}
			}
		}
	}
}

fn start_move_worker() {
	if MOVE_WORKER_STARTED.set(()).is_err() {
		return;
	}
	thread::spawn(move_worker_loop);
}

fn move_worker_loop() {
	let mut session: Option<MoveSession> = None;
	loop {
		thread::sleep(Duration::from_millis(16));
		let snapshot = match drag::snapshot_for_worker() {
			Some(snapshot) => snapshot,
			None => {
				session = None;
				continue;
			}
		};

		let needs_new_session = session.as_ref().is_none_or(|active| {
			active.hwnd != snapshot.hwnd as isize
				|| active.start_cursor_x != snapshot.start_x
				|| active.start_cursor_y != snapshot.start_y
		});

		if needs_new_session {
			let mut rect = RECT {
				left: 0,
				top: 0,
				right: 0,
				bottom: 0,
			};
			unsafe {
				if GetWindowRect(snapshot.hwnd, &mut rect) == 0 {
					session = None;
					continue;
				}
			}
			session = Some(MoveSession {
				hwnd: snapshot.hwnd as isize,
				start_cursor_x: snapshot.start_x,
				start_cursor_y: snapshot.start_y,
				origin_left: rect.left,
				origin_top: rect.top,
				width: rect.right - rect.left,
				height: rect.bottom - rect.top,
			});
		}

		let active = match &session {
			Some(active) => active,
			None => continue,
		};

		let new_x = active.origin_left + (snapshot.cursor_x - active.start_cursor_x);
		let new_y = active.origin_top + (snapshot.cursor_y - active.start_cursor_y);
		if !APPLY_MOVEWINDOW {
			continue;
		}
		unsafe {
			let ok = MoveWindow(
				active.hwnd as HWND,
				new_x,
				new_y,
				active.width,
				active.height,
				0,
			);
			let _ = ok;
		}
	}
}

pub(crate) fn unbind_mouse_hook() {
	unsafe {
		if let Ok(mut hook_guard) = MOUSE_HOOK.lock() {
			if *hook_guard != 0 {
				let _ = UnhookWindowsHookEx(*hook_guard as _);
				*hook_guard = 0;
			}
		}
	}
}

unsafe extern "system" fn low_level_mouse_proc(
	ncode: i32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	if ncode != HC_ACTION as i32 {
		return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
	}

	let mouse = &*(lparam as *const MSLLHOOKSTRUCT);
	let message = wparam as u32;
	let x = mouse.pt.x;
	let y = mouse.pt.y;

	match message {
		WM_LBUTTONDOWN => {
			let lwin_down = key_is_down(VK_LWIN as u32);
			let target_window = if lwin_down {
				draggable_window_from_point(x, y)
			} else {
				None
			};
			drag::on_down(lwin_down, target_window, x, y);
			if lwin_down && target_window.is_some() {
				return 1;
			}
		}
		WM_MOUSEMOVE => {
			if !drag::gesture_active() {
				return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
			}
			drag::on_move(x, y);
		}
		WM_LBUTTONUP => {
			let was_drag_gesture = drag::gesture_active();
			drag::on_up();
			if was_drag_gesture {
				return 1;
			}
		}
		_ => {}
	}

	CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}
