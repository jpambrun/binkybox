use std::sync::OnceLock;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
	AttachConsole, GetStdHandle, WriteConsoleW, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
};
#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;

#[allow(dead_code)]
pub(crate) fn log_error(component: &str, message: &str) {
	log_line(component, message);
}

pub(crate) fn log_info(component: &str, message: &str) {
	log_line(component, message);
}

fn log_line(component: &str, message: &str) {
	let line = format!("[{}] {}", component, message);
	if write_to_console(&line) {
		return;
	}
	eprintln!("{}", line);
	debug_log(&line);
}

#[cfg(windows)]
fn write_to_console(message: &str) -> bool {
	try_attach_parent_console();
	unsafe {
		let handle = GetStdHandle(STD_ERROR_HANDLE);
		if !is_valid_handle(handle) {
			return false;
		}
		let mut wide: Vec<u16> = message.encode_utf16().collect();
		wide.push('\n' as u16);
		let mut written = 0u32;
		WriteConsoleW(
			handle,
			wide.as_ptr() as *const _,
			wide.len() as u32,
			&mut written,
			std::ptr::null_mut(),
		) != 0
	}
}

#[cfg(not(windows))]
fn write_to_console(message: &str) -> bool {
	eprintln!("{}", message);
	true
}

#[cfg(windows)]
fn try_attach_parent_console() {
	static ATTACH_ATTEMPTED: OnceLock<()> = OnceLock::new();
	if ATTACH_ATTEMPTED.set(()).is_err() {
		return;
	}
	unsafe {
		let _ = AttachConsole(ATTACH_PARENT_PROCESS);
	}
}

#[cfg(windows)]
fn is_valid_handle(handle: HANDLE) -> bool {
	handle != std::ptr::null_mut() && handle != INVALID_HANDLE_VALUE
}

#[cfg(windows)]
fn debug_log(message: &str) {
	let mut wide: Vec<u16> = message.encode_utf16().collect();
	wide.push(0);
	unsafe {
		OutputDebugStringW(wide.as_ptr());
	}
}

#[cfg(not(windows))]
fn debug_log(message: &str) {
	eprintln!("{}", message);
}
