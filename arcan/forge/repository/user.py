# Rpository Patterns to interact with the User model
from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.user import User


class UserRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_user(self, user: User):
        self.session.add(user)
        await self.session.commit()

    async def get_user(self, username: str) -> User:
        result = await self.session.execute(select(User).filter_by(username=username))
        return result.scalar_one_or_none()

    async def update_user(self, user: User):
        await self.session.commit()

    async def delete_user(self, username: str):
        user = await self.get_user(username)
        if user:
            await self.session.delete(user)
            await self.session.commit()