from datetime import datetime

from pydantic import BaseModel


class ConversationBase(BaseModel):
    message: str
    response: str

class ConversationCreate(ConversationBase):
    pass

class ConversationUpdate(ConversationBase):
    pass

class Conversation(ConversationBase):
    id: int
    user_id: int
    created_at: datetime

    class Config:
        from_attributes = True
