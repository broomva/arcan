# %%
import os
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Annotated, Any, Callable, Dict, List, Optional, Union

from dotenv import load_dotenv
from fastapi import Depends, FastAPI, Form, Header, HTTPException, Request, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse

# %%
from fastapi.security import (
    HTTPAuthorizationCredentials,
    HTTPBearer,
    OAuth2PasswordBearer,
    OAuth2PasswordRequestForm,
)
from langchain_community.chat_message_histories import FileChatMessageHistory
from langchain_core import __version__
from langchain_core.chat_history import BaseChatMessageHistory
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langchain_core.output_parsers import StrOutputParser
from langchain_core.prompts import ChatPromptTemplate, MessagesPlaceholder
from langchain_core.runnables import ConfigurableField, ConfigurableFieldSpec
from langchain_core.runnables.history import RunnableWithMessageHistory
from langchain_openai import ChatOpenAI
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field
from pydantic import BaseModel
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.orm import Session
from typing_extensions import Annotated, TypedDict

from arcan.ai.agents import ArcanAgent
from arcan.ai.llm import LLM
from arcan.api.auth import fetch_session_from_header
from arcan.datamodel.engine import session_scope  # , session_scope_context
from arcan.datamodel.user import (
    ACCESS_TOKEN_EXPIRE_MINUTES,
    TokenModel,
    UserModel,
    UserRepository,
    UserService,
    oauth2_scheme,
    pwd_context,
)

# from arcan.spells.vector_search import (get_per_user_retriever,
#                                         per_req_config_modifier, pgVectorStore)

# %%
MIN_VERSION_LANGCHAIN_CORE = (0, 1, 0)

# Split the version string by "." and convert to integers
LANGCHAIN_CORE_VERSION = tuple(map(int, __version__.split(".")))

if LANGCHAIN_CORE_VERSION < MIN_VERSION_LANGCHAIN_CORE:
    raise RuntimeError(
        f"Minimum required version of langchain-core is {MIN_VERSION_LANGCHAIN_CORE}, "
        f"but found {LANGCHAIN_CORE_VERSION}"
    )


# %%
auth_scheme = HTTPBearer()

load_dotenv()

ENVIRONMENT = os.environ.get("ENVIRONMENT")
ARCANAI_API_TOKEN = os.environ.get("ARCANAI_API_TOKEN")

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
async def chat(
    user_id: str,
    query: str,
    # current_user: Annotated[UserModel, Depends(get_current_active_user_from_request)],
    db: Session = Depends(session_scope),
):
    if ENVIRONMENT == "cloud":
        # from arcan.api.session import ArcanSession, run_agent
        agent = ArcanAgent(user_id=user_id)
        # user = await get_current_active_user_from_request(request=Request)
        response = agent.invoke({"input": query})
    elif ENVIRONMENT == "local":
        agent = ArcanAgent(
            user_id=user_id,
        )
        response = agent.invoke({"input": query, "chat_history": []})
    return {"response": response}


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

add_routes(
    app,
    LLM(provider="ChatOpenAI").llm,
    path="/openai",
    per_req_config_modifier=fetch_session_from_header,
)


add_routes(
    app,
    LLM(provider="ChatGroq").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/groq",
)

add_routes(
    app,
    LLM(provider="ChatTogetherAI").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/together",
)

add_routes(
    app,
    runnable=LLM(provider="ChatOllama").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/ollama",
)


@app.post("/token")
async def login_for_access_token(
    form_data: Annotated[OAuth2PasswordRequestForm, Depends()],
    session: Session = Depends(session_scope),
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
        access_token=access_token,
        token_type="bearer",
        user_id=user.username,
        user=user,
    )


async def get_current_active_user_from_request(
    request: Request, session: Session = Depends(session_scope)
) -> UserModel:
    """Get the current active user from the request."""
    user_repo = UserRepository(session)
    user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
    token = await oauth2_scheme(request)
    print(token)
    user = user_interface.get_current_user(token=token)
    if not user:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid authentication credentials",
            headers={"WWW-Authenticate": "Bearer"},
        )
    # if user.disabled:
    # raise HTTPException(status_code=400, detail="Inactive user")
    return user


# @app.get("/users/me/", response_model=UserModel)
# async def read_users_me(
#     current_user: Annotated[UserModel, Depends(get_current_active_user_from_request)],
# ):
#     return current_user


# add_routes(
#     app,
#     get_per_user_retriever(vectorstore=pgVectorStore().get_vector_store()),
#     per_req_config_modifier=per_req_config_modifier,
#     enabled_endpoints=["invoke"],
# )

# %%

# def create_session_factory(
#     base_dir: Union[str, Path],
# ) -> Callable[[str], BaseChatMessageHistory]:
#     """Create a factory that can retrieve chat histories.

#     The chat histories are keyed by user ID and conversation ID.

#     Args:
#         base_dir: Base directory to use for storing the chat histories.

