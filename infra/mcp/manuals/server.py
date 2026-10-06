import asyncio
import json
import os
import time
from pathlib import Path
from typing import Any

import jwt
import uvicorn
from mcp.server import MCPServer
from mcp.server.auth.provider import AccessToken, TokenVerifier
from mcp.server.auth.settings import AuthSettings
from pydantic import AnyHttpUrl

TENANT_ID = os.environ["TENANT_ID"]
AUDIENCE = os.environ["AUDIENCE"]
MCP_PUBLIC_URL = os.environ["MCP_PUBLIC_URL"].rstrip("/")
KNOWLEDGE_PATH = Path(os.environ.get("KNOWLEDGE_PATH", "/app/knowledge"))
ALLOWED_ROLES = {"EdgeRAGDeveloper", "EdgeRAGEndUser"}
ISSUERS = {
    f"https://login.microsoftonline.com/{TENANT_ID}/v2.0",
    f"https://sts.windows.net/{TENANT_ID}/",
}


class EntraTokenVerifier(TokenVerifier):
    def __init__(self) -> None:
        self.jwks = jwt.PyJWKClient(
            f"https://login.microsoftonline.com/{TENANT_ID}/discovery/v2.0/keys",
            cache_keys=True,
        )

    async def verify_token(self, token: str) -> AccessToken | None:
        try:
            signing_key = await asyncio.to_thread(self.jwks.get_signing_key_from_jwt, token)
            claims = jwt.decode(
                token,
                signing_key.key,
                algorithms=["RS256"],
                options={"verify_aud": False},
                leeway=60,
            )
        except jwt.PyJWTError:
            return None

        audience = claims.get("aud")
        if audience not in {AUDIENCE, f"api://{AUDIENCE}"}:
            return None
        if claims.get("tid") != TENANT_ID or claims.get("iss") not in ISSUERS:
            return None

        roles = set(claims.get("roles", []))
        scopes = set(str(claims.get("scp", "")).split())
        if not roles.intersection(ALLOWED_ROLES):
            return None

        client_id = (
            claims.get("azp")
            or claims.get("appid")
            or claims.get("oid")
            or "entra-principal"
        )
        return AccessToken(
            token=token,
            client_id=client_id,
            scopes=sorted(roles | scopes),
            expires_at=int(claims.get("exp", time.time() + 300)),
            resource=f"{MCP_PUBLIC_URL}/mcp",
            subject=claims.get("sub") or claims.get("oid"),
            claims=claims,
        )


def load_documents() -> list[dict[str, str]]:
    documents = []
    for path in sorted(KNOWLEDGE_PATH.glob("*.md")):
        content = path.read_text(encoding="utf-8")
        title = next(
            (
                line.removeprefix("# ").strip()
                for line in content.splitlines()
                if line.startswith("# ")
            ),
            path.stem,
        )
        documents.append(
            {
                "title": title,
                "source": path.name,
                "content": content,
                "searchable": f"{title} {content}".lower(),
            }
        )
    if not documents:
        raise RuntimeError(f"No Markdown manuals found under {KNOWLEDGE_PATH}")
    return documents


DOCUMENTS = load_documents()

mcp = MCPServer(
    "Factory maintenance manuals",
    instructions=(
        "Search synthetic factory maintenance manuals. Return document titles, "
        "source filenames, and excerpts so responses can be cited."
    ),
    token_verifier=EntraTokenVerifier(),
    auth=AuthSettings(
        issuer_url=AnyHttpUrl(f"https://login.microsoftonline.com/{TENANT_ID}/v2.0"),
        resource_server_url=AnyHttpUrl(f"{MCP_PUBLIC_URL}/mcp"),
        required_scopes=[],
        validate_token_resource=False,
    ),
)


