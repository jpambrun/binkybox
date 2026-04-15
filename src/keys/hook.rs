use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

use windows_sys::Win32::{
	Foundation::{GetLastError, HINSTANCE, LPARAM, LRESULT, WPARAM},
	UI::{
		Input::KeyboardAndMouse::{
			keybd_event, GetAsyncKeyState, KEYEVENTF_KEYUP, VK_DOWN, VK_LCONTROL,
			VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_OEM_3, VK_Q, VK_RCONTROL,
			VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_UP,
		},
		WindowsAndMessaging::{
			CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
			HC_ACTION, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
			WM_SYSKEYDOWN, WM_SYSKEYUP,
		},
	},
};

use super::actions::{dispatch_action, Action};
use super::drag;
use super::mouse_hook;
use super::snap::SnapDirection;
use super::{key_is_down, KEYDOWN_STATE};
use crate::logging::{log_error, log_info};

static KEYBOARD_HOOK: Mutex<isize> = Mutex::new(0);
static KEYDOWN_TICKS_MS: Mutex<[u64; 256]> = Mutex::new([0; 256]);
static INTERCEPTED_WIN_KEY: AtomicU32 = AtomicU32::new(0);
static WIN_COMBO_USED: AtomicBool = AtomicBool::new(false);
static WIN_NATIVE_PASSTHROUGH: AtomicBool = AtomicBool::new(false);
const SHORTCUT_CHORD_ROLLOVER_MS: u64 = 150;

pub(crate) fn bind_shortcuts() {
	unsafe {
		if let Ok(mut hook_guard) = KEYBOARD_HOOK.lock() {
			if *hook_guard == 0 {
				let hook = SetWindowsHookExW(
					WH_KEYBOARD_LL,
					Some(low_level_keyboard_proc),
					std::ptr::null_mut() as HINSTANCE,
					0,
				);
				if hook.is_null() {
					log_error(
						"keys/hook",
						&format!(
							"failed to install keyboard hook: win32 error {}",
							GetLastError()
						),
					);
				} else {
					*hook_guard = hook as isize;
					log_info("keys/hook", "keyboard hook installed");
				}
			}
		}
	}
}

pub(crate) fn keyboard_event_loop() {
	unsafe {
		let mut msg: MSG = std::mem::zeroed();
		loop {
			let status = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
			if status > 0 {
				continue;
			}
			if status == 0 {
				log_info("keys/hook", "keyboard event loop received WM_QUIT");
			} else {
				log_error(
					"keys/hook",
					&format!(
						"keyboard event loop failed: GetMessageW returned -1, win32 error {}",
						GetLastError()
					),
				);
			}
			break;
		}
		mouse_hook::unbind_mouse_hook();
		if let Ok(mut hook_guard) = KEYBOARD_HOOK.lock() {
			if *hook_guard != 0 {
				let _ = UnhookWindowsHookEx(*hook_guard as _);
				*hook_guard = 0;
			}
		}
	}
}

