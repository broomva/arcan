from arcan.forge.database.session import engine
from arcan.forge.models import Base


# Create tables
async def create_tables():
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.create_all)
