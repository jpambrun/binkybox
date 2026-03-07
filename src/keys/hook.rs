use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::{
	Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
	UI::{
		Input::KeyboardAndMouse::{
			keybd_event, KEYEVENTF_KEYUP, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
			VK_OEM_3, VK_Q, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN,
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
use super::{key_is_down, KEYDOWN_STATE};

static KEYBOARD_HOOK: Mutex<isize> = Mutex::new(0);
static INTERCEPTED_WIN_KEY: AtomicU32 = AtomicU32::new(0);
static WIN_COMBO_USED: AtomicBool = AtomicBool::new(false);
static WIN_NATIVE_PASSTHROUGH: AtomicBool = AtomicBool::new(false);

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
					eprintln!("[keys/hook] failed to install keyboard hook");
				} else {
					*hook_guard = hook as isize;
				}
			}
		}
	}
}

pub(crate) fn keyboard_event_loop() {
	unsafe {
		let mut msg: MSG = std::mem::zeroed();
		while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
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

		if vk == VK_Q as u32 && is_win_down() && !is_ctrl_or_alt_down() {
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
				if is_win_vk(vk) {
					return 1;
				}
				return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
			}
			state[vk as usize] = true;
		}
	}

	if is_win_vk(vk) {
		INTERCEPTED_WIN_KEY.store(vk, Ordering::Relaxed);
		WIN_COMBO_USED.store(false, Ordering::Relaxed);
		WIN_NATIVE_PASSTHROUGH.store(false, Ordering::Relaxed);
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
	let action =
		shortcut_action_for_key(vk, win_down, is_ctrl_or_alt_down(), is_shift_down());
	action.is_some_and(dispatch_action)
}

fn shortcut_action_for_key(
	vk: u32,
	win_down: bool,
	ctrl_alt_down: bool,
	shift_down: bool,
) -> Option<Action> {
	if !win_down || ctrl_alt_down {
		return None;
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

fn is_ctrl_or_alt_down() -> bool {
	key_is_down(VK_LCONTROL as u32)
		|| key_is_down(VK_RCONTROL as u32)
		|| key_is_down(VK_LMENU as u32)
		|| key_is_down(VK_RMENU as u32)
}

fn is_win_vk(vk: u32) -> bool {
	vk == VK_LWIN as u32 || vk == VK_RWIN as u32
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
	fn number_shortcuts_map_to_desktops_and_shift_move() {
		assert_eq!(
			shortcut_action_for_key(b'1' as u32, true, false, false),
			Some(Action::SwitchDesktop {
				desktop: 0,
				move_window: false,
			})
		);
		assert_eq!(
			shortcut_action_for_key(b'9' as u32, true, false, true),
			Some(Action::SwitchDesktop {
				desktop: 8,
				move_window: true,
			})
		);
	}

	#[test]
	fn tilde_shortcuts_toggle_previous_desktop_and_shift_moves() {
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, false, false),
			Some(Action::TogglePreviousDesktop { move_window: false })
		);
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, false, true),
			Some(Action::TogglePreviousDesktop { move_window: true })
		);
	}

	#[test]
	fn enter_shortcut_uses_shift_for_local_domain_choice() {
		assert_eq!(
			shortcut_action_for_key(VK_RETURN as u32, true, false, false),
			Some(Action::LaunchWezterm { local: false })
		);
		assert_eq!(
			shortcut_action_for_key(VK_RETURN as u32, true, false, true),
			Some(Action::LaunchWezterm { local: true })
		);
	}

	#[test]
	fn ctrl_or_alt_blocks_shortcut_dispatch() {
		assert_eq!(
			shortcut_action_for_key(b'2' as u32, true, true, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_OEM_3 as u32, true, true, false),
			None
		);
		assert_eq!(
			shortcut_action_for_key(VK_Q as u32, false, false, false),
			None
		);
	}
}
