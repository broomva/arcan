from datetime import datetime

from sqlalchemy import Column, DateTime, Integer, String

from arcan.api.datamodel import Base, engine

Base.metadata.create_all(engine)


class Conversation(Base):
    """
    Represents a conversation entity.

    Attributes:
        id (int): The unique identifier of the conversation.
        sender (str): The sender of the message.
        message (str): The message content.
        response (str): The response to the message.
        created_at (datetime): The timestamp of when the conversation was created.
    """

    __tablename__ = "conversation"
    id = Column(Integer, primary_key=True, index=True)
    sender = Column(String)
    message = Column(String)
    response = Column(String)
    created_at = Column(DateTime, default=datetime.utcnow)
