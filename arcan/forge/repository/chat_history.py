from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.chat_history import ChatHistory


class ChatHistoryRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_chat_history(self, chat_history: ChatHistory):
        self.session.add(chat_history)
        await self.session.commit()

    async def get_chat_history(self, user_id: int) -> ChatHistory:
        result = await self.session.execute(select(ChatHistory).filter_by(user_id=user_id))
        return result.scalar_one_or_none()

    async def update_chat_history(self, chat_history: ChatHistory):
        await self.session.commit()

    async def delete_chat_history(self, user_id: int):
        chat_history = await self.get_chat_history(user_id)
        if chat_history:
            await self.session.delete(chat_history)
            await self.session.commit()
