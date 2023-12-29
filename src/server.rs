// SPDX-License-Identifier: AGPL-3.0-or-later

use clap_verbosity_flag::{ErrorLevel, Verbosity};

pub fn server(verbosity: Verbosity<ErrorLevel>, bind_address: std::net::IpAddr, bind_port: u16) -> anyhow::Result<()> {
	Ok(())
}