unsafe extern "system" fn low_level_keyboard_proc(
	ncode: i32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	if ncode != HC_ACTION as i32 {
		return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
	}

	let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
	let vk = kb.vkCode as u32;
	let message = wparam as u32;
	let injected = (kb.flags & 0x10) != 0;

	if injected {
		return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
	}

	if message == WM_KEYUP || message == WM_SYSKEYUP {
		if vk < 256 {
			if let Ok(mut state) = KEYDOWN_STATE.lock() {
				state[vk as usize] = false;
			}
		}

		if vk == VK_Q as u32 && is_win_down() && !is_ctrl_down() && !is_alt_down() {
			if dispatch_action(Action::Quit) {
				return 1;
			}
		}

		if is_win_vk(vk) {
			drag::on_cancel();
			let dragged = drag::take_consume_next_lwin_keyup();
			let combo_used = WIN_COMBO_USED.swap(false, Ordering::Relaxed);
			let native_passthrough =
				WIN_NATIVE_PASSTHROUGH.swap(false, Ordering::Relaxed);
			let intercepted_key = INTERCEPTED_WIN_KEY.swap(0, Ordering::Relaxed);
			let intercepted = intercepted_key == vk;
			if intercepted {
				if native_passthrough {
					keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
					return 1;
				}
				if should_replay_win_key(dragged, combo_used, intercepted) {
					keybd_event(vk as u8, 0, 0, 0);
					keybd_event(vk as u8, 0, KEYEVENTF_KEYUP, 0);
				}
				return 1;
			}
		}
		return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
	}

	if message != WM_KEYDOWN && message != WM_SYSKEYDOWN {
		return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
	}

	if vk < 256 {
		if let Ok(mut state) = KEYDOWN_STATE.lock() {
			if state[vk as usize] {
				if should_clear_stale_keydown_state(true, physical_key_is_down(vk)) {
					log_info(
						"keys/hook",
						&format!("cleared stale keydown state for vk {}", vk),
					);
					state[vk as usize] = false;
				}
			}
			if state[vk as usize] {
				if is_win_vk(vk) {
					return 1;
				}
				return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
			}
			state[vk as usize] = true;
		}
		if let Ok(mut ticks) = KEYDOWN_TICKS_MS.lock() {
			ticks[vk as usize] = monotonic_ms();
		}
	}

	if is_win_vk(vk) {
		INTERCEPTED_WIN_KEY.store(vk, Ordering::Relaxed);
		WIN_COMBO_USED.store(false, Ordering::Relaxed);
		WIN_NATIVE_PASSTHROUGH.store(false, Ordering::Relaxed);
		if let Some(action) = action_for_recently_pressed_key() {
			if dispatch_action(action) {
				WIN_COMBO_USED.store(true, Ordering::Relaxed);
			}
		}
		return 1;
	}

	if handle_keydown(vk) {
		if is_win_modifier_down() {
			WIN_COMBO_USED.store(true, Ordering::Relaxed);
		}
		return 1;
	}

	if is_win_modifier_down() {
		ensure_native_win_passthrough_started();
	}

	CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}

fn handle_keydown(vk: u32) -> bool {
	let win_down = is_win_modifier_down();
	let action = shortcut_action_for_key(
		vk,
		win_down,
		is_ctrl_down(),
		is_alt_down(),
		is_shift_down(),
	);
	action.is_some_and(dispatch_action)
}

fn shortcut_action_for_key(
	vk: u32,
	win_down: bool,
	ctrl_down: bool,
	alt_down: bool,
	shift_down: bool,
) -> Option<Action> {
	if !win_down || alt_down {
		return None;
	}

	match vk {
		value if value == VK_LEFT as u32 => {
			if ctrl_down && shift_down {
				return Some(Action::MoveWindowToAdjacentDesktop { delta: -1 });
			}
			if ctrl_down || shift_down {
				return None;
			}
			return Some(Action::SnapWindow {
				direction: SnapDirection::Left,
			});
		}
		value if value == VK_RIGHT as u32 => {
			if ctrl_down && shift_down {
				return Some(Action::MoveWindowToAdjacentDesktop { delta: 1 });
			}
			if ctrl_down || shift_down {
				return None;
			}
			return Some(Action::SnapWindow {
				direction: SnapDirection::Right,
			});
		}
		value if value == VK_UP as u32 => {
			if ctrl_down || shift_down {
				return None;
			}
			return Some(Action::SnapWindow {
				direction: SnapDirection::Up,
			});
		}
		value if value == VK_DOWN as u32 => {
			if ctrl_down || shift_down {
				return None;
			}
			return Some(Action::SnapWindow {
				direction: SnapDirection::Down,
			});
		}
		_ => {}
	}

	if (b'1' as u32..=b'9' as u32).contains(&vk) {
		let desktop = vk - b'1' as u32;
		return Some(Action::SwitchDesktop {
			desktop,
			move_window: shift_down,
		});
	}

	if vk == VK_OEM_3 as u32 {
		return Some(Action::TogglePreviousDesktop {
			move_window: shift_down,
		});
	}

	if vk == VK_RETURN as u32 {
		return Some(Action::LaunchWezterm { local: shift_down });
	}

	if vk == VK_Q as u32 {
		return Some(Action::Quit);
	}

	None
}

fn is_win_down() -> bool {
	key_is_down(VK_LWIN as u32) || key_is_down(VK_RWIN as u32)
}

fn is_win_modifier_down() -> bool {
	is_win_down() || INTERCEPTED_WIN_KEY.load(Ordering::Relaxed) != 0
}

