/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use windows_sys::Win32::{
	Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
	UI::{
		Controls::STATE_SYSTEM_INVISIBLE,
		Input::KeyboardAndMouse::{
			VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RETURN, VK_RMENU,
			VK_RSHIFT, VK_RWIN,
		},
		WindowsAndMessaging::{
			CallNextHookEx, EnumWindows, GetClassNameW, GetForegroundWindow, GetMessageW,
			GetTitleBarInfo, GetWindowLongW, GetWindowTextW, IsWindowVisible,
			SetForegroundWindow, SetWindowsHookExW, UnhookWindowsHookEx, GWL_EXSTYLE,
			HC_ACTION, KBDLLHOOKSTRUCT, MSG, TITLEBARINFO, WH_KEYBOARD_LL, WM_KEYDOWN,
			WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WS_EX_TOOLWINDOW,
		},
	},
};

static KEYBOARD_HOOK: Mutex<isize> = Mutex::new(0);
static KEYDOWN_STATE: Mutex<[bool; 256]> = Mutex::new([false; 256]);
static ACTION_TX: OnceLock<mpsc::Sender<Action>> = OnceLock::new();
static PRUNE_GRACE_DESKTOP: Mutex<Option<(u32, Instant)>> = Mutex::new(None);
static PREVIOUS_DESKTOP: Mutex<Option<u32>> = Mutex::new(None);

enum Action {
	SwitchDesktop { desktop: u32, move_window: bool },
	LaunchWezterm { local: bool },
}

pub async fn init() {
	start_action_worker();
	bind_shortcuts();
	keyboard_event_loop();
}

fn start_action_worker() {
	if ACTION_TX.get().is_some() {
		return;
	}
	let (tx, rx) = mpsc::channel::<Action>();
	if ACTION_TX.set(tx).is_err() {
		return;
	}
	thread::spawn(move || {
		while let Ok(action) = rx.recv() {
			match action {
				Action::SwitchDesktop {
					desktop,
					move_window,
				} => {
					let moved_window = if move_window {
						active_window_for_move()
					} else {
						None
					};
					switch_to_desktop(
						target_desktop_for_shortcut(desktop),
						0,
						moved_window,
					);
				}
				Action::LaunchWezterm { local } => {
					if local {
						launch_wezterm("local");
					} else {
						launch_wezterm("arch");
					}
				}
			}
		}
	});
}

pub fn bind_shortcuts() {
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
				}
			}
		}
	}
}

