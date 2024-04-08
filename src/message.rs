// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2023-2024 rtldg <rtldg@protonmail.com>

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum WsMessage {
	//
	// Only client->server.
	Join(String),
	// Number of current users in the party.
	// Implies pause (if count != 1 || previous >= 1).
	// Only server->client.
	Party(u32),

	//
	Resume,
	// Implies pause.
	AbsoluteSeek(f64),
	//
	Ping(String),
	//
	Pong(String),
}
