from datetime import datetime

from pydantic import BaseModel


class ChatHistoryBase(BaseModel):
    history: str

class ChatHistoryCreate(ChatHistoryBase):
    pass

class ChatHistoryUpdate(ChatHistoryBase):
    pass

class ChatHistory(ChatHistoryBase):
    id: int
    user_id: int
    updated_at: datetime

    class Config:
        from_attributes = True
