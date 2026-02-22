/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

use inputbot::KeybdKey::{self, *};
use json::JsonValue;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::os::windows::process::CommandExt;

use windows::Win32::{
	Foundation::*, UI::Controls::STATE_SYSTEM_INVISIBLE, UI::WindowsAndMessaging::*,
};

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

use crate::config;

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
	let my_config = config::read();
	for (_, value) in KEY_MAP.iter() {
		value.unbind();
	}
	EnterKey.unbind();
	bind_wezterm_shortcut();
	for i in 0..9 {
		let shortcut = process_shortcut(&my_config, i);
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
		if hwnd == HWND::default() || !is_normal_window(hwnd) {
			return None;
		}
		return Some(hwnd);
	}
}

fn focus_moved_window(window: HWND) -> bool {
	for _ in 0..10 {
		if matches!(winvd::is_window_on_current_desktop(window), Ok(true)) {
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
		return BOOL(1);
	}

	// remove windows that are not in the current virtual desktop
	let is_on_current_desktop =
		winvd::is_window_on_current_desktop(hwnd as windows::Win32::Foundation::HWND)
			.unwrap();
	if !is_on_current_desktop {
		return BOOL(1);
	}

	let _ = SetForegroundWindow(hwnd);
	return BOOL(0); // Stop enumeration
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
		return BOOL(1);
	}
	let state = unsafe { &mut *(lparam.0 as *mut OccupiedDesktopState) };
	if let Ok(index) =
		winvd::get_desktop_by_window(hwnd).and_then(|desktop| desktop.get_index())
	{
		state.highest_index = Some(match state.highest_index {
			Some(previous) => previous.max(index),
			None => index,
		});
	}
	return BOOL(1);
}

fn highest_occupied_desktop_index() -> Option<u32> {
	let mut state = OccupiedDesktopState::default();
	unsafe {
		let _ = EnumWindows(
			Some(enum_windows_and_find_highest_occupied_desktop),
			LPARAM {
				0: (&mut state as *mut OccupiedDesktopState) as isize,
			},
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
		if IsWindowVisible(hwnd) == false {
			return false;
		}
		let mut class_name_buffer: [u16; 256] = [0; 256];
		let class_len = GetClassNameW(hwnd, &mut class_name_buffer);
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
		if ti.rgstate[0] & STATE_SYSTEM_INVISIBLE.0 > 0 {
			return false;
		}
		if WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE).try_into().unwrap())
			& WS_EX_TOOLWINDOW
			!= WINDOW_EX_STYLE(0)
		{
			return false;
		}
		let mut buffer: [u16; 256] = [0; 256];
		GetWindowTextW(hwnd, &mut buffer);
		let window_title = OsString::from_wide(&buffer).to_string_lossy().into_owned();
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
		let source_desktop =
			winvd::get_current_desktop().and_then(|d| d.get_index()).ok();
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
					if winvd::move_window_to_desktop(desktop, &window).is_ok() {
						return focus_moved_window(window);
					}
					false
				});
				if !moved_and_focused {
					unsafe {
						let _ = EnumWindows(
							Some(enum_windows_and_switch_app_focus),
							LPARAM { 0: 0 },
						);
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

fn process_shortcut(config: &JsonValue, desktop: u32) -> Vec<KeybdKey> {
	let desktop_str = format!("desktop_{}", desktop + 1);
	let shortcut_sanitized =
		sanitize_keyboard_shortcut(config["shortcuts"][&desktop_str].to_string());
	let shortcut_string = match check_keyboard_shortcut(shortcut_sanitized.clone()) {
		true => shortcut_sanitized,
		false => config::get_default()["shortcuts"][&desktop_str].to_string(),
	};
	build_keyboard_shortcut(shortcut_string.as_str())
}

pub fn sanitize_keyboard_shortcut(input: String) -> String {
	let mut input = input.to_uppercase();
	input.retain(|c| !c.is_whitespace());
	return input;
}

pub fn check_keyboard_shortcut(input: String) -> bool {
	let re = Regex::new(
		r"^((([LR]CTRL)|([LR]ALT)|([LR]WIN)|([LR]SHIFT)|(F\d))\+){1,4}([A-Z\d]|(F\d))$",
	);
	if re.is_err() {
		return false;
	}
	return re.unwrap().is_match(&input);
}

fn build_keyboard_shortcut(input: &str) -> Vec<KeybdKey> {
	input
		.split('+')
		.filter_map(|part| KEY_MAP.get(&part))
		.cloned()
		.collect()
}
