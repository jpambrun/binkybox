/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keys;
mod logging;
mod tray;

use windows_sys::Win32::System::Console::{AllocConsole, FreeConsole};

#[tokio::main]
async fn main() {
	logging::log_info(
		"startup",
		&format!("binkybox version {}", env!("CARGO_PKG_VERSION")),
	);

	// Establish this app as foreground capable application so it can use SetForegroundWindow
	// Create gui console and immediately close it
	unsafe {
		let _ = AllocConsole();
		let _ = FreeConsole();
	}

	let _ = std::thread::Builder::new()
		.name("binkybox-keys".to_string())
		.spawn(keys::init);
	tray::init();
}
