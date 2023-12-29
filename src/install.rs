// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::{Read, Write};

pub fn install() -> anyhow::Result<()> {
	let scripts_dir = directories::UserDirs::new().unwrap().home_dir().join(if cfg!(windows) {
		"AppData\\Roaming\\mpv\\scripts\\"
	} else {
		".config/mpv/scripts/"
	});

	println!("- Creating {}", scripts_dir.display());
	std::io::stdout().flush().unwrap();

	std::fs::create_dir_all(&scripts_dir)?;

	println!("- Writing {}", scripts_dir.join("simulcast-mpv.lua").display());
	std::io::stdout().flush().unwrap();

	std::fs::write(scripts_dir.join("simulcast-mpv.lua"), include_str!("simulcast-mpv.lua"))?;

	let target_exe = scripts_dir.join(if cfg!(windows) {
		"simulcast-mpv.exe"
	} else {
		"simulcast-mpv"
	});
	if target_exe != std::env::current_exe()? {
		println!("- Copying current executable to scripts directory...");
		std::io::stdout().flush().unwrap();
		let _ = std::fs::copy(std::env::current_exe()?, target_exe)?;
	}

	println!("Press any key to continue...");
	std::io::stdout().flush().unwrap();
	let _ = std::io::stdin().read(&mut [0u8]).unwrap();

	Ok(())
}
