from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import pwd_context, session_scope
from arcan.forge.models.user import User


class UserRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_user(self, user: User):
        async with self.session as session:
            session.add(user)
            await session.commit()

    async def get_user(self, username: str) -> User:
        async with self.session as session:
            result = await session.execute(select(User).filter_by(username=username))
            return result.scalar_one_or_none()

    # async def update_user(self, user: User):
    #     async with self.session as session:
    #         session.add(user)
    #     await self.session.commit()

    async def delete_user(self, username: str):
        async with self.session as session:
            user = await self.get_user(username)
            if user:
                await session.delete(user)
                await session.commit()
    
    async def rehash_passwords(self):
        async with self.session as session:
            users = session.query(User).all()
            for user in users:
                if not pwd_context.identify(user.hashed_password):  # Identify if the hash is not argon2
                    user.hashed_password = pwd_context.hash(user.hashed_password)
                    session.add(user)
            session.commit()
