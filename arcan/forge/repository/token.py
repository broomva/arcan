from fastapi import Depends
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.future import select

from arcan.forge.database.session import session_scope
from arcan.forge.models.token import Token


class TokenRepository:
    def __init__(self, session: AsyncSession = Depends(session_scope)):
        self.session = session

    async def add_token(self, token: Token):
        self.session.add(token)
        await self.session.commit()
        return token

    async def get_token(self, token_str: str) -> Token:
        result = await self.session.execute(select(Token).filter(Token.access_token == token_str))
        token = result.scalars().one_or_none()
        return token
