import os
from datetime import datetime, timedelta, timezone

from fastapi import Depends, HTTPException, status
from fastapi.security import OAuth2PasswordBearer
from jose import JWTError, jwt
from passlib.context import CryptContext

from arcan.forge.database.session import pwd_context
from arcan.forge.models.user import User
from arcan.forge.repository.user import UserRepository
from arcan.forge.schemas.token import TokenData
from arcan.forge.schemas.user import UserCreate

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="token")

SECRET_KEY = os.environ.get("ARCANAI_API_KEY")
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 30

class UserService:
    def __init__(self, user_repository: UserRepository, pwd_context: CryptContext = pwd_context):
        self.user_repository = user_repository
        self.pwd_context = pwd_context

    async def authenticate_user(self, username, password):
        user = await self.user_repository.get_user(username)
        if not user:
            return False
        if not await self.verify_password(password, user.hashed_password):
            return False
        return user

    async def register_user(self, user_create: UserCreate):
        user = User(
            username=user_create.username,
            email=user_create.email,
            full_name=user_create.full_name,
            disabled=True,
            hashed_password= await self.hash_password(user_create.password),
            created_at=datetime.now(),
        )
        await self.user_repository.add_user(user)
        return user

    async def hash_password(self, password):
        return self.pwd_context.hash(password)

    async def verify_password(self, plain_password, hashed_password):
        return self.pwd_context.verify(plain_password, hashed_password)

    async def disable_user(self, username):
        user = await self.user_repository.get_user(username)
        if user:
            user.disabled = True
            await self.user_repository.update_user(user)

    async def enable_user(self, username):
        user = await self.user_repository.get_user(username)
        if user:
            user.disabled = False
            await self.user_repository.update_user(user)

    async def create_access_token(self, data: dict, expires_delta: timedelta | None = None):
        to_encode = data.copy()
        if expires_delta:
            expire = datetime.now(timezone.utc) + expires_delta
        else:
            expire = datetime.now(timezone.utc) + timedelta(minutes=15)
        to_encode.update({"exp": expire})
        encoded_jwt = jwt.encode(to_encode, SECRET_KEY, algorithm=ALGORITHM)
        return encoded_jwt

    async def get_current_user(self, token: str = Depends(oauth2_scheme)):
        credentials_exception = HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Could not validate credentials",
            headers={"WWW-Authenticate": "Bearer"},
        )
        try:
            payload = jwt.decode(token, SECRET_KEY, algorithms=[ALGORITHM])
            username: str = payload.get("sub")
            if username is None:
                raise credentials_exception
            token_data = TokenData(username=username)
        except JWTError:
            raise credentials_exception
        user = await self.user_repository.get_user(username)
        if user is None:
            raise credentials_exception
        return user

    async def get_current_active_user(self, current_user: User = Depends(get_current_user)):
        if current_user.disabled:
            raise HTTPException(status_code=400, detail="Inactive user")
        return current_user
