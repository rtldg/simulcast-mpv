-- SPDX-License-Identifier: WTFPL
-- Copyright 2023-2024 rtldg <rtldg@protonmail.com>

---------------------------------------------------------------------------------------
--                                     WARNING                                       --
-- This file will be overwritten whenever you run simulcast-mpv without arguments!!! --
--           That means when you install new versions of simulcast-mpv!!!            --
--                                   BE CAREFUL!!                                    --
---------------------------------------------------------------------------------------

mp.msg = require("mp.msg")
mp.utils = require("mp.utils")

SIMULCAST_ENABLED = true
SIMULCAST_CONNECTED = nil

local PLATFORM = mp.get_property("platform")

mp.set_property("user-data/simulcast/fuckmpv", ".")

local function setup_heartbeat()
	local latest_beat = nil
	local checked_beat = nil
	mp.observe_property("user-data/simulcast/heartbeat", "number", function(name, value)
		latest_beat = value
	end)
	return mp.add_periodic_timer(0.5, function()
		if SIMULCAST_CONNECTED ~= (latest_beat ~= checked_beat) then
			if checked_beat ~= nil then
				-- TODO:
				--mp.osd_message("SIMULCAST "..(SIMULCAST_CONNECTED and "lost connection" or "connected!"))
			end
		end
		SIMULCAST_CONNECTED = (latest_beat ~= checked_beat)
		checked_beat = latest_beat
	end)
end

local function setup_keybinds()
	local function pause_toggle()
		if mp.get_property_bool("pause") then
			if SIMULCAST_ENABLED and SIMULCAST_CONNECTED then
				mp.set_property("user-data/simulcast/fuckmpv", "queue_resume")
			else
				mp.set_property_bool("pause", false)
			end
		else
			mp.set_property_bool("pause", true)
		end
	end
	mp.add_forced_key_binding("MBTN_RIGHT", "simulcast-pause-toggle", pause_toggle)
	mp.add_forced_key_binding("space", pause_toggle)
	mp.add_forced_key_binding("p", pause_toggle)

	mp.add_key_binding("a", "simulcast-info", function()
		-- TODO: Spam `a` a few times to open a prompt to accept a custom roomid.
		mp.set_property("user-data/simulcast/fuckmpv", "print_info")
		--[[
		SIMULCAST_ENABLED = not SIMULCAST_ENABLED
		if not SIMULCAST_ENABLED then
			-- TODO: doesn't do anything yet... lol...
			mp.set_property("user-data/simulcast/fuckmpv", "disabled")
		end
		mp.osd_message("SIMULCAST " .. (SIMULCAST_ENABLED and "ON" or "OFF"), 2.0)
		]]
	end)
end

local function get_env_map()
	local environ = mp.utils.get_env_list()
	local ret = {}
	for _, envvar in ipairs(environ) do
		local a,b = string.find(envvar, "=")
		if a ~= nil and a ~= 1 then
			ret[envvar:sub(1, a-1)] = envvar:sub(b+1)
		end
	end
	return ret
end

local function get_linux_socket_directory()
	local environ = get_env_map()
	local dir = environ["XDG_RUNTIME_DIR"]
	--mp.command_native({"expand-path", "~~cache/"}) -- meh
	if dir == nil then dir = "/tmp/" end
	return dir
end

-- Linux sockets are created with 600 perms.
--   https://github.com/mpv-player/mpv/blob/c438732b239bf4e7f3d574f8fcc141f92366018a/input/ipc-unix.c#L315
local function setup_ipc_socket(dev)
	local client_sock = mp.get_property("input-ipc-server")
	if client_sock and client_sock:len() > 0 then
		return client_sock
	end

	if dev then
		client_sock = "mpvsock42"
	else
		client_sock = "mpvsock" .. mp.get_property("pid", "0")
	end

	if PLATFORM == "windows" then
		client_sock = "\\\\.\\pipe\\" .. client_sock
		mp.set_property("input-ipc-server", client_sock)
	else
		client_sock = mp.utils.join_path(get_linux_socket_directory(), client_sock)
		mp.set_property("input-ipc-server", client_sock)
	end

	return client_sock
end

local function start_executable(client_sock)
	local executable = mp.utils.join_path(mp.command_native({"expand-path", "~~home/"}), "scripts/simulcast-mpv")
	if PLATFORM == "windows" then
		executable = executable .. ".exe"
	end

	return mp.command_native_async(
		{"run", executable, "client", "--client-sock", client_sock},
		function(success, result, error)
			if success then
				local msg = "simulcast success ("..executable..") | socket = "..client_sock
				mp.msg.info(msg)
			else
				local msg = "simulcast failed ("..executable..") | result = "..tostring(result).." | error = "..tostring(error)
				mp.osd_message(msg, 20)
				mp.msg.error(msg)
			end
		end
	)
end

---------------------------------------------------------------------------------------

local DEV = false

local timer = setup_heartbeat()
setup_keybinds()
local mpvsock = setup_ipc_socket(DEV)
if DEV then
	mp.osd_message(mpvsock, 5.0)
else
	local async_abort_table = start_executable(mpvsock)
end

--[[
-- testing
mp.command_native_async(
	{name="subprocess", args={mp.command_native({"expand-path", "~~home/"}) .. "/scripts/simulcast-mpv.exe", "input-reader"}, detach=true,},
	function(success, result, error)
		--mp.osd_message("simulcast success = "..tostring(success).." | result = "..tostring(result).." | error = "..tostring(error), 10)
	end
)
]]

