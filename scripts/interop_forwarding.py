#!/usr/bin/env python3
"""Official mcp==2.1.1 SDK forwarding check against a configured running server.

Needs TAA_TOKEN in this process or --token-file (see interop_python_sdk.py)."""
import argparse
import asyncio
from mcp.client import Client
from mcp.client.streamable_http import streamable_http_client
from interop_python_sdk import bearer_client, load_token, session_id


async def run(url, token):
    for mode in ["legacy", "2026-07-28"]:
        async with bearer_client(token) as http, Client(
            streamable_http_client(url, http_client=http), mode=mode
        ) as client:
            listed = await client.list_tools()
            tool = next(t for t in listed.tools if t.name == "forwarded___fixture___echo")
            assert tool.output_schema["type"] == "object"
            modern = mode == "2026-07-28"
            assert ("mcpSessionId" in tool.input_schema["properties"]) == modern
            args = {"text": "SDK forwarding"}
            if modern:
                args["mcpSessionId"] = "NEW"
            result = await client.call_tool(tool.name, args)
            assert not result.is_error
            assert result.content[0].text == "SDK forwarding"
            if modern:
                args["mcpSessionId"] = session_id(result)
            again = await client.call_tool("forwarded___fixture___odd___name", args)
            assert not again.is_error
            bad = await client.call_tool("forwarded___fixture___missing", args)
            assert bad.is_error and bad.content[0].text.startswith("CLIENT_ERROR:")
            if modern:
                closed = await client.call_tool("agentaccess___close_session", {
                    "mcpSessionId": args["mcpSessionId"]
                })
                assert not closed.is_error
            print(f"PASS forwarding {mode}: schema, echo, ambiguous name, unknown tool, NEW/close")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8765/mcp")
    parser.add_argument("--token-file", help="read the token here instead of TAA_TOKEN")
    args = parser.parse_args()
    asyncio.run(run(args.url, load_token(args.token_file)))
