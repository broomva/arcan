#%%
from typing import Any, List, Union

from dotenv import load_dotenv
from fastapi import Depends, FastAPI, Form, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field
from sqlalchemy.orm import Session

from arcan.ai.agents import ArcanSpellsAgent
from arcan.ai.llm import LLM
from arcan.api.datamodels import get_db, get_db_context
from arcan.api.session import ArcanSession, run_agent

# %%
# from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

# from arcan.api.session.auth import requires_auth

# auth_scheme = HTTPBearer()

load_dotenv()

app = FastAPI()



# Set all CORS enabled origins
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["*"],
)


@app.get("/")
async def redirect_root_to_docs():
    return RedirectResponse("/docs")


@app.get("/api/check")
async def index():
    return {"message": "Arcan is Running!"}


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


add_routes(
    app=app,
    runnable=ArcanSpellsAgent().agent_executor.with_types(input_type=Input, output_type=Output).with_config({"run_name": "agent"}),
    path="/spells_agent",
    enable_feedback_endpoint=True,
)

add_routes(
    app,
    LLM(provider='ChatOpenAI').llm,
    path="/openai",
)

add_routes(
    app,
    LLM(provider='ChatGroq').llm,
    path="/groq",
)

add_routes(
    app,
    LLM(provider='ChatTogetherAI').llm,
    path="/together",
)