from typing import Any, List, Union

from fastapi import FastAPI
from fastapi.responses import RedirectResponse
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langchain_core.runnables import Runnable
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field

from arcan.ai.agents import ArcanSpellsAgent
from arcan.api import app


@app.get("/")
async def redirect_root_to_docs():
    return RedirectResponse("/docs")


# We need to add these input/output schemas because the current AgentExecutor
# is lacking in schemas.
class Input(BaseModel):
    input: str
    chat_history: List[Union[HumanMessage, AIMessage, FunctionMessage]] = Field(
        ...,
        extra={"widget": {"type": "chat", "input": "input", "output": "output"}},
    )


class Output(BaseModel):
    output: Any


def get_runnable() -> Runnable:
    return ArcanSpellsAgent().agent_executor


add_routes(
    app=app,
    runnable=get_runnable()
    .with_types(input_type=Input, output_type=Output)
    .with_config({"run_name": "agent"}),
    path="/chat",
    enable_feedback_endpoint=True,
    # enable_public_trace_link_endpoint=True,
    # playground_type="chat",
)

if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=8000)
