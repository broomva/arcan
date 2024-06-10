#%%
import re
from datetime import datetime, timedelta
from typing import Any, Dict

from fastapi import APIRouter, Depends, HTTPException, Request, status
from fastapi.security import OAuth2PasswordRequestForm
from jose import JWTError, jwt
from passlib.context import CryptContext
from sqlalchemy.ext.asyncio import AsyncSession

from arcan.forge.config import settings
from arcan.forge.database.session import session_scope
from arcan.forge.repository.token import TokenRepository
from arcan.forge.repository.user import UserRepository
from arcan.forge.schemas.token import Token
from arcan.forge.schemas.user import User, UserCreate

router = APIRouter()

pwd_context = CryptContext(schemes=["bcrypt"], deprecated="auto")

def hash_password(password: str) -> str:
    return pwd_context.hash(password)

def verify_password(plain_password: str, hashed_password: str) -> bool:
    return pwd_context.verify(plain_password, hashed_password)

def disable_user(self, username):
        user = self.user_repository.get_user(username)
        if user:
            user.disabled = True
            self.user_repository.update_user(user)

def enable_user(self, username):
    user = self.user_repository.get_user(username)
    if user:
        user.disabled = False
        self.user_repository.update_user(user)

def create_access_token(data: dict, expires_delta: timedelta | None = None):
    to_encode = data.copy()
    if expires_delta:
        expire = datetime.utcnow() + expires_delta
    else:
        expire = datetime.utcnow() + timedelta(minutes=15)
    to_encode.update({"exp": expire})
    encoded_jwt = jwt.encode(to_encode, settings.secret_key, algorithm=settings.algorithm)
    return encoded_jwt

@router.post("/register", response_model=User)
async def register_user(user_create: UserCreate, db: AsyncSession = Depends(session_scope)):
    user_repo = UserRepository(db)
    user = User(
        username=user_create.username,
        email=user_create.email,
        full_name=user_create.full_name,
        disabled=False,
        hashed_password=hash_password(user_create.password),
    )
    await user_repo.add_user(user)
    return user

@router.post("/token", response_model=Token)
async def login(form_data: OAuth2PasswordRequestForm = Depends(), db: AsyncSession = Depends(session_scope)):
    user_repo = UserRepository(db)
    user = await user_repo.get_user(form_data.username)
    if not user or not verify_password(form_data.password, user.hashed_password):
        raise HTTPException(status_code=status.HTTP_400_BAD_REQUEST, detail="Incorrect username or password")
    access_token_expires = timedelta(minutes=settings.access_token_expire_minutes)
    access_token = create_access_token(data={"sub": user.username}, expires_delta=access_token_expires)
    return {"access_token": access_token, "token_type": "bearer"}



def fetch_session_from_header(config: Dict[str, Any], req: Request) -> Dict[str, Any]:
    config = config.copy()
    configurable = config.get("configurable", {})

    if "arcanai_api_key" in req.headers:
        if "user_id" in req.headers:
            configurable["user_id"] = req.headers["user_id"]
            configurable["access_token"] = req.headers["arcanai_api_key"]
            config["configurable"] = configurable
            # config["configurable"]["user_id"] = req.headers["user_id"]
            # config["configurable"] = configurable
    else:
        raise HTTPException(401, "No Arcan AI API key provided")
    return config


def _is_valid_identifier(value: str) -> bool:
    """Check if the value is a valid identifier."""
    # Use a regular expression to match the allowed characters
    valid_characters = re.compile(r"^[a-zA-Z0-9-_]+$")
    return bool(valid_characters.match(value))

# %%
