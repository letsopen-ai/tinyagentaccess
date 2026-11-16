#!/usr/bin/env bash
# Linux orchestration for both official SDK interop scripts, using isolated Xvfb.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p tinyagentaccess -p taa-forward --bins
MCP_PYTHON="${MCP_PYTHON:-/tmp/mcpvenv/bin/python}"
export MCP_PYTHON
xvfb-run -a -s '-screen 0 1920x1080x24 -nolisten tcp' "$MCP_PYTHON" - <<'PY'
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

root = Path.cwd()
with tempfile.TemporaryDirectory(prefix="taa-m4-") as temporary:
    directory = Path(temporary)
    redirect = directory / "redirection.json"
    redirect.write_text(json.dumps({"mcpServers": {"fixture": {
        "command": str(root / "target/debug/taa-fake-mcp-server"), "args": ["good"]
    }}}), encoding="utf-8")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    config = directory / "server.toml"
    config.write_text(f'''bind = "127.0.0.1:{port}"
[agent_access]
forward_mcp_tools = true
[forwarding]
config_path = {json.dumps(str(redirect))}
[platform]
linux_backend = "x11"
''', encoding="utf-8")
    environment = dict(os.environ, XDG_SESSION_TYPE="x11")
    environment.pop("WAYLAND_DISPLAY", None)
    url = f"http://127.0.0.1:{port}/mcp"
    with (directory / "stderr.log").open("w+") as log:
        process = subprocess.Popen([str(root / "target/debug/tinyagentaccess"), "serve", "--config", str(config)], env=environment, stderr=log)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError("server exited before readiness")
                try:
                    urllib.request.urlopen(url, timeout=0.2)
                except urllib.error.HTTPError as error:
                    if error.code == 405:
                        break
                except OSError:
                    pass
                time.sleep(0.05)
            else:
                raise RuntimeError("server readiness timeout")
            for script in ["interop_python_sdk.py", "interop_forwarding.py"]:
                subprocess.run([os.environ["MCP_PYTHON"], str(root / "scripts" / script), "--url", url], check=True, env=environment)
        except BaseException:
            log.flush()
            log.seek(0)
            print(log.read())
            raise
        finally:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
PY
