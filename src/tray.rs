/* SPDX-FileCopyrightText: © 2023 Nadim Kobeissi <nadim@symbolic.software>
 * SPDX-License-Identifier: MIT */

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use tray_item::{IconSource, TrayItem};
use winvd::DesktopEvent;

use crate::logging::{log_error, log_info};

pub enum TrayMessage {
	Quit,
}

pub fn init() {
	loop {
		match create_tray() {
			Ok((tray, rx)) => {
				log_info("tray", "tray icon initialized");
				tokio::spawn(icon_change_listener(tray));
				match rx.recv() {
					Ok(TrayMessage::Quit) => {
						log_info("tray", "quit requested from tray");
						std::process::exit(0);
					}
					Err(err) => {
						log_error(
							"tray",
							&format!("quit channel closed; recreating tray: {}", err),
						);
					}
				}
			}
			Err(err) => {
				log_error(
					"tray",
					&format!("failed to initialize tray icon; retrying in 2s: {}", err),
				);
				std::thread::sleep(Duration::from_secs(2));
			}
		}
	}
}

fn create_tray() -> Result<(TrayItem, Receiver<TrayMessage>), String> {
	let mut tray = TrayItem::new("BinkyBox", IconSource::Resource("icon"))
		.map_err(|err| err.to_string())?;
	let (tx, rx) = mpsc::sync_channel(1);
	tray.add_menu_item("Quit", move || {
		if tx.send(TrayMessage::Quit).is_err() {
			log_error("tray", "failed to send quit message: channel closed");
		}
	})
	.map_err(|err| err.to_string())?;
	Ok((tray, rx))
}

async fn icon_change_listener(mut tray: TrayItem) {
	let (tx, rx) = std::sync::mpsc::channel::<DesktopEvent>();
	let _notifications_thread = winvd::listen_desktop_events(tx);
	for event in rx {
		if let DesktopEvent::DesktopChanged { new: n, old: _ } = event {
			if let Ok(index) = n.get_index() {
				if let Some(icon) = icon_resource_for_index(index) {
					if let Err(err) = tray.set_icon(IconSource::Resource(icon)) {
						log_error(
							"tray",
							&format!(
								"failed to update tray icon for desktop {}: {}",
								index + 1,
								err
							),
						);
					}
				}
			}
		}
	}
	log_error("tray", "desktop event listener ended");
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
