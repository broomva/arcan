from typing import List

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User
from arcan.forge.repository.chat_history import ChatHistoryRepository
from arcan.forge.schemas.chat_history import (ChatHistory, ChatHistoryCreate,
                                              ChatHistoryUpdate)
from arcan.forge.service.user import UserService

router = APIRouter()

@router.post("/chat_history/", response_model=ChatHistory)
async def create_chat_history(chat_history: ChatHistoryCreate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    chat_history_data = ChatHistory(
        history=chat_history.history,
        user_id=current_user.id,
    )
    await chat_history_repo.add_chat_history(chat_history_data)
    return chat_history_data

@router.get("/chat_history/", response_model=ChatHistory)
async def get_chat_history(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    chat_history = await chat_history_repo.get_chat_history(current_user.id)
    if not chat_history:
        raise HTTPException(status_code=404, detail="Chat history not found")
    return chat_history

@router.put("/chat_history/", response_model=ChatHistory)
async def update_chat_history(chat_history: ChatHistoryUpdate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    existing_chat_history = await chat_history_repo.get_chat_history(current_user.id)
    if not existing_chat_history:
        raise HTTPException(status_code=404, detail="Chat history not found")
    existing_chat_history.history = chat_history.history
    await chat_history_repo.update_chat_history(existing_chat_history)
    return existing_chat_history

@router.delete("/chat_history/", response_model=ChatHistory)
async def delete_chat_history(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    chat_history_repo = ChatHistoryRepository(db)
    await chat_history_repo.delete_chat_history(current_user.id)
    return {"detail": "Chat history deleted"}
