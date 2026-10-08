import argparse
import json
import os
import time

from azure.ai.projects import AIProjectClient
from azure.ai.projects.models import (
    FabricIQPreviewTool,
    PromptAgentDefinition,
)
from azure.identity import AzureCliCredential


DEFAULT_INSTRUCTIONS = """You are the Governed Floor fleet management analyst.

Use the factory-fleet-analyst Microsoft Fabric data agent for every question about
factory incidents, alarms, lifecycle events, actions, trends, or fleet status.

Rules:
- Never invent counts, trends, causes, timestamps, or action outcomes.
- Distinguish event rows from distinct incidents.
- State the time range represented by the available data.
- For a seven-day fleet brief, call the Fabric tool separately with these focused
  questions:
  1. "For the latest seven-day period represented in the data, return the
     represented time range, lifecycle event row count, distinct incident count,
     factory count, and the same counts for the preceding seven days."
  2. "For the latest seven-day period represented in the data, count incidents by
     factory and their latest incident status."
  3. "For the latest seven-day period represented in the data, show detected event
     types that affected more than one factory."
  4. "For the latest seven-day period represented in the data, show correlated
     incident activity by factory and upstream correlation ID."
  5. "For the latest seven-day period represented in the data, show governed action
     outcomes by factory."
- Do not substitute status/severity co-occurrence for correlation evidence. Describe
  correlated activity only when the Fabric result contains a correlation ID.
- Report governed action outcomes from the focused governed-action result, including
  executed and rejected outcomes when present.
- If a focused Fabric result is unexpectedly empty, repeat that exact focused
  question once before treating the evidence as unavailable. Reconcile conflicting
  tool results in favor of a successful non-empty result from the same question.
- Prefer multiple simple Fabric questions over one complex all-in-one question.
- If one Fabric result omits a requested section, make another focused tool call before
  stating that the data is unavailable.
- Treat Fabric as an analytical copy of governed lifecycle events, not as machine authority.
- Do not expose operator identities, raw alarm text, credentials, model reasoning, or manual excerpts.
- If the data is insufficient, say what is missing instead of guessing.
- For briefs, organize the response as: Executive summary, Evidence, Risks, Recommended next actions.
- Keep recommendations advisory and require governed operator approval for machine actions.
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Create or update the Governed Floor Foundry fleet agent."
    )
    parser.add_argument(
        "--project-endpoint",
        default=os.getenv("FOUNDRY_PROJECT_ENDPOINT"),
        help="Foundry project endpoint.",
    )
    parser.add_argument(
        "--model",
        default=os.getenv("FOUNDRY_AGENT_MODEL", "factory-expert-gpt54"),
        help="Foundry model deployment used to orchestrate the agent.",
    )
    parser.add_argument(
        "--connection-name",
        default=os.getenv(
            "FOUNDRY_FABRIC_CONNECTION_NAME", "factory-fabric-iq"
        ),
        help="Foundry Fabric IQ project connection.",
    )
    parser.add_argument(
        "--agent-name",
        default=os.getenv("FOUNDRY_FLEET_AGENT_NAME", "factory-fleet-manager"),
        help="Prompt agent name.",
    )
    parser.add_argument(
        "--smoke-test",
        action="store_true",
        help="Invoke the new version against live Fabric data.",
    )
    parser.add_argument(
        "--question",
        default=(
            "Summarize the current governed factory incident activity. Report event "
            "rows, distinct incidents, latest status, and any approved or rejected "
            "actions. Do not infer data that is not present."
        ),
        help="Smoke-test question.",
    )
    return parser.parse_args()


def wait_for_response(openai, response):
    while response.status in {"queued", "in_progress"}:
        time.sleep(2)
        response = openai.responses.retrieve(response.id)
    return response


def main() -> None:
    args = parse_args()
    if not args.project_endpoint:
        raise SystemExit(
            "FOUNDRY_PROJECT_ENDPOINT or --project-endpoint must be provided."
        )
    project = AIProjectClient(
        endpoint=args.project_endpoint,
        credential=AzureCliCredential(),
    )
    connection = project.connections.get(args.connection_name)
    agent = project.agents.create_version(
        agent_name=args.agent_name,
        definition=PromptAgentDefinition(
            model=args.model,
            instructions=DEFAULT_INSTRUCTIONS,
            tools=[
                FabricIQPreviewTool(
                    project_connection_id=connection.id,
                    require_approval="never",
                )
            ],
        ),
    )

    result = {
        "agent_id": agent.id,
        "agent_name": agent.name,
        "agent_version": agent.version,
        "connection_id": connection.id,
        "model": args.model,
    }

    if args.smoke_test:
        openai = project.get_openai_client()
        response = openai.responses.create(
            input=args.question,
            background=True,
            extra_body={
                "agent_reference": {
                    "name": agent.name,
                    "type": "agent_reference",
                }
            },
        )
        response = wait_for_response(openai, response)
        if response.status != "completed":
            raise RuntimeError(
                f"Agent smoke test ended with status {response.status}: "
                f"{getattr(response, 'error', None)}"
            )
        result["smoke_test"] = {
            "response_id": response.id,
            "status": response.status,
            "output": response.output_text,
        }

    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
