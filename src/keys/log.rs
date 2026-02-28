use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::OnceLock;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_TX: OnceLock<SyncSender<String>> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

fn init_logger() -> Option<SyncSender<String>> {
	let path = std::env::var_os("BINKYBOX_LOG_PATH")
		.map(PathBuf::from)
		.unwrap_or_else(|| {
			std::env::current_dir()
				.unwrap_or_else(|_| std::env::temp_dir())
				.join("binkybox.log")
		});
	let _ = LOG_PATH.set(path.clone());
	let file = OpenOptions::new().create(true).append(true).open(path).ok()?;
	let (tx, rx) = mpsc::sync_channel::<String>(512);
	thread::spawn(move || {
		let mut writer = BufWriter::new(file);
		while let Ok(line) = rx.recv() {
			let _ = writeln!(writer, "{}", line);
			let _ = writer.flush();
		}
	});
	Some(tx)
}

pub(crate) fn session(label: &str) {
	let path = log_path();
	event(&format!("session start pid={} label={} log={}", std::process::id(), label, path));
}

pub(crate) fn log_path() -> String {
	if LOG_PATH.get().is_none() {
		let _ = LOG_TX.get_or_init(|| {
			init_logger().unwrap_or_else(|| {
				let (tx, _rx) = mpsc::sync_channel::<String>(1);
				tx
			})
		});
	}
	LOG_PATH
		.get()
		.map(|p| p.display().to_string())
		.unwrap_or_else(|| "<unavailable>".to_string())
}

pub(crate) fn event(message: &str) {
	let ts = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis())
		.unwrap_or(0);
	let line = format!("{} {}", ts, message);
	let tx = LOG_TX.get_or_init(|| {
		init_logger().unwrap_or_else(|| {
			let (tx, _rx) = mpsc::sync_channel::<String>(1);
			tx
		})
	});
	match tx.try_send(line) {
		Ok(_) => {}
		Err(TrySendError::Full(_)) => {}
		Err(TrySendError::Disconnected(_)) => {}
	}
}