fn is_shift_down() -> bool {
	key_is_down(VK_LSHIFT as u32) || key_is_down(VK_RSHIFT as u32)
}

fn is_ctrl_down() -> bool {
	key_is_down(VK_LCONTROL as u32) || key_is_down(VK_RCONTROL as u32)
}

fn is_alt_down() -> bool {
	key_is_down(VK_LMENU as u32) || key_is_down(VK_RMENU as u32)
}

fn is_win_vk(vk: u32) -> bool {
	vk == VK_LWIN as u32 || vk == VK_RWIN as u32
}

fn physical_key_is_down(vk: u32) -> bool {
	unsafe { (GetAsyncKeyState(vk as i32) & i16::MIN) != 0 }
}

fn should_clear_stale_keydown_state(
	tracked_as_down: bool,
	physically_down: bool,
) -> bool {
	tracked_as_down && !physically_down
}

fn ensure_native_win_passthrough_started() {
	if WIN_NATIVE_PASSTHROUGH.load(Ordering::Relaxed) {
		return;
	}
	let win_vk = INTERCEPTED_WIN_KEY.load(Ordering::Relaxed);
	if win_vk == 0 {
		return;
	}
	unsafe {
		keybd_event(win_vk as u8, 0, 0, 0);
	}
	WIN_NATIVE_PASSTHROUGH.store(true, Ordering::Relaxed);
}

fn should_replay_win_key(dragged: bool, combo_used: bool, intercepted: bool) -> bool {
	intercepted && !dragged && !combo_used
}

fn action_for_recently_pressed_key() -> Option<Action> {
	let now_ms = monotonic_ms();
	let pressed_keys = {
		let state = KEYDOWN_STATE.lock().ok()?;
		let ticks = KEYDOWN_TICKS_MS.lock().ok()?;
		state
			.iter()
			.zip(ticks.iter())
			.enumerate()
			.filter_map(|(vk, (is_down, tick_ms))| {
				is_down.then_some((vk as u32, *tick_ms))
			})
			.collect::<Vec<_>>()
	};

	// Allow slightly out-of-order chords like Enter-then-Win.
	most_recent_shortcut_action_from_pressed_keys(
		pressed_keys.into_iter(),
		true,
		is_ctrl_down(),
		is_alt_down(),
		is_shift_down(),
		now_ms,
	)
}

fn most_recent_shortcut_action_from_pressed_keys<I>(
	pressed_keys: I,
	win_down: bool,
	ctrl_down: bool,
	alt_down: bool,
	shift_down: bool,
	now_ms: u64,
) -> Option<Action>
where
	I: IntoIterator<Item = (u32, u64)>,
{
	let mut candidate: Option<(Action, u64)> = None;
	for (vk, pressed_at_ms) in pressed_keys {
		if now_ms.saturating_sub(pressed_at_ms) > SHORTCUT_CHORD_ROLLOVER_MS {
			continue;
		}
		let Some(action) =
			shortcut_action_for_key(vk, win_down, ctrl_down, alt_down, shift_down)
		else {
			continue;
		};
		if candidate
			.as_ref()
			.is_none_or(|(_, best_pressed_at_ms)| pressed_at_ms > *best_pressed_at_ms)
		{
			candidate = Some((action, pressed_at_ms));
		}
	}
	candidate.map(|(action, _)| action)
}

