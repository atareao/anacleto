"""Minimal MCP server over stdio for integration testing.

Responds to JSON-RPC 2.0 requests per the MCP protocol.
No dependencies beyond the Python standard library.
"""

import json
import sys


def make_response(request_id, result=None, error=None):
    msg = {"jsonrpc": "2.0", "id": request_id}
    if error:
        msg["error"] = error
    else:
        msg["result"] = result
    return msg


def handle_request(request):
    method = request.get("method")
    req_id = request.get("id")

    # Notifications have no id — no response needed
    if req_id is None:
        return None

    if method == "initialize":
        return make_response(
            req_id,
            {
                "protocolVersion": "2024-11-05",
                "serverInfo": {"name": "mock-server", "version": "1.0.0"},
                "capabilities": {
                    "tools": True,
                    "resources": True,
                },
            },
        )

    if method == "tools/list":
        return make_response(
            req_id,
            {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo back the message parameter",
                        "input_schema": {
                            "type": "object",
                            "properties": {
                                "message": {
                                    "type": "string",
                                    "description": "Message to echo back",
                                },
                            },
                            "required": ["message"],
                        },
                    },
                ],
            },
        )

    if method == "tools/call":
        params = request.get("params", {})
        name = params.get("name")
        arguments = params.get("arguments", {})

        if name == "echo":
            message = arguments.get("message", "")
            return make_response(
                req_id,
                {
                    "content": [
                        {"type": "text", "text": message},
                    ],
                    "isError": False,
                },
            )

        return make_response(
            req_id,
            error={
                "code": -32601,
                "message": f"Tool not found: {name}",
            },
        )

    if method == "resources/list":
        return make_response(req_id, {"resources": []})

    # Unknown method
    return make_response(
        req_id,
        error={
            "code": -32601,
            "message": f"Method not found: {method}",
        },
    )


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue

        response = handle_request(request)
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
