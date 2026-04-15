#!/usr/bin/env python3
"""
PostToolUse hook: run `cargo check` after file-editing tools to catch compile errors early.
Receives hook JSON on stdin; only runs cargo check when a Rust source file was modified.
"""
import json
import subprocess
import sys

FILE_EDIT_TOOLS = {
    "replace_string_in_file",
    "create_file",
    "multi_replace_string_in_file",
}

try:
    payload = json.load(sys.stdin)
except (json.JSONDecodeError, ValueError):
    sys.exit(0)

tool_name = payload.get("tool_name", "")
if tool_name not in FILE_EDIT_TOOLS:
    sys.exit(0)

# Only trigger when a .rs file was touched
tool_input = payload.get("tool_input", {})
file_path = tool_input.get("filePath", "") or tool_input.get("file_path", "")
if not file_path.endswith(".rs"):
    sys.exit(0)

result = subprocess.run(
    ["cargo", "check", "--message-format=short"],
    capture_output=True,
    text=True,
)

output = (result.stdout + result.stderr).strip()
if result.returncode != 0:
    # Return a systemMessage so the agent sees the errors
    response = {
        "systemMessage": f"cargo check failed after editing {file_path}:\n\n{output}"
    }
    print(json.dumps(response))
    sys.exit(0)  # non-blocking; agent decides how to respond
