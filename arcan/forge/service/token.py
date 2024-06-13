# from arcan.forge.models.token import Token
from arcan.forge.repository.token import TokenRepository
from arcan.forge.schemas.token import Token as TokenSchema


class TokenService:
    def __init__(self, token_repository: TokenRepository,):
        self.token_repository = token_repository
    
    async def register_token(self, token_create: TokenSchema):
        return await self.token_repository.add_token(token_create)