fn keyboard_event_loop() {
	unsafe {
		let mut msg: MSG = std::mem::zeroed();
		while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
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

	if message == WM_KEYUP || message == WM_SYSKEYUP {
		if vk < 256 {
			if let Ok(mut state) = KEYDOWN_STATE.lock() {
				state[vk as usize] = false;
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
				return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
			}
			state[vk as usize] = true;
		}
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

	false
}

fn dispatch_action(action: Action) -> bool {
	if let Some(tx) = ACTION_TX.get() {
		return tx.send(action).is_ok();
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

fn key_is_down(vkey: u32) -> bool {
	if vkey >= 256 {
		return false;
	}
	if let Ok(state) = KEYDOWN_STATE.lock() {
		return state[vkey as usize];
	}
	false
}

fn launch_wezterm(domain: &str) {
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	const DETACHED_PROCESS: u32 = 0x0000_0008;
	let _ = Command::new("wezterm-gui.exe")
		.arg("start")
		.arg("--domain")
		.arg(domain)
		.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
		.spawn();
}

fn target_desktop_for_shortcut(shortcut_desktop: u32) -> u32 {
	let current_index =
		match winvd::get_current_desktop().and_then(|desktop| desktop.get_index()) {
			Ok(index) => index,
			Err(_) => return shortcut_desktop,
		};
	if current_index != shortcut_desktop {
		return shortcut_desktop;
	}
	match PREVIOUS_DESKTOP.lock() {
		Ok(guard) => match *guard {
			Some(previous) if previous != current_index => previous,
			_ => shortcut_desktop,
		},
		Err(_) => shortcut_desktop,
	}
}

fn active_window_for_move() -> Option<HWND> {
	unsafe {
		let hwnd = GetForegroundWindow();
		if hwnd.is_null() || !is_normal_window(hwnd) {
			return None;
		}
		return Some(hwnd);
	}
}

fn focus_moved_window(window: HWND) -> bool {
	for _ in 0..10 {
		if matches!(
			winvd::is_window_on_current_desktop(unsafe { std::mem::transmute(window) }),
			Ok(true)
		) {
			unsafe {
				let _ = SetForegroundWindow(window);
			}
			return true;
		}
		thread::sleep(Duration::from_millis(40));
	}
	false
}

unsafe extern "system" fn enum_windows_and_switch_app_focus(
	hwnd: HWND,
	_lparam: LPARAM,
) -> BOOL {
	if !is_normal_window(hwnd) {
		return 1;
	}

	let is_on_current_desktop =
		winvd::is_window_on_current_desktop(unsafe { std::mem::transmute(hwnd) })
			.unwrap_or(false);
	if !is_on_current_desktop {
		return 1;
	}

	let _ = SetForegroundWindow(hwnd);
	return 0;
}

#[derive(Default)]
struct OccupiedDesktopState {
	highest_index: Option<u32>,
}

unsafe extern "system" fn enum_windows_and_find_highest_occupied_desktop(
	hwnd: HWND,
	lparam: LPARAM,
) -> BOOL {
	if !is_normal_window(hwnd) {
		return 1;
	}
	let state = unsafe { &mut *(lparam as *mut OccupiedDesktopState) };
	if let Ok(index) = winvd::get_desktop_by_window(unsafe { std::mem::transmute(hwnd) })
		.and_then(|desktop| desktop.get_index())
	{
		state.highest_index = Some(match state.highest_index {
			Some(previous) => previous.max(index),
			None => index,
		});
	}
	return 1;
}

fn highest_occupied_desktop_index() -> Option<u32> {
	let mut state = OccupiedDesktopState::default();
	unsafe {
		let _ = EnumWindows(
			Some(enum_windows_and_find_highest_occupied_desktop),
			(&mut state as *mut OccupiedDesktopState) as isize,
		);
	}
	state.highest_index
}

fn protected_desktop_index() -> Option<u32> {
	const PRUNE_GRACE: Duration = Duration::from_secs(5);
	let lock = PRUNE_GRACE_DESKTOP.lock();
	if lock.is_err() {
		return None;
	}
	let mut guard = lock.unwrap();
	match *guard {
		Some((desktop, at)) if at.elapsed() < PRUNE_GRACE => Some(desktop),
		Some(_) => {
			*guard = None;
			None
		}
		None => None,
	}
}

fn is_normal_window(hwnd: HWND) -> bool {
	unsafe {
		if IsWindowVisible(hwnd) == 0 {
			return false;
		}
		let mut class_name_buffer: [u16; 256] = [0; 256];
		let class_len = GetClassNameW(
			hwnd,
			class_name_buffer.as_mut_ptr(),
			class_name_buffer.len() as i32,
		);
		if class_len > 0 {
			let class_name =
				OsString::from_wide(&class_name_buffer[..class_len as usize])
					.to_string_lossy()
					.into_owned();
			if class_name == "Progman"
				|| class_name == "WorkerW"
				|| class_name == "Shell_TrayWnd"
				|| class_name == "Windows.UI.Core.CoreWindow"
			{
				return false;
			}
		}
		let mut ti = TITLEBARINFO {
			cbSize: std::mem::size_of::<TITLEBARINFO>() as u32,
			rcTitleBar: RECT {
				left: 0,
				top: 0,
				right: 0,
				bottom: 0,
			},
			rgstate: [0; 6],
		};
		let _ = GetTitleBarInfo(hwnd, &mut ti);
		if ti.rgstate[0] & STATE_SYSTEM_INVISIBLE as u32 > 0 {
			return false;
		}
		if ((GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW as u32) != 0 {
			return false;
		}
		let mut buffer: [u16; 256] = [0; 256];
		let title_len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
		let window_title = OsString::from_wide(&buffer[..title_len as usize])
			.to_string_lossy()
			.into_owned();
		if window_title.is_empty()
			|| window_title.contains("Settings")
			|| window_title == "Program Manager"
		{
			return false;
		}
	}
	return true;
}

fn remove_tail_desktops_if_possible() {
	const REMOVE_RETRIES: u8 = 6;
	let highest_occupied = highest_occupied_desktop_index().unwrap_or(0);
	let protected_desktop = protected_desktop_index().unwrap_or(0);
	let retain_until = highest_occupied.max(protected_desktop);
	loop {
		let desktop_count = match winvd::get_desktop_count() {
			Ok(count) => count,
			Err(_) => return,
		};
		if desktop_count <= 1 || desktop_count - 1 <= retain_until {
			return;
		}
		let current_index = match winvd::get_current_desktop().and_then(|d| d.get_index())
		{
			Ok(index) => index,
			Err(_) => return,
		};
		let tail_index = desktop_count - 1;
		let fallback_index = tail_index - 1;
		if current_index > retain_until {
			if winvd::switch_desktop(retain_until).is_err() {
				return;
			}
			thread::sleep(Duration::from_millis(80));
			continue;
		}
		let mut removed = false;
		for _ in 0..REMOVE_RETRIES {
			if winvd::remove_desktop(tail_index, fallback_index).is_ok() {
				removed = true;
				break;
			}
			thread::sleep(Duration::from_millis(60));
		}
		if !removed {
			return;
		}
		thread::sleep(Duration::from_millis(40));
	}
}

fn switch_to_desktop(desktop: u32, tries: u8, moved_window: Option<HWND>) {
	if tries <= 10 {
		let source_desktop = winvd::get_current_desktop()
			.and_then(|d| d.get_index())
			.ok();
		match winvd::switch_desktop(desktop) {
			Ok(_) => {
				if let Some(source) = source_desktop {
					if source != desktop {
						if let Ok(mut guard) = PREVIOUS_DESKTOP.lock() {
							*guard = Some(source);
						}
					}
				}
				if let Ok(mut guard) = PRUNE_GRACE_DESKTOP.lock() {
					*guard = Some((desktop, Instant::now()));
				}
				let moved_and_focused = moved_window.is_some_and(|window| {
					let winvd_window = unsafe { std::mem::transmute(window) };
					if winvd::move_window_to_desktop(desktop, &winvd_window).is_ok() {
						return focus_moved_window(window);
					}
					false
				});
				if !moved_and_focused {
					unsafe {
						let _ = EnumWindows(Some(enum_windows_and_switch_app_focus), 0);
					}
				}
				remove_tail_desktops_if_possible();
			}
			Err(_) => match winvd::create_desktop() {
				Ok(_) => {
					switch_to_desktop(desktop, tries + 1, moved_window);
				}
				Err(e) => {
					println!("Error: {:?}", e);
				}
			},
		}
	}
}