class AgenticValidationCompatibility:
    def __init__(self, app: Any) -> None:
        self.app = app

    async def __call__(self, scope: dict[str, Any], receive: Any, send: Any) -> None:
        if (
            scope["type"] == "http"
            and scope["method"] == "POST"
            and scope["path"] == "/mcp"
            and not any(
                name.lower() == b"authorization" for name, _ in scope.get("headers", [])
            )
        ):
            messages = []
            body = b""
            while True:
                message = await receive()
                messages.append(message)
                body += message.get("body", b"")
                if not message.get("more_body", False):
                    break

            try:
                request = json.loads(body)
            except (json.JSONDecodeError, UnicodeDecodeError):
                request = {}

            if request.get("method") == "initialize":
                protocol_version = request.get("params", {}).get(
                    "protocolVersion", "2025-03-26"
                )
                payload = json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": request.get("id"),
                        "result": {
                            "capabilities": {
                                "prompts": {"listChanged": False},
                                "resources": {
                                    "listChanged": False,
                                    "subscribe": False,
                                },
                                "tools": {"listChanged": False},
                            },
                            "instructions": (
                                "Authentication is required to discover and call "
                                "factory maintenance tools."
                            ),
                            "protocolVersion": protocol_version,
                            "serverInfo": {
                                "name": "Factory maintenance manuals",
                                "version": "",
                            },
                        },
                    }
                ).encode()
                await send(
                    {
                        "type": "http.response.start",
                        "status": 200,
                        "headers": [
                            (b"content-type", b"application/json"),
                            (b"content-length", str(len(payload)).encode()),
                        ],
                    }
                )
                await send({"type": "http.response.body", "body": payload})
                return

            pending = iter(messages)

            async def replay_receive() -> dict[str, Any]:
                return next(pending)

            await self.app(scope, replay_receive, send)
            return

        await self.app(scope, receive, send)


def tokenize(value: str) -> set[str]:
    return {
        token.lower()
        for token in "".join(
            character if character.isalnum() else " " for character in value
        ).split()
        if len(token) >= 4
    }


def excerpt(content: str, terms: set[str]) -> str:
    paragraphs = [
        " ".join(paragraph.split())
        for paragraph in content.split("\n\n")
        if paragraph.strip() and not paragraph.lstrip().startswith("#")
    ]
    ranked = sorted(
        enumerate(paragraphs),
        key=lambda item: (
            -sum(item[1].lower().count(term) for term in terms),
            item[0],
        ),
    )
    selected_indexes = sorted(index for index, _ in ranked[:2])
    selected = " ".join(paragraphs[index] for index in selected_indexes)
    return " ".join(selected.split()[:140])


@mcp.tool()
def search_factory_manuals(query: str, max_results: int = 3) -> dict[str, Any]:
    """Search the synthetic factory manuals for maintenance evidence."""
    terms = tokenize(query)
    ranked = []
    for document in DOCUMENTS:
        score = sum(document["searchable"].count(term) for term in terms)
        ranked.append((score, document))
    ranked.sort(key=lambda item: (-item[0], item[1]["title"]))
    matches = [item for item in ranked if item[0] > 0][: max(1, min(max_results, 5))]
    if not matches:
        matches = ranked[: min(2, len(ranked))]
    return {
        "query": query,
        "results": [
            {
                "title": document["title"],
                "source": document["source"],
                "excerpt": excerpt(document["content"], terms),
                "score": score,
            }
            for score, document in matches
        ],
        "synthetic_data": True,
    }


@mcp.tool()
def list_factory_manuals() -> dict[str, Any]:
    """List the synthetic factory manuals available to the agent."""
    return {
        "documents": [
            {"title": document["title"], "source": document["source"]}
            for document in DOCUMENTS
        ],
        "synthetic_data": True,
    }


if __name__ == "__main__":
    app = mcp.streamable_http_app(
        streamable_http_path="/mcp",
        stateless_http=True,
        host="0.0.0.0",
    )
    uvicorn.run(AgenticValidationCompatibility(app), host="0.0.0.0", port=8000)
