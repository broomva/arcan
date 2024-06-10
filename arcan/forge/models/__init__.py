#%%
from sqlalchemy.ext.declarative import declarative_base

from arcan.forge.models.chat_history import ChatHistory
from arcan.forge.models.conversation import Conversation
from arcan.forge.models.token import Token
from arcan.forge.models.user import User

# Ensure that all models are registered
__all__ = ["User", "ChatHistory", "Conversation", "Token"]


Base = declarative_base()
# Create all tables
# Base.metadata.create_all(engine)

# %%
