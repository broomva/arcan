
import os

from dotenv import load_dotenv
from fastapi import APIRouter
from langchain_core import __version__
from langserve import add_routes

from arcan.casters.ai.agents import ArcanAgent
from arcan.casters.ai.llm import LLM
from arcan.forge.api.routes.auth import fetch_session_from_header

router = APIRouter()

load_dotenv()


# %%
MIN_VERSION_LANGCHAIN_CORE = (0, 1, 0)

# Split the version string by "." and convert to integers
LANGCHAIN_CORE_VERSION = tuple(map(int, __version__.split(".")))

if LANGCHAIN_CORE_VERSION < MIN_VERSION_LANGCHAIN_CORE:
    raise RuntimeError(
        f"Minimum required version of langchain-core is {MIN_VERSION_LANGCHAIN_CORE}, "
        f"but found {LANGCHAIN_CORE_VERSION}"
    )


ENVIRONMENT = os.environ.get("ENVIRONMENT")
ARCANAI_API_TOKEN = os.environ.get("ARCANAI_API_TOKEN")


@router.get("/api/check")
async def index():
    return {"message": "Arcan is Running!"}

@router.get("/api/chat")
async def chat(
    user_id: str,
    query: str,
):
    if ENVIRONMENT == "cloud":
        agent = ArcanAgent(user_id=user_id)
        response = agent.invoke({"input": query})
    elif ENVIRONMENT == "local":
        agent = ArcanAgent(
            user_id=user_id,
        )
        response = agent.invoke({"input": query, "chat_history": []})
    return {"response": response}


add_routes(
    router,
    LLM(provider="ChatOpenAI").llm,
    path="/openai",
    per_req_config_modifier=fetch_session_from_header,
)


add_routes(
    router,
    LLM(provider="ChatGroq").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/groq",
)

add_routes(
    router,
    LLM(provider="ChatTogetherAI").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/together",
)

add_routes(
    router,
    runnable=LLM(provider="ChatOllama").llm,
    per_req_config_modifier=fetch_session_from_header,
    path="/ollama",
)