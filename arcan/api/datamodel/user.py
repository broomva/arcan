# %%
import os
from datetime import datetime, timedelta, timezone
from typing import Annotated

from fastapi import Depends, FastAPI, HTTPException, status
from fastapi.security import OAuth2PasswordBearer, OAuth2PasswordRequestForm
from jose import JWTError, jwt
from passlib.context import CryptContext
from pydantic import BaseModel
from sqlalchemy import Boolean, Column, DateTime, ForeignKey, Integer, String, Text
from sqlalchemy.orm import Session, relationship

from arcan.api.datamodel import Base, engine

pwd_context = CryptContext(schemes=["bcrypt"], deprecated="auto")

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="token")

# %%
Base.metadata.create_all(engine)
# %%

# to get a string like this run:
# openssl rand -hex 32
SECRET_KEY = os.environ.get("ARCAN_API_KEY")
ALGORITHM = "HS256"
ACCESS_TOKEN_EXPIRE_MINUTES = 30


class User(Base):
    __tablename__ = "user"
    __table_args__ = {"extend_existing": True}
    username = Column(String, primary_key=True, index=True)
    email = Column(String, nullable=True)
    full_name = Column(String)
    status = Column(String)
    disabled = Column(Boolean)
    created_at = Column(DateTime, default=datetime.utcnow)
    hashed_password = Column(String)


class Token(Base):
    __tablename__ = "token"
    __table_args__ = {"extend_existing": True}
    id = Column(
        Integer,
        primary_key=True,
        index=True,
    )
    access_token = Column(String)
    token_type = Column(String)
    user_id = Column(String, ForeignKey("user.username"))
    user = relationship("User", back_populates="tokens")


User.tokens = relationship("Token", order_by=Token.id, back_populates="user")
# %%


class UserModel(BaseModel):
    username: str
    email: str | None = None
    full_name: str | None = None
    status: str | None = None
    disabled: bool | None = None
    created_at: datetime | None = None
    hashed_password: str


class TokenModel(BaseModel):
    id: int
    access_token: str
    token_type: str
    user_id: str
    user: UserModel


class TokenData(BaseModel):
    username: str | None = None


class UserInDB(BaseModel):
    hashed_password: str


class UserRepository:
    def __init__(self, session: Session):
        self.session = session

    def add_user(self, user: User):
        with self.session as db_session:
            db_session.add(user)
            db_session.commit()

    def get_user(self, username: str):
        with self.session as db_session:
            return db_session.query(User).filter_by(username=username).first()

    def update_user(self, user: User):
        with self.session as db_session:
            db_session.commit()

    def delete_user(self, username: str):
        with self.session as db_session:
            user = self.get_user(username)
            if user:
                db_session.delete(user)
                db_session.commit()


class UserService:
    def __init__(self, user_repository: UserRepository, pwd_context: CryptContext):
        self.user_repository = user_repository
        self.pwd_context = pwd_context

    def authenticate_user(self, username, password):
        user = self.user_repository.get_user(username)
        if not user:
            return False
        if not self.verify_password(password, user.hashed_password):
            return False
        return user

    def register_user(self, username, email, full_name, password):
        user = User(
            username=username,
            email=email,
            full_name=full_name,
            disabled=False,
            hashed_password=self.hash_password(password),
        )
        self.user_repository.add_user(user)

    def hash_password(self, password):
        return self.pwd_context.hash(password)

    def verify_password(self, plain_password, hashed_password):
        return self.pwd_context.verify(plain_password, hashed_password)

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

    def create_access_token(self, data: dict, expires_delta: timedelta | None = None):
        to_encode = data.copy()
        if expires_delta:
            expire = datetime.now(timezone.utc) + expires_delta
        else:
            expire = datetime.now(timezone.utc) + timedelta(minutes=15)
        to_encode.update({"exp": expire})
        encoded_jwt = jwt.encode(to_encode, SECRET_KEY, algorithm=ALGORITHM)
        return encoded_jwt

    async def get_current_user(
        self, token: Annotated[str, Depends(oauth2_scheme)]
    ) -> str:
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
        user = self.user_repository.get_user(username)
        if user is None:
            raise credentials_exception
        return user

    async def get_current_active_user(
        self,
        current_user: Annotated[User, Depends(get_current_user)],
    ):
        if current_user.disabled:
            raise HTTPException(status_code=400, detail="Inactive user")
        return current_user


# %%
