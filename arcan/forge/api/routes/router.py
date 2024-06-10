from fastapi import APIRouter

from arcan.forge.api.routes import auth, chat_history, conversation, user

base_router = APIRouter()

base_router.include_router(auth.router, tags=["auth"], prefix="/v1")
base_router.include_router(chat_history.router, tags=["chat_history"], prefix="/v1")
base_router.include_router(conversation.router, tags=["conversation"], prefix="/v1")
base_router.include_router(user.router, tags=["user"], prefix="/v1")