import os
from functools import wraps

from fastapi import HTTPException, Request, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer

security = HTTPBearer()


def requires_auth(func):
    @wraps(func)
    def wrapper(*args, token: HTTPAuthorizationCredentials = security, **kwargs):
        if token.credentials != os.environ["ARCAN_API_KEY"]:
            raise HTTPException(
                status_code=status.HTTP_401_UNAUTHORIZED,
                detail="Incorrect bearer token",
                headers={"WWW-Authenticate": "Bearer"},
            )

        return func(*args, **kwargs)

    return wrapper


def aio_requires_auth(func):
    @wraps(func)
    async def wrapper(*args, token: HTTPAuthorizationCredentials = None, **kwargs):
        if token is None or token.credentials != os.environ["ARCAN_API_KEY"]:
            raise HTTPException(
                status_code=status.HTTP_401_UNAUTHORIZED,
                detail="Incorrect bearer token",
                headers={"WWW-Authenticate": "Bearer"},
            )

        return await func(*args, **kwargs)

    return wrapper


def log_endpoint(func):
    @wraps(func)
    def wrapper(request: Request, *args, **kwargs):
        client_host = request.client.host
        client_user_agent = request.headers.get("user-agent")
        print(
            f"Endpoint hit with query: {kwargs['query']}, context_url: {kwargs['context_url']}, client_host: {client_host}, client_user_agent: {client_user_agent}"
        )
        return func(request, *args, **kwargs)

    return wrapper
