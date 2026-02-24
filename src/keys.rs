/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

use inputbot::KeybdKey::{self, *};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use windows_sys::Win32::{
	Foundation::{BOOL, HWND, LPARAM, RECT},
	UI::{
		Controls::STATE_SYSTEM_INVISIBLE,
		WindowsAndMessaging::{
			EnumWindows, GetClassNameW, GetForegroundWindow, GetTitleBarInfo,
			GetWindowLongW, GetWindowTextW, IsWindowVisible, SetForegroundWindow,
			GWL_EXSTYLE, TITLEBARINFO, WS_EX_TOOLWINDOW,
		},
	},
};

const DEFAULT_SHORTCUTS: [&str; 9] = [
	"LWIN+1", "LWIN+2", "LWIN+3", "LWIN+4", "LWIN+5", "LWIN+6", "LWIN+7", "LWIN+8",
	"LWIN+9",
];

lazy_static! {
	static ref KEY_MAP: HashMap<&'static str, KeybdKey> = [
		("CTRL", LControlKey),
		("LCTRL", LControlKey),
		("RCTRL", RControlKey),
		("ALT", LAltKey),
		("LALT", LAltKey),
		("RALT", RAltKey),
		("WIN", LSuper),
		("LWIN", LSuper),
		("RWIN", RSuper),
		("SHIFT", LShiftKey),
		("LSHIFT", LShiftKey),
		("RSHIFT", RShiftKey),
		("F1", F1Key),
		("F2", F2Key),
		("F3", F3Key),
		("F4", F4Key),
		("F5", F5Key),
		("F6", F6Key),
		("F7", F7Key),
		("F8", F8Key),
		("F9", F9Key),
		("F10", F10Key),
		("F11", F11Key),
		("F12", F12Key),
		("A", AKey),
		("B", BKey),
		("C", CKey),
		("D", DKey),
		("E", EKey),
		("F", FKey),
		("G", GKey),
		("H", HKey),
		("I", IKey),
		("J", JKey),
		("K", KKey),
		("L", LKey),
		("M", MKey),
		("N", NKey),
		("O", OKey),
		("P", PKey),
		("Q", QKey),
		("R", RKey),
		("S", SKey),
		("T", TKey),
		("U", UKey),
		("V", VKey),
		("W", WKey),
		("X", XKey),
		("Y", YKey),
		("Z", ZKey),
		("1", Numrow1Key),
		("2", Numrow2Key),
		("3", Numrow3Key),
		("4", Numrow4Key),
		("5", Numrow5Key),
		("6", Numrow6Key),
		("7", Numrow7Key),
		("8", Numrow8Key),
		("9", Numrow9Key),
		("0", Numrow0Key),
	]
	.iter()
	.cloned()
	.collect();
	static ref PRUNE_GRACE_DESKTOP: Mutex<Option<(u32, Instant)>> = Mutex::new(None);
	static ref PREVIOUS_DESKTOP: Mutex<Option<u32>> = Mutex::new(None);
}

pub async fn init() {
	bind_shortcuts();
	inputbot::handle_input_events();
}

pub fn bind_shortcuts() {
	for (_, value) in KEY_MAP.iter() {
		value.unbind();
	}
	EnterKey.unbind();
	bind_wezterm_shortcut();
	for i in 0..9 {
		let shortcut = process_shortcut(i);
		if let Some(key_to_bind) = shortcut.get(shortcut.len().saturating_sub(1)) {
			key_to_bind.blockable_bind(move || {
				if shortcut
					.iter()
					.take(shortcut.len() - 1)
					.all(|key| key.is_pressed())
				{
					for (_, value) in KEY_MAP.iter() {
						if value.is_pressed() && !shortcut.contains(value) {
							if *value == LShiftKey || *value == RShiftKey {
								continue;
							}
							return inputbot::BlockInput::DontBlock;
						}
					}
					let moved_window = if should_move_active_window(&shortcut) {
						active_window_for_move()
					} else {
						None
					};
					switch_to_desktop(target_desktop_for_shortcut(i), 0, moved_window);
					return inputbot::BlockInput::Block;
				}
				return inputbot::BlockInput::DontBlock;
			});
		}
	}
}

fn bind_wezterm_shortcut() {
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	const DETACHED_PROCESS: u32 = 0x0000_0008;
	EnterKey.blockable_bind(move || {
		if !(LSuper.is_pressed() || RSuper.is_pressed()) {
			return inputbot::BlockInput::DontBlock;
		}
		let with_shift = LShiftKey.is_pressed() || RShiftKey.is_pressed();
		if LControlKey.is_pressed()
			|| RControlKey.is_pressed()
			|| LAltKey.is_pressed()
			|| RAltKey.is_pressed()
		{
			return inputbot::BlockInput::DontBlock;
		}
		let domain = if with_shift { "local" } else { "arch" };
		let launched = Command::new("wezterm-gui.exe")
			.arg("start")
			.arg("--domain")
			.arg(domain)
			.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
			.spawn()
			.is_ok();
		if launched {
			return inputbot::BlockInput::Block;
		}
		inputbot::BlockInput::DontBlock
	});
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

fn should_move_active_window(shortcut: &[KeybdKey]) -> bool {
	if !(LShiftKey.is_pressed() || RShiftKey.is_pressed()) {
		return false;
	}
	if shortcut.contains(&LShiftKey) || shortcut.contains(&RShiftKey) {
		return false;
	}
	return true;
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

fn process_shortcut(desktop: u32) -> Vec<KeybdKey> {
	DEFAULT_SHORTCUTS
		.get(desktop as usize)
		.map_or_else(Vec::new, |shortcut| build_keyboard_shortcut(shortcut))
}

fn build_keyboard_shortcut(input: &str) -> Vec<KeybdKey> {
	input
		.split('+')
		.filter_map(|part| KEY_MAP.get(&part))
		.cloned()
		.collect()
}
