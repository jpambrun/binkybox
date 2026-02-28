/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

use std::sync::mpsc;

use tray_item::{IconSource, TrayItem};
use winvd::DesktopEvent;

pub enum TrayMessage {
	Quit,
}

pub fn init() {
	let mut tray = match TrayItem::new("BinkyBox", IconSource::Resource("icon")) {
		Ok(tray) => tray,
		Err(err) => {
			eprintln!("[tray] failed to initialize tray icon: {}", err);
			return;
		}
	};

	let (tx, rx) = mpsc::sync_channel(1);
	if let Err(err) = tray.add_menu_item("Quit", move || {
		if tx.send(TrayMessage::Quit).is_err() {
			eprintln!("[tray] failed to send quit message: channel closed");
		}
	}) {
		eprintln!("[tray] failed to add tray menu item: {}", err);
		return;
	}

	tokio::spawn(icon_change_listener(tray));
	loop {
		match rx.recv() {
			Ok(TrayMessage::Quit) => {
				std::process::exit(0);
			}
			Err(err) => {
				eprintln!("[tray] quit channel closed: {}", err);
				return;
			}
		}
	}
}

async fn icon_change_listener(mut tray: TrayItem) {
	let (tx, rx) = std::sync::mpsc::channel::<DesktopEvent>();
	let _notifications_thread = winvd::listen_desktop_events(tx);
	for event in rx {
		if let DesktopEvent::DesktopChanged { new: n, old: _ } = event {
			if let Ok(index) = n.get_index() {
				if let Some(icon) = icon_resource_for_index(index) {
					if let Err(err) = tray.set_icon(IconSource::Resource(icon)) {
						eprintln!(
							"[tray] failed to update tray icon for desktop {}: {}",
							index + 1,
							err
						);
					}
				}
			}
		}
	}
}

fn icon_resource_for_index(index: u32) -> Option<&'static str> {
	match index {
		0 => Some("num_1"),
		1 => Some("num_2"),
		2 => Some("num_3"),
		3 => Some("num_4"),
		4 => Some("num_5"),
		5 => Some("num_6"),
		6 => Some("num_7"),
		7 => Some("num_8"),
		8 => Some("num_9"),
		_ => None,
	}
}
