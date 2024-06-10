from datetime import datetime

from pydantic import BaseModel


class UserBase(BaseModel):
    username: str
    email: str | None = None
    full_name: str | None = None
    status: str | None = None
    disabled: bool | None = None

class UserCreate(UserBase):
    password: str

class User(UserBase):
    id: int
    created_at: datetime

    class Config:
        from_attributes = True
