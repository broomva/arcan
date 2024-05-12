#%%
from typing import Any, List, Union

from dotenv import load_dotenv
from fastapi import Depends, FastAPI, Form, Request
from fastapi.responses import RedirectResponse
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langchain_core.runnables import Runnable
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field
from sqlalchemy.orm import Session

from arcan.ai.agents import ArcanSpellsAgent
from arcan.api.datamodels import get_db, get_db_context
from arcan.api.session import ArcanSession, run_agent

# %%
# from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

# from arcan.api.session.auth import requires_auth

# auth_scheme = HTTPBearer()

load_dotenv()

app = FastAPI()


# @app.get("/")
# def default():
#     return {
#         "message": "Check out the API documentation at http://arcanai.tech/api/docs"
#     }

@app.get("/")
async def redirect_root_to_docs():
    return RedirectResponse("/docs")


@app.get("/api/check")
async def index():
    return {"message": "Arcan is Running!"}


# @app.get("/api/chat/{user_id}")
# async def api_user_chat(user_id: str, query: str, db: Session = Depends(get_db)):
#     arcan_session = ArcanSession(db)
#     response = run_agent(session=arcan_session, user_id=user_id, query=query)
#     return {"response": response}


# @requires_auth
@app.get("/api/chat")
async def chat(user_id: str, query: str, db: Session = Depends(get_db)):
    arcan_session = ArcanSession(db)
    response = run_agent(session=arcan_session, user_id=user_id, query=query)
    return {"response": response}

#%%


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
    runnable=get_runnable().with_types(input_type=Input, output_type=Output).with_config({"run_name": "agent"}),
    path="/spells_agent",
    enable_feedback_endpoint=True,
)
