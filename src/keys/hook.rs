use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::{
	Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
	UI::{
		Input::KeyboardAndMouse::{
			keybd_event, KEYEVENTF_KEYUP, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
			VK_Q, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN,
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
use super::log;
use super::mouse_hook;
use super::{key_is_down, KEYDOWN_STATE};

static KEYBOARD_HOOK: Mutex<isize> = Mutex::new(0);
static LWIN_INTERCEPT_ACTIVE: AtomicBool = AtomicBool::new(false);
static WIN_COMBO_USED: AtomicBool = AtomicBool::new(false);

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
				if !hook.is_null() {
					*hook_guard = hook as isize;
					log::event("keyboard_hook bind ok");
				} else {
					log::event("keyboard_hook bind failed");
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
				log::event("keyboard_hook unbound");
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
			log::event("keyboard q keyup with win");
			if dispatch_action(Action::Quit) {
				return 1;
			}
		}

		if vk == VK_LWIN as u32 {
			drag::on_cancel();
			mouse_hook::cancel_drag_move();
			let dragged = drag::take_consume_next_lwin_keyup();
			let combo_used = WIN_COMBO_USED.swap(false, Ordering::Relaxed);
			let intercepted =
				LWIN_INTERCEPT_ACTIVE.swap(false, Ordering::Relaxed);
			log::event(&format!(
				"keyboard lwin keyup intercepted={} dragged={} combo_used={}",
				intercepted, dragged, combo_used
			));
			if intercepted {
				if !dragged && !combo_used {
					unsafe {
						keybd_event(VK_LWIN as u8, 0, 0, 0);
						keybd_event(VK_LWIN as u8, 0, KEYEVENTF_KEYUP, 0);
					}
					log::event("keyboard lwin replay tap");
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
				if vk == VK_LWIN as u32 {
					return 1;
				}
				return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
			}
			state[vk as usize] = true;
		}
	}

	if vk == VK_LWIN as u32 {
		LWIN_INTERCEPT_ACTIVE.store(true, Ordering::Relaxed);
		WIN_COMBO_USED.store(false, Ordering::Relaxed);
		log::event("keyboard lwin keydown intercepted");
		return 1;
	}

	if key_is_down(VK_LWIN as u32) {
		WIN_COMBO_USED.store(true, Ordering::Relaxed);
	}

	if handle_keydown(vk) {
		return 1;
	}

	CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}

fn handle_keydown(vk: u32) -> bool {
	if !is_win_down() || is_ctrl_or_alt_down() {
		return false;
	}

	let with_shift = is_shift_down();
	if (b'1' as u32..=b'9' as u32).contains(&vk) {
		let desktop = vk - b'1' as u32;
		return dispatch_action(Action::SwitchDesktop {
			desktop,
			move_window: with_shift,
		});
	}

	if vk == VK_RETURN as u32 {
		return dispatch_action(Action::LaunchWezterm { local: with_shift });
	}

	if vk == VK_Q as u32 {
		log::event("keyboard q keydown with win");
		return dispatch_action(Action::Quit);
	}

	false
}

fn is_win_down() -> bool {
	key_is_down(VK_LWIN as u32) || key_is_down(VK_RWIN as u32)
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
