from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.conversation import Conversation


class ConversationRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_conversation(self, conversation: Conversation):
        self.session.add(conversation)
        await self.session.commit()

    async def get_conversation(self, user_id: int):
        result = await self.session.execute(select(Conversation).filter_by(user_id=user_id))
        return result.scalars().all()

    async def update_conversation(self, conversation: Conversation):
        await self.session.commit()

    async def delete_conversation(self, conversation_id: int):
        conversation = await self.session.get(Conversation, conversation_id)
        if conversation:
            await self.session.delete(conversation)
            await self.session.commit()