#     Returns:
#         A factory that can retrieve chat histories keyed by user ID and conversation ID.
#     """
#     base_dir_ = Path(base_dir) if isinstance(base_dir, str) else base_dir
#     if not base_dir_.exists():
#         base_dir_.mkdir(parents=True)

#     def get_chat_history(user_id: str, conversation_id: str) -> FileChatMessageHistory:
#         """Get a chat history from a user id and conversation id."""
#         if not _is_valid_identifier(user_id):
#             raise ValueError(
#                 f"User ID {user_id} is not in a valid format. "
#                 "User ID must only contain alphanumeric characters, "
#                 "hyphens, and underscores."
#                 "Please include a valid cookie in the request headers called 'user-id'."
#             )
#         if not _is_valid_identifier(conversation_id):
#             raise ValueError(
#                 f"Conversation ID {conversation_id} is not in a valid format. "
#                 "Conversation ID must only contain alphanumeric characters, "
#                 "hyphens, and underscores. Please provide a valid conversation id "
#                 "via config. For example, "
#                 "chain.invoke(.., {'configurable': {'conversation_id': '123'}})"
#             )

#         user_dir = base_dir_ / user_id
#         if not user_dir.exists():
#             user_dir.mkdir(parents=True)
#         file_path = user_dir / f"{conversation_id}.json"
#         return FileChatMessageHistory(str(file_path))

#     return get_chat_history


# def _per_request_config_modifier(
#     config: Dict[str, Any], request: Request
# ) -> Dict[str, Any]:
#     """Update the config"""
#     config = config.copy()
#     configurable = config.get("configurable", {})
#     # Look for a cookie named "user_id"
#     user_id = request.cookies.get("user_id", None)

#     if user_id is None:
#         raise HTTPException(
#             status_code=400,
#             detail="No user id found. Please set a cookie named 'user_id'.",
#         )

#     configurable["user_id"] = user_id
#     config["configurable"] = configurable
#     return config


# # Declare a chain
# prompt = ChatPromptTemplate.from_messages(
#     [
#         ("system", "You're an assistant by the name of Bob."),
#         MessagesPlaceholder(variable_name="history"),
#         ("human", "{human_input}"),
#     ]
# )

# chain = prompt | ChatOpenAI()


# class InputChat(TypedDict):
#     """Input for the chat endpoint."""

#     human_input: str
#     """Human input"""


# chain_with_history = RunnableWithMessageHistory(
#     chain,
#     create_session_factory("chat_histories"),
#     input_messages_key="human_input",
#     history_messages_key="history",
#     history_factory_config=[
#         ConfigurableFieldSpec(
#             id="user_id",
#             annotation=str,
#             name="User ID",
#             description="Unique identifier for the user.",
#             default="",
#             is_shared=True,
#         ),
#         ConfigurableFieldSpec(
#             id="conversation_id",
#             annotation=str,
#             name="Conversation ID",
#             description="Unique identifier for the conversation.",
#             default="",
#             is_shared=True,
#         ),
#     ],
# ).with_types(input_type=InputChat)


# add_routes(
#     app,
#     chain_with_history,
#     per_req_config_modifier=_per_request_config_modifier,
#     # Disable playground and batch
#     # 1) Playground we're passing information via headers, which is not supported via
#     #    the playground right now.
#     # 2) Disable batch to avoid users being confused. Batch will work fine
#     #    as long as users invoke it with multiple configs appropriately, but
#     #    without validation users are likely going to forget to do that.
#     #    In addition, there's likely little sense in support batch for a chatbot.
#     disabled_endpoints=["playground", "batch"],
#     path="/chain_with_history",
# )


# def _per_request_session_modifier(
#     config: Dict[str, Any], request: Request
# ) -> Dict[str, Any]:
#     """Update the config"""
#     config = config.copy()
#     configurable = config.get("configurable", {})
#     # Look for a cookie named "user_id"
#     user_id = request.cookies.get("user_id", None)

#     if user_id is None:
#         raise HTTPException(
#             status_code=400,
#             detail="No user id found. Please set a cookie named 'user_id'.",
#         )

#     agent = ArcanAgent(user_id=user_id)

#     configurable["user_id"] = user_id
#     config["configurable"] = configurable
#     return config, agent

# add_routes(
#     app,
#     ArcanAgent(),
#     path="/auth_spells",
#     per_req_config_modifier=_per_request_session_modifier,
#     # Disable playground and batch
#     # 1) Playground we're passing information via headers, which is not supported via
#     #    the playground right now.
#     # 2) Disable batch to avoid users being confused. Batch will work fine
#     #    as long as users invoke it with multiple configs appropriately, but
#     #    without validation users are likely going to forget to do that.
#     #    In addition, there's likely little sense in support batch for a chatbot.
#     disabled_endpoints=["playground", "batch"],
# )

# %%

if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="localhost", port=8000)

# %%
