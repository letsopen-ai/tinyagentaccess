#!/usr/bin/env python3
"""Wire compatibility check; install mcp==2.1.1 and run against `serve`.

/mcp requires a bearer token. Set TAA_TOKEN in this client process (never in
the server's environment) or pass --token-file. The token goes in an
`Authorization: Bearer` header on an httpx2 client handed to the transport;
mcp==2.1.1's streamable_http_client has no `headers=` parameter.
"""
import argparse
import asyncio
import json
import os
import sys
import httpx2
from mcp.client import Client
from mcp.client.streamable_http import streamable_http_client


def load_token(token_file=None):
    """Token from --token-file, else the TAA_TOKEN environment variable."""
    if token_file:
        with open(token_file, encoding="ascii") as handle:
            token = handle.read().strip()
    else:
        token = os.environ.get("TAA_TOKEN", "").strip()
    if not token:
        sys.exit("error: set TAA_TOKEN or pass --token-file (see README, Authentication)")
    return token


def bearer_client(token):
    return httpx2.AsyncClient(
        headers={"Authorization": f"Bearer {token}"},
        follow_redirects=True,
        timeout=httpx2.Timeout(30.0, read=300.0),
    )


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


async def run(url, token):
    for mode in ["legacy", "2026-07-28"]:
        async with bearer_client(token) as http, Client(
            streamable_http_client(url, http_client=http), mode=mode
        ) as client:
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
            sequence = await client.call_tool("agentaccess___execute_sequence", {
                **args, "actions": [{"type": "agentaccess___wait", "duration": 0.001}]
            })
            assert not sequence.is_error
            assert any(block.type == "image" for block in sequence.content)
            rejected = await client.call_tool("agentaccess___execute_sequence", {
                **args, "actions": [{"type": "wait", "duration": 0}]
            })
            assert rejected.is_error
            assert all(block.type == "text" for block in rejected.content)
            if modern:
                assert session_id(sequence) == args["mcpSessionId"]
                assert session_id(rejected) == args["mcpSessionId"]
                assert session_id(bad) == args["mcpSessionId"]
                for _ in range(2):
                    closed = await client.call_tool("agentaccess___close_session", args)
                    assert not closed.is_error
                    assert session_id(closed) == args["mcpSessionId"]
            print(f"PASS {mode}: list, session info, wait, sequence success/rejection, error, close")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8765/mcp")
    parser.add_argument("--token-file", help="read the token here instead of TAA_TOKEN")
    args = parser.parse_args()
    asyncio.run(run(args.url, load_token(args.token_file)))
