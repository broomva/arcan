import os
from contextlib import asynccontextmanager
from sqlite3 import DataError, IntegrityError
from typing import Any, Callable

from dotenv import load_dotenv
from fastapi import FastAPI, Request, status
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse, RedirectResponse
# %%
from fastapi.security import HTTPBearer
from langchain_core import __version__
from langchain_core.runnables import ConfigurableField
from langserve import add_routes
from langserve.pydantic_v1 import BaseModel
from loguru import logger
from pydantic import BaseModel

from arcan.casters.ai.agents import ArcanAgent
from arcan.casters.ai.llm import LLM
from arcan.forge.api.routes.auth import fetch_session_from_header
from arcan.forge.api.routes.router import base_router as router
from arcan.forge.core.config import API_PREFIX, DEBUG, PROJECT_NAME, VERSION
from arcan.forge.database.session import sessionmanager
from arcan.forge.database.tables import create_tables
from arcan.forge.exceptions import (ArcanApiError, AuthenticationFailed,
                                    EntityDoesNotExistError,
                                    InvalidOperationError, InvalidTokenError,
                                    ServiceError)

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




@asynccontextmanager
async def lifespan(_app: FastAPI):
    """
    Function that handles startup and shutdown events.
    To understand more, read https://fastapi.tiangolo.com/advanced/events/
    """
    await create_tables()
    yield
    if sessionmanager.engine is not None:
        await sessionmanager.close()


app = FastAPI(title=PROJECT_NAME, debug=DEBUG, version=VERSION, lifespan=lifespan)
app.include_router(router, prefix=API_PREFIX)

auth_scheme = HTTPBearer()

@app.get("/")
async def redirect_root_to_docs():
    return RedirectResponse("/docs")

# Set all CORS enabled origins
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["*"],
)


def create_exception_handler(
    status_code: int, initial_detail: str
) -> Callable[[Request, ArcanApiError], JSONResponse]:
    detail = {"message": initial_detail}  # Using a dictionary to hold the detail

    async def exception_handler(_: Request, exc: ArcanApiError) -> JSONResponse:
        if exc.message:
            detail["message"] = exc.message

        if exc.name:
            detail["message"] = f"{detail['message']} [{exc.name}]"

        logger.error(exc)
        return JSONResponse(
            status_code=status_code, content={"detail": detail["message"]}
        )

    return exception_handler


app.add_exception_handler(
    exc_class_or_status_code=EntityDoesNotExistError,
    handler=create_exception_handler(
        status.HTTP_404_NOT_FOUND, "Entity does not exist."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=InvalidOperationError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Can't perform the operation."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=IntegrityError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Can't process the request due to integrity error."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=DataError,
    handler=create_exception_handler(
        status.HTTP_400_BAD_REQUEST, "Data can't be processed, check the input."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=AuthenticationFailed,
    handler=create_exception_handler(
        status.HTTP_401_UNAUTHORIZED,
        "Authentication failed due to invalid credentials.",
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=InvalidTokenError,
    handler=create_exception_handler(
        status.HTTP_401_UNAUTHORIZED, "Invalid token, please re-authenticate again."
    ),
)

app.add_exception_handler(
    exc_class_or_status_code=ServiceError,
    handler=create_exception_handler(
        status.HTTP_500_INTERNAL_SERVER_ERROR,
        "A service seems to be down, try again later.",
    ),
)
