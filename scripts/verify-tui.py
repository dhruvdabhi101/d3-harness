#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/d3").resolve()


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        messages = body["messages"]
        if not any(message["role"] == "tool" for message in messages):
            delta = {"tool_calls": [{"index": 0, "id": "grep-1", "type": "function", "function": {
                "name": "grep", "arguments": json.dumps({"pattern": "needle", "path": "."})
            }}]}
            reason = "tool_calls"
        else:
            delta = {"content": "Found the saved needle."}
            reason = "stop"
        data = "data: " + json.dumps({"choices": [{"index": 0, "delta": delta, "finish_reason": None}]}) + "\n\n"
        data += "data: " + json.dumps({"choices": [{"delta": {}, "finish_reason": reason}]}) + "\n\ndata: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        self.wfile.write(data.encode())


server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()

with tempfile.TemporaryDirectory(prefix="d3-tui-") as temporary:
    root = Path(temporary)
    project = root / "project"
    project.mkdir()
    (project / "sample.txt").write_text("needle hidden-tool-output\n")
    config = root / "config.json"
    config.write_text(json.dumps({
        "provider": "mock", "model": "mock-model",
        "providers": {"mock": {"type": "openai", "base_url": f"http://127.0.0.1:{server.server_port}/v1", "api_key": "test"}}
    }))
    environment = os.environ.copy()
    environment["XDG_DATA_HOME"] = str(root / "data")
    for name in ["HARNESS_PROVIDER", "HARNESS_MODEL", "HARNESS_API_KEY"]:
        environment.pop(name, None)
    args = [str(binary), "--config", str(config), "-C", str(project)]
    subprocess.run(args + ["find needle"], env=environment, check=True, capture_output=True)
    socket = "d3-verify-" + str(os.getpid())

    def tmux(*args):
        return subprocess.check_output(["tmux", "-L", socket, *args], text=True)

    def screen():
        return tmux("capture-pane", "-p", "-t", "verify")

    def wait_for(expected):
        for _ in range(100):
            captured = screen()
            if expected in captured:
                return captured
            time.sleep(0.05)
        raise AssertionError(f"Missing {expected!r}\n{screen()}")

    command = "exec " + shlex.join(["env", "XDG_DATA_HOME=" + environment["XDG_DATA_HOME"], *args])
    try:
        tmux("new-session", "-d", "-s", "verify", "-x", "120", "-y", "40", command)
        initial = wait_for("mock/mock-model")
        assert "approval ask" in initial.splitlines()[-1], initial
        assert "Ctrl+B" not in initial and "Enter send" not in initial, initial
        tmux("send-keys", "-t", "verify", "-l", "/resume")
        tmux("send-keys", "-t", "verify", "Enter")
        wait_for("project conversations")
        tmux("send-keys", "-t", "verify", "Enter")
        restored = wait_for("Resumed project conversation")
        assert "Found the saved needle." in restored, restored
        assert "hidden-tool-output" not in restored, restored
        row = next(index for index, line in enumerate(restored.splitlines()) if "grep" in line)
        tmux("send-keys", "-t", "verify", "-l", f"\x1b[<0;8;{row + 1}M\x1b[<0;8;{row + 1}m")
        wait_for("hidden-tool-output")
        tmux("send-keys", "-t", "verify", "-l", f"\x1b[<0;8;{row + 1}M\x1b[<0;8;{row + 1}m")
        for _ in range(100):
            if "hidden-tool-output" not in screen():
                break
            time.sleep(0.05)
        assert "hidden-tool-output" not in screen(), screen()
        tmux("send-keys", "-t", "verify", "-l", "continue checking")
        tmux("send-keys", "-t", "verify", "Enter")
        wait_for("continue checking")
        wait_for("approval ask · ready")
        tmux("send-keys", "-t", "verify", "C-d")
        for _ in range(100):
            exited = subprocess.run(["tmux", "-L", socket, "has-session", "-t", "verify"], capture_output=True).returncode != 0
            if exited:
                break
            time.sleep(0.05)
        assert exited, "TUI did not exit"
        print("PASS. PTY resume picker, restored transcript, tool clicks, continuation, bottom status, and exit.")
    finally:
        subprocess.run(["tmux", "-L", socket, "kill-server"], capture_output=True)
        server.shutdown()
