import json
import logging
import os
import secrets
from contextlib import asynccontextmanager

import httpx
from azure.identity.aio import DefaultAzureCredential
from fastapi import FastAPI, Header, HTTPException, Request
from fastapi.responses import Response, StreamingResponse
from starlette.background import BackgroundTask

FOUNDRY_ENDPOINT = os.environ["FOUNDRY_ENDPOINT"].rstrip("/")
BRIDGE_SHARED_SECRET = os.environ["BRIDGE_SHARED_SECRET"]
TOKEN_SCOPE = "https://cognitiveservices.azure.com/.default"
logger = logging.getLogger("factory-model-bridge")

credential: DefaultAzureCredential
client: httpx.AsyncClient


@asynccontextmanager
async def lifespan(_: FastAPI):
    global credential, client
    credential = DefaultAzureCredential()
    client = httpx.AsyncClient(timeout=httpx.Timeout(300.0, connect=20.0))
    yield
    await client.aclose()
    await credential.close()


app = FastAPI(lifespan=lifespan)


def validate_bridge_auth(
    api_key: str | None,
    authorization: str | None,
    x_api_key: str | None,
) -> None:
    bearer = (
        authorization.removeprefix("Bearer ").strip()
        if authorization and authorization.startswith("Bearer ")
        else None
    )
    supplied = api_key or x_api_key or bearer
    if not supplied or not secrets.compare_digest(supplied, BRIDGE_SHARED_SECRET):
        raise HTTPException(status_code=401, detail="Invalid bridge credential")


@app.get("/healthz")
async def health() -> dict[str, str]:
    return {"status": "ready"}


@app.post("/openai/deployments/{deployment}/chat/completions")
async def chat_completions(
    deployment: str,
    request: Request,
    api_version: str = "2024-10-21",
    api_key: str | None = Header(default=None, alias="api-key"),
    authorization: str | None = Header(default=None),
    x_api_key: str | None = Header(default=None, alias="x-api-key"),
):
    validate_bridge_auth(api_key, authorization, x_api_key)
    token = await credential.get_token(TOKEN_SCOPE)
    body = await request.body()
    try:
        request_payload = json.loads(body)
        request_fields = sorted(request_payload.keys())
        if request_payload.get("temperature") not in (None, 1, 1.0):
            request_payload.pop("temperature")
            logger.info(
                "Removed unsupported non-default temperature for deployment=%s",
                deployment,
            )
        if "top_p" in request_payload:
            request_payload.pop("top_p")
            logger.info("Removed unsupported top_p for deployment=%s", deployment)
        body = json.dumps(request_payload).encode()
    except (json.JSONDecodeError, AttributeError):
        request_fields = []
    target = (
        f"{FOUNDRY_ENDPOINT}/openai/deployments/{deployment}/chat/completions"
        f"?api-version={api_version}"
    )
    upstream_request = client.build_request(
        "POST",
        target,
        headers={
            "Authorization": f"Bearer {token.token}",
            "Content-Type": request.headers.get("content-type", "application/json"),
            "Accept": request.headers.get("accept", "application/json"),
        },
        content=body,
    )
    upstream = await client.send(upstream_request, stream=True)
    response_headers = {
        key: value
        for key, value in upstream.headers.items()
        if key.lower() in {"content-type", "x-request-id", "apim-request-id"}
    }
    if upstream.is_error:
        content = await upstream.aread()
        await upstream.aclose()
        logger.error(
            "Foundry rejected chat request: status=%s deployment=%s fields=%s response=%s",
            upstream.status_code,
            deployment,
            request_fields,
            content.decode("utf-8", errors="replace")[:2000],
        )
        return Response(
            content=content,
            status_code=upstream.status_code,
            headers=response_headers,
        )
    if "text/event-stream" in upstream.headers.get("content-type", ""):
        return StreamingResponse(
            upstream.aiter_raw(),
            status_code=upstream.status_code,
            headers=response_headers,
            background=BackgroundTask(upstream.aclose),
        )
    content = await upstream.aread()
    await upstream.aclose()
    return Response(
        content=content,
        status_code=upstream.status_code,
        headers=response_headers,
    )
