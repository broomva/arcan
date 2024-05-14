# %%
import os
from datetime import datetime, timedelta, timezone
from typing import Annotated, Any, Dict, List, Optional, Union

from dotenv import load_dotenv
from fastapi import (Depends, FastAPI, Form, Header, HTTPException, Request,
                     status)
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import RedirectResponse
# %%
from fastapi.security import (HTTPAuthorizationCredentials, HTTPBearer,
                              OAuth2PasswordBearer, OAuth2PasswordRequestForm)
from langchain_core.messages import AIMessage, FunctionMessage, HumanMessage
from langchain_core.output_parsers import StrOutputParser
from langchain_core.prompts import ChatPromptTemplate
from langchain_core.runnables import ConfigurableField
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel, Field
from pydantic import BaseModel
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.orm import Session
from typing_extensions import Annotated

#%%
from arcan.ai.agents import ArcanAgent
from arcan.ai.llm import LLM
from arcan.api.datamodel.engine import session_scope  # , session_scope_context

# from arcan.api.datamodel.chat_history import ChatHistory
# from arcan.api.datamodel.conversation import Conversation
# from arcan.api.datamodel.user import (ACCESS_TOKEN_EXPIRE_MINUTES, TokenModel,
#                                       User, UserInDB, UserModel,
#                                       UserRepository, UserService,
#                                       oauth2_scheme, pwd_context)


# from arcan.api.session.auth import requires_auth
# from arcan.spells.vector_search import (get_per_user_retriever,
#                                         per_req_config_modifier, pgVectorStore)

#%%
auth_scheme = HTTPBearer()

load_dotenv()

ENVIRONMENT = os.environ.get("ENVIRONMENT")
ARCAN_API_TOKEN = os.environ.get("ARCAN_API_TOKEN")

# async def verify_token(x_token: Annotated[str, Header()]) -> None:
#     """Verify the token is valid."""
#     # Replace this with your actual authentication logic
#     if x_token != ARCAN_API_TOKEN:
#         raise HTTPException(status_code=400, detail="X-Token header invalid")


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
        from arcan.api.session import ArcanSession, run_agent
        arcan_session = ArcanSession(db)
        # user = await get_current_active_user_from_request(request=Request)
        response = run_agent(session=arcan_session, user_id=user_id, query=query)
    elif ENVIRONMENT == "local":
        agent = ArcanSpellsAgent(
                    user_id=user_id,
                )
        response = agent.get_response(user_content=query)
    return {"response": response}


# %%
class Input(BaseModel):
    input: str
    chat_history: List[Union[HumanMessage, AIMessage, FunctionMessage]] = Field(
        ...,
        extra={"widget": {"type": "chat", "input": "input", "output": "output"}},
    )


class Output(BaseModel):
    output: Any


# add_routes(
#     app=app,
#     runnable=ArcanSpellsAgent(
#         # database=SQLDatabase.from_uri(os.environ.get("SQLALCHEMY_URL"))
#     )
#     .agent_executor.with_types(input_type=Input, output_type=Output)
#     .with_config({"run_name": "agent"}),
#     path="/spells_agent",
#     enable_feedback_endpoint=True,
# )

# def fetch_api_key_from_header(config: Dict[str, Any], req: Request) -> Dict[str, Any]:
#     if "x-api-key" in req.headers:
#         config["configurable"]["openai_api_key"] = req.headers["x-api-key"]
#         config['configurable']['user_id'] = req.headers["user_id"]
#     else:
#         raise HTTPException(401, "No API key provided")

#     return config

# dynamic_auth_model = LLM(provider="ChatOpenAI", openai_api_key="placeholder").configurable_fields(
#     openai_api_key=ConfigurableField(
#         id="openai_api_key",
#         name="OpenAI API Key",
#         description=("API Key for OpenAI interactions"),
#     ),
# )




# dynamic_auth_model = ArcanAgent(user_id="placeholder").configurable_fields(
#     user_id=ConfigurableField(
#         id="user_id",
#         name="Arcan AI User ID",
#         description=("user_id Key for Arcan AI interactions"),
#     ),
# )

add_routes(
    app=app,
    runnable=ArcanAgent(),
    path="/spells",
)

add_routes(
    app,
    LLM(provider="ChatOpenAI").llm,
    path="/openai",
    # per_req_config_modifier=fetch_api_key_from_header,
)

add_routes(
    app,
    LLM(provider="ChatGroq").llm,
    # per_req_config_modifier=per_req_config_modifier,
    path="/groq",
)

add_routes(
    app,
    LLM(provider="ChatTogetherAI").llm,
    path="/together",
)

add_routes(
    app,
    runnable=LLM(provider="ChatOllama").llm,
    path="/ollama",
)


# @app.post("/token")
# async def login_for_access_token(
#     form_data: Annotated[OAuth2PasswordRequestForm, Depends()],
#     session: Session = Depends(session_scope),
# ) -> TokenModel:
#     user_repo = UserRepository(session)
#     user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
#     user = user_interface.authenticate_user(form_data.username, form_data.password)
#     if not user:
#         raise HTTPException(
#             status_code=status.HTTP_401_UNAUTHORIZED,
#             detail="Incorrect username or password",
#             headers={"WWW-Authenticate": "Bearer"},
#         )
#     access_token_expires = timedelta(minutes=ACCESS_TOKEN_EXPIRE_MINUTES)
#     access_token = user_interface.create_access_token(
#         data={"sub": user.username}, expires_delta=access_token_expires
#     )
#     return TokenModel(
#         id=1,
#         access_token=access_token,
#         token_type="bearer",
#         user_id=user.username,
#         user=user,
#     )


# async def get_current_active_user_from_request(
#     request: Request, session: Session = Depends(session_scope)
# ) -> UserModel:
#     """Get the current active user from the request."""
#     user_repo = UserRepository(session)
#     user_interface = UserService(user_repository=user_repo, pwd_context=pwd_context)
#     token = await oauth2_scheme(request)
#     print(token)
#     user = user_interface.get_current_user(token=token)
#     if not user:
#         raise HTTPException(
#             status_code=status.HTTP_401_UNAUTHORIZED,
#             detail="Invalid authentication credentials",
#             headers={"WWW-Authenticate": "Bearer"},
#         )
#     if user.disabled:
#         raise HTTPException(status_code=400, detail="Inactive user")
#     return user


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

if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="localhost", port=8000)

# %%
