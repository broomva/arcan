from datetime import datetime

from sqlalchemy import Column, DateTime, ForeignKey, Integer, String, Text
from sqlalchemy.orm import relationship

from arcan.forge.database.session import Base


class ChatHistory(Base):
    """
    Represents the chat history for a user_id.

    Attributes:
        id (int): The unique identifier of the chat history.
        user_id (str): The user_id of the chat.
        history (str): The chat history.
        updated_at (datetime): The timestamp of when the chat history was last updated.
    """
    __tablename__ = "chat_history"

    id = Column(Integer, primary_key=True, index=True)
    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)
    history = Column(Text, nullable=False)
    updated_at = Column(DateTime, default=datetime.utcnow)

    user = relationship("User", back_populates="chat_history")
