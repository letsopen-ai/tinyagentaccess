#!/usr/bin/env python3
"""Wire compatibility check; install mcp==2.1.1 and run against `serve`."""
import argparse
import asyncio
import json
from mcp.client import Client
from mcp.client.streamable_http import streamable_http_client


def session_id(result):
    for block in result.content:
        if block.type == "text":
            try:
                value = json.loads(block.text)
            except (ValueError, TypeError):
                continue
            if isinstance(value, dict) and "mcpSessionId" in value:
                return value["mcpSessionId"]
    raise AssertionError("missing mcpSessionId text block")


async def run(url):
    for mode in ["legacy", "2026-07-28"]:
        async with Client(streamable_http_client(url), mode=mode) as client:
            tools = await client.list_tools()
            names = {tool.name for tool in tools.tools}
            assert "agentaccess___wait" in names
            modern = mode == "2026-07-28"
            assert ("agentaccess___close_session" in names) == modern
            args = {"mcpSessionId": "NEW"} if modern else {}
            info = await client.call_tool("agentaccess___get_session_info", args)
            assert not info.is_error
            if modern:
                args["mcpSessionId"] = session_id(info)
            wait = await client.call_tool("agentaccess___wait", {**args, "duration": 0.001})
            assert not wait.is_error
            bad = await client.call_tool("agentaccess___wait", {**args, "duration": 0})
            assert bad.is_error
            if modern:
                assert session_id(bad) == args["mcpSessionId"]
                for _ in range(2):
                    closed = await client.call_tool("agentaccess___close_session", args)
                    assert not closed.is_error
                    assert session_id(closed) == args["mcpSessionId"]
            print(f"PASS {mode}: list, session info, wait, error, close")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8765/mcp")
    asyncio.run(run(parser.parse_args().url))
