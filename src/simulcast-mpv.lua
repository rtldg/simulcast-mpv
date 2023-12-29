-- SPDX-License-Identifier: WTFPL

local platform = mp.get_property("platform")
local executable = mp.command_native({"expand-path", "~~home/"}).. "/scripts/simulcast-mpv"
--mp.osd_message(scripts_dir, 10)

local client_sock = "mpvsock"..mp.get_property("window-id", "42")
if platform == "windows" then
	mp.set_property("input-ipc-server", "\\\\.\\pipe\\"..client_sock)
	executable = executable .. ".exe"
else
	mp.set_property("input-ipc-server", "/tmp/"..client_sock)
end

local async_abort_table = mp.command_native_async(
	{"run", executable, "client", "--client-sock", client_sock},
	function(success, result, error)
		--mp.osd_message("simulcast success = "..tostring(success).." | result = "..tostring(result).." | error = "..tostring(error), 10)
	end
)
