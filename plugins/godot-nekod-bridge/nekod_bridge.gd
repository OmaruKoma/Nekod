# Nekod Companion Bridge for Godot 4 (Autoload Node)
# Add this script to your mod/dev build as an Autoload (`NekodBridge`) to expose
# safe loopback property reads/writes with LIFO restoration on detach.
extends Node

const BIND_HOST := "127.0.0.1"
const LISTEN_PORT := 19543

@export var auth_token: String = "nekod_godot_bridge_token_16"

var _server := TCPServer.new()
var _peers: Array[StreamPeerTCP] = []
var _rollback_stack: Array[Dictionary] = []

func _ready() -> void:
	var err := _server.listen(LISTEN_PORT, BIND_HOST)
	if err == OK:
		print("[NekodGodot] Listening on %s:%d" % [BIND_HOST, LISTEN_PORT])
	else:
		push_warning("[NekodGodot] Failed to bind loopback port %d (err=%d)" % [LISTEN_PORT, err])

func _exit_tree() -> void:
	rollback_all()
	for peer in _peers:
		peer.disconnect_from_host()
	_peers.clear()
	_server.stop()

func _process(_delta: float) -> void:
	while _server.is_connection_available():
		var client := _server.take_connection()
		if client != null:
			_peers.append(client)

	for i in range(_peers.size() - 1, -1, -1):
		var peer := _peers[i]
		peer.poll()
		var status := peer.get_status()
		if status == StreamPeerTCP.STATUS_ERROR or status == StreamPeerTCP.STATUS_NONE:
			_peers.remove_at(i)
			continue
		if status != StreamPeerTCP.STATUS_CONNECTED:
			continue
		var available := peer.get_available_bytes()
		if available <= 0:
			continue
		var line := peer.get_utf8_string(available).strip_edges()
		if line.is_empty():
			continue
		var reply := _handle_json_line(line)
		peer.put_data((JSON.stringify(reply) + "\n").to_utf8_buffer())
		peer.disconnect_from_host()
		_peers.remove_at(i)

func _handle_json_line(raw: String) -> Dictionary:
	var parsed = JSON.parse_string(raw)
	if typeof(parsed) != TYPE_DICTIONARY:
		return {"ok": false, "error": "invalid json request"}
	if str(parsed.get("token", "")) != auth_token:
		return {"ok": false, "error": "unauthorized"}

	var op := str(parsed.get("op", ""))
	var params: Dictionary = parsed.get("params", {})
	match op:
		"ping":
			return {
				"ok": true,
				"data": {
					"engine": "godot",
					"version": Engine.get_version_info().get("string", "4.x"),
					"pid": OS.get_process_id(),
				},
			}
		"set_node_property":
			var node_path := str(params.get("node_path", ""))
			var property := str(params.get("property", ""))
			var node := get_tree().root.get_node_or_null(NodePath(node_path))
			if node == null or not property in node:
				return {"ok": false, "error": "node or property not found"}
			var prev = node.get(property)
			_rollback_stack.append({"node_path": node_path, "property": property, "old_value": prev})
			node.set(property, params.get("value"))
			return {"ok": true, "data": {"previous": prev}}
		"rollback_all":
			var count := rollback_all()
			return {"ok": true, "data": {"restored": count}}
		_:
			return {"ok": false, "error": "unknown op: " + op}

func rollback_all() -> int:
	var restored := 0
	while not _rollback_stack.is_empty():
		var item: Dictionary = _rollback_stack.pop_back()
		var node := get_tree().root.get_node_or_null(NodePath(str(item["node_path"])))
		if node != null:
			node.set(str(item["property"]), item["old_value"])
			restored += 1
	return restored
