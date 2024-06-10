#%%
import os

from dotenv import load_dotenv
from sqlalchemy.ext.declarative import declarative_base

from arcan.forge.database.session import engine

load_dotenv()
# from arcan.forge.models import Base


Base = declarative_base()

# Create tables
async def create_tables():
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)

# %%
