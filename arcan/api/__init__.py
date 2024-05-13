# %%
from datetime import datetime, timedelta, timezone
from typing import Annotated, Any, Dict, List, Optional, Union

from dotenv import load_dotenv
from fastapi import Depends, FastAPI, Form, HTTPException, Request, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse

# %%
from fastapi.security import (
    HTTPAuthorizationCredentials,
    HTTPBearer,
    OAuth2PasswordBearer,
    OAuth2PasswordRequestForm,
)
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field
from pydantic import BaseModel
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.orm import Session
from typing_extensions import Annotated

from arcan.ai.agents import ArcanSpellsAgent
from arcan.ai.llm import LLM
from arcan.api.datamodel import get_db, get_db_context
from arcan.api.datamodel.chat_history import ChatHistory
from arcan.api.datamodel.conversation import Conversation
from arcan.api.datamodel.user import (
    ACCESS_TOKEN_EXPIRE_MINUTES,
    TokenModel,
    User,
    UserInDB,
    UserModel,
    UserRepository,
    UserService,
    oauth2_scheme,
    pwd_context,
)
from arcan.api.session import ArcanSession, run_agent

# from arcan.api.session.auth import requires_auth
from arcan.spells.vector_search import (
    get_per_user_retriever,
    per_req_config_modifier,
    pgVectorStore,
)

auth_scheme = HTTPBearer()

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


# %%


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
    runnable=ArcanSpellsAgent(
        # database=SQLDatabase.from_uri(os.environ.get("SQLALCHEMY_URL"))
    )
    .agent_executor.with_types(input_type=Input, output_type=Output)
    .with_config({"run_name": "agent"}),
    path="/spells_agent",
    enable_feedback_endpoint=True,
)

add_routes(
    app,
    LLM(provider="ChatOpenAI").llm,
    per_req_config_modifier=per_req_config_modifier,
    path="/openai",
)

add_routes(
    app,
    LLM(provider="ChatGroq").llm,
    per_req_config_modifier=per_req_config_modifier,
    path="/groq",
)

add_routes(
    app,
    LLM(provider="ChatTogetherAI").llm,
    path="/together",
)


@app.post("/token")
async def login_for_access_token(
    form_data: Annotated[OAuth2PasswordRequestForm, Depends()],
    session: Session = Depends(get_db),
) -> TokenModel:
    user_repo = UserRepository(session)
    user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
    user = user_interface.authenticate_user(form_data.username, form_data.password)
    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Incorrect username or password",
            headers={"WWW-Authenticate": "Bearer"},
        )
    access_token_expires = timedelta(minutes=ACCESS_TOKEN_EXPIRE_MINUTES)
    access_token = user_interface.create_access_token(
        data={"sub": user.username}, expires_delta=access_token_expires
    )
    return TokenModel(
        id=1,
        access_token=access_token,
        token_type="bearer",
        user_id=user.username,
        user=user,
    )


async def get_current_active_user_from_request(
    request: Request, session: Session = Depends(get_db)
) -> UserModel:
    """Get the current active user from the request."""
    user_repo = UserRepository(session)
    user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
    token = await oauth2_scheme(request)
    user = user_interface.get_current_user(token=token)
    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid authentication credentials",
            headers={"WWW-Authenticate": "Bearer"},
        )
    if user.disabled:
        raise HTTPException(status_code=400, detail="Inactive user")
    return user


@app.get("/users/me/", response_model=UserModel)
async def read_users_me(
    current_user: Annotated[UserModel, Depends(get_current_active_user_from_request)],
):
    return current_user


add_routes(
    app,
    get_per_user_retriever(vectorstore=pgVectorStore().get_vector_store()),
    per_req_config_modifier=per_req_config_modifier,
    enabled_endpoints=["invoke"],
)

# %%


# @requires_auth
@app.get("/api/chat")
async def chat(
    user_id: str,
    query: str,
    current_user: Annotated[UserModel, Depends(get_current_active_user_from_request)],
    db: Session = Depends(get_db),
):
    arcan_session = ArcanSession(db)
    response = run_agent(session=arcan_session, user_id=current_user, query=query)
    return {"response": response}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="localhost", port=8000)

# %%
