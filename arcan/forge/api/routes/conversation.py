from typing import List

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.api.routes.auth import pwd_context
from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User
from arcan.forge.repository.conversation import ConversationRepository
from arcan.forge.schemas.conversation import (Conversation, ConversationCreate,
                                              ConversationUpdate)
from arcan.forge.service.user import UserService

router = APIRouter()


@router.post("/conversation/", response_model=Conversation)
async def create_conversation(conversation: ConversationCreate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    conversation_data = Conversation(
        message=conversation.message,
        response=conversation.response,
        user_id=current_user.id,
    )
    await conversation_repo.add_conversation(conversation_data)
    return conversation_data

@router.get("/conversation/", response_model=List[Conversation])
async def get_conversation(db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    conversation = await conversation_repo.get_conversation(current_user.id)
    return conversation

@router.put("/conversation/{conversation_id}", response_model=Conversation)
async def update_conversation(conversation_id: int, conversation: ConversationUpdate, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    existing_conversation = await conversation_repo.get_conversation(conversation_id)
    if not existing_conversation:
        raise HTTPException(status_code=404, detail="Conversation not found")
    if existing_conversation.user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Not authorized to update this conversation")
    existing_conversation.message = conversation.message
    existing_conversation.response = conversation.response
    await conversation_repo.update_conversation(existing_conversation)
    return existing_conversation

@router.delete("/conversation/{conversation_id}", response_model=Conversation)
async def delete_conversation(conversation_id: int, db: AsyncSession = Depends(session_scope), current_user: User = Depends(UserService.get_current_active_user)):
    conversation_repo = ConversationRepository(db)
    existing_conversation = await conversation_repo.get_conversation(conversation_id)
    if not existing_conversation:
        raise HTTPException(status_code=404, detail="Conversation not found")
    if existing_conversation.user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Not authorized to delete this conversation")
    await conversation_repo.delete_conversation(conversation_id)
    return {"detail": "Conversation deleted"}
