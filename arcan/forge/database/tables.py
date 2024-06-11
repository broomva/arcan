# database/tables.py
from arcan.forge.database.session import engine
from arcan.forge.models import Base


async def create_tables():
    async with engine.begin() as conn:
        print("Creating tables")
        print(Base.metadata.create_all)
        await conn.run_sync(Base.metadata.create_all)
