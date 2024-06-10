from typing import Any

from langchain_core.runnables import ConfigurableField
from langserve import add_routes
from pydantic import BaseModel

# from langchain_core import ArcanAgent
from arcan.casters.ai.agents import ArcanAgent
from arcan.forge import app
from arcan.forge.api.routes.auth import fetch_session_from_header


class Input(BaseModel):
    input: str

class Output(BaseModel):
    output: Any


dynamic_spells_model = (
    ArcanAgent()
    .configurable_fields(
        user_id=ConfigurableField(
            id="user_id",
            name="Arcan AI User ID",
            description=("user_id Key for Arcan AI interactions"),
        ),
        access_token = ConfigurableField(
            id="token",
            name="Arcan AI Token",
            description=("token Key for Arcan AI interactions"),
        )
    )
    .with_types(input_type=Input, output_type=Output)
)

add_routes(
    app=app,
    runnable=dynamic_spells_model,
    per_req_config_modifier=fetch_session_from_header,
    path="/spells",
)