fn monotonic_ms() -> u64 {
	static START: OnceLock<Instant> = OnceLock::new();
	START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn win_key_detection_supports_both_keys() {
		assert!(is_win_vk(VK_LWIN as u32));
		assert!(is_win_vk(VK_RWIN as u32));
		assert!(!is_win_vk(VK_Q as u32));
	}

	#[test]
	fn replay_requires_intercept_without_combo_or_drag() {
		assert!(should_replay_win_key(false, false, true));
		assert!(!should_replay_win_key(true, false, true));
		assert!(!should_replay_win_key(false, true, true));
		assert!(!should_replay_win_key(false, false, false));
	}

	#[test]
	fn stale_keydown_state_is_cleared_when_key_is_no_longer_physically_down() {
		assert!(should_clear_stale_keydown_state(true, false));
		assert!(!should_clear_stale_keydown_state(true, true));
		assert!(!should_clear_stale_keydown_state(false, false));
	}

	#[test]
	fn number_shortcuts_map_to_desktops_and_shift_move() {
		assert_eq!(
			shortcut_action_for_key(b'1' as u32, true, false, false, false),
			Some(Action::SwitchDesktop {
				desktop: 0,
				move_window: false,
			})
		);
		assert_eq!(
			shortcut_action_for_key(b'9' as u32, true, false, false, true),
			Some(Action::SwitchDesktop {
				desktop: 8,
				move_window: true,
			})
		);
	}

	#[test]
	fn tilde_shortcuts_toggle_previous_desktop_and_shift_moves() {
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, false, false, false),
			Some(Action::TogglePreviousDesktop { move_window: false })
		);
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, false, false, true),
			Some(Action::TogglePreviousDesktop { move_window: true })
		);
	}

	#[test]
	fn plain_arrow_shortcuts_map_to_snap_actions() {
		assert_eq!(
			shortcut_action_for_key(VK_LEFT as u32, true, false, false, false),
			Some(Action::SnapWindow {
				direction: SnapDirection::Left,
			})
		);
		assert_eq!(
			shortcut_action_for_key(VK_RIGHT as u32, true, false, false, false),
			Some(Action::SnapWindow {
				direction: SnapDirection::Right,
			})
		);
		assert_eq!(
			shortcut_action_for_key(VK_UP as u32, true, false, false, false),
			Some(Action::SnapWindow {
				direction: SnapDirection::Up,
			})
		);
		assert_eq!(
			shortcut_action_for_key(VK_DOWN as u32, true, false, false, false),
			Some(Action::SnapWindow {
				direction: SnapDirection::Down,
			})
		);
	}

	#[test]
	fn ctrl_shift_left_right_move_window_to_adjacent_desktops() {
		assert_eq!(
			shortcut_action_for_key(VK_LEFT as u32, true, true, false, true),
			Some(Action::MoveWindowToAdjacentDesktop { delta: -1 })
		);
		assert_eq!(
			shortcut_action_for_key(VK_RIGHT as u32, true, true, false, true),
			Some(Action::MoveWindowToAdjacentDesktop { delta: 1 })
		);
	}

	#[test]
	fn shift_up_down_remain_native() {
		assert_eq!(
			shortcut_action_for_key(VK_UP as u32, true, false, false, true),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_DOWN as u32, true, false, false, true),
			None
		);
	}

	#[test]
	fn enter_shortcut_uses_shift_for_local_domain_choice() {
		assert_eq!(
			shortcut_action_for_key(VK_RETURN as u32, true, false, false, false),
			Some(Action::LaunchWezterm { local: false })
		);
		assert_eq!(
			shortcut_action_for_key(VK_RETURN as u32, true, false, false, true),
			Some(Action::LaunchWezterm { local: true })
		);
	}

	#[test]
	fn alt_blocks_other_shortcuts_and_ctrl_only_changes_arrow_handling() {
		assert_eq!(
			shortcut_action_for_key(b'2' as u32, true, false, true, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, false, true, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_Q as u32, false, false, false, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_LEFT as u32, true, true, false, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_LEFT as u32, true, false, true, true),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_RETURN as u32, true, true, false, false),
			Some(Action::LaunchWezterm { local: false })
		);
	}

	#[test]
	fn recent_pressed_shortcut_supports_enter_then_win_order() {
		let action = most_recent_shortcut_action_from_pressed_keys(
			[(VK_RETURN as u32, 1_000)],
			true,
			false,
			false,
			false,
			1_100,
		);
		assert_eq!(action, Some(Action::LaunchWezterm { local: false }));
	}

	#[test]
	fn recent_pressed_shortcut_honors_shift_and_rollover_window() {
		assert_eq!(
			most_recent_shortcut_action_from_pressed_keys(
				[(VK_RETURN as u32, 1_000)],
				true,
				false,
				false,
				true,
				1_100,
			),
			Some(Action::LaunchWezterm { local: true })
		);
		assert_eq!(
			most_recent_shortcut_action_from_pressed_keys(
				[(VK_RETURN as u32, 1_000)],
				true,
				false,
				false,
				false,
				1_200,
			),
			None
		);
	}
}
